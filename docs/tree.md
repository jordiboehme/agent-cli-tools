# tree

List the contents of directories as a tree. A port of a documented subset of `tree(1)` 2.3.2 by Steve Baker.

```
tree [options] [directory ...]
```

With no arguments it lists the current directory, drawing the branches with the box characters when the locale is a UTF-8 one and with `|`, `` ` `` and `-` when it is not, and ends with a count of what it found:

```
.
├── Cargo.toml
├── docs
│   └── guide.md
├── README.md
└── src
    ├── lib.rs
    └── main.rs

3 directories, 5 files
```

Hidden files are left out unless `-a` asks for them, and a symbolic link to a directory is listed and counted as a directory but never followed, since `-l` is not implemented here.

## Options

| Option | Effect |
|---|---|
| `-a` | List every file, including the ones whose name starts with a dot. |
| `-d` | List directories only. Nested directories are still counted. |
| `-f` | Print each name with the path it was reached by. |
| `-i` | Leave the indentation lines out, giving a flat list. In `-J` it also takes the JSON's indentation and line breaks out. |
| `-L level` | Descend at most that many levels. Read the way `strtoul` reads it, so `2x` is 2 and anything that is not a number at all is 0, which is an error. |
| `-P pattern` | List only the files matching the pattern. Directories are not filtered by it unless `--matchdirs` is given. |
| `-I pattern` | Do not list what matches the pattern, directories included. |
| `--ignore-case` | Fold case in `-P` and `-I`. |
| `--matchdirs` | Let a directory name match `-P` too; everything under a directory that matches is then listed. |
| `--prune` | Drop every directory with nothing listed under it, the ones the depth limit stopped at and the symlinks to directories included. Ignored under `-d`. |
| `--filelimit #` | Do not open a directory holding more than that many entries. `0` means no limit. |
| `--noreport` | Leave off the count at the end, and the blank line before it. |
| `--charset X` | Draw the branches with that charset. `UTF-8` picks the box characters; every other name, `ascii` included, picks the plain ones. |
| `-o filename` | Write the listing to that file instead of standard output. |
| `-p` | Show the permissions, `[drwxr-xr-x]`, with `s`, `S`, `t` and `T` for the special bits. |
| `-s` | Show the size in bytes, right-aligned in an eleven-column field. |
| `-h` | Show the size in a four-column human-readable field, powers of 1024. |
| `--si` | Like `-h`, in powers of 1000. |
| `-D` | Show the time of last modification, or of last status change with `-c`. |
| `--timefmt fmt` | Format that time with `strftime`. Implies `-D`. |
| `-F` | Append `/`, `*`, `=`, `\|` or `@` the way `ls -F` does. |
| `-Q` | Put the names in double quotes. |
| `-C` | Colour the output even when it is not going to a terminal. |
| `-n` | Never colour the output. `-C` overrides it. |
| `-t` | Sort by time of last modification, oldest first. |
| `-c` | Sort by time of last status change, and show that time under `-D`. |
| `-v` | Sort by version, so `a2` comes before `a10`. |
| `-U` | Do not sort at all, and turn off `-r`, `--dirsfirst` and `--filesfirst`. |
| `-r` | Reverse the sort. |
| `--dirsfirst` | Put directories before files. |
| `--filesfirst` | Put files before directories. |
| `--sort X` | One of `name`, `version`, `size`, `mtime`, `ctime`, `none`. `none` is `-U` under another name, and turns off `-r`, `--dirsfirst` and `--filesfirst` with it. |
| `-J` | Print the tree as JSON, with a trailing report object. |
| `--help` | Print the usage and the help text and exit. |
| `--version` | Print version information and exit. |

Everything after `--` is a directory operand, dashes and all.

## Unsupported options

Roughly half of tree's surface is HTML, XML, two input-file formats and a set of Linux-only file facts. All of it is still recognized, so a command line parses exactly as it does upstream, and is then refused by name with exit 1. Where a stock macOS command does the same job, the message names it with the operands of the invocation already substituted, so the line can be run as it stands:

```
$ tree --du src
tree: --du is not implemented in this build
Use instead: du -sh src
See https://github.com/jordiboehme/agent-cli-tools/issues to request it
```

| Option | Use instead |
|---|---|
| `--du` | `du -sh <paths>` |
| `-X` | `tree -J <paths>` |
| `--gitignore` | `tree -I 'node_modules\|target\|.git' <paths>` |
| `--inodes` | `ls -i <paths>` |
| `-u`, `-g` | `ls -l <paths>` |
| `--device`, `-x` | `find <paths> -xdev` |
| `-H`, `-T`, `-R`, `--nolinks`, `--hintro`, `--houtro` | `tree -J <paths>` |
| `--gitfile` | `tree -I 'node_modules\|target\|.git' <paths>` |
| `-l`, `-N`, `-q`, `-A`, `-S`, `--metafirst`, `--compress`, `--condense`, `--info`, `--infofile`, `--fromfile`, `--fromtabfile`, `--fflinks`, `--hyperlink`, `--scheme`, `--authority`, `--opt-toggle` | none: nothing on a stock macOS does the same job, so the message names no command |

The HTML options point at `-J`, since JSON is the machine-readable form this build does have. Only the operands of the invocation are substituted, never an option's own argument, so `tree -H base src` says `Use instead: tree -J src`. When no operand was given, the command is shown with the same default tree itself would have used, `.`. All of these are still listed by `--help`, which is upstream's help text unchanged apart from a footer naming this project.

## Patterns

`-P` and `-I` take the same glob language:

- `*` matches any run of characters that does not cross a `/`, `**` matches one that may;
- `?` matches one character;
- `[abc]` and `[a-z]` match one of a set, `[^abc]` one outside it;
- `\` makes the next character literal;
- `|` separates alternatives, so `-I 'node_modules|target'` excludes both;
- a trailing `/` restricts the pattern to directories.

Two malformed shapes are matches rather than errors, as they are upstream. A bracket group with no closing bracket matches whatever the scan had reached when it got there, so `-I 'do['` drops `docs` and `-I 'z*['` drops nothing; and an empty alternative matches everything, so a stray trailing bar in `-I 'node_modules|'` excludes the whole tree. A pattern that is empty from end to end has no alternation in it and excludes nothing.

A pattern is tried against the whole path and against every part of it that starts after a `/`, so `-I 'b/c'` reaches `demo/a/b/c` and a pattern with no slash in it is effectively matched against the base name. Several `-P` or `-I` options are OR'd together.

`-P` never filters directories, which is why `tree -P '*.rs'` still shows every directory, empty ones included. `--prune` removes the ones that ended up with nothing in them; `--matchdirs` lets a directory match the pattern itself, and then everything under it is listed.

`--prune` treats every reason a directory has nothing under it as the same thing. A directory that was empty, one the depth limit stopped at, a symlink to a directory, one over the `--filelimit` and one that could not be opened all disappear, and the last two stop counting as errors, so `tree --prune` over a tree with an unreadable directory in it exits 0 where plain `tree` exits 2.

## Sorting

The default order is `strcoll`, so the locale decides it: under `en_US.UTF-8` the order is `Cargo.toml, docs, node_modules, README.md, src`, and under `LC_ALL=C` it is `Cargo.toml, README.md, docs, node_modules, src`, because byte order puts every capital first. Directories and files are intermixed unless `--dirsfirst` or `--filesfirst` says otherwise. `--sort=size` is largest first; `-t` and `-c` are oldest first; ties fall back to the name.

`-c` and the sort are two separate choices: `-c` decides which time `-D` prints and, on its own, which one the order follows, while `--sort` and `-t` decide the order alone. `tree -c -t` therefore sorts by modification time and prints the time of last status change.

## Meta fields

`-p`, `-s`/`-h`/`--si` and `-D` share one bracketed field in that order, followed by two spaces:

```
[drwxr-xr-x         288 Sep  7 14:56]  src
```

The size field keeps upstream's arithmetic, which includes the one place it overflows: 1048575 bytes prints as `[1024K]`, five columns in a four-column field, because the loop only divides while the size is at least a megabyte. `-D` uses ls's convention, the clock time within six months and the year outside it.

## Colour

Off unless the output is a terminal and `NO_COLOR` is unset, or `-C` forces it, which it does even down a pipe and in spite of `-n` and `NO_COLOR`. The palette comes from `TREE_COLORS`, else `LS_COLORS`, else a built-in default used only when colour was forced or `CLICOLOR` is set. A key the palette does not carry leaves that kind of entry uncoloured.

## JSON

`-J` prints the tree as an array of objects with a four-space indent, each directory carrying its contents, and a trailing report object:

```json
[
    {"type":"directory","name":".","contents":[
        {"type":"file","name":"Cargo.toml"}
    ]}
,    {"type":"report","directories":1,"files":1}
]
```

The stray indentation of that last line is upstream's, and is reproduced here. `--noreport` leaves the object out and a blank line in its place; `-i` puts the whole thing on one line. `-p`, `-s`, `-h` and `-D` add `mode`, `prot`, `size` and `time` keys; `-F` and `-Q` do not change the JSON.

## Exit status

| Status | Meaning |
|---|---|
| 0 | Everything listed, including a file operand or a directory that could not be opened. |
| 1 | A usage error, an unknown option, an unimplemented one, or an argument that would not parse. |
| 2 | A path could not be stat'ed at all, or a directory inside the tree could not be opened. |

An operand that is not a directory prints `Cargo.toml  [error opening dir]` and counts as one file; one that does not exist prints the same note, counts as nothing and makes the status 2. A directory the walk could not open, or one over the `--filelimit`, prints the note where its contents would go and makes the status 2, unless it is the operand itself, which upstream leaves at 0.

The report counts the operand as a directory only when something was listed under it, so `tree` in an empty directory reports `0 directories, 0 files` while `tree` in a directory holding one file reports `1 directory, 1 file`.

## Differences from tree 2.3.2

- **The unimplemented options above are refused**, rather than silently doing nothing, so a script that depends on one fails loudly.
- **`--sort` with a bad type reports on one stream.** Upstream writes `tree: Sort type 'foo' not valid, should be one of: ` to standard error and the list of valid types to standard output; here the whole sentence goes to standard error, so the message survives being redirected.
- **`--charset` with no argument lists the four charsets this build knows** rather than every charset name the C library carries. Only `UTF-8` selects the box characters; ANSI and IBM437 belong to `-A` and `-S`, which are not implemented, so they fall back to ASCII like any other unknown name.
- **`TREE_CHARSET` moves the line drawing only.** Upstream also lets it decide how a name is written out; here that follows the locale alone, which is what `--charset` does upstream too.
- **`--filelimit` reads its argument with `atoi`.** Upstream refuses a separate argument that does not start with a digit; here `--filelimit abc` reads as no limit, the same as `--filelimit=abc` does upstream.
- **A name is escaped as a whole.** In a UTF-8 locale a name that is valid text is printed as it stands and anything else is escaped, where upstream decides character by character within the same name. The escapes themselves match: `\ooo` in a multibyte locale, and `\t`, `\ `, `\\` and friends in a single-byte one.
- **The six-month window for `-D` is 182 days**, which is where upstream's own constant lands.
- **The JSON error flag is not sticky.** Once upstream has printed one `{"error": ...}` block, every later entry that is not a directory carries an empty `"contents":[    ]` array for the rest of the run, in later subdirectories too. That is a leak of the flag that produced the block, and this port leaves those entries with no `contents` key, the way an entry before the error is written.
- **The usage block and `--help` are printed plain.** With colour on, upstream sets every option name in bold and every placeholder in italic in both; here they are the same text without the escapes, so a `--help` captured into a file reads the same either way.
- **A long option must be spelled out.** Upstream matches long names whole, and so does this, so `tree --nore` is the same `Invalid argument` error here as there, unlike the other commands in this project, which take unambiguous prefixes.

Smaller ones: `--version` names this project instead of tree, `--help` carries a footer pointing here, and diagnostics use ASCII quotes.

See the [tree(1) manual page](https://linux.die.net/man/1/tree) for the reference behaviour this port follows.
