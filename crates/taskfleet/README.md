# taskfleet

Canonical Taskfleet command-line package. It contains the sole CLI parser,
dispatcher, and execution engine and installs only the `taskfleet` binary.
Worker creation invokes the explicit `git`, `tmux`, and `workmux` CLIs; it has
no Homebase or private create-script dependency.

The package depends on the exact same version of `taskfleet-core` and is the
sole binary package in the workspace.

Licensed under MIT. See the repository `LICENSE` file.
