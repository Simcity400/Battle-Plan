# In-Process Collector Plan

Date: 2026-06-28

## Decision

Runtime replay capture has one authoritative source: the live host `hoi4.exe`
process. Saves are not required to record, replay, bridge, or complete a live
session. Save-derived artifacts remain useful for development fixtures,
historical validation, and reverse-engineering audits, but they are not runtime
fallbacks and must not silently fill replay gaps.

The selected direction is a small in-process collector DLL plus an external
recorder:

- `collector.dll` lives inside `hoi4.exe`, hooks known safe update boundaries,
  reads live objects directly, and emits structured records.
- The external recorder owns durability, schema validation, `.battleplan`
  writing, compression, UI, diagnostics, and replay-surface compatibility.
- The replacement replay surface remains external and schema-driven.

## Non-Negotiables

- No game-state mutation: no `WriteProcessMemory`, no scripted edits, no unsafe
  calls into mutating game logic.
- Exact build gate: unsupported `hoi4.exe` builds fail closed.
- Mod/static-data profile: record the exact mod checksum and definition hashes
  used to decode live IDs.
- Hook callbacks stay tiny: identify the boundary, copy minimal roots or enqueue
  work, return quickly.
- Heavy traversal, serialization, compression, and disk writes happen outside
  hook callbacks.
- No save fallback: unsupported fields become unavailable; missing live spans
  become explicit `no_data`.
- Every record carries backend provenance, collector version, build hash, mod
  profile, capture boundary, and quality status.

## Target Architecture

```text
hoi4.exe
  collector.dll
    - build/profile gate
    - hook manager
    - live root resolver
    - bounded domain readers
    - ring buffer / pipe producer

external recorder
  - inject/attach lifecycle
  - transport consumer
  - schema adapter
  - snapshot/event writer
  - .battleplan finalizer
  - diagnostics and gap manifest

native replay app
  - unchanged consumer of snapshot stores and .battleplan containers
```

## Reference: HOI4-IC-Graphs

The example repo at `https://github.com/otti086-Org/HOI4-IC-Graphs` is useful
as architecture evidence, not as code to import. A local clone of commit
`1211247` shows:

- The app is packaged binary-first (`gui.exe` plus `_internal/DataCollection`
  DLLs) and explicitly requires Git LFS.
- It ships one `DataCollection_<hoi4-version>.dll` per supported the game build.
- The newest inspected DLL is a native x64 DLL, not .NET.
- Its export table includes `hk_CGameStateDailyUpdate`,
  `trampoline_CGameStateDailyUpdate`, `CGameStateDailyUpdate_offset`,
  `CGStateDate_offset`, `CountryDataBase_offset`, `SaveDataToFile`, and bundled
  MinHook exports.
- Static strings include `hoi4.exe`, `CCountry`, `CCountryTag`, and
  `pCCountryDataBase_actualAddress`.
- Its README says it writes a data `.txt` file in the game base-game folder and
  the GUI graphs individual/faction IC from that file.

Implication: an in-process DLL can collect country/economy data by hooking a
game update boundary and reading the country database inside the process. That
is useful architecture evidence, but daily cadence is not sufficient for replay.
Our collector should copy the architectural shape, not the cadence or packaging:
build-gated profile, tiny hook callback,
structured frames to an external recorder, no owner paths, no vendored binaries,
hourly-or-better replay cadence, and schema provenance for every captured
domain.

Constraint: the inspected repo is binary-first and does not include the hook
source or calling-convention implementation. Its exports prove that a versioned
native hook exists; they do not provide a safe trampoline implementation to
copy.

## Phase 0 - Repo Contract And Safety

Goal: make the new runtime contract explicit before implementation begins.

Tasks:

- Update `DESIGN.md`, `RESEARCH_in_process_collector.md`, and progress docs so
  saves are development-only, not runtime truth.
- Add a collector threat model: crash containment, hook timing, patch drift,
  thread context, and no-mutation rules.
- Define the initial collector profile file shape:
  `exe_sha256`, game version, supported module ranges, hook RVAs/patterns,
  root offsets, layout sentinels, and mod checksum.
- For substantial implementation passes, use a native partner early for
  architecture review. Do not use the obsolete local CLI workflow.

Acceptance:

- Docs no longer describe save cadence or save fallback as part of runtime
  capture.
- The first implementation issue can be started without re-deciding the product
  architecture.

## Phase 1 - Minimal DLL Skeleton

Goal: inject safely, prove lifecycle, and emit one heartbeat record.

Tasks:

- Create a native collector workspace with a minimal DLL entrypoint.
- Add attach/detach logging and a panic/crash boundary strategy.
- Add exact `hoi4.exe` build hash gating.
- Add collector version and profile loading.
- Implement a no-op transport heartbeat to the external recorder.

Acceptance:

- Unsupported builds refuse to run.
- Supported build emits `collector_started`, `heartbeat`, and
  `collector_stopped` records without writing game memory.
- Attach/detach does not leave background threads or handles behind.

## Phase 2 - Transport And Recorder Adapter

Goal: connect the DLL to the existing `.battleplan` pipeline.

Tasks:

- Choose transport: named pipe first, shared memory/ring buffer if pipe overhead
  becomes measurable.
- Define a binary or line-delimited structured frame format with sequence ids,
  monotonic timestamps, capture boundary, payload type, and checksum.
- Add an external recorder consumer that converts frames into the existing
  snapshot/event schema.
- Add backpressure behavior: drop diagnostics before blocking the game thread.
- Add manifest fields for `backend = "in_process_collector"`.

Backpressure policy:

- Hook callbacks never write files, block on named pipes, allocate large
  payloads, or wait for the recorder.
- Hook callbacks enqueue only bounded frame descriptors into collector-owned
  memory and return.
- A worker thread serializes frames and owns pipe/shared-memory writes.
- If the queue is full, drop order is: diagnostics/heartbeats first, optional
  high-frequency deltas second, scheduled snapshots last.
- A dropped scheduled snapshot produces an explicit manifest gap record; it is
  never hidden by a later save read.
- Transport stalls disable the affected live domain or collector session
  fail-closed rather than stalling `hoi4.exe`.

Acceptance:

- Recorder can create a `.battleplan` from synthetic collector frames.
- Lost, duplicate, out-of-order, and corrupt frames are detected and recorded in
  diagnostics.
- Native app opens the resulting container.

## Phase 3 - Hourly Clock/Identity Proof

Goal: prove hourly live capture from the in-process collector path before any
broader domain traversal.

Tasks:

- Locate date/hour and country database roots for the target build.
- Observe or hook an hourly-safe boundary.
- Emit country tag/identity rows.
- Emit one stored snapshot per observed in-game hour.
- Compare against the existing external readers and save fixtures only as
  development validation.

Acceptance:

- One live unpaused run records consecutive hourly clock/country snapshots from
  the DLL.
- No save is read or required during the run.
- Runtime overhead is small enough to be invisible at the selected MP speed.
- Output maps into replay schema with honest provenance and skipped-hour
  diagnostics.
- Recorder proof mode fails the command when requested hourly coverage is not
  met; gaps can be downgraded only with an explicit diagnostic flag.

## Phase 3b - IC-Graphs-Class Economy Proof

Goal: reproduce the narrow economy data class that HOI4-IC-Graphs proves is
viable, without reducing replay cadence to daily.

Tasks:

- Identify whether hourly economy roots can be sampled directly.
- Use the daily update boundary only for explicitly labeled rollups if no hourly
  economy boundary is safer.
- Emit country MIL/CIV/NAV-style production or factory output rows.
- Compare against save fixtures only as development validation.

Acceptance:

- Economy rows are live-derived and clearly labeled by cadence/boundary.
- Daily economy rollups do not replace the hourly replay timeline and cannot be
  counted as replay coverage.

## Phase 4 - Hourly Light-State Snapshot

Goal: prove the DLL can beat the external warm-cache bottleneck for useful
hourly recording.

Tasks:

- Emit trusted date/hour snapshots.
- Emit country identity, player/AI occupancy where live-exposed, ownership and
  controller roots, country scalars, and province/state control.
- Add before/after tick sentinel and reject torn snapshots.
- Record skipped scheduled hours explicitly.

Acceptance:

- At intended multiplayer speed, hourly capture misses no scheduled hours, or
  every miss is represented in the `.battleplan` manifest.
- Warm steady-state capture time is inside the selected budget.
- The recorder can pack and verify the result without save input; the replacement
  replay surface must consume the same container contract.

## Phase 5 - Domain Promotion

Goal: replace every current save-backed runtime row with live collector output
or an explicit unavailable badge.

Attack order:

1. Divisions, templates, equipment, org/strength, movement, and counter anchors.
2. Buildings, resources, factories, production lines, and stockpiles.
3. Focus, research, decisions, missions, projects, and market.
4. Diplomacy, wars, factions, subjects, intel, operatives, and peace conference.
5. OOB command graphs: armies, army groups, fleets, task forces, air wings.

Acceptance for each domain:

- Build-gated root and layout sentinels.
- Live-only capture path.
- Schema adapter with no-fabrication behavior.
- Fixture comparison available for development, but not required at runtime.
- UI row either becomes live-fed or remains explicitly unavailable.

## Phase 6 - High-Frequency Combat And Movement

Goal: capture the domains most likely to be lost by slow external scanning.

Tasks:

- Identify safe update boundaries for land, naval, and air combat.
- Emit combat lifecycle events: born, participant update, damage/strength delta,
  location/region update, and ended.
- Emit movement/order deltas for divisions and relevant air/naval units.
- Reconstruct engagements from live events and snapshot diffs.
- Attach coverage confidence and missed-boundary diagnostics.

Acceptance:

- Short battles that persist across at least one collector boundary are captured.
- Fully sub-boundary battles are counted as possible misses, not fabricated.
- Combat capture does not stall or destabilize the host process.

## Phase 7 - Hardening And Patch Workflow

Goal: make collector maintenance routine after the game updates.

Tasks:

- Add profile generation/check tools for each supported build.
- Add symbol/string/pattern scans inspired by the HOI4-IC-Graphs evidence.
- Add smoke tests for profile mismatch, hook failure, malformed transport, and
  torn snapshots.
- Add a release checklist: build hash, mod checksum, domain sentinels, live
  unpaused proof, and replay-container verification.

Acceptance:

- New the game build support is a repeatable profile update, not a blind rewrite.
- Unsupported builds fail with a clear user-facing message.
- A bad domain can be disabled without losing the whole replay when safe to do
  so.

## Immediate Next Tasks

Completed in the first repo/live proof pass:

1. Runtime docs now state that live `hoi4.exe` is authoritative and saves are
   development fixtures only.
2. `extractor/native/collector` exists as a native DLL crate with heartbeat ABI,
   a Windows message-hook attach export, and an in-process worker thread.
3. The collector attaches to the observed vanilla `hoi4.exe` build through a
   no-op `SetWindowsHookEx` path and emits frames without `WriteProcessMemory`.
4. Named-pipe transport from the injected DLL to Python is proven for a live
   running game.
5. The build profile records the observed executable hash, IC-Graphs-derived
   offsets, code sentinels, and decoded roots for the current clock and country
   tag table.
6. Root probing inside `hoi4.exe` decodes trusted live current/start dates, the
   country tag table, and active country-present rows.
7. `live_capture.collector_frames` validates collector/root-probe frames and
   maps verified clock probes into replay schema provenance.
8. `live_capture.collector_probe_record` writes one-shot or repeated verified
   collector probes into a normal durable snapshot store.
9. A live Portugal cadence proof recorded four collector-backed clock snapshots over
   roughly eight wall-clock seconds, advanced thirteen in-game hours
   monotonically, then packed and byte-verified a `.battleplan` container.
10. A follow-up live Portugal identity proof recorded 439 country-present rows
    per snapshot, including Portugal, then packed and byte-verified the
    collector-backed `.battleplan` container.
11. Named-pipe frames now carry a producer monotonic sequence id, recorder-side
    byte count, CRC32, and manifest diagnostics for duplicate/non-monotonic
    frame order. A live transport proof showed monotonic sequence, zero
    duplicate sequence ids, 439 country rows, and byte-verified replay packing.
12. Schema/diff tests prove collector provenance validates and collector-backed
    diffs emit exact `source = "C"` events.
13. `live_capture.collector_profile_check` verifies the running process hash
    against the profile before attach, then validates root-probe profile,
    sentinels, expected addresses, domains, country row count, and transport
    metadata. The live Portugal preflight passed against the observed build.
14. The hook backpressure contract is explicit: hook callbacks enqueue bounded
    descriptors only, worker threads own transport writes, lower-value records
    drop first, and scheduled snapshot drops become manifest gaps.
15. The live Portugal hourly proof recorded seven consecutive in-game hours
    (`1936.11.4.6` through `1936.11.4.12`), all deltas exactly `1`, zero skipped
    hours, 439 country rows per snapshot, then packed and byte-verified a
    `.battleplan` container.
16. The safer long-running worker proof replaced repeated attach polling for the
    hourly timeline: one bounded in-process worker attach streamed 30 frames over
    one named pipe, stored seven consecutive hourly snapshots
    (`1936.12.16.23` through `1936.12.17.5`), skipped zero hours, kept 439
    country rows per snapshot, and byte-verified the `.battleplan` container.
17. Worker mode is gated to `hoi4.exe` only, so the controller-side DLL load does
    not run the polling loop. A post-guard proof stored four consecutive hourly
    snapshots (`1936.12.25.9` through `1936.12.25.12`) with zero skipped hours
    and byte-verified replay packing.
18. Operational preflight remains a separate command before worker recording.
    Combining profile preflight and worker recording in one Python process caused
    unreliable back-to-back hook-load timing; separate commands are reliable.
19. Recorder proof mode now records an `hourly_requirement` manifest block and
    exits nonzero if a requested hourly proof skips an in-game hour, goes
    non-monotonic, or fails to advance the requested number of hours.
20. Negative performance finding: the existing external country-object identity
    scans are not runtime-viable. In the live Portugal run, the read-only
    country-index bridge produced no result after more than a minute, and a
    bounded external vtable scan still produced no result after 30 seconds. Those
    readers remain RE fixtures only; runtime identity must be an in-process,
    cached, fail-closed path.
21. Negative safety/performance finding: a naive in-process whole-heap vtable
    scan is also not runtime-viable. In the speed-two Portugal test it failed to
    return a frame after more than 40 wall-clock seconds, and `hoi4.exe` was no
    longer running after the controller was stopped. That path is disabled by
    default behind the explicit `country_object_scan` control flag and must not
    be part of normal capture or speed-five proof runs.
22. Worker transport finding: PID-only named pipes are unsafe for repeated proof
    runs because late workers can connect to the next recorder. Bounded worker
    controls now include a per-run pipe token and disable sidecar file logging by
    default; profile/one-shot attach keeps the legacy PID pipe.
23. User-confirmed speed-five proof: with the game explicitly set to speed five,
    a single 8-second bounded worker at 10 ms cadence recorded 72 pipe frames and
    13 unique hourly snapshots (`1937.11.30.17` through `1937.12.1.5`). The
    required 12 hourly deltas were all exactly `1`, skipped zero hours, completed
    in 11.5 wall seconds under the 15-second proof cap, kept transport sequence
    monotonic, and packed/byte-verified a `.battleplan` container.
24. Country-object preflight bridge added: the recorder can locate the live
    country array before recording, read each country object's own `tag_db_id`,
    pass bounded `tag_db_id:object_ptr` and `tag_db_id:record_ptr` rows through
    the temporary worker control file, and keep `country_object_scan:false`. The
    older localized-anchor `CountryIndexBridge` scan remains a fallback only.
    The DLL validates the expected vtable and `tag_db_id` before accepting an
    object row, validates record rows by checking they still point at the same
    country object, and re-reads scalar values per emitted frame. This remains
    useful as fallback/bootstrap evidence; political power later moved to a
    DLL-native `CPoliticalStatus` path.
25. Same-process preflight cache added: repeated recordings against the same
    running `hoi4.exe` validate the cached country-array token with
    `locate.resolve_cached`, read the tag table from `CountryDataBase`, and avoid
    heap scans. The live Portugal process resolved cached preflight in about
    0.05 seconds. Cold discovery for a fresh process still needs a
    root-anchored/hook-derived replacement before first session start is fully
    speed-five-ready.
26. Session raw-frame preservation added: `live_capture.collector_session` now
    writes `collector_frames.jsonl` with every validated DLL frame from the probe
    manifest, while the canonical replay writer still folds only unique
    game-hour snapshots. Session manifests record the raw frame count, duplicate
    same-hour frame count, and the fact that canonical snapshots are hour-keyed.
    This keeps 10 ms/sub-hour observations available for later combat, movement,
    and event-stream extraction.
27. Map-control collector-frame contract added: the Python adapter validates and
    folds collector `states` / `provinces` payloads for `state_ownership`,
    `state_control`, and `province_control` domains, mirrors province control
    into `map.control_by_province`, and refuses raw numeric/non-TAG controller
    values. Session manifests summarize max country/state/province row counts
    across the raw frame stream.
28. State-root map preflight cache added: `live_capture.collector_map_bridge`
    resolves the live state-object roots once, stores them under the same
    process-identity cache model as the country bridge, validates vtable +
    state-id identity on reuse, and re-reads owner/controller tag-db ids live.
    Live Portugal proof: cold state-root discovery resolved all 1081 states in
    about 28.5 seconds, then a same-process cache hit resolved all 1081 state
    owner/controller rows in about 0.2 seconds with raw addresses omitted from
    diagnostics. This is still a preflight bridge; cold discovery remains too
    slow for first-session speed-five startup, and province-control array roots
    still need the same cache/root treatment.
29. Static map-control root added: the DLL can read the `CGameState+0x2B0`
    `CProvince*` array directly, derive state rows through `CProvince+0xC0`,
    and read province controller tag-db ids from `CProvince+0x188`. A live
    speed-five probe emitted 1081 state rows and 10272 province rows per
    scheduled frame with no map preflight bridges supplied and zero skipped
    hours.
30. Country-object root attempt added but not solved: the DLL now tries a
    bounded `CountryDataBase` object-local scan for tag-indexed `CCountry*`
    arrays and fails closed before emitting scalars. On the current Portugal
    process, `collector_live_country_root_probe` advanced 4 game hours with zero
    skipped hours and verified the replay, but the root status remained
    `country_database_root_insufficient` and no `country_scalars` domain was
    claimed. A cold `--allow-country-cold-scan` check did resolve 355 objects,
    but took about 144.17 seconds, so it remains bootstrap/cache-only.
31. Gamestate country table root added: existing memory research showed
    `CGameState+0x310` as a tag-db-id-indexed `CCountry*` table. The DLL now
    validates that table directly, checks each candidate by country vtable,
    object-owned `tag_db_id`, tag-table decode, stable name, and duplicates, and
    emits object-backed country scalars without country preflight. Live proof
    `collector_live_gamestate_country_table_probe` supplied no country bridge,
    advanced 2 game hours at speed five with zero skipped hours, verified the
    replay, reported `gamestate_country_table_verified` on every frame, emitted
    355 country scalar rows, and kept fuel/production scheduled only.
32. Political power moved off the country-record bridge: existing politics
    research showed `CCountry+0xF90 -> CPoliticalStatus`, with
    `CPoliticalStatus+0xC8` backlinking the owning `CCountry` and
    `CPoliticalStatus+0xE0` storing fixed-point political power. The DLL now
    derives the module base from the validated country vtable, validates the
    political-status vtable/backlink, and emits `political_power` without a
    country preflight or record bridge. Live proof
    `collector_live_political_status_pp_probe` supplied no country bridge,
    advanced 2 game hours with zero skipped hours, verified the replay, emitted
    355 scalar rows per scheduled frame, and reported Portugal
    `political_power = 28.069` on the first scheduled frame.
33. Scheduled country politics core moved into the DLL: the collector now emits
    `politics_live` on scheduled frames from the same validated
    `CGameState+0x310 -> CCountry* -> CPoliticalStatus` path. It reads the
    parties vector, ruling-party pointer, party popularities, exact-size active
    idea list, idea count, election frequency, party count, and a
    low-confidence flag while keeping worker polls light. Live proof
    `collector_live_politics_core_probe` supplied no country bridge, advanced 2
    game hours with zero skipped hours, emitted 2 scheduled `politics_live`
    frames, and reported Portugal as `neutrality` with 12 active ideas,
    party_count 4, and low_confidence false.
34. Scheduled politics residual scalars moved into the DLL: the collector now
    emits `politics_residuals` on scheduled frames by reusing the validated
    `CCountry+0xF90 -> CPoliticalStatus` pointer, double-reading
    `CPoliticalStatus+0xA0` as a Jomini DateHour `last_election` and
    `CPoliticalStatus+0xEC` as the stable `elections_allowed` bool. The
    collector-frame adapter folds those measured leaves into
    `countries.*.politics` while preserving `elections_allowed = false`.
    Live proof `collector_live_politics_residuals_probe` supplied no country
    bridge, advanced 2 game hours with zero skipped hours, emitted 2 scheduled
    `politics_residuals` frames, emitted zero worker-poll politics payloads, and
    reported Portugal `last_election = 1933.3.5.1` with
    `elections_allowed = false`.
35. DLL-native appointed advisors are implemented behind a validated static
    advisor-token contract. The token catalog is sourced from `game_defs` /
    `common/characters`, while live appointment state is derived by
    intersecting that catalog with the validated `CPoliticalStatus+0x50`
    active-ideas vector. This deliberately avoids a `CAdvisor` heap sweep in
    the hot path. Missing or invalid token contracts omit
    `appointed_advisors`; a measured empty `[]` is a real "no appointed
    advisors" result. Live proof `collector_live_appointed_advisors_probe`
    supplied a 3722-token vanilla advisor contract, advanced 2 game hours in
    5.59 collector seconds with zero skipped hours, emitted 3 scheduled
    `politics_residuals` frames, emitted 1065 scheduled country rows carrying
    `appointed_advisors`, kept worker-poll politics payloads at zero, and
    measured Portugal as `appointed_advisors = []`.
36. Typed law / advisor bucket / spirit classification and active timed decisions are now
    DLL-scheduled. A static idea-group contract from `game_defs` /
    `common/ideas` is passed into collector control so the DLL can classify the
    live active-ideas vector into `politics.laws`, broad
    `politics.ideas_other`, narrower `politics.national_spirits`, complete
    `politics.idea_groups`, and active cabinet/designer
    `politics.advisor_ideas` without a save. The static advisor contract from
    `game_defs` / `common/characters`
    enriches hired active advisor tokens into `appointed_advisor_details` and
    `appointed_advisor_slots` without sweeping `CAdvisor` heap objects. The
    same scheduled frame now emits `decisions_live` from the validated
    `CCountry+0xFA0 -> CDecisionStatus` pointer, checks the status vtable plus
    owner back-pointer, records non-empty decision-status vector counts, and
    decodes active `CTimedDecision` rows from the `+0xA0` list. Live proof
    `collector_live_politics_decisions_probe` supplied a 6474-row idea-group
    contract, advanced 2 game hours at speed five with zero skipped hours,
    emitted 3 scheduled frames with `politics_live`, `politics_residuals`, and
    `decisions_live`, measured Portugal laws as `civilian_economy` /
    `export_focus` / `volunteer_only`, measured Portugal 12 active ideas and 9
    `ideas_other`, and decoded non-empty active timed decisions for 7 countries.
    Follow-up hourly proof `politics_live_idea_groups_2h_proof` verified the
    widened group maps in a replay file: 2 game hours advanced, 3 snapshots were
    stored, 2 scheduled `politics_live` frames were emitted, replay
    verification passed, canonical snapshot validation reported zero schema
    problems, `idea_groups` folded for 712 country rows, and positive
    `advisor_ideas` folded for 160 rows. Portugal measured
    `idea_groups.industrial_concern = companhia_uniao_fabril` and
    `advisor_ideas.industrial_concern = companhia_uniao_fabril`.
37. Scheduled focus core moved into the DLL: the collector now emits
    `focus_live` on scheduled frames from the validated
    `CGameState+0x310 -> CCountry*` table. It reads `focus_tree` through
    `CCountry+0x1370 -> CFocusTree`, reads the focus submanager through
    `CCountry+0x1380`, emits completed focuses, completed count, current
    national focus, current continuous focus, and progress days, and keeps
    worker polls light. The Python adapter folds those rows into
    `countries.*.focus`, protects measured-empty completed lists by measured
    TAG, and rejects impossible rows where `current` and `current_continuous`
    are both set. `paused` remains save-only until pinned. Live proof
    `collector_live_focus_probe` advanced 2 game hours with zero skipped hours,
    emitted 3 scheduled `focus_live` frames, 1065 country focus rows, and zero
    focus payloads on worker-poll frames. Portugal measured
    `focus_tree = portuguese_focus`, `completed_count = 22`, current
    `POR_standardization`, and progress `69`.
38. Scheduled research counts and raw active slots moved into the DLL as the
    first `tech_live`
    slice: the collector follows `CCountry+0xF60 -> CTechnologyStatus`,
    validates the technology-status vtable and owner backlink, gates the
    completed-tech compact vector and bounded `CTechnologyStatus+0xA0`
    `CResearchSlot*` vector, and emits `completed_tech_count`,
    `research_slot_count`, and raw `active_research[]` rows only on scheduled
    frames. The Python adapter folds these into
    `countries.*.research.tech_count`, `.research_slots`, and
    `.in_progress_live`, rejects non-scheduled research payloads, and reports
    `tech_live` diagnostics in the session manifest. The slot rows now include
    `progress_units = progress_raw / 1000`, and `tech_name` when a cache-first
    `tech_def_filter` preflight supplies a validated live tech-definition
    handle map. Progress percentage, ETA, and live research speed remain
    separate gated layers.
39. Scheduled special-project catalogs moved into the DLL as the first
    `projects_live` slice: the collector can use a cache-first
    `special_projects_bridge` for project facilities, CProject identities, and
    scientist-backed characters, then reread those pins on scheduled frames.
    The Python adapter folds top-level `special_projects` with `source = "C"`,
    rejects pointer/tag-db diagnostic leakage, and records project-domain counts
    plus bridge/cache diagnostics in the session manifest. The old
    `special_projects_scan = true` heap sweep remains gated and disabled for
    normal runs; cold discovery is only allowed when
    `--preflight-special-projects` and `--allow-special-projects-cold-scan` are
    both requested. This is still catalog/state contract proof only;
    progress/completion, owner attribution, and resolved project definition
    tokens remain open.

Next:

1. Turn the bounded hourly worker proof into the recorder's normal long-running
   capture mode with explicit session start/stop, manifest gap entries, and
   backpressure counters.
2. Audit whether any replay-critical fields remain country-record-only; add a
   bounded root-anchored record identity path only for those fields, keeping the
   record bridge as fallback/bootstrap evidence otherwise.
3. Promote the remaining politics/country-system leaves: deeper cabinet/leader
   buckets, active research slot detail/progress, project progress/definition
   joins, and deeper targeted decision/cooldown internals where the record
   layout is still unverified.
4. Widen the raw DLL stream beyond clock and division telemetry into combat,
   orders, org/strength transitions, and other sub-hour events.
5. Keep map-control on the static `CProvince` route and use the older preflight
   caches only as corroboration/fallback evidence.
6. Promote the speed-five bounded-worker proof into the normal long-running
   recorder path, preserving per-run pipe tokens, sidecar-off worker mode,
   monotonic producer sequence, duplicate snapshot suppression, and manifest
   failure on skipped scheduled hours.
7. Because the IC-Graphs repo is binary-only for the hook, do not patch the live
   game until our own trampoline/calling-convention plan is reviewed and the
   crash-risk step is intentional. Daily update hooks are not sufficient for
   replay cadence; they are only candidates for daily economy rollups.
