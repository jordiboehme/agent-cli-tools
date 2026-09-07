//! `watch`: run a command repeatedly and show its output on a full
//! screen. A port of procps-ng watch(1) for macOS, matching its options,
//! its header, its rendering rules, its messages and its exit codes.
//!
//! Upstream draws through ncurses. This port has no curses to link, so
//! it drives the terminal with the ANSI sequences in `term`: the same
//! cell grid, the same diffed redraw, spelled out rather than delegated.

use agent_cli_tools::options::{self, Arg, Item, Mode, Opt, Parser};
use agent_cli_tools::term::{self, Attrs, Color, Screen, Size};
use std::ffi::{CStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const HELP: &str = "\n\
Usage:
 watch [options] command

Options:
  -b, --beep             beep if command has a non-zero exit
  -c, --color            interpret ANSI color and style sequences
  -C, --no-color         do not interpret ANSI color and style sequences
  -d, --differences[=<permanent>]
                         highlight changes between updates
  -e, --errexit          exit if command has a non-zero exit
  -f, --follow           Follow the output and don't clear screen
  -g, --chgexit          exit when output from command changes
  -q, --equexit <cycles>
                         exit when output from command does not change
  -n, --interval <secs>  seconds to wait between updates
  -p, --precise          -n includes command running time
  -r, --no-rerun         do not rerun program on window resize
  -s, --shotsdir         directory to store screenshots
  -t, --no-title         turn off header
  -w, --no-wrap          turn off line wrapping
  -x, --exec             pass command to exec instead of \"sh -c\"

 -h, --help     display this help and exit
 -v, --version  output version information and exit

Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>
Compatible with watch from procps-ng 4.0.7.
";

/// The options watch recognizes, in the order its own usage lists them.
/// `-f` and `-s` are here so that they parse exactly as upstream parses
/// them and are then refused by name; docs/watch.md says why.
const WATCH_OPTS: &[Opt] = &[
    Opt {
        short: Some('b'),
        long: "beep",
        arg: Arg::None,
    },
    Opt {
        short: Some('c'),
        long: "color",
        arg: Arg::None,
    },
    Opt {
        short: Some('C'),
        long: "no-color",
        arg: Arg::None,
    },
    Opt {
        short: Some('d'),
        long: "differences",
        arg: Arg::Optional,
    },
    Opt {
        short: Some('e'),
        long: "errexit",
        arg: Arg::None,
    },
    Opt {
        short: Some('f'),
        long: "follow",
        arg: Arg::None,
    },
    Opt {
        short: Some('g'),
        long: "chgexit",
        arg: Arg::None,
    },
    Opt {
        short: Some('q'),
        long: "equexit",
        arg: Arg::Required,
    },
    Opt {
        short: Some('n'),
        long: "interval",
        arg: Arg::Required,
    },
    Opt {
        short: Some('p'),
        long: "precise",
        arg: Arg::None,
    },
    Opt {
        short: Some('r'),
        long: "no-rerun",
        arg: Arg::None,
    },
    Opt {
        short: Some('s'),
        long: "shotsdir",
        arg: Arg::Required,
    },
    Opt {
        short: Some('t'),
        long: "no-title",
        arg: Arg::None,
    },
    Opt {
        short: Some('w'),
        long: "no-wrap",
        arg: Arg::None,
    },
    Opt {
        short: Some('x'),
        long: "exec",
        arg: Arg::None,
    },
    Opt {
        short: Some('h'),
        long: "help",
        arg: Arg::None,
    },
    Opt {
        short: Some('v'),
        long: "version",
        arg: Arg::None,
    },
];

/// The interval bounds upstream clamps to, silently: a tenth of a second
/// and thirty-one days.
const MIN_INTERVAL: f64 = 0.1;
const MAX_INTERVAL: f64 = 2_678_400.0;

/// How many characters of an SGR parameter list are collected before the
/// sequence is given up on. Upstream's MAX_ANSIBUF.
const MAX_ANSI_PARAMS: usize = 100;

/// The message `-e` leaves on the last row before it waits for a key.
const ERREXIT_MESSAGE: &str = "command exit with a non-zero status, press a key to exit";

/// What the command line asked for, once every option has been walked.
struct Options {
    beep: bool,
    color: bool,
    differences: bool,
    cumulative: bool,
    errexit: bool,
    chgexit: bool,
    equexit: Option<u64>,
    interval: f64,
    precise: bool,
    no_rerun: bool,
    no_title: bool,
    no_wrap: bool,
    exec: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            beep: false,
            color: false,
            differences: false,
            cumulative: false,
            errexit: false,
            chgexit: false,
            equexit: None,
            interval: 2.0,
            precise: false,
            no_rerun: false,
            no_title: false,
            no_wrap: false,
            exec: false,
        }
    }
}

/// Set by the handlers, read by the loop. A handler that only stores a
/// flag keeps every terminal-restoring path in ordinary code, where the
/// screen and the saved termios can be reached.
static WINCH: AtomicBool = AtomicBool::new(false);
static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_signal: libc::c_int) {
    WINCH.store(true, Ordering::Relaxed);
}

extern "C" fn on_terminate(_signal: libc::c_int) {
    TERMINATE.store(true, Ordering::Relaxed);
}

/// Install the handlers without `SA_RESTART`, which is the point: a
/// `poll` in the sleep or a `read` from the command's pipe has to come
/// back with EINTR so the loop can look at the flags.
fn install_handlers() {
    for (signal, handler) in [
        (libc::SIGWINCH, on_winch as extern "C" fn(libc::c_int)),
        (libc::SIGINT, on_terminate as extern "C" fn(libc::c_int)),
        (libc::SIGTERM, on_terminate as extern "C" fn(libc::c_int)),
        (libc::SIGHUP, on_terminate as extern "C" fn(libc::c_int)),
    ] {
        // SAFETY: `action` is fully initialized before it is installed,
        // the handler is an `extern "C" fn` of the right signature, and
        // both handlers touch nothing but an atomic flag.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handler as libc::sighandler_t;
            libc::sigemptyset(&raw mut action.sa_mask);
            action.sa_flags = 0;
            libc::sigaction(signal, &raw const action, std::ptr::null_mut());
        }
    }
}

/// The usage block on stderr, and exit 1. Upstream prints the whole
/// block for a bad option rather than a "Try --help" line, so this does
/// not go through `options::usage_error`.
fn usage(message: Option<&str>) -> ! {
    let mut err = std::io::stderr().lock();
    if let Some(message) = message {
        let _ = writeln!(err, "watch: {message}");
    }
    let _ = err.write_all(HELP.as_bytes());
    let _ = err.flush();
    process::exit(1)
}

/// A diagnostic in the shape glibc's `error()` gives it, and the exit
/// code that goes with it. No usage block: these are failures of the
/// machine, not of the command line.
fn fatal(message: &str, code: i32) -> ! {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "watch: {message}");
    let _ = err.flush();
    process::exit(code)
}

fn strerror(code: i32) -> String {
    // SAFETY: strerror returns a valid NUL-terminated string for any
    // int, and the borrow ends before any other libc call could reuse
    // its static buffer.
    unsafe { CStr::from_ptr(libc::strerror(code)) }
        .to_string_lossy()
        .into_owned()
}

/// The interval parser, which is not the GNU duration parser: optional
/// spaces, an optional sign, digits, a `.` or `,` radix, digits. No
/// exponent, no suffix, no trailing junk. Upstream carries its own so a
/// locale with a comma radix keeps reading both spellings.
fn parse_interval(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
        at += 1;
    }
    let negative = match bytes.get(at) {
        Some(b'-') => {
            at += 1;
            true
        }
        Some(b'+') => {
            at += 1;
            false
        }
        _ => false,
    };
    let mut digits = String::new();
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        digits.push(bytes[at] as char);
        at += 1;
    }
    let mut seen = !digits.is_empty();
    if matches!(bytes.get(at), Some(b'.') | Some(b',')) {
        at += 1;
        digits.push('.');
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            digits.push(bytes[at] as char);
            seen = true;
            at += 1;
        }
    }
    if !seen || at != bytes.len() {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// A parsed interval, clamped in silence the way upstream clamps it.
fn interval_or_exit(text: &str, message: &str) -> f64 {
    match parse_interval(text) {
        Some(value) => value.clamp(MIN_INTERVAL, MAX_INTERVAL),
        // The trailing text is `strerror(EINVAL)`, which is what the
        // glibc `error()` call upstream appends to every one of these.
        None => fatal(
            &format!("{message}: '{text}': {}", strerror(libc::EINVAL)),
            1,
        ),
    }
}

/// `-q`'s argument: a whole-string decimal count, with anything under
/// one meaning one.
fn parse_cycles(text: &str) -> u64 {
    let trimmed = text.trim_start_matches([' ', '\t']);
    let value: i64 = match trimmed.parse() {
        Ok(value) => value,
        Err(_) => fatal(
            &format!(
                "failed to parse argument: '{text}': {}",
                strerror(libc::EINVAL)
            ),
            1,
        ),
    };
    if value < 1 { 1 } else { value as u64 }
}

/// This machine's name, as `gethostname` gives it: `mymac.local` rather
/// than the short name, which is what upstream puts in the header.
fn hostname() -> String {
    let mut buffer = [0 as libc::c_char; 256];
    // SAFETY: the buffer holds the number of bytes gethostname is told
    // it may fill, and the result is read only up to its first NUL.
    let rc = unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len() - 1) };
    if rc != 0 {
        return String::new();
    }
    // SAFETY: gethostname NUL-terminates within the size it was given,
    // and the last byte was left zero in case it does not.
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// The current time through `strftime("%c")`, which is the locale's own
/// idea of a full date and time.
fn timestamp() -> String {
    // SAFETY: each call is given exactly the object it documents: a
    // `time_t` to read, a `tm` to fill, and a buffer with its own size.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut broken: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&raw const now, &raw mut broken).is_null() {
            return String::new();
        }
        let mut buffer = [0 as libc::c_char; 256];
        let written = libc::strftime(
            buffer.as_mut_ptr(),
            buffer.len(),
            c"%c".as_ptr(),
            &raw const broken,
        );
        if written == 0 {
            return String::new();
        }
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

/// What to run, and how it reads in the header.
struct Spec {
    /// The operands joined with single spaces, which is both the string
    /// `sh -c` is handed and the text the header shows.
    line: OsString,
    /// The operands as they arrived, for `-x`.
    argv: Vec<OsString>,
    exec: bool,
}

impl Spec {
    fn command(&self) -> Command {
        if self.exec {
            let mut command = Command::new(&self.argv[0]);
            command.args(&self.argv[1..]);
            command
        } else {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(&self.line);
            command
        }
    }

    /// The name an exec failure is reported under, which is the program
    /// upstream handed to `execvp`.
    fn program(&self) -> String {
        if self.exec {
            self.argv[0].to_string_lossy().into_owned()
        } else {
            "/bin/sh".to_string()
        }
    }
}

/// Where a redraw is in the output stream, and what it does with the
/// next character. One of these is built per run.
struct Render<'a> {
    /// The first and the one-past-last screen row of the body.
    top: usize,
    end: usize,
    cols: usize,
    row: usize,
    col: usize,
    /// Set by `-w` once a line has filled the row: everything up to the
    /// next newline is read and dropped.
    skipping: bool,
    attrs: Attrs,
    escape: Escape,
    params: String,
    /// Bytes of a UTF-8 character split across two reads.
    partial: Vec<u8>,
    color: bool,
    wrap: bool,
    differences: bool,
    cumulative: bool,
    first_screen: bool,
    /// The body as it looked before this run, one row of characters per
    /// body row, which is what `-d` compares against.
    before: Vec<Vec<char>>,
    /// Which body cells `-d` is currently showing in reverse video, so
    /// that a cumulative run can keep them.
    highlighted: &'a mut [bool],
}

#[derive(PartialEq, Eq)]
enum Escape {
    Normal,
    /// An ESC was read and `-c` is on.
    Introducer,
    /// An `ESC (` was read; the character naming the character set is
    /// consumed and ignored.
    Charset,
    /// A `ESC [` was read; parameters are collecting.
    Params,
}

impl Render<'_> {
    /// Take the next chunk of the command's output. Characters split
    /// across a chunk boundary are held until the rest of them arrives.
    fn feed<W: Write>(&mut self, bytes: &[u8], screen: &mut Screen<W>) {
        self.partial.extend_from_slice(bytes);
        let mut chars = Vec::new();
        loop {
            match std::str::from_utf8(&self.partial) {
                Ok(text) => {
                    chars.extend(text.chars());
                    self.partial.clear();
                    break;
                }
                Err(error) => {
                    let good = error.valid_up_to();
                    chars.extend(
                        std::str::from_utf8(&self.partial[..good])
                            .unwrap_or_default()
                            .chars(),
                    );
                    match error.error_len() {
                        // A byte that can never begin a character: hand
                        // on the replacement and step over it.
                        Some(bad) => {
                            chars.push(char::REPLACEMENT_CHARACTER);
                            self.partial.drain(..good + bad);
                        }
                        // A character cut in half by the read boundary.
                        None => {
                            self.partial.drain(..good);
                            break;
                        }
                    }
                }
            }
        }
        for ch in chars {
            self.take(ch, screen);
        }
    }

    fn take<W: Write>(&mut self, ch: char, screen: &mut Screen<W>) {
        match self.escape {
            Escape::Normal => {
                match ch {
                    // Without -c the escape byte alone is dropped and
                    // the sequence behind it is shown as the text it is.
                    '\x1b' => {
                        if self.color {
                            self.escape = Escape::Introducer;
                        }
                    }
                    '\n' => self.newline(screen),
                    '\t' => self.tab(screen),
                    // The bell is the one control character that reaches
                    // the terminal; it occupies no cell.
                    '\x07' => screen.write_raw(b"\x07"),
                    _ if ch.is_control() => {}
                    _ => self.print(ch, screen),
                }
            }
            Escape::Introducer => match ch {
                '(' => self.escape = Escape::Charset,
                '[' => {
                    self.params.clear();
                    self.escape = Escape::Params;
                }
                // Anything else was never part of a sequence this
                // understands, so it is text again.
                _ => {
                    self.escape = Escape::Normal;
                    self.take(ch, screen);
                }
            },
            Escape::Charset => self.escape = Escape::Normal,
            Escape::Params => {
                if ch == 'm' {
                    self.apply_sgr();
                    self.escape = Escape::Normal;
                } else if (ch.is_ascii_digit() || ch == ';') && self.params.len() < MAX_ANSI_PARAMS
                {
                    self.params.push(ch);
                } else {
                    // Not an SGR sequence: the character that says so is
                    // consumed with the rest of it.
                    self.escape = Escape::Normal;
                }
            }
        }
    }

    /// Apply the parameters collected since `ESC [`. Reading stops at
    /// the first one that is not understood, which is what leaves 24-bit
    /// colour sequences half applied, exactly as upstream leaves them.
    fn apply_sgr(&mut self) {
        if self.params.is_empty() {
            self.attrs = Attrs::default();
            return;
        }
        let params: Vec<&str> = self.params.split(';').collect();
        let mut at = 0;
        while at < params.len() {
            let Ok(code) = params[at].parse::<u16>() else {
                return;
            };
            at += 1;
            match code {
                0 => self.attrs = Attrs::default(),
                1 => self.attrs.bold = true,
                2 => self.attrs.dim = true,
                3 => self.attrs.italic = true,
                4 => self.attrs.underline = true,
                5 => self.attrs.blink = true,
                7 => self.attrs.reverse = true,
                21 | 22 => {
                    self.attrs.bold = false;
                    self.attrs.dim = false;
                }
                23 => self.attrs.italic = false,
                24 => self.attrs.underline = false,
                25 => self.attrs.blink = false,
                27 => self.attrs.reverse = false,
                30..=37 => self.attrs.fg = Color::Indexed((code - 30) as u8),
                39 => self.attrs.fg = Color::Default,
                40..=47 => self.attrs.bg = Color::Indexed((code - 40) as u8),
                49 => self.attrs.bg = Color::Default,
                90..=97 => self.attrs.fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => self.attrs.bg = Color::Indexed((code - 100 + 8) as u8),
                38 | 48 => {
                    // Only the 256-colour form, `38;5;n`, is understood.
                    let indexed = params.get(at).is_some_and(|p| *p == "5");
                    let index = params.get(at + 1).and_then(|p| p.parse::<u8>().ok());
                    match (indexed, index) {
                        (true, Some(index)) => {
                            if code == 38 {
                                self.attrs.fg = Color::Indexed(index);
                            } else {
                                self.attrs.bg = Color::Indexed(index);
                            }
                            at += 2;
                        }
                        _ => return,
                    }
                }
                _ => return,
            }
        }
    }

    fn newline<W: Write>(&mut self, screen: &mut Screen<W>) {
        self.skipping = false;
        // A row that filled exactly has its wrap still owed; this
        // newline pays it rather than leaving a blank row behind.
        if self.col < self.cols {
            self.clear_rest_of_row(screen);
        }
        self.row += 1;
        self.col = 0;
    }

    fn tab<W: Write>(&mut self, screen: &mut Screen<W>) {
        for _ in 0..(8 - self.col % 8) {
            self.print(' ', screen);
        }
    }

    fn print<W: Write>(&mut self, ch: char, screen: &mut Screen<W>) {
        if self.skipping {
            return;
        }
        if self.col >= self.cols {
            if !self.wrap {
                self.skipping = true;
                return;
            }
            self.row += 1;
            self.col = 0;
        }
        if self.row >= self.end {
            return;
        }
        self.place(self.row, self.col, ch, screen);
        self.col += 1;
    }

    /// Write one character into a body cell, with `-d`'s reverse video
    /// on top of whatever colours are in force.
    fn place<W: Write>(&mut self, row: usize, col: usize, ch: char, screen: &mut Screen<W>) {
        let at = (row - self.top) * self.cols + col;
        let mut attrs = self.attrs;
        let mut highlight = false;
        if self.differences && !self.first_screen {
            let was = self
                .before
                .get(row - self.top)
                .and_then(|row| row.get(col))
                .copied()
                .unwrap_or(' ');
            highlight = was != ch || (self.cumulative && self.highlighted[at]);
        }
        self.highlighted[at] = highlight;
        if highlight {
            attrs.reverse = true;
        }
        screen.put(row, col, ch, attrs);
    }

    /// Blank the rest of the current row. The spaces are written cell by
    /// cell rather than erased, so that output which grew shorter is
    /// seen as a change by `-d`, `-g` and `-q`.
    fn clear_rest_of_row<W: Write>(&mut self, screen: &mut Screen<W>) {
        if self.row >= self.end {
            return;
        }
        for col in self.col..self.cols {
            self.place(self.row, col, ' ', screen);
        }
    }

    /// The command's output has ended: blank whatever the last run left
    /// on the rows this one did not reach.
    fn finish<W: Write>(&mut self, screen: &mut Screen<W>) {
        if self.col < self.cols {
            self.clear_rest_of_row(screen);
        }
        self.row += 1;
        self.col = 0;
        while self.row < self.end {
            self.clear_rest_of_row(screen);
            self.row += 1;
        }
    }
}

/// A failure of the machine rather than of the command line.
enum Fatal {
    Pipes,
    Fork(String),
}

/// The result of one run of the command.
enum Ran {
    /// The command's exit status, in the shape watch reports it.
    Status(i32),
    /// A signal arrived while the output was being read.
    Interrupted,
}

/// Run the command once, painting its output into the body of `screen`
/// as it arrives. Standard output and standard error share one pipe, so
/// the two streams interleave the way they would on a terminal, and the
/// pipe is drained to its end before the child is reaped.
fn run_command<W: Write>(
    screen: &mut Screen<W>,
    spec: &Spec,
    render: &mut Render<'_>,
) -> Result<Ran, Fatal> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe fills the two-element array it is handed.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(Fatal::Pipes);
    }
    // SAFETY: both descriptors come from a successful `pipe` and each is
    // wrapped exactly once, so ownership of them is not duplicated.
    let mut reader = unsafe { File::from_raw_fd(fds[0]) };
    let writer = unsafe { File::from_raw_fd(fds[1]) };
    // The child must not inherit the read end: it has no use for it, and
    // a copy left open in a long-lived grandchild would hold the pipe
    // open past the point this loop expects to see its end.
    // SAFETY: `fds[0]` is open and owned by `reader`.
    unsafe { libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC) };

    let size = screen.size();
    // The Command owns the two write ends until it is dropped, so it is
    // built and dropped inside this block: with a copy still open in
    // this process, the read below would never reach the end of the pipe.
    let spawned = {
        let mut command = spec.command();
        command
            .stdin(Stdio::inherit())
            .stderr(Stdio::from(match writer.try_clone() {
                Ok(clone) => clone,
                Err(_) => return Err(Fatal::Pipes),
            }))
            .stdout(Stdio::from(writer))
            .env("LINES", size.rows.to_string())
            .env("COLUMNS", size.cols.to_string());
        command.spawn()
    };

    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            let code = error.raw_os_error().unwrap_or(0);
            // A command that could not be executed is the child's
            // failure, reported the way the child reports it; anything
            // else went wrong before there was a child at all.
            return match code {
                libc::ENOENT | libc::EACCES | libc::ENOEXEC | libc::ENOTDIR | libc::EISDIR => {
                    let text = format!("{}: {}\n", spec.program(), strerror(code));
                    render.feed(text.as_bytes(), screen);
                    render.finish(screen);
                    Ok(Ran::Status(127))
                }
                _ => Err(Fatal::Fork(strerror(code))),
            };
        }
    };

    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => render.feed(&buffer[..count], screen),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                if TERMINATE.load(Ordering::Relaxed) {
                    return Ok(Ran::Interrupted);
                }
            }
            Err(_) => break,
        }
    }
    render.finish(screen);

    let status = match child.wait() {
        Ok(status) => status,
        Err(_) => return Ok(Ran::Status(127)),
    };
    Ok(Ran::Status(exit_code(&status)))
}

/// A child's status in the shape watch reports it: what it exited with,
/// or 128 plus the signal that killed it.
fn exit_code(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    match status.code() {
        Some(code) => code,
        None => 128 + status.signal().unwrap_or(0),
    }
}

fn put_str<W: Write>(screen: &mut Screen<W>, row: usize, col: usize, text: &str) {
    for (offset, ch) in text.chars().enumerate() {
        screen.put(row, col + offset, ch, Attrs::default());
    }
}

/// The two header rows. The command is trimmed to whatever the right
/// half leaves it, and everything disappears once the terminal is too
/// narrow to hold even that right half.
fn draw_header<W: Write>(screen: &mut Screen<W>, opts: &Options, command: &str, host: &str) {
    let cols = usize::from(screen.size().cols);
    screen.clear_to_end_of_row(0, 0);
    screen.clear_to_end_of_row(1, 0);
    let left = format!("Every {:.1}s: ", opts.interval);
    let right = format!("{host}: {}", timestamp());
    let right_len = right.chars().count();
    if cols < right_len {
        return;
    }
    put_str(screen, 0, cols - right_len, &right);
    let left_len = left.chars().count();
    let command_len = command.chars().count();
    let avail = cols as isize - left_len as isize - right_len as isize;
    if avail >= 0 {
        put_str(screen, 0, 0, &left);
    }
    if avail > command_len as isize {
        put_str(screen, 0, left_len, command);
    } else if avail > 3 {
        let keep = avail as usize - 4;
        let trimmed: String = command.chars().take(keep).collect();
        put_str(screen, 0, left_len, &format!("{trimmed}..."));
    }
}

/// The second header row: how long the last run took and what it exited
/// with, against the right margin.
fn draw_timing<W: Write>(screen: &mut Screen<W>, elapsed: Duration, code: i32) {
    let seconds = elapsed.as_secs_f64();
    let text = if seconds < 0.001 {
        format!("in <0.001s ({code})")
    } else if seconds > 86_400.0 {
        format!("in >1 day ({code})")
    } else {
        format!("in {seconds:.3}s ({code})")
    };
    let cols = usize::from(screen.size().cols);
    screen.clear_to_end_of_row(1, 0);
    let width = text.chars().count();
    if cols >= width {
        put_str(screen, 1, cols - width, &text);
    }
}

/// Why the sleep between two runs ended.
enum Wake {
    /// The interval ran out.
    Elapsed,
    /// Space, or a resize with `-r` off: run again now.
    Now,
    /// The `q` key.
    Quit,
    /// SIGINT, SIGTERM or SIGHUP.
    Terminate,
}

/// Read one byte from the terminal, or `None` at its end.
fn read_key() -> Option<u8> {
    let mut byte = 0u8;
    // SAFETY: the buffer is the one byte the call is told it may fill.
    let count = unsafe { libc::read(libc::STDIN_FILENO, (&raw mut byte).cast(), 1) };
    if count == 1 { Some(byte) } else { None }
}

/// Whether the terminal has a byte waiting, without waiting for one.
fn key_pending() -> bool {
    let mut fds = [libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    }];
    // SAFETY: the array holds the one descriptor the count names.
    unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) > 0 }
}

/// Wait for the interval to run out, for a key, or for a signal.
fn sleep_until(deadline: Instant, no_rerun: bool) -> Wake {
    loop {
        if TERMINATE.load(Ordering::Relaxed) {
            return Wake::Terminate;
        }
        // The flag is left standing: the next turn of the loop is what
        // resizes the screen, whether or not the sleep ended for it.
        if WINCH.load(Ordering::Relaxed) && !no_rerun {
            return Wake::Now;
        }
        let now = Instant::now();
        if now >= deadline {
            return Wake::Elapsed;
        }
        let remaining = deadline - now;
        let millis = remaining.as_millis().min(i32::MAX as u128) as i32;
        let mut fds = [libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: the array holds the one descriptor the count names.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, millis) };
        if rc < 0 {
            // EINTR: a signal arrived, and the flags above say which.
            continue;
        }
        if rc == 0 {
            return Wake::Elapsed;
        }
        match read_key() {
            Some(b'q') => return Wake::Quit,
            Some(b' ') => return Wake::Now,
            Some(_) => continue,
            // Standard input has ended, so there are no more keys to
            // wait for; the rest of the interval is simply time.
            None => {
                let now = Instant::now();
                if now < deadline {
                    std::thread::sleep(deadline - now);
                }
                return Wake::Elapsed;
            }
        }
    }
}

/// The loop: draw the header, run the command, draw its output, decide
/// whether anything says to stop, sleep. Returns the exit code.
fn watch<W: Write>(screen: &mut Screen<W>, opts: &Options, spec: &Spec) -> Result<i32, String> {
    let host = hostname();
    let command = spec.line.to_string_lossy().into_owned();
    let mut size = screen.size();
    let mut top = if opts.no_title { 0 } else { 2 };
    let mut body = body_len(size, top);
    let mut highlighted = vec![false; body];
    let mut first_screen = true;
    let mut cycles: u64 = 1;

    loop {
        if WINCH.swap(false, Ordering::Relaxed) {
            size = term::size(libc::STDIN_FILENO);
            screen.resize(size);
            top = if opts.no_title { 0 } else { 2 };
            body = body_len(size, top);
            highlighted = vec![false; body];
            first_screen = true;
        }
        let rows = usize::from(size.rows);
        let cols = usize::from(size.cols);
        let end = rows.max(top);

        if !opts.no_title {
            draw_header(screen, opts, &command, &host);
        }
        screen.flush();

        let before: Vec<Vec<char>> = (top..end).map(|row| screen.row_chars(row)).collect();
        let mut render = Render {
            top,
            end,
            cols,
            row: top,
            col: 0,
            skipping: false,
            attrs: Attrs::default(),
            escape: Escape::Normal,
            params: String::new(),
            partial: Vec::new(),
            color: opts.color,
            wrap: !opts.no_wrap,
            differences: opts.differences,
            cumulative: opts.cumulative,
            first_screen,
            before: before.clone(),
            highlighted: &mut highlighted,
        };
        let started = Instant::now();
        let ran = match run_command(screen, spec, &mut render) {
            Ok(ran) => ran,
            Err(Fatal::Pipes) => return Err("unable to create IPC pipes".to_string()),
            Err(Fatal::Fork(reason)) => return Err(format!("unable to fork process: {reason}")),
        };
        let elapsed = started.elapsed();
        let status = match ran {
            Ran::Status(status) => status,
            Ran::Interrupted => return Ok(0),
        };

        if !opts.no_title {
            draw_timing(screen, elapsed, status);
        }
        if opts.beep && status != 0 {
            screen.write_raw(b"\x07");
        }
        screen.flush();

        let after: Vec<Vec<char>> = (top..end).map(|row| screen.row_chars(row)).collect();
        let changed = after != before;

        if opts.chgexit && !first_screen && changed {
            return Ok(0);
        }
        if let Some(limit) = opts.equexit {
            if first_screen || changed {
                cycles = 1;
            } else if cycles >= limit {
                return Ok(0);
            } else {
                cycles += 1;
            }
        }
        if opts.errexit && status != 0 {
            put_str(screen, rows - 1, 0, ERREXIT_MESSAGE);
            screen.flush();
            // Whatever was typed while the command ran is not the key
            // this is waiting for.
            while key_pending() {
                if read_key().is_none() {
                    break;
                }
            }
            // This read is the wait: it blocks until a key arrives, and
            // comes back empty only at the end of the input or because
            // a signal interrupted it.
            if read_key().is_none() && TERMINATE.load(Ordering::Relaxed) {
                return Ok(0);
            }
            return Ok(status);
        }
        first_screen = false;

        let tick = if opts.precise {
            started
        } else {
            Instant::now()
        };
        let deadline = tick + Duration::from_secs_f64(opts.interval);
        match sleep_until(deadline, opts.no_rerun) {
            Wake::Quit | Wake::Terminate => return Ok(0),
            Wake::Elapsed | Wake::Now => {}
        }
    }
}

fn body_len(size: Size, top: usize) -> usize {
    usize::from(size.rows).saturating_sub(top) * usize::from(size.cols)
}

fn main() {
    // Without this a pipeline that stops reading would turn into a write
    // error instead of the silent death by signal the C original dies.
    // SAFETY: setting a disposition to SIG_DFL is always valid, and this
    // runs before any output, thread or handler exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut opts = Options::default();
    let mut operands: Vec<OsString> = Vec::new();

    // The environment is read first and any -n on the command line wins
    // over it, which falls out of walking the options afterwards.
    if let Ok(value) = std::env::var("WATCH_INTERVAL") {
        opts.interval = interval_or_exit(&value, "Could not parse interval from WATCH_INTERVAL");
    }

    // parse_partial, not parse_or_exit: -h, -v, a bad interval and a
    // refused option all have to act as they are reached, in the order
    // the command line put them. A pending parse error is reported only
    // once everything before it has been walked.
    let (items, error) =
        Parser::new("watch", WATCH_OPTS, Mode::StopAtFirstOperand).parse_partial(&args);
    for item in items {
        match item {
            Item::Flag { long: "beep", .. } => opts.beep = true,
            Item::Flag { long: "color", .. } => opts.color = true,
            Item::Flag {
                long: "no-color", ..
            } => opts.color = false,
            Item::Flag {
                long: "differences",
                value,
            } => {
                opts.differences = true;
                // Any argument at all means cumulative, which is why the
                // documented spelling, -d=permanent, is not checked for.
                opts.cumulative = value.is_some();
            }
            Item::Flag {
                long: "errexit", ..
            } => opts.errexit = true,
            Item::Flag {
                long: "chgexit", ..
            } => opts.chgexit = true,
            Item::Flag {
                long: "equexit",
                value,
            } => {
                let value = value.expect("-q takes a required argument");
                opts.equexit = Some(parse_cycles(&value.to_string_lossy()));
            }
            Item::Flag {
                long: "interval",
                value,
            } => {
                let value = value.expect("-n takes a required argument");
                opts.interval =
                    interval_or_exit(&value.to_string_lossy(), "failed to parse argument");
            }
            Item::Flag {
                long: "precise", ..
            } => opts.precise = true,
            Item::Flag {
                long: "no-rerun", ..
            } => opts.no_rerun = true,
            Item::Flag {
                long: "no-title", ..
            } => opts.no_title = true,
            Item::Flag {
                long: "no-wrap", ..
            } => opts.no_wrap = true,
            Item::Flag { long: "exec", .. } => opts.exec = true,
            // Neither has anything on macOS to point at instead.
            Item::Flag { long: "follow", .. } => options::unsupported("watch", "--follow", None),
            Item::Flag {
                long: "shotsdir", ..
            } => options::unsupported("watch", "--shotsdir", None),
            Item::Flag { long: "help", .. } => {
                print!("{HELP}");
                process::exit(0);
            }
            Item::Flag {
                long: "version", ..
            } => {
                print!("{}", agent_cli_tools::version_text("watch"));
                process::exit(0);
            }
            Item::Flag { long, .. } => unreachable!("unknown option name {long}"),
            Item::Operand(operand) => operands.push(operand),
        }
    }
    if let Some(message) = error {
        usage(Some(&message));
    }
    if operands.is_empty() {
        usage(None);
    }

    let mut line = OsString::new();
    for (at, operand) in operands.iter().enumerate() {
        if at > 0 {
            line.push(" ");
        }
        line.push(operand);
    }
    let spec = Spec {
        line,
        argv: operands,
        exec: opts.exec,
    };

    // The locale decides what strftime("%c") looks like, and nothing
    // else here reads one.
    // SAFETY: setlocale takes a NUL-terminated string and this runs
    // before any thread exists.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }
    install_handlers();

    let size = term::size(libc::STDIN_FILENO);
    // Without a terminal there is no cbreak mode to enter, and watch
    // still runs; it simply never sees a key.
    let raw = term::Raw::enable(libc::STDIN_FILENO);
    let mut screen = Screen::new(std::io::stdout(), size);
    screen.enter();
    let outcome = watch(&mut screen, &opts, &spec);
    // The terminal is put back before anything is said about how the
    // run ended, so a diagnostic is not left on the alternate screen.
    screen.leave();
    drop(raw);
    match outcome {
        Ok(code) => process::exit(code),
        Err(message) => fatal(&message, 2),
    }
}
