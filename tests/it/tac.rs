//! Integration tests that drive the built `tac` binary. The cases come
//! from the upstream coreutils suite (tests/tac/tac.pl, tac-locale.sh and
//! the shell cases around them), which runs each one three ways: with the
//! input as a file operand, piped to standard input, and redirected into
//! standard input from a file. `check` does the same, because only the
//! piped form exercises a non-seekable input.

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_tac");
const TRY: &str = "Try 'tac --help' for more information.\n";

/// A path under the system temp directory that no other test will pick,
/// so cases needing a real file never collide while the suite runs in
/// parallel.
fn temp_path(tag: &str) -> std::path::PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("tac-test-{}-{tag}-{n}", std::process::id()))
}

/// Run with no standard input at all, for cases that must fail before
/// they ever read anything: an open stdin would hang the suite instead.
fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn tac")
}

fn run_with_file(args: &[&str], input: &[u8]) -> Output {
    let path = temp_path("operand");
    std::fs::write(&path, input).expect("write input");
    let out = Command::new(BIN)
        .args(args)
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("spawn tac");
    std::fs::remove_file(&path).ok();
    out
}

fn run_piped(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tac");
    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin.write_all(input).expect("write input");
    drop(stdin);
    child.wait_with_output().expect("wait for tac")
}

fn run_redirected(args: &[&str], input: &[u8]) -> Output {
    let path = temp_path("stdin");
    std::fs::write(&path, input).expect("write input");
    let file = std::fs::File::open(&path).expect("open input");
    let out = Command::new(BIN)
        .args(args)
        .stdin(Stdio::from(file))
        .output()
        .expect("spawn tac");
    std::fs::remove_file(&path).ok();
    out
}

/// Assert one case all three ways: the answer must not depend on whether
/// the input arrived as a file operand, down a pipe, or on a redirected
/// standard input.
fn check(args: &[&str], input: &[u8], expected: &[u8]) {
    let runs: [(&str, Output); 3] = [
        ("file operand", run_with_file(args, input)),
        ("piped stdin", run_piped(args, input)),
        ("redirected stdin", run_redirected(args, input)),
    ];
    for (how, out) in runs {
        let context = format!("{args:?} on {input:?} via {how}");
        assert_eq!(out.status.code(), Some(0), "{context}");
        assert!(out.stderr.is_empty(), "{context}: stderr {:?}", out.stderr);
        assert_eq!(
            out.stdout,
            expected,
            "{context}: got {:?}, want {:?}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(expected)
        );
    }
}

#[test]
fn basic() {
    check(&[], b"", b"");
    check(&[], b"a", b"a");
    check(&[], b"\n", b"\n");
    check(&[], b"a\n", b"a\n");
    check(&[], b"a\nb", b"ba\n");
    check(&[], b"a\nb\n", b"b\na\n");
    check(&[], b"a\nb\nc", b"cb\na\n");
    check(&[], b"a\nb\nc\n", b"c\nb\na\n");
}

#[test]
fn buffer_boundaries() {
    check(&[], b"12345678\n9\n", b"9\n12345678\n");
    check(&[], b"1234567\n8\n", b"8\n1234567\n");
    check(&[], b"123456\n7\n", b"7\n123456\n");
    check(&[], b"12345\n6\n", b"6\n12345\n");
    check(&[], b"1234\n5\n", b"5\n1234\n");
    check(&[], b"123\n4\n", b"4\n123\n");

    // Upstream's double-free case: one record longer than any buffer the
    // reference implementation would have used, with no separator at all.
    let long = vec![b'o'; 16385];
    check(&[], &long, &long);
}

#[test]
fn nul_separator_when_the_separator_is_empty() {
    check(&["-s", ""], b"", b"");
    check(&["-s", ""], b"a", b"a");
    check(&["-s", ""], b"\0", b"\0");
    check(&["-s", ""], b"a\0", b"a\0");
    check(&["-s", ""], b"a\0b", b"ba\0");
    check(&["-s", ""], b"a\0b\0", b"b\0a\0");
}

#[test]
fn before_attaches_the_separator_to_the_next_record() {
    check(&["-b"], b"\na\nb\nc", b"\nc\nb\na");
    check(&["-b"], b"a\nb\n", b"\n\nba");
    check(&["-b"], b"ab", b"ab");
}

#[test]
fn string_separators() {
    check(&["-s", ":"], b"a:b:c:", b"c:b:a:");
    check(&["-s", ","], b"1,2,3", b"32,1,");
    check(&["-s", ","], b"1,2,3,", b"3,2,1,");
    check(&["-s", ":"], b"100:200:300:400:500", b"500400:300:200:100:");
    check(&["-s", "xx"], b"axxx", b"axxx");
    check(&["-s", "xx"], b"axxxx", b"xxaxx");
}

#[test]
fn before_with_a_string_separator() {
    check(&["-s", ":", "-b"], b":a:b:c", b":c:b:a");
    check(
        &["-b", "-s", ":"],
        b"100:200:300:400:500",
        b":500:400:300:200100",
    );

    // No upstream case pairs -b with a multi-byte separator. Matches stay
    // non-overlapping in this mode too, so the backward scan steps a whole
    // separator back after a hit and every record but the leading one
    // begins with a full separator: "ax" then "xxb", never a stray "x".
    check(&["-b", "-s", "xx"], b"axxxb", b"xxbax");
    check(&["-b", "-s", "xx"], b"axxxx", b"xxxxa");
}

#[test]
fn regex_separators() {
    check(
        &["-r", "-s", r"\._+"],
        b"1._2.__3.___4._",
        b"4._3.___2.__1._",
    );
    check(
        &["-r", "-s", r"\._+"],
        b"a.___b.__1._2.__3.___4._",
        b"4._3.___2.__1._b.__a.___",
    );
    check(&["-r", "-s", "[0-9]"], b"a1b2c3", b"c3b2a1");
    check(&["-r", "-s", "[0-9]"], b"a1b2c", b"cb2a1");
    check(&["-r", "-s", "[xyz]+"], b"axyz", b"zyax");
    check(&["-r", "-s", ":+"], b"a:b::c:::d::::", b":::d:::c::b:a:");
}

#[test]
fn emacs_regex_syntax_is_translated() {
    // GNU compiles the separator as an Emacs regexp, where grouping and
    // alternation are spelled with a backslash and the bare characters are
    // ordinary literals.
    check(&["-r", "-s", r"\(x\|y\)"], b"1x2y3", b"32y1x");
    check(&["-r", "-s", "|"], b"a|b|", b"b|a|");
    check(&["-r", "-s", "("], b"a(b(", b"b(a(");
    check(&["-r", "-s", ")"], b"a)b)", b"b)a)");
    check(&["-r", "-s", "{"], b"a{b{", b"b{a{");
    check(&["-r", "-s", "}"], b"a}b}", b"b}a}");
}

#[test]
fn regex_anchors() {
    check(&["-r", "-s", "^"], b"a\nb\nc\n", b"c\nb\na\n");
    check(&["-r", "-s", "$"], b"a\nb\nc\n", b"\n\nc\nba");
    check(&["-r", "-s", "^$"], b"a\nb\nc\n", b"a\nb\nc\n");
}

#[test]
fn before_with_a_regex_separator() {
    check(
        &["-b", "-r", "-s", r"\._+"],
        b"._1._2.__3.___4",
        b".___4.__3._2._1",
    );
    check(
        &["-b", "-r", "-s", r"\._+"],
        b".__x.___y.____z._1._2.__3.___4",
        b".___4.__3._2._1.____z.___y.__x",
    );
}

#[test]
fn a_multibyte_separator_is_treated_as_bytes() {
    // U+00E9 is two bytes in UTF-8, and tac must match the pair, not
    // either byte on its own.
    check(
        &["--separator=\u{e9}"],
        b"1\xc3\xa92\xc3\xa93\xc3\xa9",
        b"3\xc3\xa92\xc3\xa91\xc3\xa9",
    );
}

#[test]
fn several_files_are_reversed_separately() {
    let a = temp_path("multi-a");
    let b = temp_path("multi-b");
    std::fs::write(&a, b"a\n").unwrap();
    std::fs::write(&b, b"b\n").unwrap();
    let out = run(&["-r", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"a\nb\n");

    std::fs::write(&a, b"a\nb\n").unwrap();
    std::fs::write(&b, b"1\n2\n").unwrap();
    let out = run(&["-r", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"b\na\n2\n1\n");

    let out = run(&[a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"b\na\n2\n1\n");

    std::fs::remove_file(&a).ok();
    std::fs::remove_file(&b).ok();
}

#[test]
fn stdin_dash_may_repeat() {
    let out = run_piped(&["-", "-"], b"x\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"x\n");
    assert!(out.stderr.is_empty());
}

#[test]
fn a_missing_file_does_not_stop_the_others() {
    let good = temp_path("survivor");
    std::fs::write(&good, b"a\nb\n").unwrap();
    let out = run(&["nosuch", good.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "tac: failed to open 'nosuch' for reading: No such file or directory\n"
    );
    assert_eq!(out.stdout, b"b\na\n");
    std::fs::remove_file(&good).ok();
}

#[test]
fn a_directory_operand_is_a_read_error() {
    let dir = temp_path("dir");
    std::fs::create_dir(&dir).unwrap();
    let out = run(&[dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("tac: {}: read error: Is a directory\n", dir.display())
    );
    std::fs::remove_dir(&dir).ok();
}

#[test]
fn an_empty_regex_separator_is_rejected() {
    let out = run(&["-r", "-s", ""]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "tac: separator cannot be empty\n"
    );

    // Without -r an empty separator is not an error at all: it means NUL.
    let out = run(&["-s", "", "/dev/null"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
}

#[test]
fn a_bad_regex_is_reported() {
    let out = run(&["-r", "-s", "["]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(text.starts_with("tac: "), "got {text:?}");
    assert!(text.ends_with('\n'), "got {text:?}");
    assert!(!text.contains(TRY), "got {text:?}");
}

#[test]
fn option_errors() {
    let out = run(&["-x"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("tac: invalid option -- 'x'\n{TRY}")
    );

    let out = run(&["--foo"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("tac: unrecognized option '--foo'\n{TRY}")
    );

    let out = run(&["-s"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("tac: option requires an argument -- 's'\n{TRY}")
    );

    let out = run(&["--separator"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("tac: option '--separator' requires an argument\n{TRY}")
    );
}

#[test]
fn help_and_version() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.starts_with("Usage: tac [OPTION]... [FILE]...\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "  -r, --regex              interpret the separator as a regular expression\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("  -b, --before             attach the separator before instead of after\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "  -s, --separator=STRING   use STRING as the separator instead of newline\n"
        ),
        "{text}"
    );

    // --help must act the moment it is reached, even beside a bad option.
    let out = run(&["--help", "--foo"]);
    assert_eq!(out.status.code(), Some(0));
    let out = run(&["--foo", "--help"]);
    assert_eq!(out.status.code(), Some(1));

    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let expected = format!("tac (agent-cli-tools) {}\n", env!("CARGO_PKG_VERSION"));
    assert!(text.starts_with(&expected), "got {text}");
    assert!(text.ends_with("Home page: <https://github.com/jordiboehme/agent-cli-tools>\n"));
    assert!(out.stderr.is_empty());
}
