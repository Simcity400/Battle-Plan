# Replay requirements reconciliation — 2026-07-12

Source of truth: `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md` (687 lines,
their last full review 2026-07-09). This doc reconciles every OPEN extractor-side item
against TODAY's code (spot-checked at 6fd03c653) and assigns each to a wave-12b lane, a
serialized owner slot, or a live/RE-gated queue. LANDED items (their table rows 1, 2, 5,
6-weather, 7, 10-mostly, plus AIR in 8) are not re-listed.

## OPEN extractor-side items -> disposition

| Req item | Gap | Disposition |
|---|---|---|
| 3-ext | Counter-badge per-division fields (supply, orders-kind, dig_in, training, attrition, active abilities) + **org MAX/ratio** + battle-bubble fields (progress/bombardment/air-superiority-state) | NEW READER FIELDS (collector lib.rs + d20/d21 payloads) — lib.rs single-owner queue; scoping rides w12-libr-pairs dossier conventions; needs RE for several fields. QUEUED (wave 12c, after census batch commits) |
| 3-naval-note | FleetUnitRow drops task-force NAME | fold-side if name is in d29 payload; else reader field. w12b-fold-wave1 verifies which |
| 4 | Speed CHANGE events (pause already folds: world_fold.rs:2209) | w12b-speedpause-scope dossier (reader design + proposed lib.rs diff); implementation queued behind lib.rs owner slot |
| 4 | DLC DISPLAY names + localisation language, once per recording | w12b-dlc-meta lane (modset/recorder fence) — HELD until the census batch commits (touches main.rs validator's closed grammar) |
| 5 | D56 `supremacy_magnitude` UNIT statement | w12b-fold-wave1 (reader + defines analysis; capture only if code+defines can't decide) |
| 6 | SnowMud ACCUMULATED ground state (R-SNOWMUD) | RE + live-gated (SnowMudData source). QUEUED to the capture shopping list |
| 8 | NAVAL task-force presence rows (+air `air_base_province` placement) | w12b-fold-wave1 folds what d29/d56/d30 payloads already carry; missing payload fields become proposed reader diffs |
| 8 | Active NAVAL combat bubble fields (result_type, progress) | w12b-fold-wave1 (d49/d50 payloads exist — verify fields) |
| 8b | Pair-state/faction/capitals RIDE into snapshots | Partially riding (15 fold hits). w12b-fold-wave1 completes + verifies vs schema §3.3/§3.8/§7b |
| 8b-war | Per-participant war aggregates {warscore, division_count, ic, capitulation_progress, wargoals[]} | NEW reader surface (d60 capwar has capitulation; warscore/wargoals need RE). Scoping in w12b-fow-events-scope batch; live-gated validation |
| 9 | Verify localisation_join artifact ships in PRODUCTION preset + covers label/topbar keys | w12b-locjoin-verify (offline vs the two production runs) |
| 9b | world_tension (A16: clamped scalar or threat ledger) | lib.rs reader — queued behind lib.rs owner; scoping exists (CRACKED_LEDGER.md) |
| 9b | Alert SUB-counts + fired-alert FEED (A18-adjacent), outliner rows, nukes/supply scalars | partially carried; delta list produced by w12b-fold-wave1 audit; new-reader parts queue with 3-ext |
| 9c/A21 | Per-decision dynamic state (zero fold hits today) | needs reader RE (decision manager live state). Scoping dossier in w12b-fired-events-scope (same UI-state RE family) |
| 13/A14 | Per-country FoW/visibility (REQUIRED, core UX) | w12b-fow-scope dossier NOW (CPU-side intel/visibility state candidates from existing RE artifacts); implementation live-gated |
| 14/A18 | Fired-event stream {event_id, target_tag, chosen_option_index, fire_date} (zero fold hits) | w12b-fired-events-scope dossier NOW (event-manager RE scoping; check save-format fired_events as cross-evidence); implementation live-gated |
| 15/A22-33 | 12 deep-panel streams into hourly snapshots | LARGELY FOLD WORK over existing domains (research d13, focus d12, politics d14/15, production d11, designs d59, intel d53, trade d43, occupation d52/66, leaders d47, scripted-vars d51; power-balance + naval-combat-detail + monthly sunk_convoys ledger need reader additions). world_fold.rs is SINGLE-OWNER: w12b-fold-wave1 takes the first tranche; the rest serialize as wave 12c/12d fold slots |

## Lane queue (behind the wave-12 drain; capacity law: <=6-7 concurrent)

Priority: 1) combined-round6 gate rerun (unblocks the census commit + frees
lib.rs/main.rs), 2) w12b-fold-wave1 (world_fold owner slot), 3) w12b-fow-scope +
w12b-fired-events-scope + w12b-locjoin-verify + w12b-speedpause-scope (doc fences),
4) w12b-dlc-meta (after commit), 5) resumes (d51, test-debt, c1-stageb).

Standing law reaffirmed: every NEW runtime-state dependency discovered lands in the
replay requirements doc by the replay agent; the extractor answers HERE and in
EXTRACTION_INTERFACE_REQUESTS.md — this file is the extractor-side ledger for the
2026-07-12 pass.
