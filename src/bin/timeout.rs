//! `timeout`: run a command with a time limit. A port of GNU coreutils
//! timeout(1) for macOS, matching its options, messages, signal handling
//! and exit statuses.

use agent_cli_tools::duration::{Timeout, parse_duration};
use agent_cli_tools::signals::parse_signal;
use std::ffi::OsString;
use std::io::Write;
use std::process;

const EXIT_CANCELED: i32 = 125;

const HELP: &str = "\
Usage: timeout [OPTION]... DURATION COMMAND [ARG]...
Start COMMAND, and kill it if still running after DURATION.

Mandatory arguments to long options are mandatory for short options too.
  -f, --foreground
         when not running timeout directly from a shell prompt,
         allow COMMAND to read from the TTY and get TTY signals;
         in this mode, children of COMMAND will not be timed out
  -k, --kill-after=DURATION
         also send a KILL signal if COMMAND is still running
         this long after the initial signal was sent
  -p, --preserve-status
         exit with the same status as COMMAND,
         even when the command times out
  -s, --signal=SIGNAL
         specify the signal to be sent on timeout;
         SIGNAL may be a name like 'HUP' or a number;
         see 'kill -l' for a list of signals
  -v, --verbose
         diagnose to standard error any signal sent upon timeout
      --help
         display this help and exit
      --version
         output version information and exit

DURATION is a floating point number with an optional suffix:
's' for seconds (the default), 'm' for minutes, 'h' for hours or 'd' for days.
A duration of 0 disables the associated timeout.

Upon timeout, send the TERM signal to COMMAND, if no other SIGNAL specified.
The TERM signal kills any process that does not block or catch that signal.
It may be necessary to use the KILL signal, since this signal can't be caught.

Exit status:
  124  if COMMAND times out, and --preserve-status is not specified
  125  if the timeout command itself fails
  126  if COMMAND is found but cannot be invoked
  127  if COMMAND cannot be found
  137  if COMMAND (or timeout itself) is sent the KILL (9) signal (128+9)
  -    the exit status of COMMAND otherwise

Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>
Compatible with timeout from GNU coreutils 9.11.
";

/// Long options and whether each takes an argument.
const LONG_OPTIONS: &[(&str, bool)] = &[
    ("foreground", false),
    ("kill-after", true),
    ("preserve-status", false),
    ("signal", true),
    ("verbose", false),
    ("help", false),
    ("version", false),
];

struct Options {
    foreground: bool,
    kill_after: Timeout,
    preserve_status: bool,
    term_signal: i32,
    verbose: bool,
    timeout: Timeout,
    command: Vec<OsString>,
}

/// Print a diagnostic the way GNU `error()` does: program name, colon, text.
fn diagnose(message: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "timeout: {message}");
    let _ = err.flush();
}

/// A usage error: the diagnostic, the "Try" line, exit 125.
fn usage_error(message: &str) -> ! {
    diagnose(message);
    try_help()
}

fn try_help() -> ! {
    eprintln!("Try 'timeout --help' for more information.");
    process::exit(EXIT_CANCELED)
}

fn parse_args(args: &[OsString]) -> Options {
    let mut opts = Options {
        foreground: false,
        kill_after: Timeout::Never,
        preserve_status: false,
        term_signal: libc::SIGTERM,
        verbose: false,
        timeout: Timeout::Never,
        command: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].to_string_lossy().into_owned();
        i += 1;
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline_value) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let (full, takes_arg) = resolve_long(name, &arg);
            let value = if takes_arg {
                match inline_value {
                    Some(v) => v,
                    None => {
                        if i >= args.len() {
                            usage_error(&format!("option '--{full}' requires an argument"));
                        }
                        i += 1;
                        args[i - 1].to_string_lossy().into_owned()
                    }
                }
            } else {
                if inline_value.is_some() {
                    usage_error(&format!("option '--{full}' doesn't allow an argument"));
                }
                String::new()
            };
            apply(&mut opts, full, &value);
        } else if arg.len() > 1 && arg.starts_with('-') {
            let cluster = &arg[1..];
            for (pos, flag) in cluster.char_indices() {
                let name = match flag {
                    'f' => "foreground",
                    'p' => "preserve-status",
                    'v' => "verbose",
                    'k' => "kill-after",
                    's' => "signal",
                    other => usage_error(&format!("invalid option -- '{other}'")),
                };
                if flag == 'k' || flag == 's' {
                    let rest = &cluster[pos + flag.len_utf8()..];
                    let value = if rest.is_empty() {
                        if i >= args.len() {
                            usage_error(&format!("option requires an argument -- '{flag}'"));
                        }
                        i += 1;
                        args[i - 1].to_string_lossy().into_owned()
                    } else {
                        rest.to_string()
                    };
                    apply(&mut opts, name, &value);
                    break;
                }
                apply(&mut opts, name, "");
            }
        } else {
            i -= 1;
            break;
        }
    }
    let operands = &args[i..];
    if operands.len() < 2 {
        try_help();
    }
    let duration = operands[0].to_string_lossy();
    opts.timeout = parse_duration(&duration)
        .unwrap_or_else(|| usage_error(&format!("invalid time interval '{duration}'")));
    opts.command = operands[1..].to_vec();
    opts
}

/// Resolve a long option by exact name or unambiguous prefix, the way
/// getopt_long does, with its exact diagnostics.
fn resolve_long(name: &str, arg: &str) -> (&'static str, bool) {
    let candidates: Vec<&(&str, bool)> = LONG_OPTIONS
        .iter()
        .filter(|(n, _)| n.starts_with(name))
        .collect();
    if let Some(exact) = candidates.iter().find(|(n, _)| *n == name) {
        return **exact;
    }
    match candidates.as_slice() {
        [] => usage_error(&format!("unrecognized option '{arg}'")),
        [one] => **one,
        many => {
            let list: Vec<String> = many.iter().map(|(n, _)| format!("'--{n}'")).collect();
            usage_error(&format!(
                "option '--{name}' is ambiguous; possibilities: {}",
                list.join(" ")
            ))
        }
    }
}

fn apply(opts: &mut Options, name: &str, value: &str) {
    match name {
        "foreground" => opts.foreground = true,
        "preserve-status" => opts.preserve_status = true,
        "verbose" => opts.verbose = true,
        "kill-after" => {
            opts.kill_after = parse_duration(value)
                .unwrap_or_else(|| usage_error(&format!("invalid time interval '{value}'")));
        }
        "signal" => {
            opts.term_signal = parse_signal(value)
                .unwrap_or_else(|| usage_error(&format!("'{value}': invalid signal")));
        }
        "help" => {
            print!("{HELP}");
            process::exit(0);
        }
        "version" => {
            println!(
                "timeout (agent-cli-tools) {}\n\
                 Copyright (c) 2026 Jordi Böhme\n\
                 License MIT <https://opensource.org/license/mit>.\n\
                 This is free software: you are free to change and redistribute it.\n\
                 There is NO WARRANTY, to the extent permitted by law.",
                env!("CARGO_PKG_VERSION")
            );
            process::exit(0);
        }
        _ => unreachable!("unknown option name {name}"),
    }
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let _opts = parse_args(&args);
}
