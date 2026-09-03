# timeout

Run a command with a time limit. A port of `timeout(1)` from GNU coreutils 9.11.

```
timeout [OPTION]... DURATION COMMAND [ARG]...
```

| Option | Effect |
|---|---|
| `-f`, `--foreground` | Do not create a new process group. The command may read the TTY and receive TTY signals; only the command itself is signalled, not its children. |
| `-k`, `--kill-after=DURATION` | Also send KILL if the command is still running this long after the first signal. |
| `-p`, `--preserve-status` | Exit with the command's status even when it timed out. |
| `-s`, `--signal=SIGNAL` | Signal to send on timeout, by name (`HUP`, `sigint`) or number. Default TERM. |
| `-v`, `--verbose` | Report each signal sent on stderr. |

`DURATION` is a floating point number with an optional suffix: `s` (default), `m`, `h`, or `d`. `0` disables the timeout, `inf` is accepted. Option parsing stops at `DURATION`, so everything after it belongs to the command, flags included.

| Exit status | Meaning |
|---|---|
| 124 | The command timed out and `--preserve-status` was not given. |
| 125 | `timeout` itself failed, for example on an invalid duration or signal. |
| 126 | The command was found but could not be run. |
| 127 | The command was not found. |
| 137 | The command, or `timeout` itself, was killed with KILL (128 + 9). |
| other | The command's own exit status. |

Without `--foreground`, `timeout` runs the command in its own process group and signals the whole group, so a stubborn build tool's grandchildren go down with it. Signals sent to `timeout` (a Ctrl-C, a `kill` from a supervisor) are forwarded to the command, and if the command dies from a signal `timeout` dies the same way, so the shell sees exactly what happened.

```sh
timeout 30 npm test                                  # TERM after 30 s, exit 124
timeout -k 5 1m ./integration.sh                     # TERM after a minute, KILL 5 s later
timeout --preserve-status 10 ./flaky-server          # exit with the server's own status
timeout -v -s INT 2.5 python3 -c 'import time; time.sleep(9)'
timeout 0 make                                       # no limit, plain pass-through
```

Differences from GNU: `--version` names this project instead of coreutils, the `--help` footer points here, and diagnostics always use ASCII quotes rather than switching to curly quotes in UTF-8 locales. Everything else, down to the masking rule that turns `-s 143` into TERM, is the same.

See the [GNU coreutils manual](https://www.gnu.org/software/coreutils/manual/html_node/timeout-invocation.html) for the reference behaviour this port follows.
