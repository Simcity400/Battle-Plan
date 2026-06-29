# live_capture

Python recorder layer for the native Battle Plan app.

`live_capture/` currently turns read-only memory-reader results into
schema-conformant snapshots and portable `.battleplan` recordings. Under the
in-process collector plan, it also becomes the external recorder/adapter that
consumes collector frames and writes the same replay format. It must keep
measured zero, captured null, and absent data distinct.

Runtime capture has one authoritative source: the live host `hoi4.exe` process.
Saves are optional development fixtures and validation artifacts only; they are
not required to record or replay a live session.

## Role

This package owns capture assembly, durable snapshot writes, continuity
metadata, recorder UI/CLI entrypoints, collector-frame ingestion, and
`.battleplan` writer integration. See `../replay_ui/MASTER_PROGRESS.md` for
current product status and acceptance gates, and
`../IN_PROCESS_COLLECTOR_PLAN.md` for the collector migration plan.

## Main Pieces

| Module | Role |
| --- | --- |
| `capture.py` | Current capture loop, live process attachment hooks, single-shot and interval capture. |
| `collector_attach.py` | External no-op attach helper for loading the collector DLL into a supported `hoi4.exe` and receiving one pipe frame. |
| `collector_country_bridge.py` | Save-free country-object/country-record preflight bridge with same-process cache; feeds compact control contracts while keeping heap scans out of recorder runs. |
| `collector_frames.py` | Synthetic/decoded in-process collector frame validation and snapshot adapter. |
| `collector_map_bridge.py` | State/province map-control preflight helpers and caches; state roots are fast when cached, province array relocation remains the open blocker. |
| `collector_profile_check.py` | Profile preflight: running-exe hash match, root-probe sentinel/address/domain validation, and transport diagnostics. |
| `collector_probe_record.py` | Low-level live collector probe recorder used for bounded cadence proofs and transport debugging. |
| `collector_session.py` | Recorder-facing collector session wrapper that writes lifecycle frames, durable snapshots, `.battleplan` output, and cadence/transport diagnostics without requiring saves. |
| `collector_profiles/` | Build-gated collector profiles: executable hash, offsets, sentinels, roots, and enabled domains. |
| `registry.py` | Import-guarded reader plug registry. Failed/missing plugs omit domains instead of fabricating data. |
| `assembler.py` | Folds reader results into replay snapshots while preserving no-fabrication semantics. |
| `continuity.py` | Campaign/segment/capture sequence tagging and rollback/time-jump handling. |
| `durable_store.py` | Atomic snapshot/index writes and single-writer locking. |
| `recorder/` | GUI/CLI recorder, cadence scheduling, manifest diagnostics, collector transport adapter, packaging spec. |

## Current Recording Reality

- The live host `hoi4.exe` collector is the only runtime-authoritative source.
  Save oracle output is validation/development data only.
- Canonical replay snapshots are hourly-or-better. Daily rows can be rollups,
  not replay coverage.
- `collector_session` currently wraps repeated bounded DLL workers as one
  logical session. That bridge writes lifecycle frames, durable snapshots,
  manifests, and `.battleplan` output, but the target remains a true
  long-running collector session.
- The session writes a validated `collector_frames.jsonl` sidecar containing
  every accepted DLL frame and embeds the same byte stream into finalized
  `.battleplan` files at `payload/<game>/collector_raw/collector_frames.jsonl`.
  The replay store still keeps one canonical snapshot per unique game hour, so
  the raw stream is where duplicate 10 ms samples survive for movement,
  land-combat lifecycle events/deltas, and future event extraction.
- Country identity/scalars are now DLL-native for object-backed fields. The DLL
  reads the `CGameState+0x310` tag-db-id-indexed `CCountry*` table, validates
  each candidate by heap range, country vtable, object-owned `tag_db_id`,
  tag-table decode, stable name, and duplicate checks, then emits
  `country_scalars` without a country preflight bridge. Political power is also
  emitted on this path by validating `CCountry+0xF90 -> CPoliticalStatus` and
  reading `CPoliticalStatus+0xE0`.
- The preflight cache/control-contract path remains for fallback/bootstrap and
  for any future record-only fields: `tag_db_id:country_object_ptr` plus
  `tag_db_id:country_record_ptr`, validated by the DLL per frame. Record rows
  are additive and must not reduce object-row coverage. `country_object_scan`
  must stay false for normal and speed-five proof runs.
- The collector-frame adapter can fold `state_ownership`, `state_control`, and
  `province_control` rows when a collector frame explicitly claims those
  domains. It also folds scheduled `politics_live` rows from the DLL into
  `countries.*.politics` for ruling party, party popularities, active idea
  tokens, typed `laws`, complete `idea_groups`, active cabinet/designer
  `advisor_ideas`, broad non-law `ideas_other`, narrower `national_spirits`,
  idea count, election frequency, party count, and low-confidence diagnostics,
  and scheduled `politics_residuals` rows for
  `last_election` and `elections_allowed`. `appointed_advisors`,
  `appointed_advisor_details`, `appointed_advisor_slots`, and
  `appointed_advisor_ledgers` are folded when the session supplies a validated
  static advisor contract built from `game_defs` / `common/characters`; the
  details include `{idea_token, slot, ledger, character}` without sweeping
  `CAdvisor` heap objects in the hot path. Typed
  law / idea-group / advisor-like / non-law / national-spirit classification is
  folded when the session supplies the static `common/ideas` group contract.
  The DLL intersects those contracts with live active ideas. Missing or invalid
  contracts omit the relevant leaves, while measured-empty `[]` is real. Live
  hourly proof `politics_live_idea_groups_2h_proof` supplied 3722 advisor
  tokens and a 6474-row idea-group contract, advanced 2 game hours, stored 3
  snapshots, verified the replay, emitted 2 scheduled `politics_live` frames,
  folded `idea_groups` for 712 country rows and positive `advisor_ideas` for
  160 country rows, and measured Portugal as
  `advisor_ideas.industrial_concern = companhia_uniao_fabril`.
  It also folds scheduled `decisions_live` rows into
  `countries.*.decisions.status_present` and `.timed_live`; measured-empty
  `timed_live = []` is protected only for tags actually measured by the
  collector.
  It also folds scheduled `focus_live` rows into `countries.*.focus` for
  `focus_tree`, `completed`, `completed_count`, `current`,
  `current_continuous`, and `progress`; measured-empty `completed = []` is
  protected only for tags actually measured by `focus_live`, and `paused`
  remains save-only until a live byte is pinned. Live proof
  `collector_live_focus_probe` advanced 2 game hours with zero skipped hours,
  emitted 3 scheduled `focus_live` frames, emitted 1065 country focus rows, and
  kept worker-poll focus payloads at zero.
  It also folds scheduled `tech_live` rows into
  `countries.*.research.tech_count`, `.research_slots`, and
  `.in_progress_live` from the DLL `CCountry+0xF60 -> CTechnologyStatus`
  slice. The DLL reads the bounded `CTechnologyStatus+0xA0` `CResearchSlot*`
  vector and emits raw slot rows plus `progress_units = progress_raw / 1000`.
  When `--preflight-tech-defs` or an explicit `tech_def_filter` supplies a
  validated live registry map, those rows may also carry `tech_name`; progress
  percentage and ETA remain gated on separate static-cost and live-speed inputs.
  Worker polls must not carry research payloads.
  It can fold scheduled `projects_live` frames into top-level
  `special_projects` with `source = "C"`. The normal session path can now
  preflight/cache special-project roots into a `special_projects_bridge`
  control payload and keeps the old `special_projects_scan` heap sweep disabled
  unless explicitly requested. The bridge carries facilities, all CProject
  identities, and scientist characters to the DLL; manifests keep only counts,
  cache status, and `address_manifest_policy = "omitted"`. Project
  progress/completion, owner attribution, and resolved project definition
  tokens remain open. Worker polls must not carry `special_projects`, and
  pointer/tag-db diagnostics are rejected before folding.
  Session/probe manifests summarize
  country-object identity status, scalar-frame counts, focus-domain sample
  counts, tech-domain sample counts, project-domain sample counts, politics-
  domain sample counts, residual politics-domain sample counts, and raw-frame
  country/state/province row counts.
- Normal map-control recording no longer depends on the old state/province
  preflight caches: the DLL can read the static `CProvince*` array from
  `CGameState+0x2B0`, derive state owner/controller rows through
  `CProvince+0xC0`, and publish province controller TAGs or raw tag-db ids
  through `CProvince+0x188`. The old bridge caches remain useful as
  corroboration/fallback. A live speed-five probe captured 1081 state rows and
  10272 province rows in each scheduled frame with zero skipped hours and no map
  preflight bridges supplied.
- Division movement uses a cache-first `division_bodies.json` bridge. Normal
  recording validates cached body pins without scanning; use
  `--allow-division-cold-scan` only as an explicit bootstrap step because cold
  body discovery can take tens of seconds.
- Army order capture uses a cache-first `army_bodies.json` bridge. Normal
  recording salvages still-live cached army bodies row-by-row; use
  `--allow-army-cold-scan` only as an explicit bootstrap step because the army
  vtable sweep is outside the hot-loop budget.

## Recorder Smoke / Tests

```powershell
python live_capture/recorder/recorder_main.py --smoke
python -m live_capture.collector_attach --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --pipe
python -m live_capture.collector_profile_check --pid <hoi4-pid> --profile live_capture/collector_profiles/vanilla_latest_2026_06_28.json --dll replay_native/target/debug/collector.dll
python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_session --game-id collector_live_speed5_session --duration-s 8 --sample-ms 10 --dwell-s 0.25 --worker-bursts 3 --burst-interval-s 0.25 --until-game-hours 12 --max-wall-s 30
python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_session --game-id collector_live_focus_decision_project_roots --duration-s 8 --sample-ms 10 --dwell-s 0.25 --worker-bursts 3 --burst-interval-s 0.25 --until-game-hours 12 --max-wall-s 30 --preflight-special-projects
python -m live_capture.collector_proof_gate replay/snapshots/_collector_session/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain focus_live --require-scheduled-domain decisions_live --require-scheduled-domain projects_live
python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_session --game-id collector_live_division_cache_bootstrap --duration-s 5 --sample-ms 20 --dwell-s 0.25 --preflight-division-movement --allow-division-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_session_order_org_gate --game-id order_org_strength_cache_bootstrap --duration-s 8 --sample-ms 10 --until-game-hours 1 --max-wall-s 240 --preflight-division-movement --allow-division-cold-scan --preflight-army-orders --allow-army-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_session_order_org_gate --game-id order_org_strength_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300 --preflight-division-movement --preflight-army-orders
python -m live_capture.collector_proof_gate replay/snapshots/_collector_session_order_org_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain divisions --require-raw-domain army_orders --require-raw-event-type division_org_strength_delta
python -m live_capture.collector_probe_record --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_probe --store-root replay/snapshots --game-id collector_live_speed5_worker --dwell-s 0.25 --worker-duration-s 8 --worker-sample-ms 10 --until-game-hours 12 --max-wall-s 15 --unique-store-hours-only
python -m unittest live_capture.tests.test_collector_session live_capture.tests.test_collector_proof_gate live_capture.tests.test_recorder_cadence live_capture.tests.test_recorder_worker_cadence live_capture.tests.test_recorder_manifest_diagnostics live_capture.tests.test_replay_writer
python -m pytest live_capture
```

Run `collector_profile_check` as a separate preflight immediately before worker
recording. Do not combine it into the same recorder process; back-to-back hook
loads are more reliable as separate commands.

Use `collector_session` for recorder-facing capture work. It wraps the bounded
collector worker with explicit `session_start`/`session_stop` lifecycle frames,
feeds decoded collector frames through the durable store, finalizes a
`.battleplan`, packages the raw collector stream, and records cadence plus
transport diagnostics in the session manifest. Use `collector_probe_record`
directly only when proving collector timing, reproducing transport behavior, or
collecting raw probe fixtures.

For focus/decision/project root-contract acceptance, the unit tests above are
only executable scaffolding. The segment is live-verified only after
`collector_session` runs against the real running HOI4 process with
`hourly_requirement.met = true`, `replay_verified = true`, and
`collector_proof_gate --require-scheduled-domain focus_live --require-scheduled-domain decisions_live --require-scheduled-domain projects_live`
passes on that run's `session_manifest.json`. If the special-project cache is
missing, first run a cache-populating diagnostic with
`--preflight-special-projects --allow-special-projects-cold-scan`, then rerun
the acceptance proof without the cold-scan flag.

For division org/strength plus army-order raw-stream acceptance, first populate
same-process caches with the cold bootstrap command if needed, then rerun
without any cold-scan flags. The 2026-06-29 pid 43412 proof advanced 12 game
hours, verified replay output, observed raw `divisions` and `army_orders`, and
captured `division_org_strength_delta`. `army_order_updated` is a separate live
fixture: it requires an already-sampled offensive order to be edited/redrawn
during the capture window.

Repeated bounded workers are one logical recorder session. The raw probe
manifest keeps every verified observation, but the replay writer processes one
selected frame per unique game hour so duplicate 10 ms samples do not dominate
post-processing. Worker sequence ids restart per bounded worker; diagnostics
therefore report both global reset evidence and burst-local monotonicity. If a
later worker burst fails after earlier verified frames, the session writes the
prefix replay and manifest, then exits fail-closed with the attach error.

Country identity rows include the live `tag_db_id` from the collector tag table
even when country scalar capture is unavailable. Treat that id as the safe join
key for future state/province/division/country-object work; do not enable
heap-wide country-object scans in normal recorder runs.
The worker control file and probe manifest explicitly record
`country_object_scan: false` for this reason. A current successful scalar run
should show `country_object_identity_statuses` containing
`gamestate_country_table_verified` and nonzero `country_scalars_frames`; if a
run only has `country_identity`, it captured identity but not live country
scalars.

When `collector_probe_record` is asked for `--until-game-hours`, hourly cadence
is a required proof: the run must advance at least the requested number of
in-game hours, keep game time monotonic, and skip zero scheduled hours. The
manifest records `hourly_requirement`; the CLI exits nonzero on proof failure.
Use `--allow-hourly-gaps` only for diagnostic runs where a failed proof should
still exit 0.

For speed-five acceptance, use `--worker-sample-ms 10` until a measured
less-aggressive interval is documented, and keep `--max-wall-s` set. A run that
catches every hour but exceeds `--max-wall-s` fails with
`max_wall_s_exceeded`; that means cadence worked but speed-five throughput was
not proven. The control file intentionally does not enable
`country_object_scan`; heap-wide discovery is not allowed in proof runs. Worker
proof runs use a per-run named-pipe token and keep sidecar file logging disabled
so stale workers and local file I/O cannot contaminate the timing signal.

## Capture Semantics

- A domain is captured only if its plug runs to completion.
- `0` and `0.0` are measured zero. `null` is captured-but-not-measured or
  unavailable. An omitted domain is not captured.
- Failed plugs do not leak partial state into snapshots.
- Unknown or untrusted dates are kept out of the canonical replay index.
- Canonical snapshot filenames are keyed by trusted in-game time and segment.
- The single-writer lock prevents concurrent capture loops from corrupting a
  store.

## Store Layout

```text
<root>/<game_id>/s<segment>.<hour>.snapshot.json
<root>/<game_id>/index.json
<root>/<game_id>/_noncanonical/
<root>/_continuity/<game_id>.state.json
<root>/_continuity/<game_id>.lock
```

Portable recordings are finalized as `.battleplan` containers and can be opened
by the native app through the same snapshot-store interface.

## Safety

- Live access is read-only: legacy external reads stay under `memory_reader/`,
  and collector-frame ingestion consumes already-decoded DLL output.
- Unknown HOI4 builds fail closed.
- Save-derived data is development validation/discovery data only; do not use it
  as runtime fallback and do not present it as a fresh live read.
- Collector transport gaps are manifest data, not silently repaired by saves.
- Local capture stores, partial recordings, and finalized recordings are
  gitignored.
- Do not commit personal paths, raw dumps, or scratch probe outputs.
