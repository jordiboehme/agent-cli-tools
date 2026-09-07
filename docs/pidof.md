# pidof

Find the process ids of a running program. A port of `pidof(1)` from procps-ng 4.0.6.

```
pidof [options] [program [...]]
```

Each PROGRAM is a command name, a file name, or a path. Every matching process id is printed on one line, and the exit status says whether anything matched at all, which is what makes `pidof -q nginx && echo running` work.

| Option | Effect |
|---|---|
| `-s`, `--single-shot` | Return one pid only: the highest one, per program argument. |
| `-c`, `--check-root` | Accepted and ignored. See macOS notes. |
| `-q` | Quiet mode: print nothing at all, only set the exit status. Implies `-s`. |
| `-w`, `--with-workers` | Also consider processes whose arguments could not be read. See macOS notes. |
| `-x` | Also find shells running the named script. |
| `-o`, `--omit-pid PID,...` | Omit these processes. Repeatable. |
| `-t`, `--lightweight` | Accepted and ignored. See macOS notes. |
| `-S`, `--separator SEP` | Put SEP between the pids instead of a single space. |
| `-d SEP` | The sysvinit spelling of `-S`. |
| `-n`, `-m` | Accepted and ignored, as they are upstream. |
| `-h`, `--help` | Display the help text and exit. |
| `-V`, `--version` | Output version information and exit. |
| `-?` | Print the help text on standard error and exit 1. |

Options may appear anywhere on the command line, before or after a program name, and `--` ends option scanning, which is how a program whose name starts with a dash is looked up. Short options cluster (`-sx`) and `-o`, `-S` and `-d` take their argument attached (`-S,`) or as the next word. `--quiet` is accepted as a long form of `-q`, the way the C source has it, even though its own help text lists none.

## Matching

For every process, `pidof` derives four things: the kernel's short name for it, its argument vector, the base name and the whole of the resolved path to its executable. A process whose argument vector cannot be read is skipped entirely unless `-w` is given.

`argv[0]` with a single leading `-` removed (a login shell writes one there) is the process's own name. The program argument matches when any of these is true, tested in this order:

1. The argument equals the base name of `argv[0]`.
2. The base name of the argument equals `argv[0]`, which is what makes `pidof ./sleep` and `pidof /bin/sleep` find a process started as plain `sleep`.
3. The argument equals `argv[0]`.
4. `-w` was given and the argument equals the kernel's short name.
5. The argument equals the base name of the executable path.
6. The argument equals the executable path.

There is no rule that a program argument containing a slash means "this path and nothing else": rule 2 makes a path match by its base name as well. A base name here is whatever follows the last `/`, which is not what `basename(3)` returns: a trailing slash leaves an empty base name, so `pidof sleep/` matches nothing at all.

Rules 5 and 6 use the path the kernel reports, which has every symbolic link resolved. A program started through a link is therefore found under the link's name, from `argv[0]`, and under the name of the file it points at, from the executable path.

If none of the six matched and `-x` was given and the process has an `argv[1]`, the same first three tests are applied to `argv[1]`, guarded by a check that the process's short name is a prefix of `argv[1]`'s base name. That guard is what upstream uses to tell a script run directly from an interpreter that was invoked by hand; on macOS it behaves differently, see the macOS notes.

Finally, if nothing matched and `argv[0]` contains a space, the process has most likely rewritten its own command line, and the argument is compared against the kernel's short name instead.

## Output

One line, the pids separated by the separator, with a single trailing newline, and only when there was at least one match. Nothing at all, not even the newline, when nothing matched or under `-q`.

The pids of each program argument are grouped together, groups following the order of the arguments, and within a group the pids come out highest first. A program named twice has its pids printed twice, an empty argument is skipped, and one that matches nothing contributes no group:

```sh
pidof sleep bash          # 4310 4288 4102 981
pidof -S , sleep          # 4310,4288,4102
pidof -s sleep            # 4310
```

`-o` takes a list split on `,`, `;` and `:`, and may be repeated. `%PPID` in it means the parent of the `pidof` process itself. Each other token is read as a decimal number that must fill the whole token; a token that does not is reported and skipped, leaving the other tokens and the exit status alone:

```
pidof: illegal omit pid value (abc)!
```

A pid that is negative or out of range is accepted in silence and simply never matches anything.

## Exit status

| Status | Meaning |
|---|---|
| 0 | At least one process matched. |
| 1 | Nothing matched, or a usage error. |

There is no other status: an invalid option exits 1 as well, after printing the diagnostic and the whole help text on standard error rather than a "Try --help" line.

## macOS notes

macOS has no `/proc`, so the process table comes from `sysctl(KERN_PROC_ALL)`, the executable path from `proc_pidpath` and the argument vector from `sysctl(KERN_PROCARGS2)`. All three work for an ordinary user, with one limit: the kernel hands out the argument vector of a process only to its owner. A process belonging to another user is therefore skipped unless `-w` is given, and with `-w` it can still be found by its short name or its executable path, just not by its arguments.

That is what `-w` means here. macOS has no kernel worker threads for it to reveal, so it reduces to "also consider the processes whose arguments could not be read", which covers other users' processes and `kernel_task`.

`-c` is accepted and ignored. Upstream compares `/proc/PID/root` against its own and only does so when running as root, which is why it is silently ignored for everybody else there too; macOS has no per-process root to compare.

`-t` is accepted and ignored. macOS threads are not addressable as process ids, so there are no thread ids to add to the output and `-t` prints the same process ids as a run without it.

The kernel's short name for a process is the base name of the file it executed, kept to 16 characters. Two consequences: a program whose file name is longer is known to `-w` only by that cut-down name, and a script run directly is named after its interpreter, where Linux would name it after the script. The `-x` guard, which requires that short name to be a prefix of `argv[1]`'s base name, therefore behaves differently here. It admits a script only when the script's file name starts with the interpreter's name, and it cannot tell `./deploy.sh` from `sh deploy.sh`, because both leave `sh` running with the same arguments. In practice `-x` finds fewer scripts on macOS than on Linux, and the ones it finds it finds in both forms.

## Differences from procps-ng

- **`-x` matches on the interpreter's name.** Described just above: it is the same comparison as upstream, applied to the short name the macOS kernel supplies, which names the interpreter rather than the script.
- **`-t` and `-c` do nothing**, for the reasons above. Upstream's `-t` adds thread ids and its `-c` filters by root directory when run as root.
- **A process whose name is not valid UTF-8 cannot be matched.** Names and arguments are converted from the kernel's bytes with invalid sequences replaced, so a program whose name contains them is listed under the replacement character and no argument can equal it. Everything that is valid UTF-8, which is every ordinary program name, compares byte for byte.
- **The process table is read once**, at the start of the run, where upstream rescans it for every program argument. Every program argument on one command line therefore sees the same table. The table is not a single instant of the machine's state either way: the list of processes comes from one call, but each process's arguments and executable path are looked up one by one after it.

Smaller ones: `--version` names this project instead of procps-ng, the help footer points here, diagnostics always use ASCII quotes rather than switching to curly quotes in UTF-8 locales, and an empty option name before `=` (`pidof --=1`) is reported as `option '--' is ambiguous`, quoting the empty prefix rather than the whole token, and lists the internal names of the options that have no long form among its possibilities.

See the [pidof(1) manual page](https://man7.org/linux/man-pages/man1/pidof.1.html) for the reference behaviour this port follows.
