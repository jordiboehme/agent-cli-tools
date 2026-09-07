# watch

Run a command repeatedly and show its output on a full screen. A port of `watch(1)` from procps-ng 4.0.7.

```
watch [options] command
```

The command is run every two seconds by default, and its output redrawn in place under a two-line header. Option scanning stops at the first operand, so everything from the command on belongs to the command, dashes included: `watch -n 1 ls -l` runs `ls -l`.

| Option | Effect |
|---|---|
| `-b`, `--beep` | Ring the terminal bell after any run that exits non-zero. |
| `-c`, `--color` | Interpret the ANSI colour and style sequences in the output. |
| `-C`, `--no-color` | Do not interpret them. Last one on the command line wins. |
| `-d`, `--differences[=<permanent>]` | Show the cells that changed since the last run in reverse video. Any attached argument at all makes the highlighting cumulative. |
| `-e`, `--errexit` | On a non-zero exit, leave a message on the last row, wait for a key and exit with the command's status. |
| `-g`, `--chgexit` | Exit 0 as soon as the visible output differs from the run before it. |
| `-q`, `--equexit <cycles>` | Exit 0 once the output has been the same for that many runs. |
| `-n`, `--interval <secs>` | Seconds between runs. Clamped in silence to 0.1 and 2678400. |
| `-p`, `--precise` | Measure the interval from the start of the previous run rather than from its end. |
| `-r`, `--no-rerun` | A window resize redraws but does not run the command again. |
| `-t`, `--no-title` | Turn off the header, giving the output the whole screen. |
| `-w`, `--no-wrap` | Truncate long lines instead of wrapping them. |
| `-x`, `--exec` | Run the operands directly through `execvp` instead of handing the whole line to `sh -c`. |
| `-h`, `--help` | Display the help text and exit. |
| `-v`, `--version` | Output version information and exit. |

`-d`'s argument has to be attached, `-dpermanent` or `--differences=permanent`; `watch -d 1 date` makes `1` part of the command, as it does upstream.

`WATCH_INTERVAL` in the environment sets the interval, and `-n` on the command line overrides it. It goes through the same parser, so it fails the same way.

## Unsupported options

Both are recognized, so they parse exactly as they do upstream, and both are then refused by name with exit 1.

| Option | Why it is absent |
|---|---|
| `-f`, `--follow` | Scrolls the output like `tail -f` instead of redrawing it. It is a second rendering model on top of the one this port implements, and it conflicts with `-d`, `-e` and `-q` upstream. There is no macOS command that does it instead. |
| `-s`, `--shotsdir DIR` | Writes a screenshot file when the `s` key is pressed. It needs the screenshot file format, its naming and its collision handling to be worth having at all. There is no macOS command that does it instead. |

```
watch: --follow is not implemented in this build
See https://github.com/jordiboehme/agent-cli-tools/issues to request it
```

Neither has a macOS equivalent to point at, which is why the message names no command to use instead. Both are still listed by `--help`, which is the upstream help text unchanged.

## The interval

`-n` has a parser of its own, which is not the one `timeout` and `sleep` use: optional spaces, an optional sign, digits, a `.` or a `,` for the radix, digits. No exponent, no unit suffix, nothing trailing. `1,5` and `1.5` are the same second and a half; `1e3` and `30s` are errors:

```
watch: failed to parse argument: '1e3': Invalid argument
```

Anything that parses is then clamped to between 0.1 and 2678400 seconds without a word, so `-n 0` runs ten times a second and `-n 99999999` runs once a month.

## The header

Two rows, unless `-t`. The first carries `Every 2.0s: ` and the command on the left, and the host name, a colon and the locale's own date and time (`strftime("%c")`) on the right. What is left over between them is what the command has to fit in:

- when the terminal is narrower than the right half, the row stays empty;
- the right half is placed against the right margin;
- the command is shown whole if it fits, and otherwise cut short and marked with `...`;
- when even that does not fit, only `Every 2.0s: ` is shown, or nothing at all.

The second row carries the last run's duration and exit status against the right margin, `in 0.012s (0)`, with `in <0.001s (0)` under a millisecond and `in >1 day (0)` past a day. It is cleared before each run and written after it, so it always describes the output below it.

## The output

The command's standard output and standard error share one pipe, so they interleave the way they would on a terminal, and the whole of it is read even when the screen ran out of rows. Output starts on row 2, or on row 0 with `-t`.

- A tab advances to the next multiple of eight columns, always by at least one space.
- A bell is passed through to the terminal and occupies no cell.
- Every other non-printable character is dropped.
- A line longer than the terminal is wrapped onto the next row, or truncated with `-w`.
- Rows the output did not reach are blanked, so a command whose output got shorter does not leave the last run's tail on the screen.

Without `-c` the escape character is dropped and what followed it is shown as the text it is, so `\033[31mred` reads as `[31mred`. With `-c`, a sequence of the form `ESC [ ... m` is read as an SGR sequence: reset, bold, dim, italic, underline, blink and reverse, their unsetting counterparts, the eight foreground and background colours, their bright forms and the 256-colour `38;5;n` and `48;5;n` spellings. Reading stops at the first parameter that is not one of those, which is what leaves a 24-bit `38;2;r;g;b` sequence unapplied. Colours carry across lines and are reset at the start of each run.

`-d` compares each cell against what the previous run left there and shows the ones that differ in reverse video, on top of whatever colours are in force. Nothing is highlighted on the first screen, or on the first screen after a resize. With an argument attached, a cell that has been highlighted stays highlighted.

## Keys and signals

The terminal is put into the mode curses calls cbreak, so keys arrive unechoed and one at a time. They are read between runs:

| Key | Effect |
|---|---|
| `q` | Quit, exit 0. |
| space | Run the command now, without waiting out the rest of the interval. |

`Ctrl-C` and the other terminating signals put the terminal back and exit 0. A window resize redraws at the new size and runs the command again, unless `-r` was given; either way the next screen is treated as a first screen, so `-d` highlights nothing on it and `-g` and `-q` start over.

## Exit status

| Status | Meaning |
|---|---|
| 0 | `q`, a terminating signal, a change seen by `-g`, or output that settled under `-q`. |
| 1 | A usage error, an unknown option, an unimplemented one, or an interval that would not parse. |
| 2 | A pipe or a process could not be created. |
| _n_ | With `-e`, the command's own status: what it exited with, 128 plus the signal that killed it, or 127 when it could not be run at all. |

## macOS notes

There is no ncurses here. The screen is a grid of cells this port keeps itself, and a redraw writes only the cells that changed, each run of them preceded by an absolute cursor move. That needs a terminal that understands the alternate screen buffer, absolute cursor addressing and SGR, which every terminal on macOS does; unlike curses, it does not consult `TERM` or terminfo to find out.

The terminal's size comes from `TIOCGWINSZ` on standard input, with `LINES` and `COLUMNS` overriding it, and 24 by 80 when there is no terminal at all. Both are exported to the command, so a program that lays its output out itself sees the same size.

## Differences from procps-ng

- **`-f` and `-s` are refused**, for the reasons in the table above.
- **A double-width character takes one cell, not two.** Upstream asks `wcwidth` how wide each character is and gives a CJK ideograph or an emoji two cells, and attaches a combining mark to the cell before it. Here every printable character occupies exactly one, so a line containing them is drawn a little narrower than the terminal would draw it and can sit out of alignment with the lines around it.
- **The interval is always written with a `.`**, where upstream writes the locale's decimal separator, so a German locale shows `Every 0,5s:` there and `Every 0.5s:` here. The timestamp beside it still follows the locale, since it comes from `strftime("%c")`. The rounding to one decimal place is this platform's own: macOS's `printf` and the formatting this port uses agree on every value tried, `-n 1.25` reading as `1.2s` in both, so what is shown here is what a C program on the same machine would show.
- **The `s` key does nothing**, since screenshots come with `-s`, which is not implemented.
- **An exec failure is reported by the parent.** Upstream's child writes `command: message` into the pipe and exits 127; this port never gets as far as a child, and writes the same line into the same place itself. The status is 127 either way, and it only reaches the exit status through `-e`.
- **The shell is macOS's `/bin/sh`**, so a command that fails to parse or is not found is reported in its words rather than dash's.

Smaller ones: `--version` names this project instead of procps-ng, the help footer points here, and diagnostics always use ASCII quotes.

See the [watch(1) manual page](https://man7.org/linux/man-pages/man1/watch.1.html) for the reference behaviour this port follows.
