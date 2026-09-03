# taskfleet

Canonical Taskfleet command-line package. It contains the sole CLI parser,
dispatcher, and execution engine and installs only the `taskfleet` binary.
Worker creation invokes the explicit `git`, `tmux`, and `workmux` CLIs; it has
no Homebase or private create-script dependency.

The package, crates.io saga, and R7 distribution topology are prepared, but the
0.6.0 cut remains blocked through ADR 0002 R8-R10. The workspace version and
repository URL remain at their truthful pre-transition values.

Licensed under MIT. See the repository `LICENSE` file.
