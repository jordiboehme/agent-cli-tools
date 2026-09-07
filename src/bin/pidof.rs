//! `pidof`: find the process ids of a running program. A port of
//! procps-ng pidof(1) for macOS, matching its options, its matching
//! rules, its messages and its exit codes.

use agent_cli_tools::options::{Arg, Item, Mode, Opt, Parser};
use agent_cli_tools::proc::{self, Process};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::process;

const HELP: &str = "\n\
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

/// The options pidof recognizes, in the order its own usage lists them.
///
/// Five of them have no long form at all in procps, and `-d` is a second
/// spelling of `-S` rather than an option of its own. This parser
/// identifies every option by a long name, so those six carry a name that
/// begins with `=`: a long name is read from the token up to its first
/// `=`, so a name starting with one can never be produced by any command
/// line and the option stays exactly as short-only as procps has it.
const PIDOF_OPTS: &[Opt] = &[
    Opt {
        short: Some('s'),
        long: "single-shot",
        arg: Arg::None,
    },
    Opt {
        short: Some('c'),
        long: "check-root",
        arg: Arg::None,
    },
    Opt {
        short: Some('q'),
        long: "quiet",
        arg: Arg::None,
    },
    Opt {
        short: Some('w'),
        long: "with-workers",
        arg: Arg::None,
    },
    Opt {
        short: Some('x'),
        long: "=scripts",
        arg: Arg::None,
    },
    Opt {
        short: Some('o'),
        long: "omit-pid",
        arg: Arg::Required,
    },
    Opt {
        short: Some('t'),
        long: "lightweight",
        arg: Arg::None,
    },
    Opt {
        short: Some('S'),
        long: "separator",
        arg: Arg::Required,
    },
    Opt {
        short: Some('d'),
        long: "=sysv-separator",
        arg: Arg::Required,
    },
    Opt {
        short: Some('n'),
        long: "=no-stat",
        arg: Arg::None,
    },
    Opt {
        short: Some('m'),
        long: "=omit-relatives",
        arg: Arg::None,
    },
    Opt {
        short: Some('?'),
        long: "=usage",
        arg: Arg::None,
    },
    Opt {
        short: Some('h'),
        long: "help",
        arg: Arg::None,
    },
    Opt {
        short: Some('V'),
        long: "version",
        arg: Arg::None,
    },
];

/// What the command line asked for, once every option has been walked.
#[derive(Default)]
struct Flags {
    single_shot: bool,
    quiet: bool,
    scripts_too: bool,
    with_workers: bool,
}

/// The usage block on stderr, and exit 1. pidof has no "Try --help" line:
/// a bad option gets the whole help text instead, which is why this does
/// not go through `options::usage_error`.
fn usage(message: Option<&str>) -> ! {
    let mut err = std::io::stderr().lock();
    if let Some(message) = message {
        let _ = writeln!(err, "pidof: {message}");
    }
    let _ = err.write_all(HELP.as_bytes());
    let _ = err.flush();
    process::exit(1)
}

/// The part of `path` after its last `/`, which is what procps'
/// `get_basename` returns. Unlike `basename(3)` it does not step back
/// over a trailing slash, so `sleep/` has an empty base name; that is
/// what makes `pidof sleep/` match nothing at all.
fn base_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(at) => &path[at + 1..],
        None => path,
    }
}

/// The C `isspace` set under the C locale, which is what `strtoul` skips.
fn is_c_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// One omit token, read the way procps reads it: `strtoul(token, &end, 10)`
/// followed by a check that the whole token was consumed. Leading
/// whitespace and one sign are allowed and at least one digit is
/// required; anything left over rejects the token.
///
/// The value is then narrowed the way C narrows an `unsigned long` into
/// the `pid_t` it is assigned to, so `-o -5` and `-o 99999999999` are
/// accepted in silence and simply never match a process, exactly as they
/// are upstream.
fn parse_omit_pid(token: &str) -> Option<i32> {
    let bytes = token.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_c_whitespace(bytes[i]) {
        i += 1;
    }
    let negative = match bytes.get(i) {
        Some(b'+') => {
            i += 1;
            false
        }
        Some(b'-') => {
            i += 1;
            true
        }
        _ => false,
    };
    let first_digit = i;
    let mut value: u64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(bytes[i] - b'0'));
        i += 1;
    }
    if i == first_digit || i != bytes.len() {
        return None;
    }
    let value = if negative {
        value.wrapping_neg()
    } else {
        value
    };
    Some(value as u32 as i32)
}

/// Add one `-o` argument to the omit list: the tokens between `,`, `;`
/// and `:`, with `%PPID` standing for the parent of this process. A token
/// that is not a number is reported and skipped, and it leaves the exit
/// status alone.
fn add_to_omit_list(list: &mut Vec<i32>, value: &OsStr) {
    let text = value.to_string_lossy();
    // `strtok` never hands back an empty token, so consecutive or
    // trailing separators are simply ignored rather than rejected.
    for token in text.split([',', ';', ':']).filter(|t| !t.is_empty()) {
        if token == "%PPID" {
            list.push(std::os::unix::process::parent_id() as i32);
        } else if let Some(pid) = parse_omit_pid(token) {
            list.push(pid);
        } else {
            // procps reports this through warnx, whose own newline is
            // what turns the message's trailing one into a blank line.
            eprint!("pidof: illegal omit pid value ({token})!\n\n");
        }
    }
}

/// Whether `process` is one of the processes running `program`.
///
/// The six tests, the `-x` script rule and the setproctitle fallback are
/// procps' `select_procs` in the order it applies them.
fn matches(program: &str, process: &Process, flags: &Flags) -> bool {
    let argv = process.argv.as_deref().unwrap_or(&[]);
    // No argument vector means either a kernel worker on Linux or, here,
    // a process owned by another user. Upstream skips both unless -w.
    if argv.is_empty() && !flags.with_workers {
        return false;
    }

    // A leading '-' marks a login shell and is not part of the name.
    let arg0 = argv.first().map_or("", |a| a.as_str());
    let cmd_arg0 = arg0.strip_prefix('-').unwrap_or(arg0);
    let cmd_arg0base = base_name(cmd_arg0);
    // A process whose executable the kernel will not name is left with an
    // empty path, which no program argument can equal.
    let exe = process.exe.as_deref().unwrap_or("");
    let program_base = base_name(program);

    if program == cmd_arg0base
        || program_base == cmd_arg0
        || program == cmd_arg0
        || (flags.with_workers && program == process.comm)
        || program == base_name(exe)
        || program == exe
    {
        return true;
    }

    if flags.scripts_too && argv.len() > 1 {
        let cmd_arg1 = argv[1].as_str();
        let cmd_arg1base = base_name(cmd_arg1);
        // Upstream guards these three tests with a check that the
        // kernel's short name for the process is a prefix of argv[1]'s
        // base name. On Linux that tells a script run directly, which
        // leaves the script's own name in that field, from an
        // interpreter invoked by hand, which leaves the interpreter's.
        // Darwin records the interpreter's name for both, so keeping the
        // guard would leave -x unable to find a `#!/bin/sh` script at
        // all unless its name happened to start with `sh`. Dropping it
        // costs that distinction, and only that: the interpreter invoked
        // by hand is found as well. docs/pidof.md says so plainly.
        if program == cmd_arg1base || program_base == cmd_arg1 || program == cmd_arg1 {
            return true;
        }
    }

    // A space in argv[0] means the program most likely rewrote its own
    // command line, so its name is the only thing left to trust.
    if cmd_arg0.contains(' ') {
        return program == process.comm;
    }
    false
}

fn main() {
    // Without this a pipeline that stops reading, `pidof x | head -c1`,
    // would turn into a write error instead of the silent death by signal
    // that the C original produces.
    // SAFETY: setting a disposition to SIG_DFL is always valid, and this
    // runs before any output, thread or handler exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    let mut flags = Flags::default();
    let mut separator = OsString::from(" ");
    let mut omit: Vec<i32> = Vec::new();
    let mut programs: Vec<OsString> = Vec::new();

    // parse_partial, not parse_or_exit: -h, -V and the -o diagnostics
    // must act as they are reached, even if a later option on the same
    // line is invalid. A pending parse error is reported only once every
    // item that parsed before it has been walked.
    let (items, error) = Parser::new("pidof", PIDOF_OPTS, Mode::Permute).parse_partial(&args);
    for item in items {
        match item {
            Item::Flag { long: "quiet", .. } => {
                // procps falls through from -q into -s, so quiet mode
                // stops at the first match of every program as well.
                flags.quiet = true;
                flags.single_shot = true;
            }
            Item::Flag {
                long: "single-shot",
                ..
            } => flags.single_shot = true,
            Item::Flag {
                long: "with-workers",
                ..
            } => flags.with_workers = true,
            Item::Flag {
                long: "=scripts", ..
            } => flags.scripts_too = true,
            Item::Flag {
                long: "omit-pid",
                value,
            } => add_to_omit_list(&mut omit, &value.expect("-o takes a required argument")),
            Item::Flag {
                long: "separator" | "=sysv-separator",
                value,
            } => separator = value.expect("-S and -d take a required argument"),
            // -c has nothing to compare against here and -t has no thread
            // ids to add; -n and -m are compatibility switches upstream
            // ignores too. docs/pidof.md explains each one.
            Item::Flag {
                long: "check-root" | "lightweight" | "=no-stat" | "=omit-relatives",
                ..
            } => {}
            Item::Flag { long: "=usage", .. } => usage(None),
            Item::Flag { long: "help", .. } => {
                print!("{HELP}");
                process::exit(0);
            }
            Item::Flag {
                long: "version", ..
            } => {
                print!("{}", agent_cli_tools::version_text("pidof"));
                process::exit(0);
            }
            Item::Flag { long, .. } => unreachable!("unknown option name {long}"),
            Item::Operand(program) => programs.push(program),
        }
    }
    if let Some(message) = error {
        usage(Some(&message));
    }

    // One snapshot for the whole run, already sorted by pid descending.
    // procps rescans per program argument and prints each group's matches
    // in reverse scan order, which is this order.
    let table = proc::all();
    let mut line: Vec<u8> = Vec::new();
    let mut found = false;
    for program in &programs {
        let program = program.to_string_lossy();
        if program.is_empty() {
            continue;
        }
        for process in &table {
            if omit.contains(&process.pid) || !matches(&program, process, &flags) {
                continue;
            }
            found = true;
            if !flags.quiet {
                if !line.is_empty() {
                    line.extend_from_slice(separator.as_bytes());
                }
                line.extend_from_slice(process.pid.to_string().as_bytes());
            }
            if flags.single_shot {
                break;
            }
        }
    }
    if !flags.quiet && found {
        line.push(b'\n');
    }

    let mut out = std::io::stdout().lock();
    if out.write_all(&line).is_err() || out.flush().is_err() {
        eprintln!("pidof: write error");
        process::exit(1);
    }
    process::exit(i32::from(!found))
}
