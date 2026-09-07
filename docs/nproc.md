# nproc

Print the number of processing units available. A port of `nproc(1)` from GNU coreutils 9.11.

```
nproc [OPTION]...
```

| Option | Effect |
|---|---|
| `--all` | Print the number of installed processors, disregarding any OpenMP environment variables. |
| `--ignore=N` | Exclude N processing units from the result, if possible. The result is guaranteed to be at least 1. |
| `--help` | Display the help text and exit. |
| `--version` | Output version information and exit. |

## OpenMP environment variables

Without `--all`, two environment variables can override the plain processor count:

- `OMP_NUM_THREADS`, if it parses to a nonzero number, is used directly instead of querying the CPU count at all.
- `OMP_THREAD_LIMIT` caps the result: the final value is never higher than this, whichever of the two produced it.

Both are parsed the same way: leading whitespace is skipped, the value must start with a decimal digit, digits are read until a non-digit (saturating at `u64::MAX` on overflow), trailing whitespace is skipped, and the value is accepted only if that leaves the end of the string or a comma (anything after the comma, such as a second field in an affinity list, is ignored). Anything else, including a fractional number, a negative sign, or trailing garbage like `2bad`, makes the variable count as unset. A parsed value of `0` also counts as unset. `--all` ignores both variables entirely.

`--ignore=N` accepts leading whitespace and a single optional `+` before the digits; anything else, including a negative sign, a decimal point, or embedded non-digit characters, is rejected with an error. An `N` larger than `u64::MAX` is silently clamped rather than rejected. It is applied last, after the OpenMP variables and `--all` have produced a count: that count is reduced by `N`, but never below 1.

## Exit status

| Status | Meaning |
|---|---|
| 0 | Success. |
| 1 | A usage error: an invalid option, an invalid `--ignore` value, or an operand (`nproc` takes none). |

## macOS notes

macOS has no CPU affinity masks and no cgroup quotas, so there's no separate "processors available to this process" figure to distinguish from "processors installed." Both `sysconf(_SC_NPROCESSORS_ONLN)` and `sysconf(_SC_NPROCESSORS_CONF)` resolve to the same `hw.ncpu` sysctl, so `nproc` and `nproc --all` always agree on this platform. If `sysconf` ever fails to return a positive number, the count falls back to reading `hw.ncpu` directly via `sysctlbyname`, and then to `1` if even that fails.

```sh
make -j$(nproc)
make -j$(nproc --ignore=1)   # leave one core free
OMP_NUM_THREADS=4 nproc      # 4, no CPU query performed
```

Differences from GNU: `--version` names this project instead of coreutils, the `--help` footer points here, diagnostics always use ASCII quotes rather than switching to curly quotes in UTF-8 locales, and an empty option name before `=` (`nproc --=1`) is reported as `option '--' is ambiguous`, quoting the empty prefix, rather than GNU's `option '--=1' is ambiguous`, which quotes the whole token. Everything else, down to the exact `parse_omp` and `--ignore` parsing rules above, is the same.

See the [GNU coreutils manual](https://www.gnu.org/software/coreutils/manual/html_node/nproc-invocation.html) for the reference behaviour this port follows.
