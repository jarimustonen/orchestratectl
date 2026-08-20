# Security policy

## Supported versions

Pre-1.0: only the most recent release line gets security fixes. There is
no LTS branch.

## Reporting a vulnerability

If you find a vulnerability that should not be disclosed publicly,
please use the repository's **Security** tab to submit a private vulnerability report with:

- A description of the issue and its impact.
- Steps to reproduce, ideally with a minimal `orchestratectl` invocation
  or sample run state under `~/.orchestratectl/runs/<id>/`.
- Whether the issue has been reported elsewhere.

Please do **not** open a public GitHub issue for a vulnerability until a
fix has shipped.

The maintainers will acknowledge receipt within 7 days and aim to ship a fix or a
mitigation plan within 30 days for high-severity issues. Once a fix
ships, the security advisory is published under the GitHub Security
tab and credited to the reporter unless anonymity was requested.

## Out of scope

- Vulnerabilities in third-party tools `orchestratectl` shells out to
  (`git`, `tmux`, `workmux`, Claude Code itself). Report those upstream.
- Local-only attacks that require shell access to the running user
  account: `orchestratectl` runs unprivileged and trusts the local
  filesystem under `~/.orchestratectl/`. Read its [threat model in
  AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md) before reporting.
