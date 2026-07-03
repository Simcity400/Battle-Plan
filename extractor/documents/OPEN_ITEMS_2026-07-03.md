# Extractor -- uncracked / open items (2026-07-03, HEAD e75c23753)

Source of truth: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` (re-grep `ROOT_DISCOVERY_OPEN|FIXTURE_BLOCKED` for the live set). Build SHA `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`. Each wave-2 lane has a committed `PROBE_RUNSHEET.md` + SHA-gated probes + save-oracle; the orchestrator runs the live probes.

## 0. PRODUCTION-PIPELINE BLOCKERS (new; not yet ledger rows; block a clean V1 capture)

- **Domain 53 per-frame cost** -- ADDRESSED 2026-07-03 (discovery-cache change, this session). The full-heap `scan_qword_hits` for operative + war vtables is now a background-refreshed discovery cache (`BATTLE_PLAN_D53_DISCOVERY_REFRESH_SECS`, default 30s, min 5s); each frame revalidates cached addresses and reads row VALUES fresh, so per-frame cost drops from ~7.4s to a cheap revalidation while only address discovery lags up to the refresh interval. Fail-closed: stale drops counted into `domain53_stale_dropped_total`, >50%-of-≥8 stale fails closed and forces a synchronous rescan+retry; a generation counter prevents a slow background scan from clobbering a fresh sync repair; the scan thread is always joined (never detached) so it cannot outlive the DLL. STILL TO DO: a live full-profile capture at a non-skipping speed to measure the actual frame-time win and confirm d53 emits rows (the pre-fix `v1_full_profile` capture showed d53 at 0 rows / SOURCE_UNAVAILABLE, predating this fix).
- **Frame-time tail + speed-vs-skip gate** -- p95 ~11.4s spike is NOT d47 prewarm: the `v1_no_d53` manifest shows d47 (`characters_leaders_cabinet_live`) `read_time_us` max/avg/p95 = 0, so its prewarm cost is off the measured read path. Heaviest per-domain reads are all <500ms (`projects_live` 420ms, `naval_missions` 379ms, `scripted_state` 377ms, `ships` 356ms); the p95 tail is frame-level overhead (root-frame resolution / scheduling), needs per-frame timing to attribute (deferred perf, task #9). `compact_worker_panic_seen` + friends are FAST-ADVANCE TRANSIENTS: WORKER_PANIC/STATE_UNAVAILABLE/NO_CLOCK/BRIDGE_UNAVAILABLE appear manifest-wide but on NO per-domain rollup (see `runs/v1_no_d53_capture_870ebf_20260703/FAILURE_CLASSIFICATION.md`). The two domains showed zero rows in those specific captures (SOURCE_UNAVAILABLE). d47 CORRECTION 2026-07-03: the d47 offsets/prewarm logic are NOT broken -- a live diagnostic on the current D04 game (`runs/d47_diag`) confirms the exact production vtable `module+0x2967E30` resolves to `.?AVCCharacter@@` and 98% of 13,493 live CCharacter objects pass the production checks (type_id=73 at +0x08, valid tag_db_id 1..2048 at +0x68). Since `has_stable_roots()` only needs one root populated, the prewarm WOULD stabilize (~13k characters) on this game. The old-capture `stable_root_count=0` is therefore capture-specific, NOT an offset/root bug -- most likely a different save or a fast-advance heap-churn transient during those morning V1-capture attempts (pre-D04). Confirming it needs a fresh D04 full-profile capture (advances the game; user decision). The handoff's "confirmed correct" note stands. d53 zero-rows in the pre-fix `v1_full_profile` capture predates this session's discovery-cache fix; also re-check on a fresh capture.

## 1. FIXTURE_BLOCKED -- need a specific in-game/save/MP fixture, then probe (20)

- **`naval_combat_lifecycle_live` / 49** -- RESOLVED to SHARED_INTEGRATION_PENDING 2026-07-03: lane live gate PASSED on the paused D04 fixture (`runs/nav49_50_port_validation/live_gate/`, `validated_combat_count=4`, Rust proof `passed`, `.battleplan` byte-verified); `front+0xA4` PROMOTED to compact `location_id` (4/4 same-tick save-oracle match) + static `region_id` join. Remaining: orchestrator serial-integration into the shared collector, lifecycle-end fixture, speed-five proof.
- **`naval_combat_members_live` / 50** -- RESOLVED to SHARED_INTEGRATION_PENDING 2026-07-03: the side+0x2A0 vector was REJECTED; the corrected CFEXGroup back-ref walk (vtable 0x2A42D18 RTTI-verified, bucket by group+0x38 owning side) passed the live gate with 117/117 member rows, 8/8 sides' ship-id sets, and 101/101 CShip joins vs the same-tick save oracle (`runs/nav49_50_port_validation/`). Remaining: orchestrator serial-integration, sinking/member-change fixture, speed-five proof.
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
