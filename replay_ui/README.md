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
| `REPLAY_PARITY_COMPLETION_CHECKLIST.md` | Product parity checklist. This supersedes the narrower engineering DoD. |
| `SESSION_HANDOFF.md` | Current orchestration state, open tasks, caveats, and user dependencies. |
| `BINDING_MAP_COVERAGE.md` | Master UI binding coverage index. |
| `BINDING_MAP_*.md` | Per-cluster view binding maps and no-fabrication notes. |
| `HOI4REPLAY_CONTAINER_SPEC.md` | Portable recording container contract. |

## Current Truth

- Engineering DoD is complete: 114 registered views, menu-reachable, scrub-safe,
  no-fabrication guarded, map/weather wired, coverage checker passing.
- Product parity is still open. The app still needs full read-only interaction
  parity, tooltip parity, live recorder cadence proof, and side-by-side real
  HOI4 visual comparison.
- Data gaps are tracked by `../coverage_audit/memory_manifest.json`, not by
  scattered notes in older docs.
- User help is still needed for a mid-war/eventful paused save and real in-game
  playtest.

## Historical / Reference Docs

These are useful references but should not be treated as the current completion
truth unless they agree with the checklist above:

- `GAME_UI_REPLICA_PLAN.md`
When updating or deleting old docs, preserve unique technical evidence such as
view inventories, binding decisions, schema contracts, and reviewed caveats. Drop
or rewrite stale acceptance claims, old web-app instructions, and obsolete agent
workflow text.
