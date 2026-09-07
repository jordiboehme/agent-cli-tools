//! Integration tests that drive the built `tree` binary. Every case runs
//! in a fixture of its own, since the suite runs its tests in parallel
//! and several of them add files to the tree they list.
//!
//! Set `AGENT_CLI_TOOLS_TREE_ORACLE` to a real tree 2.3.2 binary to also
//! run the differential test at the bottom, which asserts byte-identical
//! output for the cases listed there.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
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

    /// The same fixture plus the entries a listing has to cope with and
    /// the plain one has none of: a symlink to a directory, a symlink
    /// to nothing, a directory that cannot be opened, and a name with a
    /// space in it. Kept out of `new` so the hand-written expectations
    /// everywhere else stay what they are; the differential test below
    /// is what wants the awkward entries.
    fn awkward(tag: &str) -> Demo {
        let demo = Demo::new(tag);
        let root = demo.path();
        for sub in ["d1", "empty", "locked"] {
            fs::create_dir_all(root.join(sub)).expect("create fixture directory");
        }
        for file in ["d1/a.txt", "a file.txt"] {
            fs::write(root.join(file), "").expect("write fixture file");
        }
        symlink("d1", root.join("link")).expect("link fixture directory");
        symlink("nowhere", root.join("dangling")).expect("link fixture nothing");
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000))
            .expect("lock fixture directory");
        demo
    }

    fn path(&self) -> PathBuf {
        self.dir.join("demo")
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        // A directory with no permission bits at all cannot be walked,
        // so it cannot be removed either: give them back before the
        // fixture goes. Only `awkward` makes one, and every fixture is
        // torn down here.
        let locked = self.path().join("locked");
        if locked.is_dir() {
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).ok();
        }
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

/// Upstream's whole `--help` text, the one this port reprints before its
/// own footer. Written a line at a time: a Rust line continuation eats
/// the leading whitespace of the line that follows it, which is exactly
/// the bug this pins down.
const HELP_TEXT: &[&str] = &[
    "usage: tree [-acdfghilnpqrstuvxACDFJQNSUX] [-L level [-R]] [-H [-]baseHREF]",
    "\t[-T title] [-o filename] [-P pattern] [-I pattern] [--gitignore]",
    "\t[--gitfile[=]file] [--matchdirs] [--metafirst] [--ignore-case]",
    "\t[--nolinks] [--hintro[=]file] [--houtro[=]file] [--inodes] [--device]",
    "\t[--sort[=]name] [--dirsfirst] [--filesfirst] [--filelimit[=]#] [--si]",
    "\t[--du] [--prune] [--charset[=]X] [--timefmt[=]format] [--fromfile]",
    "\t[--fromtabfile] [--fflinks] [--info] [--infofile[=]file] [--noreport]",
    "\t[--hyperlink] [--scheme[=]schema] [--authority[=]host] [--opt-toggle]",
    "\t[--compress[=]#] [--condense] [--version] [--help]",
    "\t[--] [directory ...]",
    "  ------- Listing options -------",
    "  -a            All files are listed.",
    "  -d            List directories only.",
    "  -l            Follow symbolic links like directories.",
    "  -f            Print the full path prefix for each file.",
    "  -x            Stay on current filesystem only.",
    "  -L level      Descend only level directories deep.",
    "  -R            Rerun tree when max dir level reached.",
    "  -P pattern    List only those files that match the pattern given.",
    "  -I pattern    Do not list files that match the given pattern.",
    "  --gitignore   Filter by using .gitignore files.",
    "  --gitfile X   Explicitly read a gitignore file.",
    "  --ignore-case Ignore case when pattern matching.",
    "  --matchdirs   Include directory names in -P pattern matching.",
    "  --metafirst   Print meta-data at the beginning of each line.",
    "  --prune       Prune empty directories from the output.",
    "  --info        Print information about files found in .info files.",
    "  --infofile X  Explicitly read info file.",
    "  --noreport    Turn off file/directory count at end of tree listing.",
    "  --charset X   Use charset X for terminal/HTML and indentation line output.",
    "  --filelimit # Do not descend dirs with more than # files in them.",
    "  --condense    Condense directory singletons to a single line of output.",
    "  -o filename   Output to file instead of stdout.",
    "  ------- File options -------",
    "  -q            Print non-printable characters as '?'.",
    "  -N            Print non-printable characters as is.",
    "  -Q            Quote filenames with double quotes.",
    "  -p            Print the protections for each file.",
    "  -u            Displays file owner or UID number.",
    "  -g            Displays file group owner or GID number.",
    "  -s            Print the size in bytes of each file.",
    "  -h            Print the size in a more human readable way.",
    "  --si          Like -h, but use in SI units (powers of 1000).",
    "  --du          Compute size of directories by their contents.",
    "  -D            Print the date of last modification or (-c) status change.",
    "  --timefmt fmt Print and format time according to the format fmt.",
    "  -F            Appends '/', '=', '*', '@', '|' or '>' as per ls -F.",
    "  --inodes      Print inode number of each file.",
    "  --device      Print device ID number to which each file belongs.",
    "  ------- Sorting options -------",
    "  -v            Sort files alphanumerically by version.",
    "  -t            Sort files by last modification time.",
    "  -c            Sort files by last status change time.",
    "  -U            Leave files unsorted.",
    "  -r            Reverse the order of the sort.",
    "  --dirsfirst   List directories before files (-U disables).",
    "  --filesfirst  List files before directories (-U disables).",
    "  --sort X      Select sort: name,version,size,mtime,ctime,none.",
    "  ------- Graphics options -------",
    "  -i            Don't print indentation lines.",
    "  -A            Print ANSI lines graphic indentation lines.",
    "  -S            Print with CP437 (console) graphics indentation lines.",
    "  -n            Turn colorization off always (-C overrides).",
    "  -C            Turn colorization on always.",
    "  --compress #  Compress indentation lines.",
    "  ------- XML/HTML/JSON/HYPERLINK options -------",
    "  -X            Prints out an XML representation of the tree.",
    "  -J            Prints out an JSON representation of the tree.",
    "  -H baseHREF   Prints out HTML format with baseHREF as top directory.",
    "  -T string     Replace the default HTML title and H1 header with string.",
    "  --nolinks     Turn off hyperlinks in HTML output.",
    "  --hintro X    Use file X as the HTML intro.",
    "  --houtro X    Use file X as the HTML outro.",
    "  --hyperlink   Turn on OSC 8 terminal hyperlinks.",
    "  --scheme X    Set OSC 8 hyperlink scheme, default file://",
    "  --authority X Set OSC 8 hyperlink authority/hostname.",
    "  ------- Input options -------",
    "  --fromfile    Reads paths from files (.=stdin)",
    "  --fromtabfile Reads trees from tab indented files (.=stdin)",
    "  --fflinks     Process link information when using --fromfile.",
    "  ------- Miscellaneous options -------",
    "  --opt-toggle  Enable option toggling.",
    "  --version     Print version and exit.",
    "  --help        Print usage and this help message and exit.",
    "  --            Options processing terminator.",
];

/// The footer this port adds after upstream's help text, the only part
/// of `--help` that differs.
const HELP_FOOTER: &str = "\nPart of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>\n\
Compatible with tree 2.3.2; the options it leaves out are refused by name.\n";

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

/// Run a case here and, when the oracle is present, assert that the real
/// tree answers with the same bytes and the same status.
fn agrees_with_oracle(demo: &Demo, args: &[&str]) -> Output {
    let mine = run(demo, args);
    if let Some(oracle) = std::env::var_os("AGENT_CLI_TOOLS_TREE_ORACLE") {
        let theirs = command_for(Path::new(&oracle), &demo.path(), args, "en_US.UTF-8")
            .output()
            .expect("spawn the oracle");
        assert_eq!(mine.stdout, theirs.stdout, "stdout differs for {args:?}");
        assert_eq!(mine.stderr, theirs.stderr, "stderr differs for {args:?}");
        assert_eq!(
            mine.status.code(),
            theirs.status.code(),
            "exit status differs for {args:?}"
        );
    }
    mine
}

#[test]
fn malformed_patterns_match_what_upstream_matches() {
    let demo = Demo::new("malformed");
    let all = format!(
        ".\n\
         {TEE}Cargo.toml\n\
         {TEE}docs\n\
         {TEE}node_modules\n\
         {TEE}README.md\n\
         {END}src\n"
    );

    // A bracket group with no closing bracket is a malformed pattern,
    // and upstream's matcher answers it with a match, but only once the
    // scan has got that far: `do[` reaches the bracket on docs and
    // nowhere else, and `z*[` never reaches it at all.
    let out = agrees_with_oracle(&demo, &["-L", "1", "--noreport", "-I", "do["]);
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}Cargo.toml\n\
             {TEE}node_modules\n\
             {TEE}README.md\n\
             {END}src\n"
        )
    );
    let out = agrees_with_oracle(&demo, &["-L", "1", "--noreport", "-I", "z*["]);
    assert_eq!(stdout(&out), all);
    let out = agrees_with_oracle(&demo, &["-L", "1", "--noreport", "-I", "*["]);
    assert_eq!(stdout(&out), ".\n");

    // An empty alternative matches everything, so an accidental trailing
    // bar excludes the lot; a pattern that is empty from end to end has
    // no alternation in it and excludes nothing.
    for pattern in ["node_modules|", "|node_modules", "|"] {
        let out = agrees_with_oracle(&demo, &["-L", "1", "--noreport", "-I", pattern]);
        assert_eq!(stdout(&out), ".\n", "{pattern}");
    }
    let out = agrees_with_oracle(&demo, &["-L", "1", "--noreport", "-I", ""]);
    assert_eq!(stdout(&out), all);

    // The same two rules seen from -P, where a match keeps a file
    // instead of dropping it.
    let out = agrees_with_oracle(&demo, &["--noreport", "-P", "guide.md|"]);
    assert!(stdout(&out).contains("index.js"), "{}", stdout(&out));
    let out = agrees_with_oracle(&demo, &["--noreport", "-P", "["]);
    assert!(stdout(&out).contains("index.js"), "{}", stdout(&out));
    let out = agrees_with_oracle(&demo, &["--noreport", "-P", "zz["]);
    assert!(!stdout(&out).contains("index.js"), "{}", stdout(&out));
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
    // An option's own argument is an option argument, never an operand
    // of the command that is printed instead.
    assert_eq!(
        stderr(&run(&demo, &["-H", "base", "src"])).lines().nth(1),
        Some("Use instead: tree -J src")
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
        // The HTML options and the rerun point at the JSON form, which
        // is the machine-readable output this build does have. Each of
        // their own arguments is an option argument, so none of them can
        // be mistaken for an operand of the line that is printed.
        (&["-H", "base"], "-H", "tree -J ."),
        (&["-T", "title"], "-T", "tree -J ."),
        (&["-R"], "-R", "tree -J ."),
        (&["--nolinks"], "--nolinks", "tree -J ."),
        (&["--hintro", "intro"], "--hintro", "tree -J ."),
        (&["--houtro", "outro"], "--houtro", "tree -J ."),
        (
            &["--gitfile", "ignore"],
            "--gitfile",
            "tree -I 'node_modules|target|.git' .",
        ),
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

    // Byte for byte, upstream's text and then the footer: the two-space
    // indent of every body line is easy to lose and impossible to see.
    let text = stdout(&out);
    let (body, footer) = text
        .split_once(HELP_FOOTER)
        .map_or((text.as_str(), ""), |(body, rest)| (body, rest));
    assert_eq!(footer, "", "the footer must be the end of the text");
    let expected = format!("{}\n", HELP_TEXT.join("\n"));
    assert_eq!(body, expected);

    // And against the real thing when it is here, so the checked-in copy
    // cannot drift away from it either.
    if let Some(oracle) = std::env::var_os("AGENT_CLI_TOOLS_TREE_ORACLE") {
        let theirs = command_for(Path::new(&oracle), &demo.path(), &["--help"], "en_US.UTF-8")
            .output()
            .expect("spawn the oracle");
        assert_eq!(theirs.stdout, expected.as_bytes());
    }

    let out = run(&demo, &["--version"]);
    assert_eq!(code(&out), 0);
    assert!(
        stdout(&out).starts_with("tree (agent-cli-tools) "),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains(ISSUES.trim_end_matches("/issues")));
}

/// An option argument is never read as an option name, however it is
/// spelled: `-P '--bogus'` is a pattern that matches nothing, not the
/// invalid-argument error `--bogus` on its own is.
#[test]
fn an_option_argument_is_never_a_long_option() {
    let demo = Demo::new("optarg");
    let out = agrees_with_oracle(&demo, &["-P", "--bogus"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    assert_eq!(
        stdout(&out),
        format!(
            ".\n\
             {TEE}docs\n\
             {TEE}node_modules\n\
             {BAR}{END}pkg\n\
             {END}src\n\
             \n5 directories, 0 files\n"
        )
    );

    // The same token given as an operand, and as an option name.
    let out = agrees_with_oracle(&demo, &["--", "--bogus"]);
    assert_eq!(code(&out), 2);
    assert_eq!(
        stdout(&out),
        "--bogus  [error opening dir]\n\n0 directories, 0 files\n"
    );
    let out = run(&demo, &["--bogus"]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).starts_with("tree: Invalid argument `--bogus'.\n"),
        "{}",
        stderr(&out)
    );
}

/// Upstream follows `link` with -l and lists what is under it; this
/// build refuses the option, so -l cannot be one of the byte-identical
/// cases below. Assert the refusal over the awkward fixture instead.
#[test]
fn follow_links_is_refused_where_there_are_links() {
    let demo = Demo::awkward("followlinks");
    let out = run(&demo, &["-l"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        format!(
            "tree: -l is not implemented in this build\n\
             See {ISSUES} to request it\n"
        )
    );
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

    let demo = Demo::awkward("oracle");
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
        &["-L", "2", "-a"],
        // The awkward entries: a pattern has to filter a symlink to a
        // directory, which is never descended here, and keep it when
        // the pattern asks for directories by a trailing slash.
        &["-P", "empty/"],
        &["-P", "link/"],
        &["-P", "broken"],
        &["-P", "*.RS", "--ignore-case"],
        &["-P", "*.rs", "--matchdirs", "-I", "locked"],
        &["--matchdirs", "-P", "d1", "-I", "locked"],
        &["-d", "-P", "zzz"],
        &["-I", "link"],
        &["-I", "link/"],
        // An option argument is never an option, however it is spelled.
        &["-P", "--bogus"],
        // -I locked on the two cases above and here: an unreadable
        // directory takes upstream down two paths this build does not
        // follow, and both are unrelated to what the case is for. With
        // --matchdirs upstream reports "error opening dir" and still
        // exits 0, where every other run exits 2; in JSON its printer
        // gives every later file a `"contents":[    ]` of its own. The
        // other cases here do walk the unreadable directory.
        &["-J", "-I", "locked"],
    ];
    for case in cases {
        let mine = run_in(&dir, case);
        // Same arguments, same directory, same environment: only the
        // binary differs.
        let theirs = command_for(&oracle, &dir, case, "en_US.UTF-8")
            .output()
            .expect("spawn the oracle");
        // Bytes, not lossy strings: two different invalid sequences
        // both become U+FFFD, and an escaping divergence would pass.
        assert_eq!(
            mine.stdout,
            theirs.stdout,
            "stdout differs for {case:?}\n  ours: {}\ntheirs: {}",
            String::from_utf8_lossy(&mine.stdout),
            String::from_utf8_lossy(&theirs.stdout)
        );
        assert_eq!(
            mine.stderr,
            theirs.stderr,
            "stderr differs for {case:?}\n  ours: {}\ntheirs: {}",
            String::from_utf8_lossy(&mine.stderr),
            String::from_utf8_lossy(&theirs.stderr)
        );
        assert_eq!(
            mine.status.code(),
            theirs.status.code(),
            "exit status differs for {case:?}"
        );
    }
}
