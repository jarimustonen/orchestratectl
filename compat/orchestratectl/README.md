# orchestratectl compatibility wrapper

Deprecated Cargo-only compatibility package for Taskfleet 0.6.x and 0.7.x. It
installs only the `orchestratectl` binary and delegates directly to the canonical
`taskfleet` library dispatcher. It contains no parser, state resolver, or command
implementation and is excluded from cargo-dist.

This checkout is pre-cut: version 0.5.1 is retained until the gated release
workflow performs the 0.6.0 bump. Do not publish this pre-cut package.

Licensed under MIT. See the repository `LICENSE` file.
