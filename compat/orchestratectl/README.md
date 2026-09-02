# orchestratectl compatibility wrapper

Deprecated Cargo-only compatibility package for Taskfleet 0.6.x and 0.7.x. It
installs only the `orchestratectl` binary and delegates directly to the canonical
`taskfleet` library dispatcher. It contains no parser, state resolver, or command
implementation and is excluded from cargo-dist.

The wrapper's exact-pinned crates.io leg is release-ready. Version 0.5.1 is
retained and release activation remains blocked until ADR 0002 R7 completes the
independent cargo-dist and Homebrew preparation. Do not publish it directly.

Licensed under MIT. See the repository `LICENSE` file.
