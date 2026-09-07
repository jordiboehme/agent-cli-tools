//! `tree`: list the contents of directories as a tree. A port of a
//! documented subset of tree 2.3.2 by Steve Baker, matching its output
//! byte for byte, its sorting, its messages and its exit codes. The
//! options it leaves out are still recognized and refused by name, each
//! with the macOS command to reach for instead.

use agent_cli_tools::options::{self, Arg, Item, Mode, Opt, Parser};
use std::cmp::Ordering;
use std::ffi::{CString, OsStr, OsString};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::process;

/// The first ten lines of `tree --help`, which a usage error prints on
/// its own. Reproduced byte for byte, tabs included, because that is
/// what a script grepping for an option name sees.
const USAGE: &str = "\
usage: tree [-acdfghilnpqrstuvxACDFJQNSUX] [-L level [-R]] [-H [-]baseHREF]\n\
\t[-T title] [-o filename] [-P pattern] [-I pattern] [--gitignore]\n\
\t[--gitfile[=]file] [--matchdirs] [--metafirst] [--ignore-case]\n\
\t[--nolinks] [--hintro[=]file] [--houtro[=]file] [--inodes] [--device]\n\
\t[--sort[=]name] [--dirsfirst] [--filesfirst] [--filelimit[=]#] [--si]\n\
\t[--du] [--prune] [--charset[=]X] [--timefmt[=]format] [--fromfile]\n\
\t[--fromtabfile] [--fflinks] [--info] [--infofile[=]file] [--noreport]\n\
\t[--hyperlink] [--scheme[=]schema] [--authority[=]host] [--opt-toggle]\n\
\t[--compress[=]#] [--condense] [--version] [--help]\n\
\t[--] [directory ...]\n\
";

/// The rest of `tree --help`. Every option upstream has is listed, the
/// unimplemented ones included, so a reader of the help text and a
/// reader of the man page see the same surface; the ones this build does
/// not implement say so when they are used.
const HELP_BODY: &str = "\
  ------- Listing options -------\n\
  -a            All files are listed.\n\
  -d            List directories only.\n\
  -l            Follow symbolic links like directories.\n\
  -f            Print the full path prefix for each file.\n\
  -x            Stay on current filesystem only.\n\
  -L level      Descend only level directories deep.\n\
  -R            Rerun tree when max dir level reached.\n\
  -P pattern    List only those files that match the pattern given.\n\
  -I pattern    Do not list files that match the given pattern.\n\
  --gitignore   Filter by using .gitignore files.\n\
  --gitfile X   Explicitly read a gitignore file.\n\
  --ignore-case Ignore case when pattern matching.\n\
  --matchdirs   Include directory names in -P pattern matching.\n\
  --metafirst   Print meta-data at the beginning of each line.\n\
  --prune       Prune empty directories from the output.\n\
  --info        Print information about files found in .info files.\n\
  --infofile X  Explicitly read info file.\n\
  --noreport    Turn off file/directory count at end of tree listing.\n\
  --charset X   Use charset X for terminal/HTML and indentation line output.\n\
  --filelimit # Do not descend dirs with more than # files in them.\n\
  --condense    Condense directory singletons to a single line of output.\n\
  -o filename   Output to file instead of stdout.\n\
  ------- File options -------\n\
  -q            Print non-printable characters as '?'.\n\
  -N            Print non-printable characters as is.\n\
  -Q            Quote filenames with double quotes.\n\
  -p            Print the protections for each file.\n\
  -u            Displays file owner or UID number.\n\
  -g            Displays file group owner or GID number.\n\
  -s            Print the size in bytes of each file.\n\
  -h            Print the size in a more human readable way.\n\
  --si          Like -h, but use in SI units (powers of 1000).\n\
  --du          Compute size of directories by their contents.\n\
  -D            Print the date of last modification or (-c) status change.\n\
  --timefmt fmt Print and format time according to the format fmt.\n\
  -F            Appends '/', '=', '*', '@', '|' or '>' as per ls -F.\n\
  --inodes      Print inode number of each file.\n\
  --device      Print device ID number to which each file belongs.\n\
  ------- Sorting options -------\n\
  -v            Sort files alphanumerically by version.\n\
  -t            Sort files by last modification time.\n\
  -c            Sort files by last status change time.\n\
  -U            Leave files unsorted.\n\
  -r            Reverse the order of the sort.\n\
  --dirsfirst   List directories before files (-U disables).\n\
  --filesfirst  List files before directories (-U disables).\n\
  --sort X      Select sort: name,version,size,mtime,ctime,none.\n\
  ------- Graphics options -------\n\
  -i            Don't print indentation lines.\n\
  -A            Print ANSI lines graphic indentation lines.\n\
  -S            Print with CP437 (console) graphics indentation lines.\n\
  -n            Turn colorization off always (-C overrides).\n\
  -C            Turn colorization on always.\n\
  --compress #  Compress indentation lines.\n\
  ------- XML/HTML/JSON/HYPERLINK options -------\n\
  -X            Prints out an XML representation of the tree.\n\
  -J            Prints out an JSON representation of the tree.\n\
  -H baseHREF   Prints out HTML format with baseHREF as top directory.\n\
  -T string     Replace the default HTML title and H1 header with string.\n\
  --nolinks     Turn off hyperlinks in HTML output.\n\
  --hintro X    Use file X as the HTML intro.\n\
  --houtro X    Use file X as the HTML outro.\n\
  --hyperlink   Turn on OSC 8 terminal hyperlinks.\n\
  --scheme X    Set OSC 8 hyperlink scheme, default file://\n\
  --authority X Set OSC 8 hyperlink authority/hostname.\n\
  ------- Input options -------\n\
  --fromfile    Reads paths from files (.=stdin)\n\
  --fromtabfile Reads trees from tab indented files (.=stdin)\n\
  --fflinks     Process link information when using --fromfile.\n\
  ------- Miscellaneous options -------\n\
  --opt-toggle  Enable option toggling.\n\
  --version     Print version and exit.\n\
  --help        Print usage and this help message and exit.\n\
  --            Options processing terminator.\n\
\n\
Part of agent-cli-tools <https://github.com/jordiboehme/agent-cli-tools>\n\
Compatible with tree 2.3.2; the options it leaves out are refused by name.\n\
";

/// Every option tree recognizes. The ones with no long form upstream
/// carry a name beginning with `=`, the convention this crate's parser
/// uses for a short-only option: a long name is read up to its first
/// `=`, so such a name can never be produced by a command line and the
/// option stays exactly as short-only as tree has it.
const TREE_OPTS: &[Opt] = &[
    // Implemented.
    short('a', "=all"),
    short('d', "=dironly"),
    short('f', "=fullpath"),
    short('i', "=noindent"),
    short_arg('L', "=level"),
    short_arg('P', "=include"),
    short_arg('I', "=exclude"),
    short('h', "=human"),
    short('s', "=bytes"),
    short('p', "=protections"),
    short('D', "=date"),
    short('F', "=classify"),
    short('Q', "=quote"),
    short('C', "=color"),
    short('n', "=nocolor"),
    short_arg('o', "=outfile"),
    short('J', "=json"),
    short('t', "=mtimesort"),
    short('c', "=ctimesort"),
    short('v', "=versionsort"),
    short('U', "=unsorted"),
    short('r', "=reverse"),
    long("prune", Arg::None),
    long("matchdirs", Arg::None),
    long("ignore-case", Arg::None),
    long("noreport", Arg::None),
    long("charset", Arg::Required),
    long("dirsfirst", Arg::None),
    long("filesfirst", Arg::None),
    long("sort", Arg::Required),
    long("si", Arg::None),
    long("filelimit", Arg::Required),
    long("timefmt", Arg::Required),
    long("help", Arg::None),
    long("version", Arg::None),
    // Recognized but not implemented. Registered with the argument each
    // one takes upstream, so its argument cannot be mistaken for an
    // operand and end up in the "Use instead" command line.
    short('l', "=followlinks"),
    short('x', "=onefs"),
    short('R', "=rerun"),
    short('q', "=nonprint-mask"),
    short('N', "=nonprint-raw"),
    short('u', "=owner"),
    short('g', "=group"),
    short('A', "=ansi"),
    short('S', "=cp437"),
    short('X', "=xml"),
    short_arg('H', "=html"),
    short_arg('T', "=title"),
    long("gitignore", Arg::None),
    long("gitfile", Arg::Required),
    long("metafirst", Arg::None),
    long("info", Arg::None),
    long("infofile", Arg::Required),
    long("condense", Arg::None),
    long("compress", Arg::Required),
    long("du", Arg::None),
    long("inodes", Arg::None),
    long("device", Arg::None),
    long("nolinks", Arg::None),
    long("hintro", Arg::Required),
    long("houtro", Arg::Required),
    long("hyperlink", Arg::None),
    long("scheme", Arg::Required),
    long("authority", Arg::Required),
    long("fromfile", Arg::None),
    long("fromtabfile", Arg::None),
    long("fflinks", Arg::None),
    long("opt-toggle", Arg::None),
];

const fn short(c: char, long: &'static str) -> Opt {
    Opt {
        short: Some(c),
        long,
        arg: Arg::None,
    }
}

const fn short_arg(c: char, long: &'static str) -> Opt {
    Opt {
        short: Some(c),
        long,
        arg: Arg::Required,
    }
}

const fn long(long: &'static str, arg: Arg) -> Opt {
    Opt {
        short: None,
        long,
        arg,
    }
}

/// An option this build recognizes but does not implement: how the
/// message names it, and the command line to print instead of it.
/// `instead` is a template whose `{}` is replaced by the operands the
/// caller actually gave, so the printed line can be run as it stands.
struct Deferred {
    long: &'static str,
    name: &'static str,
    instead: Option<&'static str>,
}

const DEFERRED: &[Deferred] = &[
    deferred("=followlinks", "-l", None),
    deferred("=onefs", "-x", Some("find {} -xdev")),
    deferred("=rerun", "-R", None),
    deferred("=nonprint-mask", "-q", None),
    deferred("=nonprint-raw", "-N", None),
    deferred("=owner", "-u", Some("ls -l {}")),
    deferred("=group", "-g", Some("ls -l {}")),
    deferred("=ansi", "-A", None),
    deferred("=cp437", "-S", None),
    deferred("=xml", "-X", Some("tree -J {}")),
    deferred("=html", "-H", None),
    deferred("=title", "-T", None),
    deferred(
        "gitignore",
        "--gitignore",
        Some("tree -I 'node_modules|target|.git' {}"),
    ),
    deferred("gitfile", "--gitfile", None),
    deferred("metafirst", "--metafirst", None),
    deferred("info", "--info", None),
    deferred("infofile", "--infofile", None),
    deferred("condense", "--condense", None),
    deferred("compress", "--compress", None),
    deferred("du", "--du", Some("du -sh {}")),
    deferred("inodes", "--inodes", Some("ls -i {}")),
    deferred("device", "--device", Some("find {} -xdev")),
    deferred("nolinks", "--nolinks", None),
    deferred("hintro", "--hintro", None),
    deferred("houtro", "--houtro", None),
    deferred("hyperlink", "--hyperlink", None),
    deferred("scheme", "--scheme", None),
    deferred("authority", "--authority", None),
    deferred("fromfile", "--fromfile", None),
    deferred("fromtabfile", "--fromtabfile", None),
    deferred("fflinks", "--fflinks", None),
    deferred("opt-toggle", "--opt-toggle", None),
];

const fn deferred(
    long: &'static str,
    name: &'static str,
    instead: Option<&'static str>,
) -> Deferred {
    Deferred {
        long,
        name,
        instead,
    }
}

// ---------------------------------------------------------------- charset

/// The four indentation pieces: the branch to a middle child, the branch
/// to the last child, and the two continuations drawn under them.
struct Charset {
    tee: &'static str,
    corner: &'static str,
    vertical: &'static str,
    blank: &'static str,
}

/// The vertical continuation is `|` followed by two NON-BREAKING spaces,
/// not two ordinary ones. Copying tree's output out of a terminal and
/// pasting it into a test would silently substitute plain spaces, so the
/// bytes are spelled out here.
const UTF8_CHARSET: Charset = Charset {
    tee: "\u{251c}\u{2500}\u{2500} ",
    corner: "\u{2514}\u{2500}\u{2500} ",
    vertical: "\u{2502}\u{a0}\u{a0} ",
    blank: "    ",
};

const ASCII_CHARSET: Charset = Charset {
    tee: "|-- ",
    corner: "`-- ",
    vertical: "|   ",
    blank: "    ",
};

fn is_utf8_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "utf-8" || name == "utf8"
}

/// UTF-8 line drawing only when something says the terminal can take it:
/// an explicit `--charset`, `TREE_CHARSET`, or the locale's own codeset.
/// Anything else, `--charset=ascii` included, falls back to ASCII.
fn choose_charset(requested: Option<&str>) -> &'static Charset {
    if let Some(name) = requested {
        return pick(is_utf8_name(name));
    }
    if let Ok(name) = std::env::var("TREE_CHARSET") {
        return pick(is_utf8_name(&name));
    }
    pick(is_utf8_name(&locale_codeset()))
}

fn pick(utf8: bool) -> &'static Charset {
    if utf8 { &UTF8_CHARSET } else { &ASCII_CHARSET }
}

/// The locale's own codeset, which is what decides how a name is
/// written out. `--charset` moves the line drawing without moving this,
/// exactly as it does upstream.
fn locale_codeset() -> String {
    // SAFETY: nl_langinfo returns a pointer to static storage owned by
    // the C library, valid until the next setlocale; nothing here calls
    // setlocale again, and the string is copied before returning.
    unsafe {
        let p = libc::nl_langinfo(libc::CODESET);
        if p.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

// ---------------------------------------------------------------- options

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Name,
    Version,
    Size,
    Mtime,
    Ctime,
    None,
}

struct Options {
    all: bool,
    dir_only: bool,
    full_path: bool,
    no_indent: bool,
    level: Option<i64>,
    include: Vec<Vec<u8>>,
    exclude: Vec<Vec<u8>>,
    human: bool,
    si: bool,
    bytes: bool,
    protections: bool,
    date: bool,
    classify: bool,
    quote: bool,
    force_color: bool,
    no_color: bool,
    json: bool,
    ctime: bool,
    sort: Sort,
    unsorted: bool,
    reverse: bool,
    dirs_first: bool,
    files_first: bool,
    prune: bool,
    match_dirs: bool,
    ignore_case: bool,
    no_report: bool,
    file_limit: u64,
    timefmt: Option<CString>,
    charset: &'static Charset,
    utf8_names: bool,
    palette: Option<Palette>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            all: false,
            dir_only: false,
            full_path: false,
            no_indent: false,
            level: None,
            include: Vec::new(),
            exclude: Vec::new(),
            human: false,
            si: false,
            bytes: false,
            protections: false,
            date: false,
            classify: false,
            quote: false,
            force_color: false,
            no_color: false,
            json: false,
            ctime: false,
            sort: Sort::Name,
            unsorted: false,
            reverse: false,
            dirs_first: false,
            files_first: false,
            prune: false,
            match_dirs: false,
            ignore_case: false,
            no_report: false,
            file_limit: 0,
            timefmt: None,
            charset: &ASCII_CHARSET,
            utf8_names: false,
            palette: None,
        }
    }
}

impl Options {
    fn wants_meta(&self) -> bool {
        self.protections || self.bytes || self.human || self.si || self.date
    }
}

/// tree's own usage error: the message, then the usage block, exit 1.
/// Not `options::usage_error`, which adds the GNU "Try --help" line tree
/// does not print.
fn fail(message: &str, with_usage: bool) -> ! {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "tree: {message}");
    if with_usage {
        let _ = err.write_all(USAGE.as_bytes());
    }
    let _ = err.flush();
    process::exit(1)
}

// ------------------------------------------------------------ glob matching

/// Split a pattern on its top-level `|`, tree's alternation. A `|`
/// inside a bracket group or behind a backslash is a literal.
fn alternatives(pattern: &[u8]) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut in_class = false;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' => i += 1,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'|' if !in_class => {
                parts.push(&pattern[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&pattern[start..]);
    parts
}

fn fold(byte: u8, ignore_case: bool) -> u8 {
    if ignore_case {
        byte.to_ascii_lowercase()
    } else {
        byte
    }
}

/// tree's `patmatch` for one alternative: `*` stops at a `/`, `**`
/// crosses them, `?` takes one byte, `[...]` and `[^...]` are byte
/// classes with ranges, and a backslash escapes what follows.
fn glob(pattern: &[u8], text: &[u8], ignore_case: bool) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            let (rest, crosses) = if pattern.len() > 1 && pattern[1] == b'*' {
                (&pattern[2..], true)
            } else {
                (&pattern[1..], false)
            };
            for take in 0..=text.len() {
                if !crosses && text[..take].contains(&b'/') {
                    break;
                }
                if glob(rest, &text[take..], ignore_case) {
                    return true;
                }
            }
            false
        }
        b'?' => !text.is_empty() && glob(&pattern[1..], &text[1..], ignore_case),
        b'[' => {
            if text.is_empty() {
                return false;
            }
            match class_match(pattern, text[0], ignore_case) {
                Some((matched, rest)) => matched && glob(rest, &text[1..], ignore_case),
                None => false,
            }
        }
        b'\\' if pattern.len() > 1 => {
            !text.is_empty()
                && fold(pattern[1], ignore_case) == fold(text[0], ignore_case)
                && glob(&pattern[2..], &text[1..], ignore_case)
        }
        expected => {
            !text.is_empty()
                && fold(expected, ignore_case) == fold(text[0], ignore_case)
                && glob(&pattern[1..], &text[1..], ignore_case)
        }
    }
}

/// Read one `[...]` group at the head of `pattern` and say whether
/// `byte` is in it, handing back what follows the closing bracket. An
/// unterminated group never matches, which is what tree's own reader
/// does with it.
fn class_match(pattern: &[u8], byte: u8, ignore_case: bool) -> Option<(bool, &[u8])> {
    let mut i = 1;
    let negate = pattern.get(i) == Some(&b'^');
    if negate {
        i += 1;
    }
    let mut hit = false;
    let byte = fold(byte, ignore_case);
    while i < pattern.len() && pattern[i] != b']' {
        let mut low = pattern[i];
        if low == b'\\' && i + 1 < pattern.len() {
            i += 1;
            low = pattern[i];
        }
        if pattern.get(i + 1) == Some(&b'-') && pattern.get(i + 2).is_some_and(|c| *c != b']') {
            let mut high = pattern[i + 2];
            i += 2;
            if high == b'\\' && i + 1 < pattern.len() {
                i += 1;
                high = pattern[i];
            }
            if byte >= fold(low, ignore_case) && byte <= fold(high, ignore_case) {
                hit = true;
            }
        } else if fold(low, ignore_case) == byte {
            hit = true;
        }
        i += 1;
    }
    if i >= pattern.len() {
        return None;
    }
    Some((hit != negate, &pattern[i + 1..]))
}

/// Whether any of `patterns` matches this entry. tree tries a pattern
/// against the whole path and against every suffix of it that starts
/// after a `/`, so `-I 'b/c'` reaches `demo/a/b/c` and a pattern with no
/// slash at all is effectively matched against the base name. A trailing
/// `/` in the pattern restricts it to directories.
fn matches_any(patterns: &[Vec<u8>], path: &[u8], is_dir: bool, ignore_case: bool) -> bool {
    for pattern in patterns {
        for alt in alternatives(pattern) {
            let (alt, dirs_only) = match alt.strip_suffix(b"/") {
                Some(stripped) => (stripped, true),
                None => (alt, false),
            };
            if dirs_only && !is_dir {
                continue;
            }
            let mut at = 0;
            loop {
                if glob(alt, &path[at..], ignore_case) {
                    return true;
                }
                match path[at..].iter().position(|b| *b == b'/') {
                    Some(slash) => at += slash + 1,
                    None => break,
                }
            }
        }
    }
    false
}

// -------------------------------------------------------------- version sort

/// A port of glibc's `strverscmp`, which is what tree's `-v` sorts with
/// and what macOS's C library does not have: digit runs compare as
/// numbers, and a run with a leading zero compares as a fraction, so
/// `a02` sorts before `a1`.
fn version_cmp(a: &[u8], b: &[u8]) -> Ordering {
    const S_N: usize = 0;
    const S_I: usize = 3;
    const S_F: usize = 6;
    const S_Z: usize = 9;
    const CMP: i8 = 2;
    const LEN: i8 = 3;
    const NEXT: [usize; 12] = [
        S_N, S_I, S_Z, // S_N
        S_N, S_I, S_I, // S_I
        S_N, S_F, S_F, // S_F
        S_N, S_F, S_Z, // S_Z
    ];
    #[rustfmt::skip]
    const RESULT: [i8; 36] = [
        CMP, CMP, CMP, CMP, LEN, CMP, CMP, CMP, CMP, // S_N
        CMP,  -1,  -1,   1, LEN, LEN,   1, LEN, LEN, // S_I
        CMP, CMP, CMP, CMP, CMP, CMP, CMP, CMP, CMP, // S_F
        CMP,   1,   1,  -1, CMP, CMP,  -1, CMP, CMP, // S_Z
    ];

    let at = |s: &[u8], i: usize| -> i32 { s.get(i).map_or(0, |b| i32::from(*b)) };
    let class = |c: i32| -> usize {
        usize::from(c == i32::from(b'0'))
            + usize::from(c.is_positive() && (c as u8).is_ascii_digit())
    };

    let (mut i, mut j) = (0usize, 0usize);
    let (mut c1, mut c2) = (at(a, i), at(b, j));
    i += 1;
    j += 1;
    let mut state = S_N + class(c1);
    let mut diff = c1 - c2;
    while diff == 0 {
        if c1 == 0 {
            return Ordering::Equal;
        }
        state = NEXT[state];
        c1 = at(a, i);
        c2 = at(b, j);
        i += 1;
        j += 1;
        state += class(c1);
        diff = c1 - c2;
    }
    match RESULT[state * 3 + class(c2)] {
        CMP => diff.cmp(&0),
        LEN => {
            while at(a, i - 1).is_positive() && (at(a, i - 1) as u8).is_ascii_digit() {
                if !(at(b, j - 1).is_positive() && (at(b, j - 1) as u8).is_ascii_digit()) {
                    return Ordering::Greater;
                }
                i += 1;
                j += 1;
            }
            if at(b, j - 1).is_positive() && (at(b, j - 1) as u8).is_ascii_digit() {
                Ordering::Less
            } else {
                diff.cmp(&0)
            }
        }
        other => other.cmp(&0),
    }
}

/// `strcoll` so the locale decides the order, exactly as tree does, with
/// a byte-order tie-break: a comparator that calls two distinct names
/// equal is not a total order, and Rust's sort refuses to run on one.
fn collate(a: &OsStr, b: &OsStr) -> Ordering {
    let (left, right) = (CString::new(a.as_bytes()), CString::new(b.as_bytes()));
    if let (Ok(left), Ok(right)) = (left, right) {
        // SAFETY: both are NUL-terminated C strings that outlive the
        // call, which is all strcoll asks of its arguments.
        let order = unsafe { libc::strcoll(left.as_ptr(), right.as_ptr()) };
        if order != 0 {
            return order.cmp(&0);
        }
    }
    a.as_bytes().cmp(b.as_bytes())
}

// -------------------------------------------------------------------- colour

/// The colour codes tree looks up per entry, from `TREE_COLORS`, else
/// `LS_COLORS`, else its own defaults. A key the palette does not carry
/// leaves that kind of entry uncoloured, exactly as upstream leaves it.
struct Palette {
    keys: Vec<(String, String)>,
    extensions: Vec<(String, String)>,
    reset: String,
}

/// tree's own palette, used only when neither variable is set. It is the
/// old GNU default set, which is why a sticky directory here is plain
/// `di` rather than the `tw` a modern dircolors would give it.
const DEFAULT_COLORS: &str = "no=00:fi=00:di=01;34:ln=01;36:pi=40;33:so=01;35:\
bd=40;33;01:cd=40;33;01:or=40;31;01:ex=01;32";

impl Palette {
    fn parse(spec: &str) -> Palette {
        let mut palette = Palette {
            keys: Vec::new(),
            extensions: Vec::new(),
            reset: "0".into(),
        };
        for item in spec.split(':') {
            let Some((key, value)) = item.split_once('=') else {
                continue;
            };
            match key {
                "rs" => palette.reset = value.into(),
                _ if key.starts_with('*') => {
                    palette.extensions.push((key[1..].into(), value.into()));
                }
                _ => palette.keys.push((key.into(), value.into())),
            }
        }
        palette
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, code)| code.as_str())
    }

    /// The colour for a mode that is not a directory or a link: the file
    /// type first, then the executable bit, then the name's extension.
    fn for_mode(&self, mode: u32, name: &[u8]) -> Option<&str> {
        match mode & libc::S_IFMT as u32 {
            m if m == libc::S_IFIFO as u32 => return self.get("pi"),
            m if m == libc::S_IFSOCK as u32 => return self.get("so"),
            m if m == libc::S_IFBLK as u32 => return self.get("bd"),
            m if m == libc::S_IFCHR as u32 => return self.get("cd"),
            _ => {}
        }
        if mode & 0o4000 != 0
            && let Some(code) = self.get("su")
        {
            return Some(code);
        }
        if mode & 0o2000 != 0
            && let Some(code) = self.get("sg")
        {
            return Some(code);
        }
        if mode & 0o111 != 0
            && let Some(code) = self.get("ex")
        {
            return Some(code);
        }
        self.extensions
            .iter()
            .find(|(suffix, _)| name.ends_with(suffix.as_bytes()))
            .map(|(_, code)| code.as_str())
            .or_else(|| self.get("fi"))
    }
}

/// tree colours only when it has a palette and either was forced or is
/// writing to a terminal that has not asked for plain text, which is why
/// `-C` shows colour even down a pipe and beats both `-n` and `NO_COLOR`.
fn choose_palette(force: bool, no_color: bool) -> Option<Palette> {
    // SAFETY: isatty only inspects the descriptor and cannot fail in a
    // way that matters here.
    let tty = unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 };
    let force = force || std::env::var_os("CLICOLOR_FORCE").is_some();
    let no_color = no_color || std::env::var_os("NO_COLOR").is_some();
    let spec = std::env::var("TREE_COLORS")
        .or_else(|_| std::env::var("LS_COLORS"))
        .ok();
    let palette = match spec {
        Some(spec) => Some(Palette::parse(&spec)),
        None if force || std::env::var_os("CLICOLOR").is_some() => {
            Some(Palette::parse(DEFAULT_COLORS))
        }
        None => None,
    };
    match palette {
        Some(palette) if force || (!no_color && tty) => Some(palette),
        _ => None,
    }
}

// ------------------------------------------------------------------- walking

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Directory,
    File,
    Link,
    Fifo,
    Socket,
    Block,
    Char,
    Unknown,
}

impl Kind {
    fn json(self) -> &'static str {
        match self {
            Kind::Directory => "directory",
            Kind::File => "file",
            Kind::Link => "link",
            Kind::Fifo => "fifo",
            Kind::Socket => "socket",
            Kind::Block => "block",
            Kind::Char => "char",
            Kind::Unknown => "unknown",
        }
    }
}

/// What is under a directory line, if anything.
enum Kids {
    /// Nothing to descend into: not a directory, a symlink this build
    /// does not follow, or a directory below the depth limit.
    None,
    /// The bracketed note tree prints after the name instead of the
    /// contents, `[error opening dir]` or the filelimit one.
    Note(String),
    List(Vec<Entry>),
}

struct Entry {
    /// The base name as the directory gave it, or, for a root, the
    /// operand exactly as it was typed.
    name: OsString,
    /// The path this entry is reached by, which is also what `-f`
    /// prints: the root operand with each base name appended.
    path: OsString,
    kind: Kind,
    is_dir: bool,
    /// The mode `-F` classifies by, which follows a symlink; the mode
    /// `-p` prints is the entry's own, in `mode`.
    target_mode: u32,
    link: Option<OsString>,
    mode: u32,
    size: u64,
    /// Both times are kept, because the two are chosen independently:
    /// `-c` decides which one `-D` prints, while `--sort` decides which
    /// one the order follows, and `-c -t` picks one of each.
    mtime: i64,
    ctime: i64,
    /// Set when the entry could not be stat'ed at all, which is the only
    /// case that makes a root exit 2.
    missing: bool,
    kids: Kids,
}

fn join(parent: &OsStr, name: &OsStr) -> OsString {
    let mut bytes = parent.as_bytes().to_vec();
    if !bytes.ends_with(b"/") && !bytes.is_empty() {
        bytes.push(b'/');
    }
    bytes.extend_from_slice(name.as_bytes());
    OsString::from_vec(bytes)
}

fn kind_of(meta: &fs::Metadata) -> Kind {
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        Kind::Link
    } else if file_type.is_dir() {
        Kind::Directory
    } else if file_type.is_fifo() {
        Kind::Fifo
    } else if file_type.is_socket() {
        Kind::Socket
    } else if file_type.is_block_device() {
        Kind::Block
    } else if file_type.is_char_device() {
        Kind::Char
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Unknown
    }
}

/// Stat one name and fill in everything the formatters need, without
/// descending: `lstat` decides what the entry is and what `-p`, `-s` and
/// `-D` report, `stat` decides whether it counts and lists as a
/// directory, so a symlink to one counts as a directory without being
/// followed.
fn describe(parent: &OsStr, name: OsString) -> Entry {
    let path = join(parent, &name);
    let own = fs::symlink_metadata(Path::new(&path));
    let missing = own.is_err();
    let (kind, mode, size, mtime, ctime) = match &own {
        Ok(meta) => (
            kind_of(meta),
            meta.mode(),
            meta.size(),
            meta.mtime(),
            meta.ctime(),
        ),
        Err(_) => (Kind::Unknown, 0, 0, 0, 0),
    };
    let link = if kind == Kind::Link {
        fs::read_link(Path::new(&path))
            .ok()
            .map(std::path::PathBuf::into_os_string)
    } else {
        None
    };
    let followed = if kind == Kind::Link {
        fs::metadata(Path::new(&path)).ok()
    } else {
        own.ok()
    };
    Entry {
        name,
        path,
        kind,
        is_dir: followed.as_ref().is_some_and(|m| m.is_dir()),
        target_mode: followed.as_ref().map_or(0, MetadataExt::mode),
        link,
        mode,
        size,
        mtime,
        ctime,
        missing,
        kids: Kids::None,
    }
}

/// Read one directory, filter it the way the options ask and sort it,
/// then walk into whatever is left. `matched` carries `--matchdirs`
/// down: once a directory name has matched `-P`, everything below it is
/// listed whether it matches or not.
fn walk(entry: &mut Entry, depth: i64, matched: bool, o: &Options, failed: &mut bool) {
    // --prune turns every reason a directory has no contents into the
    // same thing, an empty directory to be dropped, and records no
    // error for it. -d turns --prune off, so it turns this off too.
    let pruning = o.prune && !o.dir_only;
    let reader = match fs::read_dir(Path::new(&entry.path)) {
        Ok(reader) => reader,
        Err(_) => {
            if !pruning {
                entry.kids = Kids::Note("error opening dir".into());
                *failed = true;
            }
            return;
        }
    };
    let mut kids: Vec<Entry> = Vec::new();
    for item in reader.flatten() {
        let name = item.file_name();
        if !o.all && name.as_bytes().starts_with(b".") {
            continue;
        }
        let kid = describe(&entry.path, name);
        if o.dir_only && !kid.is_dir {
            continue;
        }
        if !o.exclude.is_empty()
            && matches_any(&o.exclude, kid.path.as_bytes(), kid.is_dir, o.ignore_case)
        {
            continue;
        }
        if !matched && !o.include.is_empty() && !kid.is_dir {
            let hit = matches_any(&o.include, kid.path.as_bytes(), false, o.ignore_case);
            if !hit {
                continue;
            }
        }
        kids.push(kid);
    }
    if o.file_limit > 0 && kids.len() as u64 > o.file_limit {
        if !pruning {
            entry.kids = Kids::Note(format!(
                "{} entries exceeds filelimit, not opening dir",
                kids.len()
            ));
            *failed = true;
        }
        return;
    }
    sort_entries(&mut kids, o);
    let deeper = o.level.is_none_or(|limit| depth + 1 < limit);
    for kid in &mut kids {
        // A symlink to a directory is listed and counted as one but is
        // never descended: this build has no -l.
        if kid.is_dir && kid.kind != Kind::Link && deeper {
            let matched = matched
                || (o.match_dirs
                    && !o.include.is_empty()
                    && matches_any(&o.include, kid.path.as_bytes(), true, o.ignore_case));
            walk(kid, depth + 1, matched, o, failed);
        }
    }
    // -d and --prune together leave every directory in place upstream,
    // since with files gone the emptiness test would remove them all.
    if pruning {
        // Anything with nothing listed under it goes, whether that is an
        // empty directory, one the depth limit stopped at, or a symlink
        // to one, which this build never descends.
        kids.retain(|kid| !kid.is_dir || matches!(&kid.kids, Kids::List(list) if !list.is_empty()));
    }
    entry.kids = Kids::List(kids);
}

fn sort_entries(entries: &mut [Entry], o: &Options) {
    if o.unsorted {
        return;
    }
    match o.sort {
        Sort::Name => entries.sort_by(|a, b| collate(&a.name, &b.name)),
        Sort::Version => entries.sort_by(|a, b| {
            version_cmp(a.name.as_bytes(), b.name.as_bytes())
                .then_with(|| collate(&a.name, &b.name))
        }),
        Sort::Size => {
            entries.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| collate(&a.name, &b.name)))
        }
        Sort::Mtime => entries.sort_by(|a, b| {
            a.mtime
                .cmp(&b.mtime)
                .then_with(|| collate(&a.name, &b.name))
        }),
        Sort::Ctime => entries.sort_by(|a, b| {
            a.ctime
                .cmp(&b.ctime)
                .then_with(|| collate(&a.name, &b.name))
        }),
        Sort::None => {}
    }
    if o.reverse {
        entries.reverse();
    }
    // Stable, so the sort just applied survives inside each group.
    if o.dirs_first {
        entries.sort_by_key(|e| u8::from(!e.is_dir));
    } else if o.files_first {
        entries.sort_by_key(|e| u8::from(e.is_dir));
    }
}

// ----------------------------------------------------------------- meta text

/// tree's `psize`. `-h` and `--si` render into a four-column field, with
/// one documented overflow: 1048575 bytes prints as `1024K`, five
/// columns wide, because the loop only divides while the value is at
/// least the square of the unit.
fn psize(out: &mut String, size: u64, human: bool, si: bool) {
    if !human && !si {
        let _ = write!(out, " {size:>11}");
        return;
    }
    let units: &[u8] = if si { b"bkMGTPEZY" } else { b"BKMGTPEZY" };
    let unit = if si { 1000u64 } else { 1024 };
    let mut size = size;
    let mut idx = usize::from(size >= unit);
    while size >= unit * unit && idx + 1 < units.len() {
        size /= unit;
        idx += 1;
    }
    if idx == 0 {
        let _ = write!(out, " {size:>4}");
        return;
    }
    let scaled = size as f64 / unit as f64;
    let suffix = units[idx] as char;
    // The switch to no decimals happens 52 bytes before the round ten,
    // which is where tree's own rounding puts it.
    if (size + 52) / unit >= 10 {
        let _ = write!(out, " {scaled:>3.0}{suffix}");
    } else {
        let _ = write!(out, " {scaled:>3.1}{suffix}");
    }
}

/// The `drwxr-xr-x` field, with `s`, `S`, `t` and `T` where the special
/// bits are set.
fn protections(out: &mut String, mode: u32, kind: Kind) {
    out.push(' ');
    out.push(match kind {
        Kind::Directory => 'd',
        Kind::Link => 'l',
        Kind::Fifo => 'p',
        Kind::Socket => 's',
        Kind::Block => 'b',
        Kind::Char => 'c',
        _ => '-',
    });
    const BITS: [(u32, char); 9] = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    for (at, (bit, letter)) in BITS.iter().enumerate() {
        let set = mode & bit != 0;
        let special = match at {
            2 => mode & 0o4000 != 0,
            5 => mode & 0o2000 != 0,
            8 => mode & 0o1000 != 0,
            _ => false,
        };
        out.push(match (special, set, at) {
            (true, true, 8) => 't',
            (true, false, 8) => 'T',
            (true, true, _) => 's',
            (true, false, _) => 'S',
            (false, true, _) => *letter,
            (false, false, _) => '-',
        });
    }
}

fn strftime(format: &std::ffi::CStr, when: i64) -> String {
    // SAFETY: localtime_r fills a caller-owned struct from a caller-owned
    // time, and strftime writes at most the buffer length it is given.
    // A zeroed tm is a valid starting point for both.
    unsafe {
        let mut parts: libc::tm = std::mem::zeroed();
        let when = when as libc::time_t;
        if libc::localtime_r(&when, &mut parts).is_null() {
            return String::new();
        }
        let mut buffer = [0u8; 256];
        let written = libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format.as_ptr(),
            &parts,
        );
        String::from_utf8_lossy(&buffer[..written]).into_owned()
    }
}

/// The time `-D` prints, which `-c` switches from modification to
/// status change.
fn shown_time(entry: &Entry, o: &Options) -> i64 {
    if o.ctime { entry.ctime } else { entry.mtime }
}

/// ls's date convention: clock time for the last six months, the year
/// for anything older or further ahead.
fn date_text(when: i64, o: &Options) -> String {
    if let Some(format) = &o.timefmt {
        return strftime(format, when);
    }
    const SIX_MONTHS: i64 = 182 * 24 * 60 * 60;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    if when > now - SIX_MONTHS && when < now + SIX_MONTHS {
        strftime(c"%b %e %R", when)
    } else {
        strftime(c"%b %e  %Y", when)
    }
}

/// tree's `fillinfo`: every field writes itself with a leading space,
/// and the first of those spaces becomes the opening bracket. That is
/// why `-s` alone is eleven columns wide inside the brackets while
/// `-p -s` gives the size twelve.
fn info_field(entry: &Entry, o: &Options) -> String {
    let mut text = String::new();
    if o.protections {
        protections(&mut text, entry.mode, entry.kind);
    }
    if o.bytes || o.human || o.si {
        psize(&mut text, entry.size, o.human, o.si);
    }
    if o.date {
        let _ = write!(text, " {}", date_text(shown_time(entry, o), o));
    }
    if text.starts_with(' ') {
        text.replace_range(0..1, "[");
        text.push(']');
        text.push_str("  ");
    }
    text
}

// ----------------------------------------------------------------- name text

/// How a name is written out, which tree decides from the locale rather
/// than from `--charset`: a multibyte locale prints anything printable
/// as it stands and escapes the rest as `\ooo`, while a single-byte one
/// also escapes the space and the backslash and gives the usual control
/// characters their letter escapes.
fn escape(raw: &[u8], utf8: bool, quoted: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    if utf8 {
        let text = std::str::from_utf8(raw).is_ok();
        for byte in raw {
            let printable = match byte {
                0x00..=0x1f | 0x7f => false,
                0x80..=0xff => text,
                _ => true,
            };
            if printable {
                out.push(*byte);
            } else {
                out.extend_from_slice(format!("\\{byte:03o}").as_bytes());
            }
        }
        return out;
    }
    for byte in raw {
        match byte {
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            // Inside -Q's quotes the space needs no escape and the
            // quote does; outside them it is the other way round.
            b' ' if !quoted => out.extend_from_slice(b"\\ "),
            b'"' if quoted => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x00..=0x1f | 0x7f..=0xff => {
                out.extend_from_slice(format!("\\{byte:03o}").as_bytes());
            }
            _ => out.push(*byte),
        }
    }
    out
}

fn colour_of<'a>(entry: &Entry, palette: &'a Palette) -> Option<&'a str> {
    match entry.kind {
        Kind::Directory => palette.get("di"),
        // A link whose target is gone takes the orphan colour where the
        // palette has one, and the ordinary link colour where it has not.
        Kind::Link if entry.target_mode == 0 => palette.get("or").or_else(|| palette.get("ln")),
        Kind::Link => palette.get("ln"),
        _ => palette.for_mode(entry.mode, entry.name.as_bytes()),
    }
}

/// The `/`, `*`, `=`, `|` or `>` `-F` appends, chosen from the mode a
/// symlink points at, which is why `slink -> real/` gets the slash.
fn classify(entry: &Entry) -> Option<char> {
    if entry.is_dir {
        return Some('/');
    }
    match entry.target_mode & libc::S_IFMT as u32 {
        m if m == libc::S_IFSOCK as u32 => Some('='),
        m if m == libc::S_IFIFO as u32 => Some('|'),
        m if m == libc::S_IFREG as u32 && entry.target_mode & 0o111 != 0 => Some('*'),
        _ => None,
    }
}

/// One printed name: the base name or, under `-f`, the whole path,
/// quoted by `-Q`, coloured, followed by ` -> target` for a symlink and
/// by the `-F` suffix for everything.
fn name_text(entry: &Entry, o: &Options, root: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let shown = if o.full_path && !root {
        entry.path.as_bytes()
    } else {
        entry.name.as_bytes()
    };
    push_name(
        &mut out,
        shown,
        o,
        o.palette.as_ref().and_then(|p| colour_of(entry, p)),
    );
    if let Some(target) = &entry.link {
        out.extend_from_slice(b" -> ");
        // The target is coloured as the thing it points at, and as an
        // ordinary file when it points at nothing.
        let colour = o.palette.as_ref().and_then(|p| {
            if entry.target_mode == 0 {
                p.get("mi").or_else(|| p.get("fi"))
            } else if entry.is_dir {
                p.get("di")
            } else {
                p.for_mode(entry.target_mode, target.as_bytes())
            }
        });
        push_name(&mut out, target.as_bytes(), o, colour);
    }
    if o.classify {
        // A link printed without its target, which is what a root
        // symlink is, gets ls -F's @ rather than the target's own mark.
        if entry.kind == Kind::Link && entry.link.is_none() {
            out.push(b'@');
        } else if let Some(suffix) = classify(entry) {
            out.push(suffix as u8);
        }
    }
    out
}

fn push_name(out: &mut Vec<u8>, raw: &[u8], o: &Options, colour: Option<&str>) {
    if let Some(colour) = colour {
        out.extend_from_slice(format!("\x1b[{colour}m").as_bytes());
    }
    if o.quote {
        out.push(b'"');
    }
    out.extend_from_slice(&escape(raw, o.utf8_names, o.quote));
    if o.quote {
        out.push(b'"');
    }
    if colour.is_some() {
        let reset = o.palette.as_ref().map_or("0", |p| p.reset.as_str());
        out.extend_from_slice(format!("\x1b[{reset}m").as_bytes());
    }
}

// ------------------------------------------------------------------ counting

#[derive(Default)]
struct Counts {
    directories: u64,
    files: u64,
}

// -------------------------------------------------------------- text output

fn write_entry(out: &mut dyn Write, entry: &Entry, o: &Options, root: bool) -> io::Result<()> {
    if o.wants_meta() {
        out.write_all(info_field(entry, o).as_bytes())?;
    }
    out.write_all(&name_text(entry, o, root))?;
    if let Kids::Note(note) = &entry.kids {
        write!(out, "  [{note}]")?;
    }
    out.write_all(b"\n")
}

fn write_children(
    out: &mut dyn Write,
    kids: &[Entry],
    prefix: &str,
    o: &Options,
    counts: &mut Counts,
) -> io::Result<()> {
    for (at, kid) in kids.iter().enumerate() {
        let last = at + 1 == kids.len();
        if kid.is_dir {
            counts.directories += 1;
        } else {
            counts.files += 1;
        }
        if !o.no_indent {
            out.write_all(prefix.as_bytes())?;
            out.write_all(
                if last {
                    o.charset.corner
                } else {
                    o.charset.tee
                }
                .as_bytes(),
            )?;
        }
        write_entry(out, kid, o, false)?;
        if let Kids::List(grandkids) = &kid.kids
            && !grandkids.is_empty()
        {
            let mut deeper = prefix.to_string();
            if !o.no_indent {
                deeper.push_str(if last {
                    o.charset.blank
                } else {
                    o.charset.vertical
                });
            }
            write_children(out, grandkids, &deeper, o, counts)?;
        }
    }
    Ok(())
}

/// A root and everything under it. The root itself counts as a directory
/// only when something was printed below it, which is why an empty
/// directory reports "0 directories, 0 files".
fn write_root(
    out: &mut dyn Write,
    root: &Entry,
    o: &Options,
    counts: &mut Counts,
) -> io::Result<()> {
    write_entry(out, root, o, true)?;
    match &root.kids {
        Kids::List(kids) if !kids.is_empty() => {
            counts.directories += 1;
            write_children(out, kids, "", o, counts)?;
        }
        // A root that is not there at all counts as nothing; one that
        // is there but cannot be opened counts as a file, the way tree
        // counts anything it could not descend into.
        Kids::Note(note) if note.starts_with("error") => counts.files += u64::from(!root.missing),
        Kids::Note(_) => counts.directories += 1,
        _ => {}
    }
    Ok(())
}

fn report_text(counts: &Counts, o: &Options) -> String {
    fn plural<'a>(n: u64, one: &'a str, many: &'a str) -> &'a str {
        if n == 1 { one } else { many }
    }
    if o.dir_only {
        format!(
            "\n{} director{}\n",
            counts.directories,
            plural(counts.directories, "y", "ies")
        )
    } else {
        format!(
            "\n{} director{}, {} file{}\n",
            counts.directories,
            plural(counts.directories, "y", "ies"),
            counts.files,
            plural(counts.files, "", "s")
        )
    }
}

// -------------------------------------------------------------- json output

fn json_string(out: &mut Vec<u8>, raw: &[u8]) {
    out.push(b'"');
    for byte in raw {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1f => {
                out.extend_from_slice(format!("\\u{byte:04x}").as_bytes());
            }
            _ => out.push(*byte),
        }
    }
    out.push(b'"');
}

fn json_entry(
    out: &mut dyn Write,
    entry: &Entry,
    indent: usize,
    o: &Options,
    counts: &mut Counts,
    root: bool,
) -> io::Result<()> {
    // -i takes the indentation out of the JSON as well, leaving one long
    // line, which is what upstream does with the two options together.
    let pad = if o.no_indent {
        String::new()
    } else {
        " ".repeat(indent)
    };
    let eol = if o.no_indent { "" } else { "\n" };
    let mut head = Vec::new();
    head.extend_from_slice(pad.as_bytes());
    head.extend_from_slice(format!("{{\"type\":\"{}\",\"name\":", entry.kind.json()).as_bytes());
    let name = if o.full_path && !root {
        entry.path.as_bytes()
    } else {
        entry.name.as_bytes()
    };
    json_string(&mut head, name);
    if let Some(target) = &entry.link {
        head.extend_from_slice(b",\"target\":");
        json_string(&mut head, target.as_bytes());
    }
    if o.protections {
        let mut prot = String::new();
        protections(&mut prot, entry.mode, entry.kind);
        head.extend_from_slice(
            format!(
                ",\"mode\":\"{:04o}\",\"prot\":\"{}\"",
                entry.mode & 0o7777,
                prot.trim_start()
            )
            .as_bytes(),
        );
    }
    if o.bytes || o.human || o.si {
        let mut size = String::new();
        psize(&mut size, entry.size, o.human, o.si);
        let size = size.trim();
        if o.human || o.si {
            head.extend_from_slice(format!(",\"size\":\"{size}\"").as_bytes());
        } else {
            head.extend_from_slice(format!(",\"size\":{size}").as_bytes());
        }
    }
    if o.date {
        head.extend_from_slice(
            format!(",\"time\":\"{}\"", date_text(shown_time(entry, o), o)).as_bytes(),
        );
    }
    out.write_all(&head)?;
    match &entry.kids {
        Kids::Note(note) => {
            // The closing of an error block is upstream's oddity: four
            // spaces whatever the depth, and a line break before them
            // only when the entry is a root.
            let break_before = if root { eol } else { "" };
            let indent = if o.no_indent { "" } else { "    " };
            write!(
                out,
                ",\"contents\":[{{\"error\": \"{note}\"}}{break_before}{indent}]}}"
            )?;
        }
        Kids::List(kids) if !kids.is_empty() => {
            write!(out, ",\"contents\":[{eol}")?;
            for (at, kid) in kids.iter().enumerate() {
                if kid.is_dir {
                    counts.directories += 1;
                } else {
                    counts.files += 1;
                }
                json_entry(out, kid, indent + 4, o, counts, false)?;
                if at + 1 < kids.len() {
                    out.write_all(b",")?;
                }
                out.write_all(eol.as_bytes())?;
            }
            write!(out, "{pad}]}}")?;
        }
        _ => out.write_all(b"}")?,
    }
    Ok(())
}

fn json_root(
    out: &mut dyn Write,
    root: &Entry,
    o: &Options,
    counts: &mut Counts,
) -> io::Result<()> {
    match &root.kids {
        Kids::List(kids) if !kids.is_empty() => counts.directories += 1,
        Kids::Note(note) if note.starts_with("error") => counts.files += u64::from(!root.missing),
        Kids::Note(_) => counts.directories += 1,
        _ => {}
    }
    json_entry(out, root, 4, o, counts, true)
}

// --------------------------------------------------------------------- main

/// Which of the option-driven exits the command line asked for first.
enum Immediate {
    Help,
    Version,
    Unsupported(&'static str),
}

/// Keep only the first of the options that exit before any listing, so
/// they act in the order the command line put them.
fn remember(what: Immediate, immediate: &mut Option<Immediate>) {
    if immediate.is_none() {
        *immediate = Some(what);
    }
}

fn use_instead(template: &str, operands: &[OsString]) -> String {
    let mut paths = String::new();
    for (at, operand) in operands.iter().enumerate() {
        if at > 0 {
            paths.push(' ');
        }
        paths.push_str(&operand.to_string_lossy());
    }
    if paths.is_empty() {
        paths.push('.');
    }
    template.replacen("{}", &paths, 1)
}

/// tree matches a long option by its whole name, where this crate's
/// parser also accepts an unambiguous prefix. Rejecting an inexact name
/// here keeps `tree --nore` the error it is upstream, and means the
/// parser's ambiguity diagnostic can never be reached.
fn reject_inexact_long_options(args: &[OsString]) {
    for arg in args {
        let text = arg.to_string_lossy();
        if text == "--" {
            return;
        }
        let Some(rest) = text.strip_prefix("--") else {
            continue;
        };
        let name = rest.split('=').next().unwrap_or(rest);
        let known = TREE_OPTS.iter().find(|opt| opt.long == name);
        // An `=` on an option that takes nothing is not a GNU-style
        // complaint upstream, it is the same invalid-argument error as
        // an option that does not exist.
        let allowed = known.is_some_and(|opt| opt.arg != Arg::None || name == rest);
        if !allowed {
            fail(&format!("Invalid argument `{text}'."), true);
        }
    }
}

/// tree's own wording for what the shared parser reports in GNU's.
fn translate(message: &str) -> ! {
    if let Some(rest) = message.strip_prefix("invalid option -- '") {
        let flag = rest.trim_end_matches('\'');
        fail(&format!("Invalid argument -`{flag}'."), true);
    }
    if let Some(rest) = message.strip_prefix("option requires an argument -- '") {
        let flag = rest.trim_end_matches('\'');
        fail(&format!("Missing argument to -{flag} option."), false);
    }
    if let Some(rest) = message.strip_prefix("option '--")
        && let Some(name) = rest.strip_suffix("' requires an argument")
    {
        if name == "charset" {
            let mut err = io::stderr().lock();
            let _ = writeln!(err, "tree: Missing argument to --charset");
            let _ = writeln!(err, "Valid charsets include:");
            for name in ["ANSI", "ASCII", "IBM437", "UTF-8"] {
                let _ = writeln!(err, "  {name}");
            }
            let _ = err.flush();
            process::exit(1);
        }
        fail(&format!("Missing argument to --{name}"), false);
    }
    fail(message, true)
}

/// tree reads a level with `strtoul`, so trailing rubbish is ignored and
/// anything that does not begin with a digit reads as zero, which is one
/// level too few and the error below.
fn parse_level(value: &OsStr) -> i64 {
    let text = value.to_string_lossy();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let level: i64 = digits.parse().unwrap_or(0);
    if level < 1 {
        fail("Invalid level, must be greater than 0.", false);
    }
    level
}

fn parse_sort(value: &OsStr) -> Sort {
    match value.to_string_lossy().as_ref() {
        "name" => Sort::Name,
        "version" => Sort::Version,
        "size" => Sort::Size,
        "mtime" => Sort::Mtime,
        "ctime" => Sort::Ctime,
        "none" => Sort::None,
        other => fail(
            &format!(
                "Sort type '{other}' not valid, should be one of: name,version,size,mtime,ctime,none"
            ),
            false,
        ),
    }
}

fn main() {
    // Without this a pipeline that stops reading would turn into a write
    // error instead of the silent death by signal the C original dies.
    // SAFETY: setting a disposition to SIG_DFL is always valid, and this
    // runs before any output, thread or handler exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    // strcoll orders the listing and nl_langinfo names the charset;
    // neither sees the environment until the locale is set from it.
    // SAFETY: setlocale takes a NUL-terminated string and this runs
    // before any thread exists.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    reject_inexact_long_options(&args);

    let mut o = Options::default();
    let mut operands: Vec<OsString> = Vec::new();
    let mut immediate: Option<Immediate> = None;
    let mut charset: Option<String> = None;
    let mut output: Option<OsString> = None;

    // parse_partial, not parse_or_exit: --help and --version act as they
    // are reached, and a refused option has to be reported before a
    // later parse error is. The operands are collected first either way,
    // since the "Use instead" line quotes them.
    let (items, error) = Parser::new("tree", TREE_OPTS, Mode::Permute).parse_partial(&args);
    for item in items {
        match item {
            Item::Operand(operand) => operands.push(operand),
            Item::Flag { long, value } => {
                let required = || value.clone().expect("option takes a required argument");
                match long {
                    "=all" => o.all = true,
                    "=dironly" => o.dir_only = true,
                    "=fullpath" => o.full_path = true,
                    "=noindent" => o.no_indent = true,
                    "=level" => o.level = Some(parse_level(&required())),
                    "=include" => o.include.push(required().as_bytes().to_vec()),
                    "=exclude" => o.exclude.push(required().as_bytes().to_vec()),
                    "=human" => o.human = true,
                    "=bytes" => o.bytes = true,
                    "=protections" => o.protections = true,
                    "=date" => o.date = true,
                    "=classify" => o.classify = true,
                    "=quote" => o.quote = true,
                    "=color" => o.force_color = true,
                    "=nocolor" => o.no_color = true,
                    "=outfile" => output = Some(required()),
                    "=json" => o.json = true,
                    "=mtimesort" => o.sort = Sort::Mtime,
                    "=ctimesort" => {
                        o.sort = Sort::Ctime;
                        o.ctime = true;
                    }
                    "=versionsort" => o.sort = Sort::Version,
                    "=unsorted" => o.unsorted = true,
                    "=reverse" => o.reverse = true,
                    "prune" => o.prune = true,
                    "matchdirs" => o.match_dirs = true,
                    "ignore-case" => o.ignore_case = true,
                    "noreport" => o.no_report = true,
                    "charset" => charset = Some(required().to_string_lossy().into_owned()),
                    "dirsfirst" => {
                        o.dirs_first = true;
                        o.files_first = false;
                    }
                    "filesfirst" => {
                        o.files_first = true;
                        o.dirs_first = false;
                    }
                    "sort" => {
                        o.sort = parse_sort(&required());
                        // none is -U by another spelling, and disables
                        // -r and the two grouping options with it.
                        o.unsorted = o.sort == Sort::None;
                    }
                    "si" => o.si = true,
                    "filelimit" => {
                        let text = required().to_string_lossy().into_owned();
                        let digits: String =
                            text.chars().take_while(char::is_ascii_digit).collect();
                        o.file_limit = digits.parse().unwrap_or(0);
                    }
                    "timefmt" => {
                        o.date = true;
                        o.timefmt = CString::new(required().as_bytes()).ok();
                    }
                    "help" => remember(Immediate::Help, &mut immediate),
                    "version" => remember(Immediate::Version, &mut immediate),
                    other => remember(Immediate::Unsupported(other), &mut immediate),
                }
            }
        }
    }

    match immediate {
        Some(Immediate::Help) => {
            print!("{USAGE}{HELP_BODY}");
            process::exit(0);
        }
        Some(Immediate::Version) => {
            print!("{}", agent_cli_tools::version_text("tree"));
            process::exit(0);
        }
        Some(Immediate::Unsupported(long)) => {
            let refused = DEFERRED
                .iter()
                .find(|d| d.long == long)
                .unwrap_or_else(|| unreachable!("unknown option name {long}"));
            let instead = refused
                .instead
                .map(|template| use_instead(template, &operands));
            options::unsupported("tree", refused.name, instead.as_deref());
        }
        None => {}
    }
    if let Some(message) = error {
        translate(&message);
    }

    o.charset = choose_charset(charset.as_deref());
    o.utf8_names = is_utf8_name(&locale_codeset());
    o.palette = choose_palette(o.force_color, o.no_color);
    if operands.is_empty() {
        operands.push(OsString::from("."));
    }

    let mut sink: Box<dyn Write> = match &output {
        Some(path) => match File::create(Path::new(path)) {
            Ok(file) => Box::new(BufWriter::new(file)),
            Err(_) => fail(
                &format!("invalid filename '{}'", path.to_string_lossy()),
                false,
            ),
        },
        None => Box::new(BufWriter::new(io::stdout())),
    };

    let mut failed = false;
    let mut counts = Counts::default();
    let mut roots: Vec<Entry> = Vec::new();
    for operand in &operands {
        // The root line is the operand exactly as it was typed, trailing
        // slash and all, except under -f, where it is also the prefix
        // every name below it is built from.
        let shown = if o.full_path {
            let bytes = operand.as_bytes();
            let trimmed = bytes.strip_suffix(b"/").filter(|rest| !rest.is_empty());
            OsString::from_vec(trimmed.unwrap_or(bytes).to_vec())
        } else {
            operand.clone()
        };
        let mut root = describe(OsStr::new(""), shown);
        root.path = root.name.clone();
        // A root that is a symlink is listed as the thing it points at:
        // tree prints the operand alone, with no arrow, and opens it.
        root.link = None;
        if root.missing {
            root.kids = Kids::Note("error opening dir".into());
            failed = true;
        } else if !root.is_dir {
            root.kids = Kids::Note("error opening dir".into());
        } else {
            // A root that cannot be opened is not the same failure as a
            // root that is not there: tree exits 0 for the first and 2
            // for the second, so the walker's flag is dropped here.
            let mut root_failed = false;
            walk(&mut root, 0, false, &o, &mut root_failed);
            match root.kids {
                // A root that could not be listed is not the same
                // failure as a root that is not there: tree exits 0 for
                // the first and 2 for the second, so the walker's flag
                // is dropped along with it.
                Kids::Note(_) => {}
                // Under --prune the walker leaves nothing behind for a
                // root it could not list, and upstream reports that as
                // the ordinary unopenable root.
                Kids::None => root.kids = Kids::Note("error opening dir".into()),
                Kids::List(_) => failed |= root_failed,
            }
        }
        roots.push(root);
    }

    let written = if o.json {
        write_json(&mut sink, &roots, &o, &mut counts)
    } else {
        write_plain(&mut sink, &roots, &o, &mut counts)
    };
    let _ = written;
    let _ = sink.flush();
    process::exit(i32::from(failed) * 2)
}

fn write_plain(
    out: &mut dyn Write,
    roots: &[Entry],
    o: &Options,
    counts: &mut Counts,
) -> io::Result<()> {
    for root in roots {
        write_root(out, root, o, counts)?;
    }
    if !o.no_report {
        out.write_all(report_text(counts, o).as_bytes())?;
    }
    Ok(())
}

fn write_json(
    out: &mut dyn Write,
    roots: &[Entry],
    o: &Options,
    counts: &mut Counts,
) -> io::Result<()> {
    let eol = if o.no_indent { "" } else { "\n" };
    let pad = if o.no_indent { "" } else { "    " };
    write!(out, "[{eol}")?;
    for (at, root) in roots.iter().enumerate() {
        json_root(out, root, o, counts)?;
        if at + 1 < roots.len() {
            write!(out, ",{eol}")?;
        }
    }
    out.write_all(eol.as_bytes())?;
    if !o.no_report {
        if o.dir_only {
            write!(
                out,
                ",{pad}{{\"type\":\"report\",\"directories\":{}}}",
                counts.directories
            )?;
        } else {
            write!(
                out,
                ",{pad}{{\"type\":\"report\",\"directories\":{},\"files\":{}}}",
                counts.directories, counts.files
            )?;
        }
    }
    writeln!(out, "{eol}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(bytes: u64, human: bool, si: bool) -> String {
        let mut out = String::new();
        psize(&mut out, bytes, human, si);
        out.replace_range(0..1, "[");
        out.push(']');
        out
    }

    #[test]
    fn human_sizes_keep_the_documented_overflow() {
        assert_eq!(size(0, true, false), "[   0]");
        assert_eq!(size(999, true, false), "[ 999]");
        assert_eq!(size(1023, true, false), "[1023]");
        assert_eq!(size(1024, true, false), "[1.0K]");
        assert_eq!(size(1100, true, false), "[1.1K]");
        assert_eq!(size(10187, true, false), "[9.9K]");
        assert_eq!(size(10188, true, false), "[ 10K]");
        assert_eq!(size(1048575, true, false), "[1024K]");
        assert_eq!(size(1048576, true, false), "[1.0M]");
        assert_eq!(size(1073741824, true, false), "[1.0G]");
    }

    #[test]
    fn si_sizes_divide_by_a_thousand() {
        assert_eq!(size(999, false, true), "[ 999]");
        assert_eq!(size(1023, false, true), "[1.0k]");
        assert_eq!(size(9947, false, true), "[9.9k]");
        assert_eq!(size(9948, false, true), "[ 10k]");
        assert_eq!(size(999999, false, true), "[1000k]");
        assert_eq!(size(1000000, false, true), "[1.0M]");
    }

    #[test]
    fn byte_sizes_are_eleven_columns_wide() {
        assert_eq!(size(0, false, false), "[          0]");
        assert_eq!(size(1234, false, false), "[       1234]");
    }

    #[test]
    fn protections_show_the_special_bits() {
        let mut out = String::new();
        protections(&mut out, 0o755, Kind::Directory);
        assert_eq!(out, " drwxr-xr-x");
        out.clear();
        protections(&mut out, 0o1777, Kind::Directory);
        assert_eq!(out, " drwxrwxrwt");
        out.clear();
        protections(&mut out, 0o4755, Kind::File);
        assert_eq!(out, " -rwsr-xr-x");
        out.clear();
        protections(&mut out, 0o2644, Kind::File);
        assert_eq!(out, " -rw-r-Sr--");
    }

    #[test]
    fn globs_follow_trees_rules() {
        let ok = |pattern: &str, text: &str| glob(pattern.as_bytes(), text.as_bytes(), false);
        assert!(ok("*.rs", "lib.rs"));
        assert!(!ok("*.rs", "src/lib.rs"));
        assert!(ok("**.rs", "src/lib.rs"));
        assert!(ok("?ib.rs", "lib.rs"));
        assert!(ok("[nd]ocs", "docs"));
        assert!(!ok("[^d]ocs", "docs"));
        assert!(ok("[a-z]ocs", "docs"));
        assert!(ok("a\\*b", "a*b"));
        assert!(!ok("a\\*b", "azzb"));
    }

    #[test]
    fn patterns_match_any_path_suffix_at_a_slash() {
        let patterns = vec![b"b/c".to_vec()];
        assert!(matches_any(&patterns, b"demo/a/b/c", true, false));
        assert!(!matches_any(&patterns, b"demo/a/xb/c", true, false));
        let base = vec![b"node_modules".to_vec()];
        assert!(matches_any(&base, b"./node_modules", true, false));
        let dirs_only = vec![b"src/".to_vec()];
        assert!(matches_any(&dirs_only, b"./src", true, false));
        assert!(!matches_any(&dirs_only, b"./src", false, false));
        let either = vec![b"lib.rs|guide.md".to_vec()];
        assert!(matches_any(&either, b"./docs/guide.md", false, false));
    }

    #[test]
    fn version_order_matches_strverscmp() {
        let mut names = ["a10", "a1", "a02", "a2b", "a2", "a-1"];
        names.sort_by(|a, b| version_cmp(a.as_bytes(), b.as_bytes()));
        assert_eq!(names, ["a-1", "a02", "a1", "a2", "a2b", "a10"]);
        let mut more = ["x10", "x9", "x 1"];
        more.sort_by(|a, b| version_cmp(a.as_bytes(), b.as_bytes()));
        assert_eq!(more, ["x 1", "x9", "x10"]);
    }

    #[test]
    fn json_strings_escape_what_json_must() {
        let mut out = Vec::new();
        json_string(&mut out, b"tab\tname");
        assert_eq!(out, b"\"tab\\tname\"");
        out.clear();
        json_string(&mut out, b"quote\"name");
        assert_eq!(out, b"\"quote\\\"name\"");
        out.clear();
        json_string(&mut out, b"ctrl\x01name");
        assert_eq!(out, b"\"ctrl\\u0001name\"");
        out.clear();
        json_string(&mut out, "\u{fc}mlaut".as_bytes());
        assert_eq!(out, "\"\u{fc}mlaut\"".as_bytes());
    }

    #[test]
    fn use_instead_substitutes_the_operands() {
        let one = vec![OsString::from("src")];
        assert_eq!(use_instead("du -sh {}", &one), "du -sh src");
        assert_eq!(use_instead("du -sh {}", &[]), "du -sh .");
        let two = vec![OsString::from("a"), OsString::from("b")];
        assert_eq!(use_instead("find {} -xdev", &two), "find a b -xdev");
    }
}
