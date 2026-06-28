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
  writing, compression, UI, diagnostics, and native-app compatibility.
- The native replay app remains external and schema-driven.

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
- For substantial implementation passes, use a Codex-native partner early for
  architecture review. Do not use the obsolete local Codex CLI workflow.

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
- The recorder can replay the result in the native app without save input.

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
  unpaused proof, native-app replay open.

Acceptance:

- New the game build support is a repeatable profile update, not a blind rewrite.
- Unsupported builds fail with a clear user-facing message.
- A bad domain can be disabled without losing the whole replay when safe to do
  so.

## Immediate Next Tasks

Completed in the first repo/live proof pass:

1. Runtime docs now state that live `hoi4.exe` is authoritative and saves are
   development fixtures only.
2. `replay_native/collector` exists as a native DLL crate with heartbeat ABI,
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
    country object, and re-reads scalar values per emitted frame. This proves the
    contract for save-free country scalars including political power.
25. Same-process preflight cache added: repeated recordings against the same
    running `hoi4.exe` validate the cached country-array token with
    `locate.resolve_cached`, read the tag table from `CountryDataBase`, and avoid
    heap scans. The live Portugal process resolved cached preflight in about
    0.05 seconds. Cold discovery for a fresh process still needs a
    root-anchored/hook-derived replacement before first session start is fully
    speed-five-ready.

Next:

1. Turn the bounded hourly worker proof into the recorder's normal long-running
   capture mode with explicit session start/stop, manifest gap entries, and
   backpressure counters.
2. Replace heap-scan identity discovery with a bounded, root-anchored or
   hook-derived country-object identity path. Normal capture must reduce this to
   cached pointer validation plus direct reads; no heap-wide scans in the
   recorder loop.
3. Start adding hourly-readable domains to the worker frame, beginning with
   country/economy roots that can be sampled safely without code patching.
4. Promote the speed-five bounded-worker proof into the normal long-running
   recorder path, preserving per-run pipe tokens, sidecar-off worker mode,
   monotonic producer sequence, duplicate snapshot suppression, and manifest
   failure on skipped scheduled hours.
5. Because the IC-Graphs repo is binary-only for the hook, do not patch the live
   game until our own trampoline/calling-convention plan is reviewed and the
   crash-risk step is intentional. Daily update hooks are not sufficient for
   replay cadence; they are only candidates for daily economy rollups.
