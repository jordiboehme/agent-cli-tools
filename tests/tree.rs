//! Integration tests that drive the built `tree` binary. Every case runs
//! in a fixture of its own, since the suite runs its tests in parallel
//! and several of them add files to the tree they list.
//!
//! Set `AGENT_CLI_TOOLS_TREE_ORACLE` to a real tree 2.3.2 binary to also
//! run the differential test at the bottom, which asserts byte-identical
//! output for the cases listed there.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_tree");
const ISSUES: &str = "https://github.com/jordiboehme/agent-cli-tools/issues";

/// The line-drawing pieces, written as escapes rather than pasted
/// characters: the vertical continuation is a bar and two NON-BREAKING
/// spaces, which a copy out of a terminal would quietly turn into
/// ordinary ones.
const TEE: &str = "\u{251c}\u{2500}\u{2500} ";
const END: &str = "\u{2514}\u{2500}\u{2500} ";
const BAR: &str = "\u{2502}\u{a0}\u{a0} ";
const GAP: &str = "    ";

/// The fixture from the reference document, built fresh per test in a
/// directory named after it and removed when the test ends.
struct Demo {
    dir: PathBuf,
}

impl Demo {
    fn new(tag: &str) -> Demo {
        let dir = std::env::temp_dir().join(format!("tree-test-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let demo = dir.join("demo");
        for sub in ["docs", "node_modules/pkg", "src"] {
            fs::create_dir_all(demo.join(sub)).expect("create fixture directory");
        }
        fs::write(demo.join(".gitignore"), ".env\n").expect("write fixture file");
        for file in [
            "Cargo.toml",
            "README.md",
            "docs/guide.md",
            "node_modules/pkg/index.js",
            "src/lib.rs",
            "src/main.rs",
        ] {
            fs::write(demo.join(file), "").expect("write fixture file");
        }
        Demo { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("demo")
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).ok();
    }
}

/// A run inside `dir` with everything that could colour, re-charset or
/// re-collate the output taken out of the environment, so the expected
/// bytes do not depend on the shell the suite was started from.
fn command(dir: &Path, args: &[&str], locale: &str) -> Command {
    command_for(Path::new(BIN), dir, args, locale)
}

fn command_for(binary: &Path, dir: &Path, args: &[&str], locale: &str) -> Command {
    let mut cmd = Command::new(binary);
    cmd.args(args);
    cmd.current_dir(dir);
    cmd.stdin(Stdio::null());
    for name in [
        "LS_COLORS",
        "TREE_COLORS",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "NO_COLOR",
        "TREE_CHARSET",
        "LANG",
        "LC_CTYPE",
        "LC_COLLATE",
    ] {
        cmd.env_remove(name);
    }
    cmd.env("LC_ALL", locale);
    cmd
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    command(dir, args, "en_US.UTF-8")
        .output()
        .expect("spawn tree")
}

fn run(demo: &Demo, args: &[&str]) -> Output {
    run_in(&demo.path(), args)
}

fn run_locale(demo: &Demo, locale: &str, args: &[&str]) -> Output {
    command(&demo.path(), args, locale)
        .output()
        .expect("spawn tree")
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

/// The listing `tree` prints in the fixture with no options at all.
fn default_listing() -> String {
    format!(
        ".\n\
         {TEE}Cargo.toml\n\
         {TEE}docs\n\
         {BAR}{END}guide.md\n\
         {TEE}node_modules\n\
         {BAR}{END}pkg\n\
         {BAR}{GAP}{END}index.js\n\
         {TEE}README.md\n\
         {END}src\n\
         {GAP}{TEE}lib.rs\n\
         {GAP}{END}main.rs\n\
         \n5 directories, 6 files\n"
    )
}

#[test]
fn default_listing_draws_the_tree_and_counts_it() {
    let demo = Demo::new("default");
    let out = run(&demo, &[]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    assert_eq!(stdout(&out), default_listing());
}

#[test]
fn all_includes_dotfiles() {
    let demo = Demo::new("all");
    let out = run(&demo, &["-a"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.starts_with(&format!(".\n{TEE}.gitignore\n{TEE}Cargo.toml\n")),
        "unexpected head: {text}"
    );
    assert!(text.ends_with("\n5 directories, 7 files\n"), "{text}");
}

#[test]
fn directories_only() {
    let demo = Demo::new("dironly");
    let out = run(&demo, &["-d"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}docs\n\
             {TEE}node_modules\n\
             {BAR}{END}pkg\n\
             {END}src\n\
             \n5 directories\n"
        )
    );
}

#[test]
fn depth_limit() {
    let demo = Demo::new("depth");
    let out = run(&demo, &["-L", "1"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}Cargo.toml\n\
             {TEE}docs\n\
             {TEE}node_modules\n\
             {TEE}README.md\n\
             {END}src\n\
             \n4 directories, 2 files\n"
        )
    );

    for bad in [["-L", "0"], ["-L", "abc"]] {
        let out = run(&demo, &bad);
        assert_eq!(code(&out), 1);
        assert_eq!(stdout(&out), "");
        assert_eq!(
            stderr(&out),
            "tree: Invalid level, must be greater than 0.\n"
        );
    }

    let out = run(&demo, &["-L"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stderr(&out), "tree: Missing argument to -L option.\n");

    // strtoul, not a strict parse: a level with rubbish behind it is the
    // number in front of the rubbish.
    let out = run(&demo, &["-L", "1x"]);
    assert_eq!(code(&out), 0);
    assert!(!stdout(&out).contains("guide.md"), "{}", stdout(&out));
}

#[test]
fn exclude_patterns() {
    let demo = Demo::new("exclude");
    let out = run(&demo, &["-I", "node_modules"]);
    assert_eq!(code(&out), 0);
    assert!(!stdout(&out).contains("node_modules"), "{}", stdout(&out));
    assert!(stdout(&out).ends_with("\n3 directories, 5 files\n"));

    let out = run(&demo, &["-I", "node_modules|docs"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).ends_with("\n2 directories, 4 files\n"));
}

#[test]
fn include_patterns_do_not_filter_directories() {
    let demo = Demo::new("include");
    let out = run(&demo, &["-P", "*.rs"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}docs\n\
             {TEE}node_modules\n\
             {BAR}{END}pkg\n\
             {END}src\n\
             {GAP}{TEE}lib.rs\n\
             {GAP}{END}main.rs\n\
             \n5 directories, 2 files\n"
        )
    );

    let out = run(&demo, &["-P", "*.rs", "--prune"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {END}src\n\
             {GAP}{TEE}lib.rs\n\
             {GAP}{END}main.rs\n\
             \n2 directories, 2 files\n"
        )
    );

    // --matchdirs lets a directory match instead, and then everything
    // under it is listed whether it matches or not.
    let out = run(&demo, &["-P", "docs", "--matchdirs", "--noreport"]);
    assert!(stdout(&out).contains("guide.md"), "{}", stdout(&out));

    let out = run(&demo, &["-P", "*.RS", "--ignore-case", "--noreport"]);
    assert!(stdout(&out).contains("lib.rs"), "{}", stdout(&out));
}

#[test]
fn full_paths_and_no_indent() {
    let demo = Demo::new("fullpath");
    let out = run(&demo, &["-f"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}./Cargo.toml\n\
             {TEE}./docs\n\
             {BAR}{END}./docs/guide.md\n\
             {TEE}./node_modules\n\
             {BAR}{END}./node_modules/pkg\n\
             {BAR}{GAP}{END}./node_modules/pkg/index.js\n\
             {TEE}./README.md\n\
             {END}./src\n\
             {GAP}{TEE}./src/lib.rs\n\
             {GAP}{END}./src/main.rs\n\
             \n5 directories, 6 files\n"
        )
    );

    let out = run(&demo, &["-f", "-i"]);
    assert_eq!(
        stdout(&out),
        ".\n\
         ./Cargo.toml\n\
         ./docs\n\
         ./docs/guide.md\n\
         ./node_modules\n\
         ./node_modules/pkg\n\
         ./node_modules/pkg/index.js\n\
         ./README.md\n\
         ./src\n\
         ./src/lib.rs\n\
         ./src/main.rs\n\
         \n5 directories, 6 files\n"
    );
}

#[test]
fn no_report_drops_the_blank_line_too() {
    let demo = Demo::new("noreport");
    let out = run(&demo, &["--noreport"]);
    assert_eq!(code(&out), 0);
    let full = default_listing();
    let body = full.split("\n\n5 directories").next().expect("a body");
    assert_eq!(stdout(&out), format!("{body}\n"));
}

#[test]
fn dirs_first_and_files_first() {
    let demo = Demo::new("order");
    let out = run(&demo, &["--dirsfirst", "--noreport"]);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}docs\n\
             {BAR}{END}guide.md\n\
             {TEE}node_modules\n\
             {BAR}{END}pkg\n\
             {BAR}{GAP}{END}index.js\n\
             {TEE}src\n\
             {BAR}{TEE}lib.rs\n\
             {BAR}{END}main.rs\n\
             {TEE}Cargo.toml\n\
             {END}README.md\n"
        )
    );

    let out = run(&demo, &["--filesfirst", "--noreport"]);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}Cargo.toml\n\
             {TEE}README.md\n\
             {TEE}docs\n\
             {BAR}{END}guide.md\n\
             {TEE}node_modules\n\
             {BAR}{END}pkg\n\
             {BAR}{GAP}{END}index.js\n\
             {END}src\n\
             {GAP}{TEE}lib.rs\n\
             {GAP}{END}main.rs\n"
        )
    );
}

#[test]
fn ascii_charset() {
    let demo = Demo::new("ascii");
    let out = run(&demo, &["--charset=ascii", "--noreport"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        ".\n\
         |-- Cargo.toml\n\
         |-- docs\n\
         |   `-- guide.md\n\
         |-- node_modules\n\
         |   `-- pkg\n\
         |       `-- index.js\n\
         |-- README.md\n\
         `-- src\n\
         \x20   |-- lib.rs\n\
         \x20   `-- main.rs\n"
    );
}

#[test]
fn c_locale_sorts_by_bytes() {
    let demo = Demo::new("locale");
    let out = run_locale(&demo, "C", &["-L", "1", "--noreport"]);
    assert_eq!(code(&out), 0);
    // Byte order puts every capital before every lowercase, where the
    // en_US collation folds them together.
    assert_eq!(
        stdout(&out),
        ".\n\
         |-- Cargo.toml\n\
         |-- README.md\n\
         |-- docs\n\
         |-- node_modules\n\
         `-- src\n"
    );
}

#[test]
fn file_and_missing_arguments() {
    let demo = Demo::new("operands");
    let out = run(&demo, &["Cargo.toml"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        "Cargo.toml  [error opening dir]\n\n0 directories, 1 file\n"
    );

    let out = run(&demo, &["nonexistent"]);
    assert_eq!(code(&out), 2);
    assert_eq!(
        stdout(&out),
        "nonexistent  [error opening dir]\n\n0 directories, 0 files\n"
    );
    assert_eq!(stderr(&out), "");
}

#[test]
fn multiple_arguments_share_one_report() {
    let demo = Demo::new("multi");
    let out = run(&demo, &["src", "docs"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            "src\n\
             {TEE}lib.rs\n\
             {END}main.rs\n\
             docs\n\
             {END}guide.md\n\
             \n2 directories, 3 files\n"
        )
    );
}

#[test]
fn json_output() {
    let demo = Demo::new("json");
    let out = run(&demo, &["-J"]);
    assert_eq!(code(&out), 0);
    // Written a line at a time: a Rust line continuation would eat the
    // leading spaces that are the whole point of the shape.
    let expected = [
        r#"["#,
        r#"    {"type":"directory","name":".","contents":["#,
        r#"        {"type":"file","name":"Cargo.toml"},"#,
        r#"        {"type":"directory","name":"docs","contents":["#,
        r#"            {"type":"file","name":"guide.md"}"#,
        r#"        ]},"#,
        r#"        {"type":"directory","name":"node_modules","contents":["#,
        r#"            {"type":"directory","name":"pkg","contents":["#,
        r#"                {"type":"file","name":"index.js"}"#,
        r#"            ]}"#,
        r#"        ]},"#,
        r#"        {"type":"file","name":"README.md"},"#,
        r#"        {"type":"directory","name":"src","contents":["#,
        r#"            {"type":"file","name":"lib.rs"},"#,
        r#"            {"type":"file","name":"main.rs"}"#,
        r#"        ]}"#,
        r#"    ]}"#,
        r#",    {"type":"report","directories":5,"files":6}"#,
        r#"]"#,
        "",
    ]
    .join("\n");
    assert_eq!(stdout(&out), expected);

    // --noreport leaves the report object out and a blank line in its
    // place, and -i takes every indent and line break out of the JSON.
    let out = run(&demo, &["-J", "--noreport"]);
    assert!(stdout(&out).ends_with("    ]}\n\n]\n"), "{}", stdout(&out));

    let out = run(&demo, &["-J", "-i", "--noreport"]);
    let flat = stdout(&out);
    assert!(flat.starts_with("[{\"type\":\"directory\""), "{flat}");
    assert!(flat.ends_with("]}]}]\n"), "{flat}");
    assert!(!flat.contains("\n    "), "{flat}");
}

#[test]
fn size_fields() {
    let demo = Demo::new("sizes");
    let big = demo.path().join("big.bin");
    // Sparse, so the test does not write a megabyte to make one.
    fs::File::create(&big)
        .expect("create the large file")
        .set_len(1_048_575)
        .expect("size the large file");

    let out = run(&demo, &["-s", "--noreport", "-L", "1"]);
    assert!(
        stdout(&out).contains(&format!("{TEE}[    1048575]  big.bin\n")),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains(&format!("{TEE}[          0]  Cargo.toml\n")),
        "{}",
        stdout(&out)
    );

    // -h is a four-column field, and 1048575 is the documented value
    // that overflows it: the loop only divides while the size is at
    // least a megabyte, so this prints as 1024K rather than 1.0M.
    let out = run(&demo, &["-h", "--noreport", "-L", "1"]);
    assert!(
        stdout(&out).contains(&format!("{TEE}[1024K]  big.bin\n")),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains(&format!("{TEE}[   0]  Cargo.toml\n")),
        "{}",
        stdout(&out)
    );

    let out = run(&demo, &["--si", "--noreport", "-L", "1"]);
    assert!(
        stdout(&out).contains(&format!("{TEE}[1.0M]  big.bin\n")),
        "{}",
        stdout(&out)
    );
}

#[test]
fn permissions_and_dates() {
    let demo = Demo::new("meta");
    let out = run(&demo, &["-p", "--noreport", "-L", "1"]);
    assert!(
        stdout(&out).contains(&format!("{TEE}[drwxr-xr-x]  docs\n")),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains(&format!("{TEE}[-rw-r--r--]  Cargo.toml\n")),
        "{}",
        stdout(&out)
    );

    // --timefmt implies -D and hands the format straight to strftime.
    let out = run(&demo, &["--timefmt", "%Y", "--noreport", "-L", "1"]);
    let year = stdout(&out);
    let field = year.lines().next().expect("a root line");
    assert!(
        field.starts_with("[20") && field.ends_with("]  ."),
        "{year}"
    );
}

#[test]
fn sorting_by_time_picks_the_right_clock() {
    let demo = Demo::new("times");
    let dir = demo.path().join("times");
    fs::create_dir_all(&dir).expect("create the times directory");
    // a is written first and given the newer modification time, b second
    // and given the older one, so the two orders disagree: by
    // modification it is b then a, by status change a then b, since
    // writing the times in that order is what set each status change.
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    let now = std::time::SystemTime::now();
    for (name, when) in [("a", now), ("b", old)] {
        let path = dir.join(name);
        fs::write(&path, "").expect("write a timed file");
        fs::File::open(&path)
            .expect("open a timed file")
            .set_times(fs::FileTimes::new().set_modified(when))
            .expect("set a modification time");
    }

    let by_mtime = format!("times\n{TEE}b\n{END}a\n");
    let by_ctime = format!("times\n{TEE}a\n{END}b\n");
    for (args, expected) in [
        (["-t"].as_slice(), &by_mtime),
        (["--sort=mtime"].as_slice(), &by_mtime),
        // -c switches the clock -D shows as well as the one it sorts by,
        // and -t after it switches only the sort back.
        (["-c", "-t"].as_slice(), &by_mtime),
        (["-c"].as_slice(), &by_ctime),
        (["--sort=ctime"].as_slice(), &by_ctime),
    ] {
        let mut all = vec!["--noreport"];
        all.extend_from_slice(args);
        all.push("times");
        let out = run(&demo, &all);
        assert_eq!(code(&out), 0, "{args:?}");
        assert_eq!(stdout(&out), **expected, "{args:?}");
    }

    // --sort=none is -U under another name, so it turns off -r and the
    // two grouping options the same way.
    let unsorted = stdout(&run(&demo, &["--sort=none", "--noreport"]));
    for args in [
        ["--sort=none", "-r"],
        ["--sort=none", "--dirsfirst"],
        ["--sort=none", "--filesfirst"],
    ] {
        let mut all = vec!["--noreport"];
        all.extend_from_slice(&args);
        assert_eq!(stdout(&run(&demo, &all)), unsorted, "{args:?}");
    }
}

#[test]
fn filelimit_and_prune_stop_a_walk() {
    let demo = Demo::new("limits");
    let many = demo.path().join("outer/many");
    fs::create_dir_all(&many).expect("create the crowded directory");
    for name in ["x", "y", "z"] {
        fs::write(many.join(name), "").expect("write a crowded file");
    }

    // The limit hit on the operand itself names the directory and
    // leaves the status at 0.
    let out = run(&demo, &["--filelimit", "2", "outer/many"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        "outer/many  [3 entries exceeds filelimit, not opening dir]\n\n1 directory, 0 files\n"
    );

    // The same limit hit inside the tree makes the status 2.
    let out = run(&demo, &["--filelimit", "2", "outer"]);
    assert_eq!(code(&out), 2);
    assert_eq!(
        stdout(&out),
        format!(
            "outer\n\
             {END}many  [3 entries exceeds filelimit, not opening dir]\n\
             \n2 directories, 0 files\n"
        )
    );

    // --prune turns both a crowded directory and one the depth limit
    // stopped at into nothing to list, and drops them.
    let out = run(&demo, &["--filelimit", "2", "--prune", "outer"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "outer\n\n0 directories, 0 files\n");

    let out = run(&demo, &["-L", "1", "--prune"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}Cargo.toml\n\
             {END}README.md\n\
             \n1 directory, 2 files\n"
        )
    );
}

#[test]
fn option_errors() {
    let demo = Demo::new("errors");
    let out = run(&demo, &["--sort=foo"]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out),
        "tree: Sort type 'foo' not valid, should be one of: \
         name,version,size,mtime,ctime,none\n"
    );

    for (args, message) in [
        (["-Z"], "tree: Invalid argument -`Z'.\n"),
        (["--bogus"], "tree: Invalid argument `--bogus'.\n"),
        // tree matches a long name whole, so an abbreviation is not one.
        (["--nore"], "tree: Invalid argument `--nore'.\n"),
    ] {
        let out = run(&demo, &args);
        assert_eq!(code(&out), 1, "{args:?}");
        assert_eq!(stdout(&out), "");
        let text = stderr(&out);
        assert!(text.starts_with(message), "{text}");
        assert!(
            text.contains("usage: tree [-acdfghilnpqrstuvxACDFJQNSUX]"),
            "{text}"
        );
        assert!(text.ends_with("\t[--] [directory ...]\n"), "{text}");
    }
}

#[test]
fn unsupported_options_print_a_runnable_command() {
    let demo = Demo::new("unsupported");

    let out = run(&demo, &["--du", "src"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        format!(
            "tree: --du is not implemented in this build\n\
             Use instead: du -sh src\n\
             See {ISSUES} to request it\n"
        )
    );
    assert_eq!(
        stderr(&run(&demo, &["--du"])).lines().nth(1),
        Some("Use instead: du -sh .")
    );
    assert_eq!(
        stderr(&run(&demo, &["--du", "src", "docs"])).lines().nth(1),
        Some("Use instead: du -sh src docs")
    );

    // Every option the spec defers, with the command it names instead.
    // The ones taking an argument are given one, so the argument can
    // never be mistaken for an operand of the Use instead line.
    let with_command: &[(&[&str], &str, &str)] = &[
        (&["--du"], "--du", "du -sh ."),
        (&["-X"], "-X", "tree -J ."),
        (
            &["--gitignore"],
            "--gitignore",
            "tree -I 'node_modules|target|.git' .",
        ),
        (&["--inodes"], "--inodes", "ls -i ."),
        (&["-u"], "-u", "ls -l ."),
        (&["-g"], "-g", "ls -l ."),
        (&["--device"], "--device", "find . -xdev"),
        (&["-x"], "-x", "find . -xdev"),
    ];
    for (args, name, instead) in with_command {
        let out = run(&demo, args);
        assert_eq!(code(&out), 1, "{args:?}");
        assert_eq!(
            stderr(&out),
            format!(
                "tree: {name} is not implemented in this build\n\
                 Use instead: {instead}\n\
                 See {ISSUES} to request it\n"
            ),
            "{args:?}"
        );
    }

    let without_command: &[(&[&str], &str)] = &[
        (&["-l"], "-l"),
        (&["-N"], "-N"),
        (&["-q"], "-q"),
        (&["-A"], "-A"),
        (&["-S"], "-S"),
        (&["--metafirst"], "--metafirst"),
        (&["--compress", "2"], "--compress"),
        (&["--condense"], "--condense"),
        (&["--info"], "--info"),
        (&["--infofile", "notes"], "--infofile"),
        (&["--fromfile"], "--fromfile"),
        (&["--fromtabfile"], "--fromtabfile"),
        (&["--fflinks"], "--fflinks"),
        (&["--hyperlink"], "--hyperlink"),
        (&["--scheme", "file://"], "--scheme"),
        (&["--authority", "host"], "--authority"),
        (&["-H", "base"], "-H"),
        (&["-T", "title"], "-T"),
        (&["-R"], "-R"),
        (&["--nolinks"], "--nolinks"),
        (&["--hintro", "intro"], "--hintro"),
        (&["--houtro", "outro"], "--houtro"),
        (&["--gitfile", "ignore"], "--gitfile"),
    ];
    for (args, name) in without_command {
        let out = run(&demo, args);
        assert_eq!(code(&out), 1, "{args:?}");
        assert_eq!(
            stderr(&out),
            format!(
                "tree: {name} is not implemented in this build\n\
                 See {ISSUES} to request it\n"
            ),
            "{args:?}"
        );
    }
}

#[test]
fn help_and_version() {
    let demo = Demo::new("help");
    let out = run(&demo, &["--help"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    assert!(stdout(&out).starts_with("usage: tree "), "{}", stdout(&out));
    assert!(stdout(&out).contains("-J            Prints out an JSON"));

    let out = run(&demo, &["--version"]);
    assert_eq!(code(&out), 0);
    assert!(
        stdout(&out).starts_with("tree (agent-cli-tools) "),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains(ISSUES.trim_end_matches("/issues")));
}

/// The differential test: the same fixture, the same environment, the
/// same arguments, given to a real tree 2.3.2 and to this one. Skipped
/// unless AGENT_CLI_TOOLS_TREE_ORACLE names an executable, so the suite
/// still runs on a machine without one.
#[test]
fn matches_the_real_tree() {
    let Some(oracle) = std::env::var_os("AGENT_CLI_TOOLS_TREE_ORACLE") else {
        return;
    };
    let oracle = PathBuf::from(oracle);
    assert!(
        oracle.is_file(),
        "AGENT_CLI_TOOLS_TREE_ORACLE does not name a file: {}",
        oracle.display()
    );

    let demo = Demo::new("oracle");
    let dir = demo.path();
    let cases: &[&[&str]] = &[
        &[],
        &["-a"],
        &["-d"],
        &["-L", "1"],
        &["-I", "node_modules"],
        &["-f"],
        &["--noreport"],
        &["--dirsfirst"],
        &["--filesfirst"],
        &["--charset", "ascii"],
        &["-P", "*.rs"],
        &["-P", "*.rs", "--prune"],
        &["-J"],
        &["-L", "2", "-a"],
    ];
    for case in cases {
        let mine = run_in(&dir, case);
        // Same arguments, same directory, same environment: only the
        // binary differs.
        let theirs = command_for(&oracle, &dir, case, "en_US.UTF-8")
            .output()
            .expect("spawn the oracle");
        assert_eq!(
            String::from_utf8_lossy(&mine.stdout),
            String::from_utf8_lossy(&theirs.stdout),
            "stdout differs for {case:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&mine.stderr),
            String::from_utf8_lossy(&theirs.stderr),
            "stderr differs for {case:?}"
        );
        assert_eq!(
            mine.status.code(),
            theirs.status.code(),
            "exit status differs for {case:?}"
        );
    }
}
