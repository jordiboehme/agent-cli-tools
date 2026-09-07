//! `tac`: write each file to standard output, last line first. A port of
//! GNU coreutils tac(1) for macOS, matching its options, record model,
//! messages and exit codes.

use agent_cli_tools::options::{Arg, Item, Mode, Opt, Parser};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::process;

const HELP: &str = "\
Usage: tac [OPTION]... [FILE]...
Write each FILE to standard output, last line first.

With no FILE, or when FILE is -, read standard input.

Mandatory arguments to long options are mandatory for short options too.
  -b, --before             attach the separator before instead of after
  -r, --regex              interpret the separator as a regular expression
  -s, --separator=STRING   use STRING as the separator instead of newline
      --help
         display this help and exit
      --version
         output version information and exit

Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>
Compatible with tac from GNU coreutils 9.11.
";

const TAC_OPTS: &[Opt] = &[
    Opt {
        short: Some('b'),
        long: "before",
        arg: Arg::None,
    },
    Opt {
        short: Some('r'),
        long: "regex",
        arg: Arg::None,
    },
    Opt {
        short: Some('s'),
        long: "separator",
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

/// A diagnostic in GNU `error()` shape. Only the getopt failures get the
/// "Try 'tac --help'" line, so every other message goes through here.
fn diagnose(message: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "tac: {message}");
    let _ = err.flush();
}

fn strerror(code: i32) -> String {
    // SAFETY: strerror returns a pointer to a valid NUL-terminated string
    // for any int, and the borrow ends before any other libc call could
    // reuse its static buffer.
    unsafe { CStr::from_ptr(libc::strerror(code)) }
        .to_string_lossy()
        .into_owned()
}

/// The bare strerror text for an I/O failure. `io::Error`'s own Display
/// appends " (os error N)", which GNU's messages never carry.
fn os_message(err: &std::io::Error) -> String {
    match err.raw_os_error() {
        Some(code) => strerror(code),
        None => err.to_string(),
    }
}

/// GNU's `quoteaf`: a name always wrapped in single quotes, with any
/// embedded quote escaped the way a shell needs. Used for the open
/// failure, which quotes even a perfectly ordinary file name.
fn quote_always(name: &OsStr) -> String {
    format!("'{}'", name.to_string_lossy().replace('\'', "'\\''"))
}

/// GNU's `quotef`: the same quoting, but applied only to a name that
/// needs it. Used for the read error, which leaves ordinary names bare.
fn quote_if_needed(name: &OsStr) -> String {
    let plain = name.as_bytes().iter().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'%' | b'+' | b',' | b'-' | b'.' | b'/' | b':' | b'=' | b'@' | b'_'
            )
    });
    if plain && !name.is_empty() {
        name.to_string_lossy().into_owned()
    } else {
        quote_always(name)
    }
}

/// Translate the Emacs-flavoured regular expression GNU tac compiles into
/// the POSIX extended syntax `regcomp` speaks. Emacs syntax spells
/// grouping and alternation with a backslash and treats the bare
/// characters as literals, so both readings have to be swapped. Braces
/// carry no interval meaning there either, so they become literals too.
/// Everything else, escape sequences and bracket expressions included,
/// passes through untouched.
fn to_posix_ere(pattern: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pattern.len());
    let mut i = 0;
    while i < pattern.len() {
        let byte = pattern[i];
        if byte == b'\\' && i + 1 < pattern.len() {
            let next = pattern[i + 1];
            match next {
                b'(' | b')' | b'|' => out.push(next),
                _ => {
                    out.push(b'\\');
                    out.push(next);
                }
            }
            i += 2;
        } else {
            if matches!(byte, b'(' | b')' | b'{' | b'}' | b'|') {
                out.push(b'\\');
            }
            out.push(byte);
            i += 1;
        }
    }
    out
}

/// A compiled POSIX extended regular expression, owning its `regex_t`.
struct Regex {
    compiled: libc::regex_t,
}

impl Regex {
    fn new(pattern: &[u8]) -> Result<Self, String> {
        // regcomp reads a C string, so the pattern ends at its first NUL.
        // That is the documented limit of regex mode here.
        let end = pattern
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(pattern.len());
        let text = CString::new(to_posix_ere(&pattern[..end])).expect("NUL already trimmed");
        // SAFETY: regex_t is a plain struct of integers and pointers with
        // no validity requirement of its own, and regcomp overwrites every
        // field it uses before anything reads one.
        let mut compiled: libc::regex_t = unsafe { std::mem::zeroed() };
        // SAFETY: `compiled` is a live regex_t this call owns for its whole
        // duration, and `text` outlives the call, so the pattern pointer
        // stays valid throughout it.
        let code = unsafe {
            libc::regcomp(
                &mut compiled,
                text.as_ptr(),
                libc::REG_EXTENDED | libc::REG_NEWLINE,
            )
        };
        if code != 0 {
            return Err(regerror(code, &compiled));
        }
        Ok(Self { compiled })
    }

    /// The match with the greatest start position strictly before `limit`,
    /// searching only `region[..limit]`, as `(start, length)`.
    ///
    /// GNU calls `re_search` with a negative range, which walks candidate
    /// start positions downward; `regexec` has no such mode, so the walk
    /// is explicit here and a candidate is accepted only when the match it
    /// finds begins at that very position. A match reported further right
    /// belongs to a candidate the walk already passed.
    ///
    /// `region` is the operand's data with one extra byte on the end, and
    /// it is borrowed mutably because the terminator has to move: regexec
    /// reads a C string, and REG_NEWLINE anchoring is only right when that
    /// string ends exactly where the search region does. Writing a 0 at
    /// `limit` and putting back the byte it displaced costs the same for
    /// any input size; copying the region per search instead made the run
    /// quadratic in bytes copied, since there is one search per match.
    fn search_back(&self, region: &mut [u8], limit: usize) -> Option<(usize, usize)> {
        if limit == 0 {
            return None;
        }
        let displaced = region[limit];
        region[limit] = 0;

        let mut result = None;
        let mut start = limit - 1;
        loop {
            // REG_NOTBOL keeps '^' from matching at a candidate that is
            // not really the start of a line in the original data.
            let eflags = if start > 0 && region[start - 1] != b'\n' {
                libc::REG_NOTBOL
            } else {
                0
            };
            // SAFETY: regmatch_t is two plain integers, so an all-zero
            // value is a valid one; regexec overwrites both before they
            // are read.
            let mut found: libc::regmatch_t = unsafe { std::mem::zeroed() };
            // SAFETY: `region[limit]` is 0 and `start < limit`, so the
            // offset pointer is inside `region` and names a valid C
            // string; `found` is one regmatch_t and nmatch says so.
            let code = unsafe {
                libc::regexec(
                    &self.compiled,
                    region.as_ptr().add(start).cast(),
                    1,
                    &mut found,
                    eflags,
                )
            };
            if code == 0 && found.rm_so == 0 {
                result = Some((start, found.rm_eo as usize));
                break;
            }
            if start == 0 {
                break;
            }
            start -= 1;
        }

        region[limit] = displaced;
        result
    }
}

impl Drop for Regex {
    fn drop(&mut self) {
        // SAFETY: `compiled` came from a regcomp that returned success and
        // is freed exactly once, here.
        unsafe { libc::regfree(&mut self.compiled) };
    }
}

/// The message `regerror` has for a failed compile, without the trailing
/// NUL. GNU prints exactly this text after its "tac: " prefix.
fn regerror(code: i32, compiled: &libc::regex_t) -> String {
    // SAFETY: regerror accepts the regex_t handed to the regcomp that
    // failed, and a null buffer with size 0 asks only for the length.
    let needed = unsafe { libc::regerror(code, compiled, std::ptr::null_mut(), 0) };
    let mut buffer = vec![0u8; needed.max(1)];
    // SAFETY: the buffer is `needed` bytes, the size regerror just asked
    // for, so the write cannot overrun it.
    unsafe { libc::regerror(code, compiled, buffer.as_mut_ptr().cast(), buffer.len()) };
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

enum Separator {
    Fixed(Vec<u8>),
    Regex(Regex),
}

/// Write `data` back with its records reversed.
///
/// Both modes walk backward from the end, and each match splits the data
/// at one boundary: the end of the match by default, so a record keeps the
/// separator that terminates it, or the start of the match under `-b`, so
/// the separator leads the record that follows it. Everything from a
/// boundary to the previous one is a record, emitted as soon as it is
/// known, and whatever is left at the front is emitted last.
fn reverse(
    data: &[u8],
    separator: &Separator,
    before: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let mut past_end = data.len();
    match separator {
        Separator::Fixed(needle) => {
            let width = needle.len();
            // The first candidate is the last position a match could still
            // end at the very end of the data; after a hit the scan steps a
            // whole separator back, so matches never overlap.
            let mut at = match data.len().checked_sub(width) {
                Some(at) => at as isize,
                None => -1,
            };
            while at >= 0 {
                let start = at as usize;
                if &data[start..start + width] == needle.as_slice() {
                    let boundary = if before { start } else { start + width };
                    out.write_all(&data[boundary..past_end])?;
                    past_end = boundary;
                    at -= width as isize;
                } else {
                    at -= 1;
                }
            }
        }
        Separator::Regex(regex) => {
            // One scratch copy per operand, with room for the terminator
            // each search moves to its own limit. Records are still read
            // out of `data`, which nothing here disturbs.
            let mut region = Vec::with_capacity(data.len() + 1);
            region.extend_from_slice(data);
            region.push(0);

            // The next search never starts at the previous match's own
            // start, so an empty match can never be found twice, and it
            // never looks past that start either, which is what keeps a
            // greedy separator from swallowing the record before it.
            let mut limit = data.len();
            while limit > 0 {
                let Some((start, length)) = regex.search_back(&mut region, limit) else {
                    break;
                };
                let boundary = if before { start } else { start + length };
                out.write_all(&data[boundary..past_end])?;
                past_end = boundary;
                limit = start;
            }
        }
    }
    out.write_all(&data[..past_end])
}

/// Read one operand whole. GNU reads a seekable file backward in chunks
/// and spools a non-seekable one to a temp file; reading it all into
/// memory reaches the same records without either mechanism.
fn read_operand(name: &OsStr) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    if name.as_bytes() == b"-" {
        std::io::stdin()
            .lock()
            .read_to_end(&mut data)
            .map_err(|err| format!("standard input: read error: {}", os_message(&err)))?;
        return Ok(data);
    }
    let mut file = std::fs::File::open(name).map_err(|err| {
        format!(
            "failed to open {} for reading: {}",
            quote_always(name),
            os_message(&err)
        )
    })?;
    // Opening a directory succeeds here just as it does under GNU, so the
    // EISDIR arrives on the first read and the diagnostic is a read error.
    file.read_to_end(&mut data).map_err(|err| {
        format!(
            "{}: read error: {}",
            quote_if_needed(name),
            os_message(&err)
        )
    })?;
    Ok(data)
}

fn write_error(err: &std::io::Error) -> ! {
    diagnose(&format!("write error: {}", os_message(err)));
    process::exit(1)
}

fn main() {
    // The Rust runtime ignores SIGPIPE, which would turn `tac big | head`
    // into a write error instead of the silent death by signal that GNU
    // tac and every other filter in a pipeline produce.
    // SAFETY: setting a disposition to SIG_DFL is always valid, and this
    // runs before any output, thread or handler exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    let mut before = false;
    let mut regex = false;
    let mut separator: Option<OsString> = None;
    let mut files: Vec<OsString> = Vec::new();

    // parse_partial, not parse_or_exit: --help and --version must act as
    // soon as they are reached, even if a later option on the same line is
    // invalid. A pending parse error is reported only once every item that
    // parsed before it has been walked.
    let (items, error) = Parser::new("tac", TAC_OPTS, Mode::Permute).parse_partial(&args);
    for item in items {
        match item {
            Item::Flag { long: "before", .. } => before = true,
            Item::Flag { long: "regex", .. } => regex = true,
            Item::Flag {
                long: "separator",
                value,
            } => separator = value,
            Item::Flag { long: "help", .. } => {
                print!("{HELP}");
                process::exit(0);
            }
            Item::Flag {
                long: "version", ..
            } => {
                print!("{}", agent_cli_tools::version_text("tac"));
                process::exit(0);
            }
            Item::Flag { long, .. } => unreachable!("unknown option name {long}"),
            Item::Operand(name) => files.push(name),
        }
    }
    if let Some(message) = error {
        agent_cli_tools::options::usage_error("tac", &message, 1);
    }

    let bytes: Vec<u8> = separator
        .as_deref()
        .map_or_else(|| vec![b'\n'], |value| value.as_bytes().to_vec());

    let separator = if regex {
        if bytes.is_empty() {
            diagnose("separator cannot be empty");
            process::exit(1);
        }
        match Regex::new(&bytes) {
            Ok(regex) => Separator::Regex(regex),
            Err(message) => {
                diagnose(&message);
                process::exit(1);
            }
        }
    } else if bytes.is_empty() {
        // An empty separator is not an error without -r: GNU reads it as
        // the NUL byte, which is how it reverses find -print0 output.
        Separator::Fixed(vec![0])
    } else {
        Separator::Fixed(bytes)
    };

    if files.is_empty() {
        files.push(OsString::from("-"));
    }

    let mut status = 0;
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for name in &files {
        match read_operand(name) {
            Ok(data) => {
                if let Err(err) = reverse(&data, &separator, before, &mut out) {
                    write_error(&err);
                }
            }
            Err(message) => {
                // A failed operand never stops the ones after it; only the
                // final exit status remembers that one went wrong.
                diagnose(&message);
                status = 1;
            }
        }
    }
    if let Err(err) = out.flush() {
        write_error(&err);
    }
    process::exit(status)
}
