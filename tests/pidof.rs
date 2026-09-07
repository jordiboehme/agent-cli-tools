//! Integration tests that drive the built `pidof` binary. Every case
//! spawns the processes it asks about, so its expectations hold whatever
//! else the machine happens to be running. The suite runs in parallel and
//! several cases spawn a `sleep`, so nothing here asserts on how many
//! processes matched a shared name, only on whether the pids the case
//! owns are among them; a case that needs an exact answer invents a name
//! no other case can produce.

use agent_cli_tools::proc;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_pidof");

/// The stand-in for `sleep` wherever a test needs a program running under
/// a name of its own: this crate's `timeout`, waiting out a long timeout.
/// A copy of a macOS system binary cannot serve, because a launch
/// constraint kills a copy of one with SIGKILL the moment it execs; a
/// binary this project built carries no such constraint.
const SLEEPER: &str = env!("CARGO_BIN_EXE_timeout");

/// How long a spawned process waits before giving up on its own. Long
/// enough that no test outruns it, short enough that one left behind by
/// a crashed test is gone before anybody notices.
const LIFETIME: &str = "60";

/// The usage block, spelled out here rather than shared with the binary,
/// so that a change to either one has to be made deliberately in both.
const USAGE: &str = "\n\
Usage:
 pidof [options] [program [...]]

Options:
 -s, --single-shot         return one PID only
 -c, --check-root          omit processes with different root
 -q,                       quiet mode, only set the exit code
 -w, --with-workers        show kernel workers too
 -x                        also find shells running the named scripts
 -o, --omit-pid <PID,...>  omit processes with PID
 -t, --lightweight         list threads too
 -S, --separator SEP       use SEP as separator put between PIDs
 -h, --help     display this help and exit
 -V, --version  output version information and exit

Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>
Compatible with pidof from procps-ng 4.0.6.
";

/// A name no other test in the suite, and no other run of it, will pick.
fn unique(tag: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{tag}{}n{n}", std::process::id())
}

/// A scratch directory holding one test's programs, removed with them.
struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(unique("pidof-test-"));
        std::fs::create_dir_all(&dir).expect("create the workspace");
        // The kernel reports a fully resolved executable path, and the
        // system temp directory is reached through a symlink on macOS, so
        // a case comparing whole paths needs the resolved spelling.
        Self {
            dir: std::fs::canonicalize(&dir).expect("resolve the workspace"),
        }
    }

    /// A copy of the sleeper program under a name of the test's choosing.
    fn program(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::copy(SLEEPER, &path).expect("copy the sleeper program");
        path
    }

    /// A symbolic link beside the program it points at.
    fn link(&self, target: &str, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::os::unix::fs::symlink(target, &path).expect("link to the program");
        path
    }

    /// An executable shell script that stays alive as long as a sleeper.
    fn script(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        // Two commands, so no shell can turn the last one into an exec
        // and leave the script with nothing of itself in its arguments.
        std::fs::write(&path, format!("#!/bin/sh\n/bin/sleep {LIFETIME}\nexit 0\n"))
            .expect("write the script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the script executable");
        path
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// A process that sits idle until the test that spawned it lets it go.
struct Sleeper {
    child: Child,
}

impl Sleeper {
    /// Run `program` with `args`, optionally under a chosen `argv[0]`,
    /// holding no descriptor of the test's own.
    fn spawn(program: &Path, args: &[&str], arg0: Option<&str>) -> Self {
        let mut command = Command::new(program);
        command.args(args);
        if let Some(arg0) = arg0 {
            command.arg0(arg0);
        }
        // A process group of its own, so the signal on Drop reaches
        // whatever the process started as well: a script leaves a
        // `sleep` of its own behind.
        command.process_group(0);
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|err| panic!("spawn {}: {err}", program.display()));
        // The kernel needs a moment to finish the exec before the
        // process table shows the program's own name and arguments.
        std::thread::sleep(Duration::from_millis(250));
        Self { child }
    }

    /// `/bin/sleep`, under its own name or one the test made up.
    fn sleep(arg0: Option<&str>) -> Self {
        Self::spawn(Path::new("/bin/sleep"), &[LIFETIME], arg0)
    }

    /// A copy of the sleeper program, run through the path given.
    fn program(path: &Path) -> Self {
        Self::spawn(path, &[LIFETIME, "/bin/sleep", LIFETIME], None)
    }

    fn pid(&self) -> i32 {
        self.child.id() as i32
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        // SAFETY: kill accepts any pid, and a negative one names a
        // process group: the one this child was given at spawn, so the
        // signal reaches its children too.
        unsafe { libc::kill(-self.pid(), libc::SIGKILL) };
        self.child.wait().ok();
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn pidof")
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

/// The pids on standard output, which also checks the shape of the line:
/// single spaces between the pids and one trailing newline, or nothing
/// at all when there was no match.
fn pids(output: &Output) -> Vec<i32> {
    let text = stdout(output);
    let Some(body) = text.strip_suffix('\n') else {
        assert_eq!(text, "", "output without a trailing newline");
        return Vec::new();
    };
    body.split(' ')
        .map(|field| {
            field
                .parse()
                .unwrap_or_else(|_| panic!("a pid in {text:?}"))
        })
        .collect()
}

/// The kernel's short name for a process, which is what the `-x` guard
/// and the `-w` name rule compare against.
fn comm_of(pid: i32) -> String {
    proc::all()
        .into_iter()
        .find(|p| p.pid == pid)
        .unwrap_or_else(|| panic!("pid {pid} is listed"))
        .comm
}

/// The two pids in the order pidof prints them, highest first.
fn descending(a: &Sleeper, b: &Sleeper) -> (i32, i32) {
    (a.pid().max(b.pid()), a.pid().min(b.pid()))
}

#[test]
fn finds_processes_by_name() {
    let one = Sleeper::sleep(None);
    let two = Sleeper::sleep(None);

    let out = run(&["sleep"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    let found = pids(&out);
    assert!(found.contains(&one.pid()), "{found:?}");
    assert!(found.contains(&two.pid()), "{found:?}");
    assert!(
        found.windows(2).all(|w| w[0] > w[1]),
        "descending {found:?}"
    );

    // The pids came back through a parse of exactly one line of
    // single-space-separated fields, which is the whole output format.
    let rebuilt: Vec<String> = found.iter().map(i32::to_string).collect();
    assert_eq!(stdout(&out), format!("{}\n", rebuilt.join(" ")));
}

#[test]
fn single_shot_prints_one_pid_per_program() {
    let workspace = Workspace::new();
    let first_name = unique("ssa");
    let first_program = workspace.program(&first_name);
    let one = Sleeper::program(&first_program);
    let two = Sleeper::program(&first_program);
    let second_name = unique("ssb");
    let other = Sleeper::program(&workspace.program(&second_name));

    let (high, low) = descending(&one, &two);
    assert_eq!(stdout(&run(&[&first_name])), format!("{high} {low}\n"));

    let out = run(&["-s", &first_name]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), format!("{high}\n"));
    assert_eq!(
        stdout(&run(&["--single-shot", &first_name])),
        format!("{high}\n")
    );

    // One pid per program argument, not one for the whole command line.
    assert_eq!(
        stdout(&run(&["-s", &first_name, &second_name])),
        format!("{high} {}\n", other.pid())
    );

    // Options are recognized after an operand too.
    assert_eq!(stdout(&run(&[&first_name, "-s"])), format!("{high}\n"));
}

#[test]
fn a_path_argument_matches_by_basename_and_by_full_path() {
    // One started as /bin/sleep, one whose argv[0] is the bare name.
    let by_path = Sleeper::sleep(None);
    let by_name = Sleeper::sleep(Some("sleep"));

    // The whole path matches argv[0] of the first and the resolved
    // executable of both.
    let found = pids(&run(&["/bin/sleep"]));
    assert!(found.contains(&by_path.pid()), "{found:?}");
    assert!(found.contains(&by_name.pid()), "{found:?}");

    // A path argument also matches by its own base name, which is what
    // finds a process whose argv[0] is just the name.
    let found = pids(&run(&["./sleep"]));
    assert!(found.contains(&by_name.pid()), "{found:?}");
    assert!(!found.contains(&by_path.pid()), "{found:?}");

    // A trailing slash leaves an empty base name, so nothing matches.
    let out = run(&["sleep/"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");
}

#[test]
fn a_renamed_copy_matches_its_own_name_and_its_resolved_target() {
    let workspace = Workspace::new();
    let name = unique("mys");
    let program = workspace.program(&name);
    let link_name = unique("lnk");
    let link = workspace.link(&name, &link_name);

    let direct = Sleeper::program(&program);
    let through_link = Sleeper::program(&link);

    // The link's own name finds only the process started through it.
    assert_eq!(
        stdout(&run(&[&link_name])),
        format!("{}\n", through_link.pid())
    );

    // The copy's name finds both: the second one through the executable
    // path, which the kernel reports with every symbolic link resolved.
    let (high, low) = descending(&direct, &through_link);
    assert_eq!(stdout(&run(&[&name])), format!("{high} {low}\n"));

    // And so does the copy's whole path.
    assert_eq!(
        stdout(&run(&[program.to_str().expect("a text path")])),
        format!("{high} {low}\n")
    );
}

#[test]
fn separator_and_omission() {
    let workspace = Workspace::new();
    let name = unique("sep");
    let program = workspace.program(&name);
    let one = Sleeper::program(&program);
    let two = Sleeper::program(&program);
    let (high, low) = descending(&one, &two);

    assert_eq!(stdout(&run(&["-S", ",", &name])), format!("{high},{low}\n"));
    assert_eq!(stdout(&run(&["-S", "", &name])), format!("{high}{low}\n"));
    assert_eq!(
        stdout(&run(&["--separator", " | ", &name])),
        format!("{high} | {low}\n")
    );
    // -d is the sysvinit spelling of the same option.
    assert_eq!(stdout(&run(&["-d", ":", &name])), format!("{high}:{low}\n"));

    // -o drops the pid it names and leaves the rest.
    assert_eq!(
        stdout(&run(&["-o", &high.to_string(), &name])),
        format!("{low}\n")
    );
    // Its argument is a list, split on any of ',', ';' and ':', and the
    // option may be repeated.
    let out = run(&["-o", &format!("{high};{low}"), &name]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    let out = run(&["-o", &high.to_string(), "-o", &low.to_string(), &name]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");

    // A token that is not a number is reported, skipped, and leaves both
    // the other tokens and the exit status alone.
    let out = run(&["-o", &format!("abc,{high}"), &name]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "pidof: illegal omit pid value (abc)!\n\n");
    assert_eq!(stdout(&out), format!("{low}\n"));

    // A pid that is out of range, or negative, is accepted in silence.
    let out = run(&["-o", "-5,0,99999999999", &name]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    assert_eq!(stdout(&out), format!("{high} {low}\n"));

    // %PPID is the parent of pidof, which is this test process.
    let own_name = std::env::current_exe()
        .expect("this test binary's path")
        .file_name()
        .expect("a file name")
        .to_string_lossy()
        .into_owned();
    let me = std::process::id() as i32;
    assert!(pids(&run(&[&own_name])).contains(&me));
    assert!(!pids(&run(&["-o", "%PPID", &own_name])).contains(&me));
}

#[test]
fn quiet_prints_nothing_and_only_sets_the_status() {
    let workspace = Workspace::new();
    let name = unique("qui");
    let running = Sleeper::program(&workspace.program(&name));
    assert!(running.pid() > 0);

    let out = run(&["-q", &name]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");

    // The long form the C source carries even though its usage text
    // shows none.
    let out = run(&["--quiet", &name]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");

    let out = run(&["-q", &unique("nosuchprogram")]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");

    // Quiet is about the pids. An option diagnostic is reported as the
    // option is read, well before there is any output to suppress.
    let out = run(&["-q", "-o", "abc", &name]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "pidof: illegal omit pid value (abc)!\n\n");
}

#[test]
fn several_programs_are_grouped_in_argument_order() {
    let workspace = Workspace::new();
    let first_name = unique("gra");
    let first = Sleeper::program(&workspace.program(&first_name));
    let second_name = unique("grb");
    let second = Sleeper::program(&workspace.program(&second_name));
    let (a, b) = (first.pid(), second.pid());

    assert_eq!(
        stdout(&run(&[&first_name, &second_name])),
        format!("{a} {b}\n")
    );
    assert_eq!(
        stdout(&run(&[&second_name, &first_name])),
        format!("{b} {a}\n")
    );
    // A program named twice has its pids printed twice.
    assert_eq!(
        stdout(&run(&[&first_name, &first_name])),
        format!("{a} {a}\n")
    );
    // An empty argument is skipped, and one that matches nothing simply
    // contributes no group.
    assert_eq!(stdout(&run(&["", &first_name])), format!("{a}\n"));
    assert_eq!(
        stdout(&run(&[&first_name, "nosuchprogram", &second_name])),
        format!("{a} {b}\n")
    );
    // Groups run into each other with nothing between them when the
    // separator is empty, exactly as they do inside one group.
    assert_eq!(
        stdout(&run(&["-S", "", &first_name, &second_name])),
        format!("{a}{b}\n")
    );
}

#[test]
fn no_match_and_no_arguments() {
    let out = run(&[&unique("nosuchprogram")]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");

    let out = run(&[]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");
}

#[test]
fn script_matching_requires_x() {
    let workspace = Workspace::new();
    let plain_name = format!("{}.sh", unique("scr"));
    let plain = workspace.script(&plain_name);
    let running = Sleeper::spawn(&plain, &[], None);

    // A script is never found by its own name without -x.
    assert_eq!(code(&run(&[&plain_name])), 1);

    // With -x it is found only when the kernel's short name for the
    // process is a prefix of the script's file name. Linux puts the
    // script's own name there, Darwin the interpreter's, so a script
    // named like this one stays unmatched here.
    let interpreter = comm_of(running.pid());
    let out = run(&["-x", &plain_name]);
    assert_eq!(code(&out), 1, "the process is named {interpreter:?}");

    // A script whose name starts with the interpreter's does pass that
    // guard, which is what exercises the rule on this platform.
    let named = format!("{interpreter}{}.sh", unique("run"));
    let script = workspace.script(&named);
    let direct = Sleeper::spawn(&script, &[], None);
    let through_sh = Sleeper::spawn(Path::new("/bin/sh"), &[script.to_str().unwrap()], None);

    let out = run(&["-x", &named]);
    assert_eq!(code(&out), 0);
    let found = pids(&out);
    assert!(found.contains(&direct.pid()), "{found:?}");
    // Darwin gives both forms the same short name, so unlike on Linux
    // the explicitly invoked interpreter is found as well.
    assert!(found.contains(&through_sh.pid()), "{found:?}");

    // The script's whole path matches the interpreter's own argument.
    let found = pids(&run(&["-x", script.to_str().unwrap()]));
    assert!(found.contains(&direct.pid()), "{found:?}");

    // Without -x neither form is found.
    assert_eq!(code(&run(&[&named])), 1);
}

#[test]
fn workers_option_reaches_a_process_without_arguments() {
    let launchd = proc::all()
        .into_iter()
        .find(|p| p.pid == 1)
        .expect("launchd is listed");

    // pid 1 belongs to root, so an ordinary user cannot read its
    // arguments and only -w considers it at all.
    if launchd.argv.is_none() {
        let found = pids(&run(&[&launchd.comm]));
        assert!(!found.contains(&1), "{found:?}");
    }
    let found = pids(&run(&["-w", &launchd.comm]));
    assert!(found.contains(&1), "{found:?}");

    // The executable path is readable for it either way.
    if let Some(exe) = launchd.exe.as_deref() {
        let found = pids(&run(&["-w", exe]));
        assert!(found.contains(&1), "{found:?}");
    }
}

#[test]
fn a_name_the_kernel_cut_short_matches_only_where_that_name_is_used() {
    let workspace = Workspace::new();
    // A file name longer than the 16 characters the kernel remembers, so
    // that the short name is something no other rule ever compares
    // against: not the base name of argv[0] and not that of the
    // executable path.
    let long = format!("{}-with-a-long-name", unique("cut"));
    let program = workspace.program(&long);
    let plain = Sleeper::program(&program);
    let title = "a rewritten title";
    let titled = Sleeper::spawn(&program, &[LIFETIME, "/bin/sleep", LIFETIME], Some(title));
    let short = comm_of(plain.pid());
    assert!(short.len() < long.len(), "{long:?} was kept whole");

    // A space in argv[0] means the program rewrote its own command line,
    // and then the short name is the only thing left to match on.
    let found = pids(&run(&[&short]));
    assert!(found.contains(&titled.pid()), "{title}: {found:?}");
    assert!(!found.contains(&plain.pid()), "{found:?}");

    // -w adds the short name as a test of its own, which reaches the
    // process that kept its arguments too.
    let found = pids(&run(&["-w", &short]));
    assert!(found.contains(&titled.pid()), "{found:?}");
    assert!(found.contains(&plain.pid()), "{found:?}");

    // The whole name still finds both, through argv[0] and through the
    // executable path.
    let found = pids(&run(&[&long]));
    assert!(found.contains(&titled.pid()), "{found:?}");
    assert!(found.contains(&plain.pid()), "{found:?}");
}

#[test]
fn a_login_shells_leading_dash_is_not_part_of_the_name() {
    let workspace = Workspace::new();
    let program = workspace.program(&unique("dsh"));
    // A name that is neither the program's file name nor its path, so
    // only argv[0] can match it.
    let name = unique("login");
    let running = Sleeper::spawn(
        &program,
        &[LIFETIME, "/bin/sleep", LIFETIME],
        Some(&format!("-{name}")),
    );

    assert_eq!(stdout(&run(&[&name])), format!("{}\n", running.pid()));
    // The dash is dropped from the process, not from the argument, so
    // the name written with one matches nothing. `--` is what gets it
    // past option scanning to be an argument at all.
    let out = run(&["--", &format!("-{name}")]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
}

#[test]
fn option_errors_and_help() {
    let out = run(&["-Z"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        format!("pidof: invalid option -- 'Z'\n{USAGE}")
    );

    let out = run(&["--bogus"]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out),
        format!("pidof: unrecognized option '--bogus'\n{USAGE}")
    );

    // The options with no long form stay short-only.
    for bogus in ["--x", "--scripts", "--no-stat", "--usage"] {
        let out = run(&[bogus]);
        assert_eq!(code(&out), 1, "{bogus}");
        assert_eq!(
            stderr(&out),
            format!("pidof: unrecognized option '{bogus}'\n{USAGE}")
        );
    }

    let out = run(&["-S"]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out),
        format!("pidof: option requires an argument -- 'S'\n{USAGE}")
    );

    let out = run(&["--separator"]);
    assert_eq!(code(&out), 1);
    assert_eq!(
        stderr(&out),
        format!("pidof: option '--separator' requires an argument\n{USAGE}")
    );

    // -h is the usage block on standard output, -? the same text on
    // standard error with no diagnostic and a failing status.
    let out = run(&["-h"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), USAGE);
    assert_eq!(stderr(&out), "");
    assert_eq!(stdout(&run(&["--help"])), USAGE);

    let out = run(&["-?"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), USAGE);

    // An earlier --help acts before a later bad option is inspected.
    let out = run(&["--help", "-Z"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), USAGE);

    let out = run(&["-V"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stderr(&out), "");
    let version = stdout(&out);
    assert!(
        version.starts_with(&format!(
            "pidof (agent-cli-tools) {}\n",
            env!("CARGO_PKG_VERSION")
        )),
        "{version:?}"
    );
    assert!(
        version.contains("Home page: <https://github.com/jordiboehme/agent-cli-tools>\n"),
        "{version:?}"
    );
    assert_eq!(stdout(&run(&["--version"])), version);
}

#[test]
fn the_macos_specific_options_are_accepted() {
    let workspace = Workspace::new();
    let name = unique("mac");
    let running = Sleeper::program(&workspace.program(&name));
    let expected = format!("{}\n", running.pid());

    for options in [
        vec!["-c"],
        vec!["-t"],
        vec!["-w"],
        vec!["-n"],
        vec!["-m"],
        vec!["-ctwnm"],
        vec!["--check-root"],
        vec!["--lightweight"],
        vec!["--with-workers"],
    ] {
        let mut args = options.clone();
        args.push(&name);
        let out = run(&args);
        assert_eq!(code(&out), 0, "{options:?}");
        assert_eq!(stderr(&out), "", "{options:?}");
        assert_eq!(stdout(&out), expected, "{options:?}");
    }
}
