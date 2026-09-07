//! `timeout`: run a command with a time limit. A port of GNU coreutils
//! timeout(1) for macOS, matching its options, messages, signal handling
//! and exit statuses.

use agent_cli_tools::duration::{Timeout, parse_duration};
use agent_cli_tools::options::{Arg, Item, Mode, Opt, Parser};
use agent_cli_tools::signals::{parse_signal, signal_name};
use std::ffi::{CStr, CString, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::process;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

const EXIT_TIMEDOUT: i32 = 124;
const EXIT_CANCELED: i32 = 125;
const EXIT_CANNOT_INVOKE: i32 = 126;
const EXIT_ENOENT: i32 = 127;

/// Signals GNU forwards to the command (term-sig.h, Darwin subset), unless
/// they were already ignored when timeout started.
const FORWARDED: &[i32] = &[
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGHUP,
    libc::SIGTERM,
    libc::SIGPIPE,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGILL,
    libc::SIGTRAP,
    libc::SIGABRT,
    libc::SIGBUS,
    libc::SIGFPE,
    libc::SIGSEGV,
    libc::SIGXCPU,
    libc::SIGXFSZ,
    libc::SIGSYS,
    libc::SIGVTALRM,
    libc::SIGPROF,
    libc::SIGEMT,
];

/// Write end of the self-pipe the signal handler reports into.
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

const HELP: &str = "\
Usage: timeout [OPTION]... DURATION COMMAND [ARG]...
Start COMMAND, and kill it if still running after DURATION.

Mandatory arguments to long options are mandatory for short options too.
  -f, --foreground
         when not running timeout directly from a shell prompt,
         allow COMMAND to read from the TTY and get TTY signals;
         in this mode, children of COMMAND will not be timed out
  -k, --kill-after=DURATION
         also send a KILL signal if COMMAND is still running
         this long after the initial signal was sent
  -p, --preserve-status
         exit with the same status as COMMAND,
         even when the command times out
  -s, --signal=SIGNAL
         specify the signal to be sent on timeout;
         SIGNAL may be a name like 'HUP' or a number;
         see 'kill -l' for a list of signals
  -v, --verbose
         diagnose to standard error any signal sent upon timeout
      --help
         display this help and exit
      --version
         output version information and exit

DURATION is a floating point number with an optional suffix:
's' for seconds (the default), 'm' for minutes, 'h' for hours or 'd' for days.
A duration of 0 disables the associated timeout.

Upon timeout, send the TERM signal to COMMAND, if no other SIGNAL specified.
The TERM signal kills any process that does not block or catch that signal.
It may be necessary to use the KILL signal, since this signal can't be caught.

Exit status:
  124  if COMMAND times out, and --preserve-status is not specified
  125  if the timeout command itself fails
  126  if COMMAND is found but cannot be invoked
  127  if COMMAND cannot be found
  137  if COMMAND (or timeout itself) is sent the KILL (9) signal (128+9)
  -    the exit status of COMMAND otherwise

Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>
Compatible with timeout from GNU coreutils 9.11.
";

/// The options this command recognizes, handed to the shared parser.
const TIMEOUT_OPTS: &[Opt] = &[
    Opt {
        short: Some('f'),
        long: "foreground",
        arg: Arg::None,
    },
    Opt {
        short: Some('k'),
        long: "kill-after",
        arg: Arg::Required,
    },
    Opt {
        short: Some('p'),
        long: "preserve-status",
        arg: Arg::None,
    },
    Opt {
        short: Some('s'),
        long: "signal",
        arg: Arg::Required,
    },
    Opt {
        short: Some('v'),
        long: "verbose",
        arg: Arg::None,
    },
    Opt {
        short: None,
        long: "help",
        arg: Arg::None,
    },
    Opt {
        short: None,
        long: "version",
        arg: Arg::None,
    },
];

struct Options {
    foreground: bool,
    kill_after: Timeout,
    preserve_status: bool,
    term_signal: i32,
    verbose: bool,
    timeout: Timeout,
    command: Vec<OsString>,
}

/// Print a diagnostic the way GNU `error()` does: program name, colon, text.
fn diagnose(message: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "timeout: {message}");
    let _ = err.flush();
}

/// A usage error: the diagnostic, the "Try" line, exit 125.
fn usage_error(message: &str) -> ! {
    agent_cli_tools::options::usage_error("timeout", message, EXIT_CANCELED)
}

fn try_help() -> ! {
    eprintln!("Try 'timeout --help' for more information.");
    process::exit(EXIT_CANCELED)
}

fn parse_args(args: &[OsString]) -> Options {
    let mut opts = Options {
        foreground: false,
        kill_after: Timeout::Never,
        preserve_status: false,
        term_signal: libc::SIGTERM,
        verbose: false,
        timeout: Timeout::Never,
        command: Vec::new(),
    };
    // parse_partial, not parse_or_exit: an earlier `--help` or `--version`
    // must act as soon as it is reached, even if a later option on the
    // same line is invalid, the way GNU getopt itself behaves. Only once
    // every item is walked does a pending parse error get reported.
    let (items, error) =
        Parser::new("timeout", TIMEOUT_OPTS, Mode::StopAtFirstOperand).parse_partial(args);
    let mut operands = Vec::new();
    for item in items {
        match item {
            Item::Flag { long, value } => {
                let text = value
                    .as_deref()
                    .map_or(String::new(), |v| v.to_string_lossy().into_owned());
                apply(&mut opts, long, &text);
            }
            Item::Operand(operand) => operands.push(operand),
        }
    }
    if let Some(message) = error {
        usage_error(&message);
    }
    if operands.len() < 2 {
        try_help();
    }
    let mut operands = operands.into_iter();
    let duration_arg = operands.next().expect("checked len above");
    let duration = duration_arg.to_string_lossy();
    opts.timeout = parse_duration(&duration)
        .unwrap_or_else(|| usage_error(&format!("invalid time interval '{duration}'")));
    opts.command = operands.collect();
    opts
}

fn apply(opts: &mut Options, name: &str, value: &str) {
    match name {
        "foreground" => opts.foreground = true,
        "preserve-status" => opts.preserve_status = true,
        "verbose" => opts.verbose = true,
        "kill-after" => {
            opts.kill_after = parse_duration(value)
                .unwrap_or_else(|| usage_error(&format!("invalid time interval '{value}'")));
        }
        "signal" => {
            opts.term_signal = parse_signal(value)
                .unwrap_or_else(|| usage_error(&format!("'{value}': invalid signal")));
        }
        "help" => {
            print!("{HELP}");
            process::exit(0);
        }
        "version" => {
            print!("{}", agent_cli_tools::version_text("timeout"));
            process::exit(0);
        }
        _ => unreachable!("unknown option name {name}"),
    }
}

/// Async-signal-safe: one byte per signal into the self-pipe. A full pipe
/// means the main loop is dozens of signals behind; dropping one is fine.
extern "C" fn on_signal(signal: libc::c_int) {
    let fd = SIGNAL_PIPE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = signal as u8;
        unsafe { libc::write(fd, (&byte as *const u8).cast(), 1) };
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn strerror(code: i32) -> String {
    unsafe { CStr::from_ptr(libc::strerror(code)) }
        .to_string_lossy()
        .into_owned()
}

fn self_pipe() -> (i32, i32) {
    let mut fds = [0i32; 2];
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            diagnose(&format!("pipe: {}", strerror(errno())));
            process::exit(EXIT_CANCELED);
        }
        for fd in fds {
            libc::fcntl(
                fd,
                libc::F_SETFL,
                libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK,
            );
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    (fds[0], fds[1])
}

/// Install `on_signal` for `signal`. Unless `force`, a signal the parent
/// shell already set to SIG_IGN stays ignored, as GNU leaves it.
fn install(signal: i32, force: bool) {
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        if !force
            && (libc::sigaction(signal, std::ptr::null(), &mut old) != 0
                || old.sa_sigaction == libc::SIG_IGN)
        {
            return;
        }
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = libc::SA_RESTART;
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

fn unblock(signal: i32) {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

fn install_handlers(term_signal: i32) {
    install(libc::SIGALRM, true);
    if term_signal != 0 {
        install(term_signal, true);
    }
    for &signal in FORWARDED {
        install(signal, false);
    }
    install(libc::SIGCHLD, true);
    unblock(libc::SIGALRM);
    unblock(libc::SIGCHLD);
}

fn deadline_after(timeout: Timeout) -> Option<Instant> {
    match timeout {
        Timeout::Never => None,
        Timeout::After(span) => Instant::now().checked_add(span),
    }
}

/// Setting the core limit to zero is what makes re-raising a fatal signal
/// on ourselves safe; GNU falls back to a plain exit code when that fails.
fn disable_core_dumps() -> bool {
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } == 0 {
        return true;
    }
    diagnose(&format!(
        "warning: disabling core dumps failed: {}",
        strerror(errno())
    ));
    false
}

/// WCOREFLAG from sys/wait.h.
fn core_dumped(status: libc::c_int) -> bool {
    status & 0o200 != 0
}

/// In the child after fork: restore the tty signals and exec. On failure
/// report like sh does: 127 when not found, 126 otherwise.
fn exec_child(argv: &[*const libc::c_char], name: &str) -> ! {
    unsafe {
        libc::signal(libc::SIGTTIN, libc::SIG_DFL);
        libc::signal(libc::SIGTTOU, libc::SIG_DFL);
        libc::execvp(argv[0], argv.as_ptr());
    }
    let code = errno();
    diagnose(&format!(
        "failed to run command '{name}': {}",
        strerror(code)
    ));
    let status = if code == libc::ENOENT {
        EXIT_ENOENT
    } else {
        EXIT_CANNOT_INVOKE
    };
    unsafe { libc::_exit(status) }
}

struct Monitor<'a> {
    opts: &'a Options,
    command_name: String,
    child: libc::pid_t,
    pipe_read: i32,
    deadline: Option<Instant>,
    kill_after: Timeout,
    term_signal: i32,
    timed_out: bool,
    preserve_status: bool,
}

impl Monitor<'_> {
    fn wait(mut self) -> i32 {
        loop {
            let mut status: libc::c_int = 0;
            let reaped = unsafe { libc::waitpid(self.child, &mut status, libc::WNOHANG) };
            if reaped == self.child {
                return self.exit_status(status);
            }
            if reaped < 0 {
                let code = errno();
                if code == libc::EINTR {
                    continue;
                }
                diagnose(&format!("error waiting for command: {}", strerror(code)));
                return self.finish(EXIT_CANCELED);
            }
            self.poll_once();
        }
    }

    /// Sleep until a signal arrives or the deadline passes, then act on both.
    fn poll_once(&mut self) {
        let wait_ms = match self.deadline {
            None => -1,
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let round_up = u128::from(remaining.subsec_nanos() % 1_000_000 != 0);
                (remaining.as_millis() + round_up).min(i32::MAX as u128) as i32
            }
        };
        let mut fds = libc::pollfd {
            fd: self.pipe_read,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut fds, 1, wait_ms) };
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.deadline = None;
            self.handle_signal(libc::SIGALRM);
        }
        let mut buffer = [0u8; 64];
        loop {
            let n = unsafe { libc::read(self.pipe_read, buffer.as_mut_ptr().cast(), buffer.len()) };
            if n <= 0 {
                break;
            }
            for &byte in &buffer[..n as usize] {
                let signal = i32::from(byte);
                if signal != libc::SIGCHLD {
                    self.handle_signal(signal);
                }
            }
        }
    }

    /// GNU's `cleanup()`: the timer fires as SIGALRM and becomes the
    /// configured signal, anything else is forwarded as itself. The first
    /// signal of either kind arms the --kill-after timer, once.
    fn handle_signal(&mut self, mut signal: i32) {
        if signal == libc::SIGALRM {
            self.timed_out = true;
            signal = self.term_signal;
        }
        if self.kill_after != Timeout::Never {
            self.term_signal = libc::SIGKILL;
            self.deadline = deadline_after(self.kill_after);
            self.kill_after = Timeout::Never;
        }
        if self.opts.verbose {
            let name = signal_name(signal).map_or_else(|| signal.to_string(), str::to_string);
            diagnose(&format!(
                "sending signal {name} to command '{}'",
                self.command_name
            ));
        }
        unsafe {
            libc::kill(self.child, signal);
            if !self.opts.foreground {
                // Ignore it ourselves first: the group includes this process.
                libc::signal(signal, libc::SIG_IGN);
                libc::kill(0, signal);
                if signal != libc::SIGKILL && signal != libc::SIGCONT {
                    // A stopped command cannot act on TERM until it continues.
                    libc::kill(self.child, libc::SIGCONT);
                    libc::kill(0, libc::SIGCONT);
                }
            }
        }
    }

    fn exit_status(&mut self, status: libc::c_int) -> i32 {
        let code = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else if libc::WIFSIGNALED(status) {
            let signal = libc::WTERMSIG(status);
            if core_dumped(status) {
                diagnose("the monitored command dumped core");
            }
            if !self.timed_out && disable_core_dumps() {
                // Die the same way the command did, so the shell sees it.
                unsafe {
                    libc::signal(signal, libc::SIG_DFL);
                    unblock(signal);
                    libc::raise(signal);
                }
            }
            if self.timed_out && signal == libc::SIGKILL {
                self.preserve_status = true;
            }
            signal + 128
        } else {
            diagnose(&format!("unknown status from command ({status})"));
            1
        };
        self.finish(code)
    }

    fn finish(&self, code: i32) -> i32 {
        if self.timed_out && !self.preserve_status {
            EXIT_TIMEDOUT
        } else {
            code
        }
    }
}

fn run(opts: &Options) -> i32 {
    let command_name = opts.command[0].to_string_lossy().into_owned();
    let cstrings: Vec<CString> = opts
        .command
        .iter()
        .map(|arg| CString::new(arg.as_bytes()).expect("argv never contains NUL"))
        .collect();
    let mut argv: Vec<*const libc::c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    argv.push(std::ptr::null());

    // Our own process group, so the whole command tree can be signalled.
    if !opts.foreground {
        unsafe { libc::setpgid(0, 0) };
    }
    let (pipe_read, pipe_write) = self_pipe();
    SIGNAL_PIPE.store(pipe_write, Ordering::Relaxed);
    install_handlers(opts.term_signal);
    unsafe {
        // Do not stop if a background command needs the tty.
        libc::signal(libc::SIGTTIN, libc::SIG_IGN);
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
    }

    let child = unsafe { libc::fork() };
    if child < 0 {
        diagnose(&format!("fork system call failed: {}", strerror(errno())));
        return EXIT_CANCELED;
    }
    if child == 0 {
        exec_child(&argv, &command_name);
    }
    Monitor {
        opts,
        command_name,
        child,
        pipe_read,
        deadline: deadline_after(opts.timeout),
        kill_after: opts.kill_after,
        term_signal: opts.term_signal,
        timed_out: false,
        preserve_status: opts.preserve_status,
    }
    .wait()
}

fn main() {
    // The Rust runtime ignores SIGPIPE, and an ignored signal survives
    // exec: without this every command run under timeout would start
    // with SIGPIPE ignored, so `yes | head -1` would report a write
    // error instead of dying quietly. GNU starts at SIG_DFL, installs a
    // caught handler over it, and exec puts a caught handler back to
    // SIG_DFL, so its command always sees the default. Restoring it
    // here also lets `install(SIGPIPE, false)` below see SIG_DFL and
    // forward the signal, which it will not do for an ignored one.
    // SAFETY: setting a disposition to SIG_DFL is always valid, and this
    // runs before any output, thread or handler exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let opts = parse_args(&args);
    process::exit(run(&opts));
}
