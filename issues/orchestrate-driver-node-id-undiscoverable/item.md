---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: open
priority: normal
---

# Driver run has no node_id in envelope; child spawn requires guessing

## Description

Symptom: a driver run created via `orchestratectl run create --kind orchestrate ... --output json` returns:
```json
{"run_id":"...","dir":"...","supervisor":"orchestrator-in-main-conversation","kind":"orchestrate","lifecycle":"interactive"}
```
**No `node_id` field.** Plus:
- `run show <driver>` -> `manifest.node_count: 0`
- `node list <driver>` -> `{"nodes":[]}`

And `--parent-node-id` is REQUIRED to spawn a child:
```
orchestratectl run create --kind orchestrated --parent-run-id <driver> --parent-node-id <???> ...
```

So the orchestrator has no documented or derivable way to know what value to pass. The agent that hit this in deutschpad-v2 (2026-06-28) had to guess `n-0001` — which worked, but only by coincidence (it's what the first node would be IF one existed).

Compounding bug — `/orchestrate` §2 and `worktree-orchestrated` examples use a different placeholder:
```
--parent-node-id n-driver-001
```
The binary rejects this:
```
{"error":{"code":"invalid_id","message":"invalid node id \"n-driver-001\": expected n-NNNN (n- followed by 4-10 ASCII digits)"}}
```

So the SKILL explicitly tells the agent to pass a value that is BOTH undocumented and syntactically invalid.

Fix direction:
1. Create a `n-0001` "driver node" automatically inside `run create --kind orchestrate`, return it in the success envelope as `node_id`, and ensure `run show`/`node list` see it. This makes `n-0001` the documented, programmatically-discoverable id.
2. Update SKILL examples: replace every `n-driver-001` with `n-0001`. Add a sentence to /orchestrate §2: "Capture `node_id` from the envelope — it will be `n-0001` for a fresh driver run."
3. Document the node-id format (`n-` + 4-10 ASCII digits) in `orchestratectl-overview` so agents stop inventing arbitrary slugs.

Acceptance:
- After `run create --kind orchestrate`, success envelope contains `node_id: "n-0001"`.
- `run show <driver>` reports `manifest.node_count: 1`.
- `node list <driver>` returns 1 node.
- Every SKILL example referencing `n-driver-001` is corrected.

Severity: BLOCKING for /orchestrate (child-spawn is unreachable without it).
