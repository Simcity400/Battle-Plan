# Battle Plan — agent operating rules

This is a **private** repo: a passive, offline, host-side multiplayer-stats / **full-game
replay** tool for the owner's own game group. Read `DESIGN.md` for the vision (maximal-
depth replay: economy, military, divisions, provinces, combat, focuses, decisions, diplomacy —
"everything you'd see in a save game"). It is the owner's own game, read-only/observational —
there is no anti-cheat concern.


## Codex subagent spawning

- Never combine `fork_context: true` with an explicit `agent_type`.
- For full-history forks, omit `agent_type` so the child inherits the parent
  role.
- For `explorer`, `worker`, or other explicit agent types, do not use
  `fork_context: true`; pass the needed context in the prompt instead.
- Do not interrupt a running subagent unless something is actually wrong, such
  as a compile break, duplicated/conflicting scope, unsafe behavior, or work
  that violates these instructions. Let agents complete in their own time.

## Subagent acceptance for recording-layer lanes

Subagents are production owners, not scouts. A subagent may mark a lane complete
only if its work measurably reduces the remaining production extraction
backlog. Anything else is `NOT COMPLETE` and the same agent stays on the same
slice.

Each recording-layer subagent owns a lane-local mini production stack before it
touches shared entrypoints:

- final compact domain ID assigned by the orchestrator registry;
- lane-local extractor/module;
- compact record structs and encoder;
- lane-local decoder/gate code;
- tests;
- live proof output against the running game;
- `.battleplan` verification where possible;
- final registration patch plan.

Subagents should do as much implementation as possible in isolated lane files.
They must avoid editing shared collector/recorder entrypoints directly except
for explicit registration hooks. Their output should be merge-ready production
work, not just research or a broad "slot-ready" summary. The main agent owns
domain ID assignment, the shared registry, orchestration, review, serializing
registration hooks, combined proofs, and conflict resolution.

`MERGE-READY` means the lane has already proven its own production-shaped path:
lane-owned DLL collector/extractor module, compact records, Rust decoder/gate
module, tests, live hourly-or-better proof at speed five/current equivalent,
zero skipped game hours, no partial/source/worker/bridge failures, and
`.battleplan` verification where possible. A packet that still leaves major
collector/recorder implementation to the orchestrator is not merge-ready.

Keep one injected DLL collector/coordinator. Modularization means lane-owned
collector domains and Rust decoder modules registered into the one coordinator
and recorder, not multiple independent DLLs or collectors.

`COMPLETE` means exactly one of:

- Production extraction shipped: DLL-only capture implemented, compact binary
  records emitted, Rust recorder decodes them, `.battleplan` verifies,
  hourly-or-better at speed five or current equivalent, zero skipped game
  hours, and no partial/source/worker/bridge failures.
- A blocker was removed: an unknown root/path/offset became a validated
  production path, exact offsets/functions/structures are documented, and
  implementation can proceed without more discovery.
- A candidate was conclusively rejected: exact probe evidence proves it invalid,
  the search space is reduced, and nobody should retry that path.

Do not accept broad read-only summaries, promising roots without production
impact, "more research needed", "no compact records implemented yet", "could
not verify due to time", or file-reading reports as complete.

Every subagent report must explicitly include:

- Completion status: `COMPLETE` or `NOT COMPLETE`.
- Backlog reduced: yes/no.
- Exact backlog item removed or narrowed.
- Exact files changed.
- Exact roots/offsets/functions validated or rejected.
- Compact schema/record changes, if any.
- Rust decoder/gate changes, if any.
- Proof commands/results.
- DLL-only: yes/no.
- Compact stream: yes/no.
- Rust decoded: yes/no.
- `.battleplan` verified: yes/no.
- Speed-five/current-equivalent hourly-or-lower: yes/no.
- Zero skipped hours: yes/no.
- Any partial/source/worker/bridge failures: yes/no.


## Architecture (one line)

Runtime replay capture has one cadence floor: **hourly-or-better live frames from
`hoi4.exe`**. Daily rows are acceptable only as auxiliary rollups; they never
satisfy replay coverage. The in-process collector is the intended runtime
source, with the legacy memory reader retained for research/validation and the
save oracle retained for development fixtures only. Offsets are **rediscovered
per build** from live and save-derived development invariants, never hardcoded
blindly. Fail closed: if layout validation fails after a patch, omit/badge the
affected field rather than emit guessed data.
