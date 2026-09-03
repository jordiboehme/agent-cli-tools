```
              <! . . . . . . . . . . !>
              <! . . . . . . . . . . !>        NEXT
              <! . . . .[][][] . . . !>
              <! . . . . .[] . . . . !>        [][]
              <! . . . . . . . . . . !>          [][]
              <! . . . . . . . . . . !>
              <! . . . . . . . . . . !>        LINES
              <![][][] . . .[][][][] !>        00000
              <![][][][] .[][][][][] !>
              <![][][][][][][][][][] !>
              <!====================!>
                \/\/\/\/\/\/\/\/\/\/

 █████╗  ██████╗ ███████╗███╗   ██╗████████╗       ██████╗██╗     ██╗
██╔══██╗██╔════╝ ██╔════╝████╗  ██║╚══██╔══╝      ██╔════╝██║     ██║
███████║██║  ███╗█████╗  ██╔██╗ ██║   ██║   █████╗██║     ██║     ██║
██╔══██║██║   ██║██╔══╝  ██║╚██╗██║   ██║   ╚════╝██║     ██║     ██║
██║  ██║╚██████╔╝███████╗██║ ╚████║   ██║         ╚██████╗███████╗██║
╚═╝  ╚═╝ ╚═════╝ ╚══════╝╚═╝  ╚═══╝   ╚═╝          ╚═════╝╚══════╝╚═╝
████████╗ ██████╗  ██████╗ ██╗     ███████╗
╚══██╔══╝██╔═══██╗██╔═══██╗██║     ██╔════╝
   ██║   ██║   ██║██║   ██║██║     ███████╗
   ██║   ██║   ██║██║   ██║██║     ╚════██║
   ██║   ╚██████╔╝╚██████╔╝███████╗███████║
   ╚═╝    ╚═════╝  ╚═════╝ ╚══════╝╚══════╝
```

# agent-cli-tools

**The missing CLI commands for agent harnesses.**

[![CI](https://github.com/jordiboehme/agent-cli-tools/actions/workflows/ci.yml/badge.svg)](https://github.com/jordiboehme/agent-cli-tools/actions/workflows/ci.yml)
[![Release](https://github.com/jordiboehme/agent-cli-tools/actions/workflows/release.yml/badge.svg)](https://github.com/jordiboehme/agent-cli-tools/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/jordiboehme/agent-cli-tools)](https://github.com/jordiboehme/agent-cli-tools/releases/latest)
[![Homebrew](https://img.shields.io/badge/homebrew-jordiboehme%2Ftap-orange)](https://github.com/jordiboehme/homebrew-tap)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey)](#install)
[![License: MIT](https://img.shields.io/github/license/jordiboehme/agent-cli-tools)](LICENSE)

AI coding agents learned the shell on Linux. Put one in a macOS terminal and sooner or later it types `timeout 30 npm test`, gets `command not found`, and tries again next session. Claude Code does it even when a hard rule in CLAUDE.md says not to ([anthropics/claude-code#90291](https://github.com/anthropics/claude-code/issues/90291)). Hook scripts and loop runners written on Linux fail the same way, silently, with exit 127.

agent-cli-tools ships the few commands macOS never had, each a faithful re-implementation of the original, as small standalone Rust binaries. The agent's habits work. Nothing else on the system changes.

The banner is the whole idea: a piece drops into the one hole that has its shape, the line clears, and the rest of the wall stays where it was.

## Why not `brew install coreutils`?

It works, and it is a sledgehammer. It installs about a hundred commands, and once its `gnubin` directory is on the PATH the GNU versions of `sed`, `date`, `stat`, `ls`, and friends replace the macOS ones. Their flags differ, so scripts and tools that expect BSD behaviour start failing in new and interesting ways. A small tap that adds only what is missing keeps the userland yours, no assimilation required.

## Principles

- **Fill gaps, never shadow.** A command is in scope only if stock macOS ships nothing by that name. GNU-flavoured versions of `sed`, `stat`, or `date` will never be here, however often an agent wishes for `sed -i`.
- **Exact compatibility.** Options, quirks, messages, and exit codes match the reference implementation. A script written against the GNU man page behaves the same here.
- **macOS only.** Linux has these commands. Windows has other problems.
- **Nothing else.** No configuration, no daemon, no runtime dependencies. One static binary per command.

## Install

### Homebrew

```sh
brew install jordiboehme/tap/agent-cli-tools
```

Signed and notarized binaries for Apple silicon and Intel.

### Release archive

Download the archive for your architecture from the [latest release](https://github.com/jordiboehme/agent-cli-tools/releases/latest), verify it against `SHA256SUMS`, and copy the binaries somewhere on your PATH:

```sh
tar xzf agent-cli-tools-v*-macos-arm64.tar.gz
sudo cp agent-cli-tools-v*-macos-arm64/timeout /usr/local/bin/
```

### From source

```sh
cargo install --git https://github.com/jordiboehme/agent-cli-tools
```

Check that it took:

```sh
timeout --version
```

## Commands

### timeout

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

## Roadmap

Candidates, in rough order of how often agents reach for them. All are absent from stock macOS, so none would shadow an existing command.

| Command | Origin | Why agents type it |
|---|---|---|
| `nproc` | coreutils | `make -j$(nproc)` is in half the build scripts ever written. |
| `tac` | coreutils | "Last matching line" pipelines, `grep ... \| tac \| head -1`. |
| `shuf` | coreutils | Random samples and `shuf -n 1`. |
| `sponge` | moreutils | Edit a file in place from a pipeline, `jq ... f \| sponge f`. |
| `flock` | util-linux | Lock guards in cron, launchd, and loop-runner scripts. |
| `setsid` | util-linux | Detaching daemons the Linux way. |

Not planned: `realpath` and `readlink -f` (macOS has had them since Ventura and Monterey 12.3), `md5sum` and `sha256sum` (in `/sbin` on current macOS), `wget`, `tree`, and `watch` (agents recover with `curl`, `find`, and a loop), and anything that already exists on macOS in BSD form.

## Development

```sh
cargo build
cargo nextest run          # or: cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

Adding a command: create `src/bin/<name>.rs` and `tests/<name>.rs`, add a `[[bin]]` entry to `Cargo.toml`, add the binary to the `BINARIES` list in `.github/workflows/release.yml`, and document it above. Behaviour must match the reference implementation's man page and, where the man page is vague, its source.

## License

MIT. See [LICENSE](LICENSE).
