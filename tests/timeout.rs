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
    assert!(stderr(&out).is_empty());

    // Long option prefixes are accepted and --help wins as soon as it is seen.
    let out = timeout(&["--he"]);
    assert_eq!(code(&out), 0);
}
