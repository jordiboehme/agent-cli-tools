//! Integration tests that drive the built `timeout` binary.

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_timeout");
const TRY: &str = "Try 'timeout --help' for more information.\n";

fn timeout(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn timeout")
}

fn timed(args: &[&str]) -> (Output, Duration) {
    let start = Instant::now();
    let output = timeout(args);
    (output, start.elapsed())
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .unwrap_or_else(|| panic!("killed by signal {:?}", output.status.signal()))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// --- argument parsing, help, version -------------------------------------

#[test]
fn no_operands_prints_only_the_try_line() {
    for args in [&[][..], &["5"][..], &["-v"][..]] {
        let out = timeout(args);
        assert_eq!(code(&out), 125);
        assert_eq!(stderr(&out), TRY);
    }
}

#[test]
fn invalid_duration() {
    let out = timeout(&["abc", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: invalid time interval 'abc'\n{TRY}")
    );

    let out = timeout(&["-k", "xyz", "1", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: invalid time interval 'xyz'\n{TRY}")
    );
}

#[test]
fn invalid_signal() {
    let out = timeout(&["-s", "FOO", "1", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: 'FOO': invalid signal\n{TRY}")
    );
}

#[test]
fn unknown_options() {
    let out = timeout(&["-x", "1", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: invalid option -- 'x'\n{TRY}")
    );

    let out = timeout(&["--bogus", "1", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: unrecognized option '--bogus'\n{TRY}")
    );

    let out = timeout(&["--v", "1", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!(
            "timeout: option '--v' is ambiguous; possibilities: '--verbose' '--version'\n{TRY}"
        )
    );

    let out = timeout(&["--verbose=1", "1", "true"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: option '--verbose' doesn't allow an argument\n{TRY}")
    );
}

#[test]
fn missing_option_arguments() {
    let out = timeout(&["-k"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: option requires an argument -- 'k'\n{TRY}")
    );

    let out = timeout(&["--kill-after"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: option '--kill-after' requires an argument\n{TRY}")
    );
}

#[test]
fn help_and_version() {
    let out = timeout(&["--help"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).starts_with("Usage: timeout [OPTION]... DURATION COMMAND [ARG]...\n"));
    assert!(stdout(&out).contains("  -k, --kill-after=DURATION\n"));
    assert!(
        stdout(&out)
            .contains("  124  if COMMAND times out, and --preserve-status is not specified\n")
    );

    let out = timeout(&["--version"]);
    assert_eq!(code(&out), 0);
    let expected = format!("timeout (agent-cli-tools) {}\n", env!("CARGO_PKG_VERSION"));
    assert!(stdout(&out).starts_with(&expected), "got {}", stdout(&out));
    assert!(
        stdout(&out).ends_with("Home page: <https://github.com/jordiboehme/agent-cli-tools>\n")
    );
    assert!(stderr(&out).is_empty());

    // Long option prefixes are accepted and --help wins as soon as it is seen.
    let out = timeout(&["--he"]);
    assert_eq!(code(&out), 0);
}

#[test]
fn help_wins_over_a_later_bad_option() {
    let out = timeout(&["--help", "--bogus"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).starts_with("Usage: timeout"));

    let out = timeout(&["--bogus", "--help"]);
    assert_eq!(code(&out), 125);
    assert_eq!(
        stderr(&out),
        format!("timeout: unrecognized option '--bogus'\n{TRY}")
    );
}

// --- running commands ------------------------------------------------------

#[test]
fn passes_the_exit_status_through() {
    assert_eq!(code(&timeout(&["10", "true"])), 0);
    assert_eq!(code(&timeout(&["10", "false"])), 1);
    assert_eq!(code(&timeout(&["10", "sh", "-c", "exit 3"])), 3);
    assert_eq!(stdout(&timeout(&["10", "echo", "hi"])), "hi\n");
}

#[test]
fn option_parsing_stops_at_the_duration() {
    let out = timeout(&["10", "echo", "-v", "--help", "x"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "-v --help x\n");

    let out = timeout(&["--", "10", "echo", "y"]);
    assert_eq!(stdout(&out), "y\n");
}

#[test]
fn zero_disables_the_timeout() {
    let out = timeout(&["0", "sh", "-c", "sleep 0.3; exit 7"]);
    assert_eq!(code(&out), 7);
}

#[test]
fn times_out_with_124() {
    let (out, elapsed) = timed(&["0.2", "sleep", "10"]);
    assert_eq!(code(&out), 124);
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    assert!(stderr(&out).is_empty());
}

#[test]
fn duration_suffixes_and_forms_work_end_to_end() {
    assert_eq!(code(&timeout(&["1m", "true"])), 0);
    assert_eq!(code(&timeout(&["0.1h", "true"])), 0);
    assert_eq!(code(&timeout(&["inf", "true"])), 0);
    assert_eq!(code(&timeout(&["1e-3", "sleep", "5"])), 124);
    assert_eq!(code(&timeout(&["1e-10000", "sleep", "5"])), 124);
}

#[test]
fn preserve_status_reports_how_the_command_ended() {
    let out = timeout(&["-p", "0.2", "sleep", "10"]);
    assert_eq!(code(&out), 143);

    let out = timeout(&[
        "--pres",
        "0.2",
        "sh",
        "-c",
        "trap 'exit 3' TERM; sleep 10; exit 0",
    ]);
    assert_eq!(code(&out), 3);

    // Without a timeout -p changes nothing.
    assert_eq!(code(&timeout(&["-p", "10", "sh", "-c", "exit 5"])), 5);
}

#[test]
fn verbose_names_the_signal_and_the_command() {
    let out = timeout(&["-v", "0.2", "sleep", "10"]);
    assert_eq!(code(&out), 124);
    assert_eq!(
        stderr(&out),
        "timeout: sending signal TERM to command 'sleep'\n"
    );

    let out = timeout(&["-v", "-s", "143", "0.2", "sleep", "10"]);
    assert_eq!(
        stderr(&out),
        "timeout: sending signal TERM to command 'sleep'\n"
    );

    let out = timeout(&["-v", "-s", "sigint", "0.2", "sleep", "10"]);
    assert_eq!(code(&out), 124);
    assert_eq!(
        stderr(&out),
        "timeout: sending signal INT to command 'sleep'\n"
    );
}

#[test]
fn kill_signal_takes_the_group_down_including_timeout() {
    let out = timeout(&["-s", "KILL", "0.2", "sleep", "10"]);
    assert_eq!(out.status.signal(), Some(libc::SIGKILL));
}

#[test]
fn foreground_kill_signal_reports_137() {
    let out = timeout(&["--foreground", "-s", "KILL", "0.2", "sleep", "10"]);
    assert_eq!(code(&out), 137);
}

#[test]
fn kill_after_escalates_when_term_is_ignored() {
    let script = "trap '' TERM; exec sleep 10";
    let (out, elapsed) = timed(&["--foreground", "-v", "-k", "0.3", "0.2", "sh", "-c", script]);
    assert_eq!(code(&out), 137);
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    assert_eq!(
        stderr(&out),
        "timeout: sending signal TERM to command 'sh'\ntimeout: sending signal KILL to command 'sh'\n"
    );

    // Without --foreground the KILL goes to the group, timeout included.
    let (out, _) = timed(&["-k", "0.3", "0.2", "sh", "-c", script]);
    assert_eq!(out.status.signal(), Some(libc::SIGKILL));
}

#[test]
fn ignored_term_without_kill_after_waits_for_the_command() {
    let (out, elapsed) = timed(&["0.2", "sh", "-c", "trap '' TERM; exec sleep 0.6"]);
    assert_eq!(code(&out), 124);
    assert!(elapsed >= Duration::from_millis(550), "took {elapsed:?}");
}

#[test]
fn command_not_found_is_127() {
    let out = timeout(&["10", "no_such_command_xyz"]);
    assert_eq!(code(&out), 127);
    assert_eq!(
        stderr(&out),
        "timeout: failed to run command 'no_such_command_xyz': No such file or directory\n"
    );
}

#[test]
fn command_not_executable_is_126() {
    let path = std::env::temp_dir().join(format!("agent-cli-tools-noexec-{}", std::process::id()));
    std::fs::write(&path, "not a program").unwrap();
    let out = timeout(&["10", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert_eq!(code(&out), 126);
    assert_eq!(
        stderr(&out),
        format!(
            "timeout: failed to run command '{}': Permission denied\n",
            path.display()
        )
    );
}

#[test]
fn signals_the_whole_process_group_by_default() {
    let pidfile = std::env::temp_dir().join(format!("agent-cli-tools-pgrp-{}", std::process::id()));
    let script = format!("sleep 30 & echo $! > '{}'; wait", pidfile.display());
    let out = timeout(&["0.3", "sh", "-c", &script]);
    assert_eq!(code(&out), 124);
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::fs::remove_file(&pidfile).ok();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!alive(pid), "background sleep {pid} survived the timeout");
}

#[test]
fn foreground_signals_only_the_direct_child() {
    let pidfile = std::env::temp_dir().join(format!("agent-cli-tools-fg-{}", std::process::id()));
    // The grandchild must not hold the captured stdio, or output() would wait for it.
    let script = format!(
        "sleep 30 </dev/null >/dev/null 2>&1 & echo $! > '{}'; wait",
        pidfile.display()
    );
    let out = timeout(&["--foreground", "0.3", "sh", "-c", &script]);
    assert_eq!(code(&out), 124);
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::fs::remove_file(&pidfile).ok();
    let survived = alive(pid);
    unsafe { libc::kill(pid, libc::SIGKILL) };
    assert!(
        survived,
        "grandchild {pid} should not be signalled in --foreground mode"
    );
}

#[test]
fn forwards_sigint_and_dies_the_same_way() {
    let mut child = Command::new(BIN)
        .args(["10", "sleep", "10"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGINT));
}

#[test]
fn forwarded_signal_caught_by_the_command_yields_its_exit_status() {
    let mut child = Command::new(BIN)
        .args(["10", "sh", "-c", "trap 'exit 9' INT; sleep 10; exit 0"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(9));
}

/// The command starts with SIGPIPE at its default disposition, as it
/// does under GNU timeout: a writer whose reader has gone dies quietly
/// instead of reporting a write error.
#[test]
fn the_command_starts_with_sigpipe_at_its_default() {
    let out = timeout(&["5", "sh", "-c", "yes | head -1"]);
    assert_eq!(stderr(&out), "");
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "y\n");

    // And the disposition itself: a command that raises SIGPIPE on
    // itself dies of it, which timeout re-raises, so the status here is
    // signal 13 rather than an exit code.
    let out = timeout(&["5", "sh", "-c", "kill -s PIPE $$"]);
    assert_eq!(out.status.signal(), Some(libc::SIGPIPE));
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}
