# demo scripts

Two small shell scripts used as a smoke-demo for `/orchestrate`. They count
the files tracked in this git repository and report the number.

## Scripts

### `count-files.sh`

Prints the number of git-tracked files in the repository as a bare integer
(followed by a newline). It `cd`s to the repository root first, so it works
regardless of where you invoke it from. The bare-integer output is intended to
be captured by other scripts.

```sh
bash scripts/demo/count-files.sh
# or, since it is executable:
./scripts/demo/count-files.sh
```

Example output (the count is environment-relative — it reflects the
git-tracked files at run time, so your number may differ):

```
324
```

### `wrap.sh`

Calls `count-files.sh` and wraps its output in a human-readable sentence.

```sh
bash scripts/demo/wrap.sh
# or, since it is executable:
./scripts/demo/wrap.sh
```

Example output (illustrative — the count varies with the repo state):

```
The orchestratectl repo has 324 files.
```

## Dependency

`wrap.sh` depends on `count-files.sh`: it locates the script in its own
directory (via `BASH_SOURCE`) and executes it to obtain the count, so both
files must live in the same directory.
