//! `nproc`: print the number of processing units available. A port of GNU
//! coreutils nproc(1) for macOS, matching its options, OpenMP environment
//! variable handling and messages.

use agent_cli_tools::options::{Arg, Item, Mode, Opt, Parser};
use std::ffi::{CString, OsString};
use std::io::Write;
use std::process;

const HELP: &str = "\
Usage: nproc [OPTION]...
Print the number of processing units available to the current process,
which may be less than the number of online processors.
If the 'OMP_NUM_THREADS' or 'OMP_THREAD_LIMIT' environment variables are set,
then they will determine the minimum and maximum returned value respectively.

      --all
         print the number of installed processors,
         disregarding any OpenMP environment variables, or CPU quotas.
      --ignore=N
         if possible, exclude N processing units.
         The result is guaranteed to be at least 1.
      --help
         display this help and exit
      --version
         output version information and exit

Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>
Compatible with nproc from GNU coreutils 9.11.
";

const NPROC_OPTS: &[Opt] = &[
    Opt {
        short: None,
        long: "all",
        arg: Arg::None,
    },
    Opt {
        short: None,
        long: "ignore",
        arg: Arg::Required,
    },
    Opt {
        short: None,
        long: "help",
        arg: Arg::None,
    },
    Opt {
        short: None,
        long: "version",
        arg: Arg::None,
    },
];

/// A usage error: the diagnostic, the "Try" line, exit 1.
fn usage_error(message: &str) -> ! {
    agent_cli_tools::options::usage_error("nproc", message, 1)
}

/// GNU's `xdectoint`-style scan of an OpenMP variable: 0 means unset or
/// invalid. Leading whitespace is skipped, the first remaining byte must
/// be a decimal digit, the digits read saturate at `u64::MAX`, trailing
/// whitespace is skipped, and the value is accepted only at the end of the
/// string or at a comma (with anything past it ignored).
fn parse_omp(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_c_whitespace(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() || !bytes[i].is_ascii_digit() {
        return 0;
    }
    let mut value: u64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(bytes[i] - b'0'));
        i += 1;
    }
    while i < bytes.len() && is_c_whitespace(bytes[i]) {
        i += 1;
    }
    if i == bytes.len() || bytes[i] == b',' {
        value
    } else {
        0
    }
}

/// The C `isspace` set under the C locale: space, tab, newline, vertical
/// tab, form feed, carriage return.
fn is_c_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `--ignore=N`: leading whitespace and a single optional `+`, then digits
/// only, saturating at `u64::MAX`. Anything else is not a valid number.
fn parse_ignore(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_c_whitespace(bytes[i]) {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'+' {
        i += 1;
    }
    if i >= bytes.len() || !bytes[i].is_ascii_digit() {
        return None;
    }
    let mut value: u64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(bytes[i] - b'0'));
        i += 1;
    }
    if i == bytes.len() { Some(value) } else { None }
}

/// The processor count sysconf reports, falling back to `sysctlbyname` and
/// then to 1 when even that fails. On macOS both `_SC_NPROCESSORS_ONLN`
/// and `_SC_NPROCESSORS_CONF` resolve to `hw.ncpu`, so `all` and not-`all`
/// return the same number here.
fn cpus(all: bool) -> u64 {
    let name = if all {
        libc::_SC_NPROCESSORS_CONF
    } else {
        libc::_SC_NPROCESSORS_ONLN
    };
    let n = unsafe { libc::sysconf(name) };
    if n > 0 {
        return n as u64;
    }
    sysctl_ncpu().unwrap_or(1)
}

fn sysctl_ncpu() -> Option<u64> {
    let key = CString::new("hw.ncpu").expect("no NUL in literal");
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let rc = unsafe {
        libc::sysctlbyname(
            key.as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 && value > 0 {
        Some(value as u64)
    } else {
        None
    }
}

fn env_omp(name: &str) -> u64 {
    std::env::var(name).map(|v| parse_omp(&v)).unwrap_or(0)
}

fn main() {
    // The Rust runtime ignores SIGPIPE, which would turn `nproc | head -c1`
    // into a write error instead of the silent death by signal that GNU
    // nproc and every other filter in a pipeline produce.
    // SAFETY: setting a disposition to SIG_DFL is always valid, and this
    // runs before any output, thread or handler exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    let mut all = false;
    let mut ignore: u64 = 0;
    let mut operand: Option<OsString> = None;

    // parse_partial, not parse_or_exit: --help and --version must act as
    // soon as they are reached, even if a later option on the same line
    // is invalid. Only once every item is walked is a pending parse error
    // reported, and only then is an extra operand checked for.
    let (items, error) = Parser::new("nproc", NPROC_OPTS, Mode::Permute).parse_partial(&args);
    for item in items {
        match item {
            Item::Flag { long: "all", .. } => all = true,
            Item::Flag {
                long: "ignore",
                value,
            } => {
                let text = value
                    .as_deref()
                    .map_or(String::new(), |v| v.to_string_lossy().into_owned());
                ignore = parse_ignore(&text)
                    .unwrap_or_else(|| usage_error(&format!("invalid number: '{text}'")));
            }
            Item::Flag { long: "help", .. } => {
                print!("{HELP}");
                process::exit(0);
            }
            Item::Flag {
                long: "version", ..
            } => {
                print!("{}", agent_cli_tools::version_text("nproc"));
                process::exit(0);
            }
            Item::Flag { long, .. } => unreachable!("unknown option name {long}"),
            Item::Operand(text) => {
                if operand.is_none() {
                    operand = Some(text);
                }
            }
        }
    }
    if let Some(message) = error {
        usage_error(&message);
    }
    if let Some(extra) = operand {
        usage_error(&format!("extra operand '{}'", extra.to_string_lossy()));
    }

    let n = if all {
        cpus(true)
    } else {
        let threads = env_omp("OMP_NUM_THREADS");
        let mut limit = env_omp("OMP_THREAD_LIMIT");
        if limit == 0 {
            limit = u64::MAX;
        }
        if threads != 0 {
            threads.min(limit)
        } else {
            cpus(false).min(limit)
        }
    };
    let n = if ignore < n { n - ignore } else { 1 };

    let mut out = std::io::stdout().lock();
    if writeln!(out, "{n}").is_err() || out.flush().is_err() {
        eprintln!("nproc: write error");
        process::exit(1);
    }
}
