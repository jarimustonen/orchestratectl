# orchestratectl compatibility wrapper

Deprecated Cargo-only compatibility package for Taskfleet 0.6.x and 0.7.x. It
installs only the `orchestratectl` binary and delegates directly to the canonical
`taskfleet` library dispatcher. It contains no parser, state resolver, or command
implementation and is excluded from cargo-dist.

The wrapper's exact-pinned crates.io leg and R7 distribution topology are
prepared. Version 0.5.1 is retained and release activation remains blocked
through ADR 0002 R8-R10. Do not publish it directly.

Licensed under MIT. See the repository `LICENSE` file.
