# tac

Write each file to standard output, last line first. A port of `tac(1)` from GNU coreutils 9.11.

```
tac [OPTION]... [FILE]...
```

With no FILE, or when FILE is `-`, read standard input. Each file is reversed on its own and the results are concatenated in operand order, so `tac a b` prints all of `a` reversed, then all of `b` reversed. `-` may appear more than once; the second one simply finds standard input already at end of file.

| Option | Effect |
|---|---|
| `-b`, `--before` | Attach the separator to the start of the record it precedes instead of the end of the record it follows. |
| `-r`, `--regex` | Interpret the separator as a regular expression. |
| `-s`, `--separator=STRING` | Use STRING as the separator instead of a newline. The last `-s` on the command line wins. |
| `--help` | Display the help text and exit. |
| `--version` | Output version information and exit. |

Options may appear anywhere on the command line, before or after a file name, and `--` ends option scanning. Short options cluster (`-br`) and `-s` takes its argument attached (`-s:`) or as the next word.

## The record model

By default the input is a sequence of records, each ending with the separator. `tac` prints them in reverse order and every record keeps the separator that terminates it. If the input does not end with a separator, the final piece is a record without one, so the missing separator stays missing and that piece ends up glued in front of what used to be the record before it:

```sh
printf 'a\nb\nc\n' | tac    # c\nb\na\n
printf 'a\nb\nc'   | tac    # cb\na\n
```

Matches are found by scanning backward and never overlap, which is what makes a multi-character separator behave the way it does: with `-s xx`, `axxx` comes back unchanged (the only match is the last `xx`, and it already ends the input), while `axxxx` becomes `xxaxx`.

`-b` moves the boundary to the other side of the separator: records are separator-then-content, and the leading piece before the first separator, which may be empty, is printed last.

```sh
printf 'a\nb\n' | tac -b     # \n\nba
printf '100:200:300' | tac -b -s :   # :300:200100
```

The separator is a sequence of bytes, never characters. A multi-byte character works because its bytes are matched as a unit, and no locale setting changes that.

An empty separator is not an error without `-r`: like GNU, it means the NUL byte, which is what makes `tac -s ''` the counterpart of `find -print0`. With `-r` an empty separator is rejected.

## Regular expression separators

`-r` compiles the separator as a POSIX extended regular expression with `REG_NEWLINE`, so `.` does not match a newline, `^` matches at the start of the data and after every newline, and `$` matches at the end and before every newline.

The search runs backward: at each step `tac` takes the match with the greatest start position, and the next search may not start at or after that position. Matches therefore never repeat, an empty match is never reused, and a greedy separator cannot reach backward across the record before it.

```sh
printf 'a1b2c3' | tac -r -s '[0-9]'     # c3b2a1
printf 'a:b::c:::d::::' | tac -r -s ':+'   # :::d:::c::b:a:
```

GNU compiles the separator as an Emacs regular expression, where grouping and alternation are spelled `\(`, `\)` and `\|` and the bare characters are literals. This port keeps that spelling by translating the pattern before compiling it: `\(`, `\)` and `\|` become `(`, `)` and `|`, and a bare `(`, `)`, `{`, `}` or `|` becomes an escaped literal. Everything else passes through unchanged.

A pattern that fails to compile is reported with the message the C library gives and exits 1. The wording comes from the platform, so it is macOS phrasing rather than glibc's: `tac -r -s '\'` says `tac: trailing backslash (\)` where GNU says `tac: Trailing backslash`.

## Exit status

| Status | Meaning |
|---|---|
| 0 | Success. |
| 1 | A file could not be opened or read, a usage error, an invalid or empty `-r` separator, or a write error. |

A file that cannot be opened or read does not stop the operands after it. The diagnostic is printed, the remaining files are still reversed, and the final status is 1:

```
tac: failed to open 'nosuch' for reading: No such file or directory
tac: somedir: read error: Is a directory
```

Like every other filter, `tac` dies from `SIGPIPE` when its reader goes away, so `tac big.log | head -1` is silent and exits 141 rather than reporting a broken pipe.

## Differences from GNU

- **Whole-file reads instead of backward streaming.** GNU reads a seekable file backward in 8 KiB chunks so its memory stays bounded by the longest record, and copies a non-seekable input such as a pipe into a temporary file under `$TMPDIR` first. This port reads each operand fully into memory instead, so there is no temporary file, no `$TMPDIR` dependency, and nothing to clean up, at the cost of holding the whole input at once. The records produced are identical; only the mechanism differs. The `--help` text therefore omits GNU's line about buffering non-seekable input to `$TMPDIR`.
- **Regular expressions are translated to POSIX extended syntax.** The translation described above makes ordinary patterns behave identically, and it leaves two residual differences. `\{m,n\}` is a literal brace sequence here, which is also how GNU treats it, but by a different route: GNU compiles without interval support at all, while this port escapes the braces into literals. And POSIX character classes such as `[[:digit:]]` work here, where GNU's Emacs syntax rejects them. Backreferences and the Emacs word operators `\w`, `\W`, `\b`, `\<` and `\>` are not part of POSIX extended syntax and are not available.
- **NUL bytes are invisible to regex mode.** `regexec` works on NUL-terminated C strings, so in `-r` mode both the separator and the data end at their first NUL byte. Binary input containing NUL bytes is only reversed up to the first one. Fixed-string mode has no such limit and handles NUL bytes throughout, including as the separator itself.

Smaller ones: `--version` names this project instead of coreutils, the `--help` footer points here, and diagnostics always use ASCII quotes rather than switching to curly quotes in UTF-8 locales.

See the [GNU coreutils manual](https://www.gnu.org/software/coreutils/manual/html_node/tac-invocation.html) for the reference behaviour this port follows.
