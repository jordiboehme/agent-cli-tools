//! A GNU `getopt_long`-style parser shared by every command in this crate.
//! Each binary describes its options as a static table and gets back the
//! command line as an ordered list of flags and operands, or the exact GNU
//! diagnostic text to report. Ported from the parser proven in
//! `src/bin/timeout.rs`, generalized with `Arg::Optional` and the two
//! operand-scanning modes GNU getopt itself supports.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::process;

/// Whether an option takes an argument, and if so whether it may be omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    None,
    Required,
    Optional,
}

/// One recognized option: its short letter (if any), its long name, and
/// whether it takes an argument. `long` is the sole identity carried into
/// `Item::Flag`, so callers can match on it without re-deriving a name.
#[derive(Debug, Clone, Copy)]
pub struct Opt {
    pub short: Option<char>,
    pub long: &'static str,
    pub arg: Arg,
}

/// Whether an operand ends option scanning. GNU's default is `Permute`:
/// options and operands may be interleaved anywhere. `StopAtFirstOperand`
/// is what a command with its own trailing argv, like `timeout`'s COMMAND,
/// needs: everything from the first operand on is passed through as-is,
/// dashes included, rather than being reparsed as options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Options are recognized wherever they appear, before or after an
    /// operand. This does not reorder argv the way libc's getopt_long
    /// permute mode does: operands and flags simply come back as `Item`s
    /// in the order the command line gave them.
    Permute,
    StopAtFirstOperand,
}

/// One parsed item, in the order the command line gave them.
///
/// `Flag.value` is an `OsString`, not a `String`: an option argument (a
/// separator, a path) is not required to be valid UTF-8, and this parser
/// must not corrupt it before the command it belongs to ever sees it. A
/// caller converts with `to_string_lossy()` where it wants text and
/// `OsStrExt::as_bytes()` where it wants bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Flag {
        long: &'static str,
        value: Option<OsString>,
    },
    Operand(OsString),
}

pub struct Parser {
    program: &'static str,
    opts: &'static [Opt],
    mode: Mode,
}

impl Parser {
    pub fn new(program: &'static str, opts: &'static [Opt], mode: Mode) -> Self {
        Self {
            program,
            opts,
            mode,
        }
    }

    /// Parse `args`, returning every item in order, or the GNU diagnostic
    /// text (without the program-name prefix a caller adds on report) if
    /// any token failed to parse. Discards whatever parsed before an
    /// error; use this when nothing needs to act until the whole line is
    /// known good. A command with an option that must take effect the
    /// moment it is reached, such as `--help`, wants `parse_partial`
    /// instead.
    pub fn parse(&self, args: &[OsString]) -> Result<Vec<Item>, String> {
        let (items, error) = self.parse_partial(args);
        match error {
            Some(message) => Err(message),
            None => Ok(items),
        }
    }

    /// Parse as far as possible: returns the items recognized before the
    /// first failure, plus that failure's diagnostic text, if the scan
    /// stopped early. Unlike GNU getopt_long, which keeps scanning past a
    /// bad option and can report several, this stops at the first one and
    /// goes no further. A caller that must let an earlier option such as
    /// `--help` act before a later bad option is even inspected walks the
    /// returned items in order, then checks for a pending error only once
    /// that walk is done.
    pub fn parse_partial(&self, args: &[OsString]) -> (Vec<Item>, Option<String>) {
        let mut items = Vec::new();
        let mut i = 0;
        // Set once by `--`, or by the first operand in StopAtFirstOperand:
        // every remaining token is an operand, dashes and all.
        let mut operands_only = false;
        while i < args.len() {
            let raw = &args[i];
            if operands_only {
                items.push(Item::Operand(raw.clone()));
                i += 1;
                continue;
            }
            let text = raw.to_string_lossy();
            if text == "--" {
                operands_only = true;
                i += 1;
                continue;
            }
            if let Some(rest) = text.strip_prefix("--") {
                // "--" is two bytes, both ASCII, so this byte offset into
                // the lossy text is also correct against the raw bytes.
                let (name, eq_at) = match rest.find('=') {
                    Some(eq) => (&rest[..eq], Some(2 + eq)),
                    None => (rest, None),
                };
                let opt = match self.resolve_long(name, &text) {
                    Ok(opt) => opt,
                    Err(message) => return (items, Some(message)),
                };
                i += 1;
                let inline = eq_at.map(|at| os_from_bytes(&raw.as_bytes()[at + 1..]));
                let value = match opt.arg {
                    Arg::None => {
                        if inline.is_some() {
                            return (
                                items,
                                Some(format!("option '--{}' doesn't allow an argument", opt.long)),
                            );
                        }
                        None
                    }
                    Arg::Required => match inline {
                        Some(v) => Some(v),
                        None => {
                            if i >= args.len() {
                                return (
                                    items,
                                    Some(format!("option '--{}' requires an argument", opt.long)),
                                );
                            }
                            let v = args[i].clone();
                            i += 1;
                            Some(v)
                        }
                    },
                    // getopt_long never reaches across argv for an optional
                    // argument, only across the same token's '='.
                    Arg::Optional => inline,
                };
                items.push(Item::Flag {
                    long: opt.long,
                    value,
                });
            } else if text.len() > 1 && text.starts_with('-') {
                // Every char up to and including the one that takes a
                // value is a single-byte ASCII short option letter, so its
                // position in this lossy text is also its byte offset in
                // the raw argument, which is what the value is sliced from.
                let cluster = text[1..].to_string();
                let raw_bytes = raw.as_bytes();
                i += 1;
                for (pos, flag) in cluster.char_indices() {
                    let opt = match self.resolve_short(flag) {
                        Ok(opt) => opt,
                        Err(message) => return (items, Some(message)),
                    };
                    match opt.arg {
                        Arg::None => {
                            items.push(Item::Flag {
                                long: opt.long,
                                value: None,
                            });
                        }
                        Arg::Required => {
                            let rest = &raw_bytes[1 + pos + flag.len_utf8()..];
                            let value = if rest.is_empty() {
                                if i >= args.len() {
                                    return (
                                        items,
                                        Some(format!("option requires an argument -- '{flag}'")),
                                    );
                                }
                                let v = args[i].clone();
                                i += 1;
                                v
                            } else {
                                os_from_bytes(rest)
                            };
                            items.push(Item::Flag {
                                long: opt.long,
                                value: Some(value),
                            });
                            break;
                        }
                        Arg::Optional => {
                            let rest = &raw_bytes[1 + pos + flag.len_utf8()..];
                            let value = if rest.is_empty() {
                                None
                            } else {
                                Some(os_from_bytes(rest))
                            };
                            items.push(Item::Flag {
                                long: opt.long,
                                value,
                            });
                            break;
                        }
                    }
                }
            } else {
                if self.mode == Mode::StopAtFirstOperand {
                    operands_only = true;
                }
                items.push(Item::Operand(raw.clone()));
                i += 1;
            }
        }
        (items, None)
    }

    /// Parse or report the GNU-shaped error and exit: `<program>: <diagnostic>`,
    /// the "Try" line, then `exit_code`.
    pub fn parse_or_exit(&self, args: &[OsString], exit_code: i32) -> Vec<Item> {
        self.parse(args)
            .unwrap_or_else(|message| usage_error(self.program, &message, exit_code))
    }

    /// Resolve a long option by exact name or unambiguous prefix, the way
    /// getopt_long does, with its exact diagnostics. `full` is the whole
    /// original token (`--kill-after=1`, not just `kill-after`), since
    /// that is what the unrecognized-option diagnostic quotes.
    fn resolve_long(&self, name: &str, full: &str) -> Result<Opt, String> {
        let candidates: Vec<&Opt> = self
            .opts
            .iter()
            .filter(|o| o.long.starts_with(name))
            .collect();
        if let Some(exact) = candidates.iter().find(|o| o.long == name) {
            return Ok(**exact);
        }
        match candidates.as_slice() {
            [] => Err(format!("unrecognized option '{full}'")),
            [one] => Ok(**one),
            many => {
                let list: Vec<String> = many.iter().map(|o| format!("'--{}'", o.long)).collect();
                Err(format!(
                    "option '--{name}' is ambiguous; possibilities: {}",
                    list.join(" ")
                ))
            }
        }
    }

    fn resolve_short(&self, flag: char) -> Result<Opt, String> {
        self.opts
            .iter()
            .find(|o| o.short == Some(flag))
            .copied()
            .ok_or_else(|| format!("invalid option -- '{flag}'"))
    }
}

/// Build an option argument straight from the raw bytes it occupied,
/// bypassing any UTF-8 conversion that could corrupt a non-UTF-8 value.
fn os_from_bytes(bytes: &[u8]) -> OsString {
    OsStr::from_bytes(bytes).to_os_string()
}

/// Print a diagnostic the way GNU `error()` does, add the "Try" line, exit.
pub fn usage_error(program: &str, message: &str, exit_code: i32) -> ! {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "{program}: {message}");
    let _ = writeln!(err, "Try '{program} --help' for more information.");
    let _ = err.flush();
    process::exit(exit_code)
}

/// The text for an option a command recognizes but has not implemented,
/// pointing at a workaround and where to ask for it. Split from
/// `unsupported` below so it can be unit-tested without a process exit.
pub fn unsupported_text(program: &str, option: &str, use_instead: Option<&str>) -> String {
    let mut text = format!("{program}: {option} is not implemented in this build\n");
    if let Some(instead) = use_instead {
        text.push_str(&format!("Use instead: {instead}\n"));
    }
    text.push_str(&format!(
        "See {}/issues to request it\n",
        env!("CARGO_PKG_REPOSITORY")
    ));
    text
}

pub fn unsupported(program: &str, option: &str, use_instead: Option<&str>) -> ! {
    let mut err = std::io::stderr().lock();
    let _ = write!(err, "{}", unsupported_text(program, option, use_instead));
    let _ = err.flush();
    process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    const OPTS: &[Opt] = &[
        Opt {
            short: Some('a'),
            long: "all",
            arg: Arg::None,
        },
        Opt {
            short: Some('k'),
            long: "kill-after",
            arg: Arg::Required,
        },
        Opt {
            short: Some('d'),
            long: "differences",
            arg: Arg::Optional,
        },
        Opt {
            short: None,
            long: "help",
            arg: Arg::None,
        },
        Opt {
            short: Some('v'),
            long: "verbose",
            arg: Arg::None,
        },
        Opt {
            short: None,
            long: "version",
            arg: Arg::None,
        },
    ];

    fn parse(mode: Mode, args: &[&str]) -> Result<Vec<Item>, String> {
        let owned: Vec<OsString> = args.iter().map(OsString::from).collect();
        Parser::new("t", OPTS, mode).parse(&owned)
    }

    /// Looks up `long` in `OPTS` and hands back the `&'static str` already
    /// owned by that entry, so a test fixture can build an `Item::Flag`
    /// without leaking memory of its own.
    fn long_name(long: &str) -> &'static str {
        OPTS.iter()
            .find(|o| o.long == long)
            .map(|o| o.long)
            .expect("known option")
    }

    fn flag(long: &str) -> Item {
        Item::Flag {
            long: long_name(long),
            value: None,
        }
    }
    fn flag_with(long: &str, value: &str) -> Item {
        Item::Flag {
            long: long_name(long),
            value: Some(OsString::from(value)),
        }
    }
    fn operand(text: &str) -> Item {
        Item::Operand(OsString::from(text))
    }

    #[test]
    fn long_options_exact_and_by_prefix() {
        assert_eq!(parse(Mode::Permute, &["--all"]).unwrap(), vec![flag("all")]);
        assert_eq!(parse(Mode::Permute, &["--al"]).unwrap(), vec![flag("all")]);
        assert_eq!(
            parse(Mode::Permute, &["--kill-after=1"]).unwrap(),
            vec![flag_with("kill-after", "1")]
        );
        assert_eq!(
            parse(Mode::Permute, &["--kill=1"]).unwrap(),
            vec![flag_with("kill-after", "1")]
        );
        assert_eq!(
            parse(Mode::Permute, &["--kill-after", "1"]).unwrap(),
            vec![flag_with("kill-after", "1")]
        );
    }

    #[test]
    fn ambiguous_prefix_lists_the_possibilities() {
        assert_eq!(
            parse(Mode::Permute, &["--v"]).unwrap_err(),
            "option '--v' is ambiguous; possibilities: '--verbose' '--version'"
        );
    }

    #[test]
    fn rejects_unknown_and_malformed_options() {
        assert_eq!(
            parse(Mode::Permute, &["--bogus"]).unwrap_err(),
            "unrecognized option '--bogus'"
        );
        assert_eq!(
            parse(Mode::Permute, &["-x"]).unwrap_err(),
            "invalid option -- 'x'"
        );
        assert_eq!(
            parse(Mode::Permute, &["--kill-after"]).unwrap_err(),
            "option '--kill-after' requires an argument"
        );
        assert_eq!(
            parse(Mode::Permute, &["-k"]).unwrap_err(),
            "option requires an argument -- 'k'"
        );
        assert_eq!(
            parse(Mode::Permute, &["--all=1"]).unwrap_err(),
            "option '--all' doesn't allow an argument"
        );
    }

    #[test]
    fn short_options_cluster_and_take_attached_arguments() {
        assert_eq!(
            parse(Mode::Permute, &["-av"]).unwrap(),
            vec![flag("all"), flag("verbose")]
        );
        assert_eq!(
            parse(Mode::Permute, &["-k5"]).unwrap(),
            vec![flag_with("kill-after", "5")]
        );
        assert_eq!(
            parse(Mode::Permute, &["-ak5"]).unwrap(),
            vec![flag("all"), flag_with("kill-after", "5")]
        );
        assert_eq!(
            parse(Mode::Permute, &["-k", "5"]).unwrap(),
            vec![flag_with("kill-after", "5")]
        );
    }

    #[test]
    fn optional_arguments_must_be_attached() {
        assert_eq!(
            parse(Mode::Permute, &["-d"]).unwrap(),
            vec![flag("differences")]
        );
        assert_eq!(
            parse(Mode::Permute, &["-dpermanent"]).unwrap(),
            vec![flag_with("differences", "permanent")]
        );
        assert_eq!(
            parse(Mode::Permute, &["--differences"]).unwrap(),
            vec![flag("differences")]
        );
        assert_eq!(
            parse(Mode::Permute, &["--differences=permanent"]).unwrap(),
            vec![flag_with("differences", "permanent")]
        );
        assert_eq!(
            parse(Mode::Permute, &["-d", "1"]).unwrap(),
            vec![flag("differences"), operand("1")]
        );
    }

    #[test]
    fn operand_handling_differs_by_mode() {
        assert_eq!(
            parse(Mode::Permute, &["x", "-a", "y"]).unwrap(),
            vec![operand("x"), flag("all"), operand("y")]
        );
        assert_eq!(
            parse(Mode::StopAtFirstOperand, &["x", "-a"]).unwrap(),
            vec![operand("x"), operand("-a")]
        );
        assert_eq!(
            parse(Mode::Permute, &["--", "-a"]).unwrap(),
            vec![operand("-a")]
        );
        assert_eq!(
            parse(Mode::StopAtFirstOperand, &["--", "-a"]).unwrap(),
            vec![operand("-a")]
        );
        assert_eq!(parse(Mode::Permute, &["-"]).unwrap(), vec![operand("-")]);
    }

    #[test]
    fn parse_partial_keeps_items_seen_before_the_failure() {
        let owned: Vec<OsString> = ["--all", "--bogus"].iter().map(OsString::from).collect();
        let (items, error) = Parser::new("t", OPTS, Mode::Permute).parse_partial(&owned);
        assert_eq!(items, vec![flag("all")]);
        assert_eq!(error, Some("unrecognized option '--bogus'".to_string()));

        let owned: Vec<OsString> = ["--bogus", "--all"].iter().map(OsString::from).collect();
        let (items, error) = Parser::new("t", OPTS, Mode::Permute).parse_partial(&owned);
        assert_eq!(items, vec![]);
        assert_eq!(error, Some("unrecognized option '--bogus'".to_string()));
    }

    #[test]
    fn flag_values_preserve_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;

        // A lone continuation byte, 0x80, is never valid UTF-8 on its own;
        // a lossy conversion would corrupt it into the replacement char.
        let raw = OsStr::from_bytes(b"--kill-after=\x80").to_os_string();
        let (items, error) = Parser::new("t", OPTS, Mode::Permute).parse_partial(&[raw]);
        assert_eq!(error, None);
        match items.as_slice() {
            [Item::Flag { long, value }] => {
                assert_eq!(*long, "kill-after");
                assert_eq!(value.as_deref(), Some(OsStr::from_bytes(b"\x80")));
            }
            other => panic!("expected one flag, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_options_print_a_runnable_command() {
        assert_eq!(
            unsupported_text("tac", "--regex", Some("tac -s '\\n'")),
            "tac: --regex is not implemented in this build\n\
             Use instead: tac -s '\\n'\n\
             See https://github.com/jordiboehme/agent-cli-tools/issues to request it\n"
        );
        assert_eq!(
            unsupported_text("nproc", "--all-sockets", None),
            "nproc: --all-sockets is not implemented in this build\n\
             See https://github.com/jordiboehme/agent-cli-tools/issues to request it\n"
        );
    }
}
