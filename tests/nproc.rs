//! Integration tests that drive the built `nproc` binary.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_nproc");
const TRY: &str = "Try 'nproc --help' for more information.\n";

/// A command with the ambient environment's OpenMP variables scrubbed, so
/// the test's expectations are never at the mercy of whatever the shell
/// running the suite happens to export.
fn command(args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(args);
    cmd.env_remove("OMP_NUM_THREADS");
    cmd.env_remove("OMP_THREAD_LIMIT");
    cmd
}

fn run(args: &[&str]) -> Output {
    command(args).output().expect("spawn nproc")
}

fn run_env(vars: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = command(args);
    for (key, value) in vars {
        cmd.env(key, value);
    }
    cmd.output().expect("spawn nproc")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited with a status")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The plain, no-argument answer, used as "however many CPUs this machine
/// has" in tests that don't care about the exact number.
fn cpus() -> u64 {
    stdout(&run(&[])).trim().parse().expect("a number")
}

#[test]
fn prints_a_positive_number() {
    let out = run(&[]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    let n: u64 = stdout(&out).trim().parse().expect("a number");
    assert!(n >= 1);

    let all_out = run(&["--all"]);
    assert_eq!(code(&all_out), 0);
    assert_eq!(stderr(&all_out), "");
    let all_n: u64 = stdout(&all_out).trim().parse().expect("a number");
    assert!(all_n >= 1);

    // On macOS both _SC_NPROCESSORS_ONLN and _SC_NPROCESSORS_CONF resolve
    // to hw.ncpu, so the online and installed counts always agree here.
    assert_eq!(n, all_n);
}

#[test]
fn omp_num_threads_overrides_the_count() {
    let cpus = cpus().to_string();
    let cases: &[(&str, &str)] = &[
        ("1", "1"),
        ("2", "2"),
        ("0", cpus.as_str()),
        ("-1", cpus.as_str()),
        ("2,2,1", "2"),
        ("2,ignored", "2"),
        ("2bad", cpus.as_str()),
        (" 3 ,x", "3"),
        ("99999999999999999999999", "18446744073709551615"),
    ];
    for (value, expected) in cases {
        let out = run_env(&[("OMP_NUM_THREADS", value)], &[]);
        assert_eq!(code(&out), 0, "value {value:?}");
        assert_eq!(stdout(&out), format!("{expected}\n"), "value {value:?}");
    }
}

#[test]
fn omp_thread_limit_caps_the_count() {
    let cpus = cpus().to_string();

    let out = run_env(&[("OMP_THREAD_LIMIT", "1")], &[]);
    assert_eq!(stdout(&out), "1\n");

    let out = run_env(&[("OMP_NUM_THREADS", "3"), ("OMP_THREAD_LIMIT", "2")], &[]);
    assert_eq!(stdout(&out), "2\n");

    let out = run_env(&[("OMP_THREAD_LIMIT", "0")], &[]);
    assert_eq!(stdout(&out), format!("{cpus}\n"));

    let out = run_env(&[("OMP_THREAD_LIMIT", " 1,2")], &[]);
    assert_eq!(stdout(&out), "1\n");

    // --all disregards both OpenMP variables entirely.
    let out = run_env(&[("OMP_NUM_THREADS", "1")], &["--all"]);
    assert_eq!(stdout(&out), format!("{cpus}\n"));
}

#[test]
fn ignore_subtracts_but_never_goes_below_one() {
    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore=40"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "2\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore=41"]);
    assert_eq!(stdout(&out), "1\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore=42"]);
    assert_eq!(stdout(&out), "1\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore=0"]);
    assert_eq!(stdout(&out), "42\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore= 1"]);
    assert_eq!(stdout(&out), "41\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore=+1"]);
    assert_eq!(stdout(&out), "41\n");

    let out = run_env(
        &[("OMP_NUM_THREADS", "42")],
        &["--ignore=18446744073709551616"],
    );
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "1\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore", "2"]);
    assert_eq!(stdout(&out), "40\n");

    let out = run_env(&[("OMP_NUM_THREADS", "42")], &["--ignore=1", "--ignore=2"]);
    assert_eq!(stdout(&out), "40\n");
}

#[test]
fn rejects_an_invalid_ignore_value() {
    for value in ["x", "-1", "2x", "2k", "0x2", "2 ", "1 2", "1,2", ""] {
        let arg = format!("--ignore={value}");
        let out = run(&[&arg]);
        assert_eq!(code(&out), 1, "value {value:?}");
        assert_eq!(
            stderr(&out),
            format!("nproc: invalid number: '{value}'\n{TRY}"),
            "value {value:?}"
        );
    }
}

#[test]
fn operands_are_an_error() {
    let out = run(&["x"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr(&out), format!("nproc: extra operand 'x'\n{TRY}"));

    let out = run(&["x", "y"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr(&out), format!("nproc: extra operand 'x'\n{TRY}"));

    let out = run(&["-"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr(&out), format!("nproc: extra operand '-'\n{TRY}"));

    let out = run(&["--", "x"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr(&out), format!("nproc: extra operand 'x'\n{TRY}"));
}

#[test]
fn option_errors() {
    let out = run(&["--bogus"]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out),
        format!("nproc: unrecognized option '--bogus'\n{TRY}")
    );

    let out = run(&["-a"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr(&out), format!("nproc: invalid option -- 'a'\n{TRY}"));

    let out = run(&["--ignore"]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out),
        format!("nproc: option '--ignore' requires an argument\n{TRY}")
    );
}

#[test]
fn help_and_version() {
    let out = run(&["--help"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.starts_with("Usage: nproc [OPTION]...\n"));
    assert!(text.contains("      --ignore=N\n"));
    assert!(text.contains("The result is guaranteed to be at least 1."));

    let out = run(&["--help", "--bogus"]);
    assert_eq!(code(&out), 0);

    let out = run(&["--bogus", "--help"]);
    assert_eq!(code(&out), 1);

    let out = run(&["--version"]);
    assert_eq!(code(&out), 0);
    let expected = format!("nproc (agent-cli-tools) {}\n", env!("CARGO_PKG_VERSION"));
    assert!(stdout(&out).starts_with(&expected), "got {}", stdout(&out));
    assert!(
        stdout(&out).ends_with("Home page: <https://github.com/jordiboehme/agent-cli-tools>\n")
    );
    assert!(stderr(&out).is_empty());
}
