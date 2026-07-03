# Extractor -- uncracked / open items (2026-07-03, HEAD e75c23753)

Source of truth: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` (re-grep `ROOT_DISCOVERY_OPEN|FIXTURE_BLOCKED` for the live set). Build SHA `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`. Each wave-2 lane has a committed `PROBE_RUNSHEET.md` + SHA-gated probes + save-oracle; the orchestrator runs the live probes.

## 0. PRODUCTION-PIPELINE BLOCKERS (new; not yet ledger rows; block a clean V1 capture)

- **Domain 53 per-frame cost** -- `diplomacy_intel_operatives_live` reads ~7.4s/frame (max 15.7s): `read_operatives` + `read_war_rows` each do a FULL-HEAP `scan_qword_hits` over `DOMAIN53_HEAP_LO..HI` (0x10000000000..0x7FF000000000), 3 sweeps/frame on a ~9GB game. VALIDATED/correct, but the extraction METHOD makes every `--production` capture skip hours at any speed. Fix: reduced-cadence the operative/war heap scans (low-churn -> refresh ~once/game-hour, reuse between), or find a per-country operative/war root to replace the sweep. Cheap sub-scopes (agencies/cryptology/relations/factions/subjects walk the country table) are fine.
- **Frame-time tail + speed-vs-skip gate** -- even without d53, frames avg ~1.15s but p95 ~11.4s (periodic spikes, likely d47 prewarm rescans). At the tested speed the game advanced ~1.5 game-h/s so hours still skipped. Clean V1 needs a lower game speed OR the deferred sub-100ms perf work (task #9). Also every capture shows `compact_worker_panic_seen` + `compact_source_unavailable`: classify transient mid-frame-mutation artifacts (fast advance) vs real always-fail domains.

## 1. FIXTURE_BLOCKED -- need a specific in-game/save/MP fixture, then probe (20)

- **`naval_combat_lifecycle_live` / 49** -- 870ebf chain re-pinned 2026-07-03 from the Bay-of-Biscay fixture (lane `CHAIN_REVERSAL_870EBF.md`): body vtable 0x29C6F18, sides body+0x28/+0x30, side+0x18 parent back-ref; battle location (front+0xA4) is an UNVALIDATED candidate (compact rows carry it absent + `STATUS_LOCATION_CANDIDATE_UNVALIDATED`). Remaining: run the lane `LIVE_VALIDATION_RUNSHEET.md` during an active naval battle (`validated_combat_count>0` + manual location check), then port constants into the collector (documented follow-up).
- **`naval_combat_members_live` / 50** -- Side member-group vector candidate pinned at side+0x2A0/+0x2A8/+0x2B0 (both captured sides), NOT promoted — group->member layout resolves live only via CFEXMember vtable matching; CShip join via CFEXMember+0x08 high-u32. Required-join failures quarantine bridge artifacts (`.REJECTED.*`) and zero validated counts. Needs the same live-battle runsheet as domain 49.
- **Focus paused state** -- Same-build paused-true oracle or proof-backed rejection.
- **Project start/completion lifecycle** -- Controlled before/after project fixture and shared compact rows.
- **Rewards/reward history** -- Before/after reward-option fixture and semantic root.
- **Naval missions** -- Controlled mission-change fixture, then compact mission rows or exact rejection.
- **Home base/repair/retreat** -- Controlled port-change, damaged-repairing, and retreat fixtures, then shared compact rows or exact rejection.
- **Naval combat lifecycle** -- Active two-sided naval battle fixture and shared compact proof.
- **Naval combat members** -- Active naval battle with sinking/member-change fixture.
- **Convoy combat/losses** -- Active convoy battle/loss route fixture, then compact convoy-event rows or exact rejection.
- **Ace identity/assignment** -- Shared roster registration and speed-five proof, plus controlled command `0x2FB5` ace assignment/loss fixture for assignment, participation, death, and wounding semantics.
- **`CSupplySystem` province inputs** -- Rerun the shared compact gate with the game actually advancing at speed five/current equivalent and require zero skipped hours.
- **Hub motorization** -- Same-frame hub toggle fixture with at least two known levels, ideally one level `1` and one level `3`, then capture or reject the exact level field/function output through DLL compact rows.
- **Operations** -- Same-tick running-operation fixture and shared rows, or exact rejection.
- **Spy networks/intel map** -- Same-tick intel-map fixture and shared rows.
- **Diplomacy action queues** -- Live fixture and compact event rows.
- **Scripted GUI open/state** -- Controlled fixture or exact rejection as non-replay-relevant.
- **Delayed events/event queues** -- Controlled delayed-event fixture and compact rows.
- **Hotjoin/tag switches** -- Same-build MP session (2+ players) with a controlled join/hotjoin and a tag-switch/player-country reassignment; validate session-slot/player-array deltas.
- **Chat/messages** -- Same-build MP session with unique chat text, `--expect-text` oracle, and a stable pollable message container with count/text delta.

## 2. ROOT_DISCOVERY_OPEN -- need a production-safe root/hook or a proof-backed rejection (22)

- **`country_economy_counters_live` / 58** -- Orchestrator/user decision: (a) DERIVE factory totals in the replay from domain-23 state building levels (already extracted) plus a small offmap-factory extraction, OR (b) before/after DELTA fixture (change a country's factory count by a known amount in-game, re-probe for the offset that moved by that delta — may find nothing if purely recomputed). Resources need a resource-flow delta fixture or better oracle.
- **Character definitions** -- Shared production character/cabinet route plus static portrait/trait joins.
- **Factories/industry capacity** -- Audit all UI-visible economy counters and capture or reject each.
- **Resources** -- Close resource import/export flows and market stockpile rows.
- **Market stockpile/sellable equipment** -- Find an alternate production root, or prove-backed rejection for remaining market-stockpile semantics.
- **Resource trade/import/export** -- Production root or proof-backed rejection.
- **Licensing** -- New root or proof-backed rejection.
- **Advisors/cabinet** -- Domain 47 advisor roster/appointee/portrait rows remain blocked pending current-build `CAdvisor` / portrait-feed revalidation; static joins still open.
- **Military high command/theorists/designers** -- Keep excluded until advisor/appointee roots and role semantics are revalidated or exactly blocked.
- **Direct focus/research/decision events** -- Exact function/hook contract or proof-backed no-go.
- **Division manpower** -- Pin field/root and compact rows.
- **Division assignment/order linkage** -- Full order geometry and assignment rows.
- **Sortie/combat lifecycle** -- Deeper instrumentation/root or proof-backed rejection.
- **Kill/cause attribution** -- Production attribution rows or exact blocker.
- **Reinforcement reason** -- Root or event hook.
- **Garrisons** -- Production root or exact blocker.
- **Weather causal internals** -- Extract, reject as non-replay-relevant, or fixture-block with proof.
- **Exact province supply tooltip** -- Root/probe that matches UI/save oracle or exact blocker.
- **Convoy/logistics pressure** -- Same-frame foreign-homebase/convoy pressure fixture, plus roots for convoy/port/disrupted pressure; current save has zero disrupted-supply anchors and tag `D04` does not join live.
- **Attrition reason/flag** -- Capture exact reason rows or proof-backed blocker.
- **On-action/effect runtime state** -- Hook/root audit or proof-backed rejection.
- **Mission/decision scripted state** -- Capture or reject field by field.

## 3. Notes for the working agent
- HARD RULES: read `extractor/documents/HANDOFF_2026-07-03.md` section 2. Fail-closed BOTH directions (never promote an unproven offset; never drop real data behind a plausible excuse). Every promotion needs same-frame oracle or controlled-delta proof.
- Delegation: all code via `codex exec`; verify every result yourself + adversarial Codex gate before commit (`C:/Users/Example/.claude/CODEX_PROTOCOL.md`). `codex exec resume` roots its sandbox at the INVOCATION cwd -- always cd into the worktree first.
- Shared hot files (collector/src/lib.rs, recorder_host/src/main.rs, recorder_host/src/world_fold.rs, compact_contract/src/lib.rs): integrate ONE packet at a time, gated, committed; worktree workers stay lane-local.
- DLC asset layers (for asset/sprite work): vanilla + integrated_dlc/* + enabled dlc/* (per dlc_load.json) + mods, plus .tga<->.dds extension fallback. No common/localisation in DLC dirs.
