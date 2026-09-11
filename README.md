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
              <![][][] .[][][][][][] !>
              <!====================!>
                \/\/\/\/\/\/\/\/\/\/

 █████╗  ██████╗ ███████╗███╗   ██╗████████╗
██╔══██╗██╔════╝ ██╔════╝████╗  ██║╚══██╔══╝
███████║██║  ███╗█████╗  ██╔██╗ ██║   ██║
██╔══██║██║   ██║██╔══╝  ██║╚██╗██║   ██║
██║  ██║╚██████╔╝███████╗██║ ╚████║   ██║
╚═╝  ╚═╝ ╚═════╝ ╚══════╝╚═╝  ╚═══╝   ╚═╝
 ██████╗██╗     ██╗      ████████╗ ██████╗  ██████╗ ██╗     ███████╗
██╔════╝██║     ██║      ╚══██╔══╝██╔═══██╗██╔═══██╗██║     ██╔════╝
██║     ██║     ██║█████╗   ██║   ██║   ██║██║   ██║██║     ███████╗
██║     ██║     ██║╚════╝   ██║   ██║   ██║██║   ██║██║     ╚════██║
╚██████╗███████╗██║         ██║   ╚██████╔╝╚██████╔╝███████╗███████║
 ╚═════╝╚══════╝╚═╝         ╚═╝    ╚═════╝  ╚═════╝ ╚══════╝╚══════╝
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

- **Fill gaps, never shadow.** A command is in scope only if stock macOS ships nothing by that name and homebrew-core has no formula by that name. GNU-flavoured versions of `sed`, `stat`, or `date` will never be here, however often an agent wishes for `sed -i`, and where Homebrew already packages the real thing this tap installs that instead of a second copy.
- **Exact compatibility.** Options, quirks, messages, and exit codes match the reference implementation. A script written against the GNU man page behaves the same here.
- **macOS only.** Linux has these commands. Windows has other problems.
- **Nothing else.** No configuration, no daemon, no runtime dependencies. One static binary per command.

## Install

### Homebrew

```sh
brew install jordiboehme/tap/agent-cli-tools
```

Signed and notarized binaries for Apple silicon and Intel. Homebrew installs `tree` and `watch` alongside them, from homebrew-core.

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

| Command | Origin | What it does |
|---|---|---|
| [`nproc`](docs/nproc.md) | GNU coreutils 9.11 | Print the number of processing units available. |
| [`tac`](docs/tac.md) | GNU coreutils 9.11 | Print files with the lines reversed, last line first. |
| [`timeout`](docs/timeout.md) | GNU coreutils 9.11 | Run a command with a time limit. |

Each page documents the options, exit codes, and the few places where output differs from the original.

`tree` and `watch` are missing from macOS too, but Homebrew already carries the genuine upstreams, so the formula depends on them rather than shipping a second copy of each: `brew install` brings them along and the commands work. `pidof` is gone as of v0.3.0. Homebrew's `pidof` formula is a different program, not the procps-ng one, so there was no honest way to keep the name.

## Worth installing alongside

The tap fills gaps. It does nothing about what macOS ships old or what homebrew-core already owns, and agents trip over both just as hard. Three formulae cover most of it:

```sh
brew install bash gh ripgrep
```

- **`bash`**: macOS ships bash 3.2, frozen in carbonite since 2007 (the last GPLv2 release, and Apple stopped there). Agents write for bash 5: `declare -A`, `mapfile`, `${var,,}`. On 3.2 each of those fails, with a different error every time. Homebrew's bash 5 takes over for `bash script.sh` and `#!/usr/bin/env bash`. A `#!/bin/bash` shebang still gets 3.2, so ask your agent for `env`.
- **`gh`**: agents open pull requests and read issues with it, and `git` alone can't. Stock macOS doesn't have it.
- **`ripgrep`**: `rg` is the grep agents grew up with. Claude Code bundles one for its own search tool and still types `rg` in the shell, where it isn't.

Agent typing `flock` or `sponge`? Those are `brew install flock moreutils`.

## Roadmap

Candidates, in rough order of how often agents reach for them. All are absent from stock macOS and unpackaged by homebrew-core, so none would shadow an existing command.

| Command | Origin | Why agents type it |
|---|---|---|
| `shuf` | coreutils | Random samples and `shuf -n 1`. |
| `setsid` | util-linux | Detaching daemons the Linux way. |

Not planned: `realpath` and `readlink -f` (macOS has had them since Ventura and Monterey 12.3), `md5sum` and `sha256sum` (in `/sbin` on current macOS), `wget` (agents recover with `curl`), anything that already exists on macOS in BSD form, and anything homebrew-core already packages under its own name, `sponge` and `flock` among them.

## Development

```sh
cargo build
cargo nextest run          # or: cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

Adding a command: check first that stock macOS ships nothing by that name and that `brew info <name>` finds no homebrew-core formula, then create `src/bin/<name>.rs` and `tests/<name>.rs`, add a `[[bin]]` entry to `Cargo.toml`, add the binary to the `BINARIES` list in `.github/workflows/release.yml`, and document it above. Behaviour must match the reference implementation's man page and, where the man page is vague, its source.

## License

MIT. See [LICENSE](LICENSE).
