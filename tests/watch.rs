//! Integration tests that drive the built `watch` binary. watch only
//! makes sense in front of a terminal, so every case that inspects a
//! frame allocates a pseudo-terminal, hands the slave to the binary as
//! its standard input, output and error in a session of its own, and
//! reads the master. What arrives there is a stream of cursor moves and
//! cell writes, so the harness carries a small terminal of its own and
//! asserts on the rows it paints rather than on the bytes.
//!
//! Every case that only reads a diagnostic uses plain pipes: watch
//! reports a usage or interval error before it touches the terminal, and
//! these tests are what holds it to that.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_watch");

/// How long any single expectation may take. Long enough that a loaded
/// machine still gets there, short enough that a hung binary is reported
/// as a failure rather than as a suite that never ends.
const DEADLINE: Duration = Duration::from_secs(5);

/// `_IO('t', 97)`, the macOS spelling of TIOCSCTTY, which the libc crate
/// does not define for Apple targets.
const TIOCSCTTY: libc::c_ulong = 0x2000_7461;

/// The usage block, spelled out here rather than shared with the binary,
/// so that a change to either one has to be made deliberately in both.
const USAGE: &str = "\n\
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

/// This machine's name the way the binary asks for it, so the header
/// test compares against the same answer rather than a guess at it.
fn hostname() -> String {
    let mut buffer = [0 as libc::c_char; 256];
    let rc = unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len() - 1) };
    assert_eq!(rc, 0, "gethostname failed");
    unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// One watch process, the terminal it was given, and everything read
/// from that terminal so far.
struct Run {
    child: Child,
    /// `None` once the terminal has been torn down, which is the last
    /// thing a run needs before it can be reaped.
    master: Option<File>,
    raw: Vec<u8>,
    rows: usize,
    cols: usize,
    ended: bool,
    status: Option<ExitStatus>,
}

impl Drop for Run {
    fn drop(&mut self) {
        // A case that failed part way through must not leave a watch
        // running: kill and reap on every path. Both calls fail
        // harmlessly if the process is already gone.
        let _ = self.child.kill();
        // Closing the master tears the terminal down. Without it a watch
        // that filled the terminal nobody is reading any more cannot
        // finish dying: the kernel holds it in the exit path until its
        // output drains, and there is nothing left to drain it.
        self.master = None;
        let _ = self.child.wait();
    }
}

impl Run {
    /// Start watch on a pseudo-terminal of the given size.
    fn new(args: &[&str], rows: u16, cols: u16) -> Run {
        Run::with_env(args, rows, cols, &[])
    }

    fn with_env(args: &[&str], rows: u16, cols: u16, env: &[(&str, &str)]) -> Run {
        let mut master_fd = 0 as libc::c_int;
        let mut slave_fd = 0 as libc::c_int;
        let mut window = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // A terminal with no echo, no canonical input and no output
        // translation: what reaches the master is exactly what watch
        // wrote, and a key written to it is not reflected back.
        let mut mode: libc::termios = unsafe { std::mem::zeroed() };
        mode.c_cflag = libc::CS8 | libc::CREAD | libc::CLOCAL;
        mode.c_cc[libc::VMIN] = 1;
        mode.c_ispeed = libc::B38400;
        mode.c_ospeed = libc::B38400;
        let rc = unsafe {
            libc::openpty(
                &raw mut master_fd,
                &raw mut slave_fd,
                std::ptr::null_mut(),
                &raw mut mode,
                &raw mut window,
            )
        };
        assert_eq!(rc, 0, "openpty failed");
        let master = unsafe { File::from_raw_fd(master_fd) };
        let slave = unsafe { File::from_raw_fd(slave_fd) };

        let mut command = Command::new(BIN);
        command
            .args(args)
            .stdin(Stdio::from(slave.try_clone().expect("dup slave")))
            .stdout(Stdio::from(slave.try_clone().expect("dup slave")))
            .stderr(Stdio::from(slave.try_clone().expect("dup slave")))
            .env_remove("LINES")
            .env_remove("COLUMNS")
            .env_remove("WATCH_INTERVAL");
        for (name, value) in env {
            command.env(name, value);
        }
        // SAFETY: both calls are async-signal-safe, which is all a
        // pre_exec closure may use, and they run after the standard
        // descriptors have been pointed at the slave, so descriptor 0 is
        // the terminal this makes controlling.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, TIOCSCTTY, 0);
                Ok(())
            });
        }
        let child = command.spawn().expect("spawn watch");
        // The parent's copy of the slave has to go, or the master never
        // sees the terminal close when watch exits.
        drop(slave);
        Run {
            child,
            master: Some(master),
            raw: Vec::new(),
            rows: usize::from(rows),
            cols: usize::from(cols),
            ended: false,
            status: None,
        }
    }

    /// Read whatever the terminal has, waiting no longer than `budget`.
    fn pump(&mut self, budget: Duration) {
        if self.ended {
            return;
        }
        let Some(master) = self.master.as_mut() else {
            return;
        };
        let mut fds = [libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        let millis = budget.as_millis().min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, millis) };
        if ready <= 0 {
            return;
        }
        let mut buffer = [0u8; 4096];
        match master.read(&mut buffer) {
            Ok(0) => self.ended = true,
            Ok(count) => self.raw.extend_from_slice(&buffer[..count]),
            // Once the last slave descriptor is gone a master read fails
            // with EIO rather than reporting an end of file.
            Err(_) => self.ended = true,
        }
    }

    fn screen(&self) -> Vec<String> {
        render(&self.raw, self.rows, self.cols)
    }

    /// Which cells of the current frame are in reverse video.
    fn reverse(&self) -> Vec<Vec<bool>> {
        render_cells(&self.raw, self.rows, self.cols)
            .into_iter()
            .map(|row| row.into_iter().map(|(_, reverse)| reverse).collect())
            .collect()
    }

    /// Read until the screen satisfies `predicate`, or fail with what
    /// the screen looked like when the time ran out.
    fn wait_for(&mut self, what: &str, predicate: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let screen = self.screen();
            if predicate(&screen) {
                return screen;
            }
            if self.ended || Instant::now() >= deadline {
                panic!("never saw {what}; screen was:\n{}", screen.join("\n"));
            }
            self.pump(Duration::from_millis(50));
        }
    }

    /// Read until these exact bytes have come out of the terminal.
    fn wait_for_bytes(&mut self, needle: &[u8], what: &str) {
        let deadline = Instant::now() + DEADLINE;
        while !self.raw.windows(needle.len()).any(|w| w == needle) {
            if self.ended || Instant::now() >= deadline {
                panic!("never saw {what} in the terminal output");
            }
            self.pump(Duration::from_millis(50));
        }
    }

    fn key(&mut self, bytes: &[u8]) {
        self.master
            .as_mut()
            .expect("the terminal is open")
            .write_all(bytes)
            .expect("write to the terminal");
    }

    /// Wait for watch to exit, still reading the terminal so that it
    /// never blocks on a full one.
    fn wait_exit(&mut self) -> ExitStatus {
        if let Some(status) = self.status {
            return status;
        }
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                // Whatever it wrote on the way out is still in flight.
                let drain = Instant::now() + Duration::from_millis(300);
                while Instant::now() < drain && !self.ended {
                    self.pump(Duration::from_millis(20));
                }
                self.status = Some(status);
                return status;
            }
            if Instant::now() >= deadline {
                panic!(
                    "watch did not exit; screen was:\n{}",
                    self.screen().join("\n")
                );
            }
            self.pump(Duration::from_millis(50));
        }
    }
}

/// The terminal the tests read with: enough of one to follow watch's
/// absolute cursor moves, its clears, the characters between them and
/// the one attribute the tests care about. Every cell comes back with
/// whether it was written in reverse video, which is how `-d` marks a
/// cell that changed.
fn render_cells(raw: &[u8], rows: usize, cols: usize) -> Vec<Vec<(char, bool)>> {
    let mut grid = vec![vec![(' ', false); cols]; rows];
    let (mut row, mut col) = (0usize, 0usize);
    let mut reverse = false;
    let text = String::from_utf8_lossy(raw);
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                if chars.next() != Some('[') {
                    continue;
                }
                let mut params = String::new();
                let mut final_byte = '\0';
                for next in chars.by_ref() {
                    if next.is_ascii_digit() || next == ';' || next == '?' {
                        params.push(next);
                    } else {
                        final_byte = next;
                        break;
                    }
                }
                match final_byte {
                    'H' => {
                        let mut parts = params.split(';');
                        let first = parts.next().and_then(|p| p.parse::<usize>().ok());
                        let second = parts.next().and_then(|p| p.parse::<usize>().ok());
                        row = first.unwrap_or(1).saturating_sub(1);
                        col = second.unwrap_or(1).saturating_sub(1);
                    }
                    'J' if params == "2" => grid = vec![vec![(' ', false); cols]; rows],
                    // watch writes every attribute as one reset-first
                    // sequence, so a 7 anywhere in it means reverse and
                    // its absence means the attribute is off again.
                    'm' => reverse = params.split(';').any(|p| p == "7"),
                    _ => {}
                }
            }
            '\r' => col = 0,
            '\n' => {
                row += 1;
                col = 0;
            }
            _ if ch.is_control() => {}
            _ => {
                if row < rows && col < cols {
                    grid[row][col] = (ch, reverse);
                }
                col += 1;
            }
        }
    }
    grid
}

/// The characters of each row, which is what most cases assert on.
fn render(raw: &[u8], rows: usize, cols: usize) -> Vec<String> {
    render_cells(raw, rows, cols)
        .into_iter()
        .map(|row| row.into_iter().map(|(ch, _)| ch).collect())
        .collect()
}

/// Run watch with plain pipes, for the cases that never reach a screen.
fn plain(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env_remove("LINES")
        .env_remove("COLUMNS")
        .env_remove("WATCH_INTERVAL");
    for (name, value) in env {
        command.env(OsStr::new(name), OsStr::new(value));
    }
    command.output().expect("run watch")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_header_shows_interval_command_and_host() {
    let mut run = Run::new(&["-n", "0.5", "echo", "hi"], 24, 80);
    let host = hostname();
    let screen = run.wait_for("the first frame", |screen| {
        screen[0].starts_with("Every 0.5s: echo hi") && screen[2].trim() == "hi"
    });
    assert!(
        screen[0].contains(&format!("{host}: ")),
        "the header should name the host: {:?}",
        screen[0]
    );
    // The timing row is right-aligned and reports the exit status.
    assert!(
        screen[1].trim_start().starts_with("in ") && screen[1].ends_with("(0)"),
        "unexpected timing row: {:?}",
        screen[1]
    );
}

#[test]
fn no_title_hides_the_header() {
    let mut run = Run::new(&["-t", "-n", "0.2", "echo", "hi"], 24, 80);
    let screen = run.wait_for("the output on the first row", |screen| {
        screen[0].trim() == "hi"
    });
    assert!(
        !screen.iter().any(|row| row.contains("Every")),
        "no header was asked for: {screen:?}"
    );
}

#[test]
fn a_long_command_is_trimmed_to_what_the_header_has_room_for() {
    // The right half of the header is the host and the time, whose
    // widths belong to the machine, so what the command has left is
    // read off the frame rather than assumed.
    let command = format!("echo {}", "a".repeat(200));
    let host = hostname();
    let mut run = Run::new(&["-n", "2", &command], 24, 80);
    let marker = format!("{host}: ");
    let screen = run.wait_for("a header with the host", |screen| {
        screen[0].contains(&marker)
    });
    let left = "Every 2.0s: ";
    let right_at = screen[0].find(&marker).expect("the host in the header");
    let avail = right_at - left.len();
    assert!(
        avail > 3,
        "the header should have room for a trimmed command"
    );
    let shown = screen[0][left.len()..right_at].trim_end();
    // Whatever fits, less four columns, and then the three that say so.
    assert_eq!(shown.len(), avail - 1, "unexpected width: {shown:?}");
    assert!(
        shown.ends_with("..."),
        "expected a trimmed command: {shown:?}"
    );
    assert!(
        command.starts_with(&shown[..shown.len() - 3]),
        "the trimmed command should be a prefix of the whole one: {shown:?}"
    );
}

#[test]
fn a_bad_interval_is_reported_before_the_terminal_is_touched() {
    for argument in ["abc", "1e3", "", "1.2.3", "5s"] {
        let output = plain(&["-n", argument, "echo", "hi"], &[]);
        assert_eq!(output.status.code(), Some(1), "for -n {argument:?}");
        assert_eq!(
            stderr_of(&output),
            format!("watch: failed to parse argument: '{argument}': Invalid argument\n"),
            "for -n {argument:?}"
        );
        assert_eq!(stdout_of(&output), "");
    }
}

#[test]
fn an_interval_takes_a_comma_radix_and_is_clamped_in_silence() {
    let mut comma = Run::new(&["-n", "1,5", "echo", "hi"], 24, 80);
    comma.wait_for("a comma radix read as a decimal point", |screen| {
        screen[0].starts_with("Every 1.5s: echo hi")
    });

    let mut zero = Run::new(&["-n", "0", "echo", "hi"], 24, 80);
    zero.wait_for("an interval clamped up to the minimum", |screen| {
        screen[0].starts_with("Every 0.1s: echo hi")
    });

    let mut huge = Run::new(&["-n", "99999999", "echo", "hi"], 24, 80);
    huge.wait_for("an interval clamped down to the maximum", |screen| {
        screen[0].starts_with("Every 2678400.0s: echo hi")
    });
}

#[test]
fn watch_interval_sets_the_default_and_loses_to_an_option() {
    let mut from_env = Run::with_env(&["echo", "hi"], 24, 80, &[("WATCH_INTERVAL", "0.5")]);
    from_env.wait_for("the interval from the environment", |screen| {
        screen[0].starts_with("Every 0.5s: echo hi")
    });

    let mut overridden = Run::with_env(
        &["-n", "1.5", "echo", "hi"],
        24,
        80,
        &[("WATCH_INTERVAL", "0.5")],
    );
    overridden.wait_for("the interval from the command line", |screen| {
        screen[0].starts_with("Every 1.5s: echo hi")
    });

    let output = plain(&["echo", "hi"], &[("WATCH_INTERVAL", "abc")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output)
            .starts_with("watch: Could not parse interval from WATCH_INTERVAL: 'abc'"),
        "unexpected message: {:?}",
        stderr_of(&output)
    );
}

#[test]
fn chgexit_returns_when_the_output_changes() {
    let mut run = Run::new(&["-g", "-n", "0.2", "date"], 24, 80);
    assert_eq!(run.wait_exit().code(), Some(0));
}

#[test]
fn equexit_returns_when_the_output_settles() {
    let mut run = Run::new(&["-q", "2", "-n", "0.2", "echo", "constant"], 24, 80);
    assert_eq!(run.wait_exit().code(), Some(0));
}

#[test]
fn errexit_waits_for_a_key_then_exits_with_the_command_status() {
    let mut run = Run::new(&["-e", "-n", "0.2", "exit", "3"], 24, 80);
    run.wait_for("the errexit message on the last row", |screen| {
        screen[23].starts_with("command exit with a non-zero status, press a key to exit")
    });
    run.key(b"x");
    assert_eq!(run.wait_exit().code(), Some(3));
}

#[test]
fn exec_skips_the_shell() {
    let mut shell = Run::new(&["-n", "0.2", "echo $((3+4))"], 24, 80);
    shell.wait_for("the shell's arithmetic", |screen| screen[2].trim() == "7");

    let mut direct = Run::new(&["-x", "-n", "0.2", "echo", "$((3+4))"], 24, 80);
    direct.wait_for("the unevaluated text", |screen| {
        screen[2].trim() == "$((3+4))"
    });
}

#[test]
fn the_q_key_quits_and_restores_the_terminal() {
    let mut run = Run::new(&["-n", "0.2", "echo", "hi"], 24, 80);
    run.wait_for("the first frame", |screen| screen[2].trim() == "hi");
    run.key(b"q");
    assert_eq!(run.wait_exit().code(), Some(0));
    run.wait_for_bytes(b"\x1b[?1049l", "the alternate screen being left");
}

#[test]
fn long_lines_wrap_unless_no_wrap_is_given() {
    let command = "printf '%100s' | tr ' ' x";
    let mut wrapped = Run::new(&["-n", "0.2", command], 24, 80);
    wrapped.wait_for("a wrapped line", |screen| {
        screen[2] == "x".repeat(80) && screen[3].trim() == "x".repeat(20)
    });

    let mut truncated = Run::new(&["-w", "-n", "0.2", command], 24, 80);
    truncated.wait_for("a truncated line", |screen| {
        screen[2] == "x".repeat(80) && screen[3].trim().is_empty()
    });

    // A line that fills the row exactly and ends there, with no newline
    // behind it: the wrap it is owed must not become a blank row, and
    // the rows below it still have to be cleared.
    let exact = "printf '%80s' | tr ' ' x";
    let mut boundary = Run::new(&["-n", "0.2", exact], 24, 80);
    boundary.wait_for("a line that ends on the margin", |screen| {
        screen[2] == "x".repeat(80) && screen[3].trim().is_empty()
    });
}

#[test]
fn tabs_advance_to_the_next_multiple_of_eight() {
    let mut run = Run::new(&["-n", "0.2", "printf 'a\\tb\\nabcdefgh\\tc\\n'"], 24, 80);
    let screen = run.wait_for("the tabbed output", |screen| screen[2].starts_with('a'));
    assert_eq!(screen[2].trim_end(), "a       b");
    assert_eq!(screen[3].trim_end(), "abcdefgh        c");
}

#[test]
fn color_sequences_are_interpreted_only_with_the_option() {
    let command = "printf '\\033[31mred\\033[0m\\n'";
    let mut plainly = Run::new(&["-n", "0.2", command], 24, 80);
    plainly.wait_for("the escape shown as text", |screen| {
        screen[2].trim_end() == "[31mred[0m"
    });

    let mut colored = Run::new(&["-c", "-n", "0.2", command], 24, 80);
    colored.wait_for("the colored text", |screen| screen[2].trim_end() == "red");
    colored.wait_for_bytes(b"\x1b[0;31m", "a red foreground");
}

/// A counter file one case owns, removed with it, so the command a case
/// runs can tell one run from the next and change its output in a way
/// the case knows the shape of ahead of time.
struct Counter {
    path: PathBuf,
}

impl Counter {
    fn new(tag: &str) -> Counter {
        let path = std::env::temp_dir().join(format!("watch-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Counter { path }
    }

    /// The shell that advances the counter and leaves it in `$n`.
    fn bump(&self) -> String {
        let file = self.path.display();
        format!("n=$(cat {file} 2>/dev/null || echo 0); n=$((n+1)); printf %s \"$n\" > {file}")
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A command whose output moves one cell at a time: `keep 1 x`, then
/// `keep 2 x`, then `keep 2 y` and nothing further. Column 5 changes
/// between the first two frames and column 7 between the next two, so a
/// case can say exactly which cell `-d` should have marked and which it
/// should have left alone.
fn stepping_command(counter: &Counter) -> String {
    format!(
        "{}; [ \"$n\" -ge 2 ] && a=2 || a=1; [ \"$n\" -ge 3 ] && b=y || b=x; echo \"keep $a $b\"",
        counter.bump()
    )
}

/// Step to the next frame with the space key and wait for it to land.
/// The interval these cases use is long enough that nothing arrives on
/// its own, so a frame on the screen is one the case asked for.
fn step_to(run: &mut Run, body: &str) {
    run.key(b" ");
    run.wait_for(body, |screen| screen[2].starts_with(body));
}

#[test]
fn nothing_is_highlighted_on_the_first_screen() {
    let counter = Counter::new("first");
    let command = stepping_command(&counter);
    let mut run = Run::new(&["-d", "-x", "-n", "30", "sh", "-c", &command], 24, 80);
    run.wait_for("keep 1 x", |screen| screen[2].starts_with("keep 1 x"));
    let reverse = run.reverse();
    assert!(
        reverse.iter().all(|row| row.iter().all(|cell| !cell)),
        "the first screen has nothing to compare against, so nothing is marked"
    );
}

#[test]
fn differences_highlight_only_the_cells_that_changed() {
    let counter = Counter::new("diff");
    let command = stepping_command(&counter);
    let mut run = Run::new(&["-d", "-x", "-n", "30", "sh", "-c", &command], 24, 80);
    run.wait_for("keep 1 x", |screen| screen[2].starts_with("keep 1 x"));

    step_to(&mut run, "keep 2 x");
    let reverse = run.reverse();
    assert!(reverse[2][5], "the digit that changed should be marked");
    assert!(
        reverse[2][..5].iter().all(|cell| !cell) && !reverse[2][6] && !reverse[2][7],
        "the text beside it did not change and should not be marked"
    );

    step_to(&mut run, "keep 2 y");
    let reverse = run.reverse();
    assert!(reverse[2][7], "the letter that changed should be marked");
    assert!(
        !reverse[2][5],
        "a cell that has stopped changing loses its mark"
    );
}

#[test]
fn cumulative_differences_keep_a_cell_marked() {
    let counter = Counter::new("permanent");
    let command = stepping_command(&counter);
    let mut run = Run::new(
        &["-dpermanent", "-x", "-n", "30", "sh", "-c", &command],
        24,
        80,
    );
    run.wait_for("keep 1 x", |screen| screen[2].starts_with("keep 1 x"));

    step_to(&mut run, "keep 2 x");
    assert!(
        run.reverse()[2][5],
        "the digit that changed should be marked"
    );

    step_to(&mut run, "keep 2 y");
    let reverse = run.reverse();
    assert!(reverse[2][7], "the letter that changed should be marked");
    assert!(
        reverse[2][5],
        "an argument to -d keeps a cell marked once it has changed"
    );
}

#[test]
fn a_failed_run_leaves_through_errexit_even_when_the_screen_changed() {
    // Upstream answers the command's status before it compares screens,
    // so a run that both failed and changed exits through -e with the
    // command's status rather than through -g with a zero one.
    let counter = Counter::new("errexit-chgexit");
    let command = format!(
        "{}; echo \"run $n\"; [ \"$n\" -ge 2 ] && exit 3; exit 0",
        counter.bump()
    );
    let mut run = Run::new(
        &["-e", "-g", "-x", "-n", "0.3", "sh", "-c", &command],
        24,
        80,
    );
    run.wait_for("the errexit message", |screen| {
        screen[23].starts_with("command exit with a non-zero status, press a key to exit")
    });
    run.key(b"x");
    assert_eq!(run.wait_exit().code(), Some(3));
}

#[test]
fn usage_errors_and_unsupported_options() {
    let no_command = plain(&[], &[]);
    assert_eq!(no_command.status.code(), Some(1));
    assert_eq!(stderr_of(&no_command), USAGE);
    assert_eq!(stdout_of(&no_command), "");

    let bad_option = plain(&["-z", "echo", "hi"], &[]);
    assert_eq!(bad_option.status.code(), Some(1));
    assert_eq!(
        stderr_of(&bad_option),
        format!("watch: invalid option -- 'z'\n{USAGE}")
    );

    for (option, name) in [("--follow", "--follow"), ("--shotsdir=/tmp", "--shotsdir")] {
        let output = plain(&[option, "echo", "hi"], &[]);
        assert_eq!(output.status.code(), Some(1), "for {option}");
        assert_eq!(
            stderr_of(&output),
            format!(
                "watch: {name} is not implemented in this build\n\
                 See https://github.com/jordiboehme/agent-cli-tools/issues to request it\n"
            ),
            "for {option}"
        );
    }

    let help = plain(&["-h"], &[]);
    assert_eq!(help.status.code(), Some(0));
    assert_eq!(stdout_of(&help), USAGE);
    assert_eq!(stderr_of(&help), "");

    let version = plain(&["-v"], &[]);
    assert_eq!(version.status.code(), Some(0));
    assert!(
        stdout_of(&version).starts_with(&format!(
            "watch (agent-cli-tools) {}\n",
            env!("CARGO_PKG_VERSION")
        )),
        "unexpected version text: {:?}",
        stdout_of(&version)
    );
}

#[test]
fn flags_after_the_command_belong_to_the_command() {
    let mut run = Run::new(&["-n", "0.2", "echo", "-t"], 24, 80);
    let screen = run.wait_for("the command's own flag", |screen| screen[2].trim() == "-t");
    assert!(
        screen[0].starts_with("Every 0.2s: echo -t"),
        "the header should show the whole command: {:?}",
        screen[0]
    );
}
