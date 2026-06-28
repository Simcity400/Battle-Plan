# replay_ui

This directory is now the documentation and product-parity control room for the
native Battle Plan app.

This directory used to contain a loopback/browser replay prototype. That runtime
has been removed so the repo has one replay-app target: the native Rust/wgpu app
in `../replay_native/`, fed by live-memory capture and portable `.battleplan`
recordings. Keep this README as the index for what is canonical, historical, or
still open.

## Canonical Current Docs

| File | Use |
| --- | --- |
| `MASTER_PROGRESS.md` | Single current product-parity status, remaining-work, acceptance-gate, and user-dependency document. |
| `BINDING_MAP_COVERAGE.md` | Master UI binding coverage index. |
| `BINDING_MAP_*.md` | Per-cluster view binding maps and no-fabrication notes. |
| `HOI4REPLAY_CONTAINER_SPEC.md` | Portable recording container contract. |

## Progress Source

`MASTER_PROGRESS.md` is the only current progress, remaining-work,
acceptance-gate, and user-dependency document. Data gaps are tracked in
`../coverage_audit/memory_manifest.json` and summarized in
`../coverage_audit/COVERAGE_REPORT.md`.

## Historical / Reference Docs

These are useful references but should not be treated as the current completion
truth unless they agree with `MASTER_PROGRESS.md`:

- `GAME_UI_REPLICA_PLAN.md`
When updating or deleting old docs, preserve unique technical evidence such as
view inventories, binding decisions, schema contracts, and reviewed caveats. Drop
or rewrite stale acceptance claims, old web-app instructions, and obsolete agent
workflow text.
