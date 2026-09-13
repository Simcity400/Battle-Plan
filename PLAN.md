# Battle Plan: plan and status

This is the plan of record. Only the owner changes the decisions in section 2. Agents keep section 1 current and record outcomes in section 7 as short, numbered records with artifact paths. Read section 1 first: it says what is being done, what is left and where the evidence is. The long narrative records written before 2026-09-11 remain in git history (`PLAN.md` at commit `e4a68197b`); section 7 supersedes them.

## 1. Status

Snapshot 2026-09-13. Branch `t3code/23d29856`. Replay format 14 (spool magic `H4SE`), 148 typed tables, 1,407 tests, 870 tracked files. Pinned game build Operation Postern v1.19.2.0.a729.

| Phase | Status | Evidence |
|---|---|---|
| 1.0 Archive | Done 2026-09-09 | 7.1 |
| 1.1 Sampling proof | Done 2026-09-09 | 7.2 |
| 1.2 Format and spool | Done 2026-09-09 | 7.3 |
| 1.3 Collector and recorder | Done 2026-09-09 | 7.4 |
| 1.4 Oracle v0 | Done 2026-09-09 | 7.5 |
| 1.5 Viewer v0 | Done 2026-09-09 | 7.6 |
| 1.6 Knowledge harvest | Done 2026-09-09 | 7.7 |
| 1.7 Delete the old tree | Done 2026-09-09 | 7.8 |
| 2.1 Port the state domains | In progress: takeover repaired raw-copy and comparator defects; fresh oracle passes exist, but latest paired 43.94% throughput and 39.94 ms p95 still fail both performance goals; fresh-save differences remain open | 7.9 to 7.17 |
| 2.2 Gamefiles completeness | Not started | |
| 2.3 Map playback | Playback, scrubbing, branch selection and headless export exist since 1.5 and were exercised on the 2.1 replay; the phase check has not been recorded as such | 7.15 |
| 2.4 Smoke script and reference pairs | Not started; `tools/smoke.ps1` has the driving actions only and one reference pair exists | |
| 2.5 Packaging | Not started | |
| 3 and later | Not started | |

### Current work

- Takeover checkpoint 2026-09-13 (7.16) supersedes the unverified rollout claims below. Live testing is no longer waiting for the owner. Crash containment, ownership/reuse, raw-mirror exactness, comparator joins and real Autonomy comparison are repaired and exercised. The invalid 13-table IntelRest mirror was deleted; those tables use their existing typed hourly walks. Research/buildings batching and removal of a duplicate Relations walk reduce measured steady-state callback cost, but the current copy/decode architecture does not approach 90%. No new cadence, format, viewer or worker-count changes have been made. Next is further copy-path and decode-index optimization, with fresh-save checks and another paired measurement; do not mark 2.1 complete.
- Structural redesign for performance (owner scope 2026-09-11, both goals kept; push to 90% as the primary target, UX decisions delegated to the agent): profile projects in-place batching and caching to about 45 to 50 ms, staggered background tiers to the low twenties, raw-copy split with a single decode worker toward about 4 ms. Stagger tiers approved and implemented (see Phase 1 record below); parallel spool declined. Stale tables emit `unchanged` (format-neutral), compare in the oracle only on refresh hours, and are labeled by refresh hour.
- Gap closure (owner scope 2026-09-11, all 8): 6 intel joins (OperativeMemberships, Cryptology, IntelNetworks, Autonomy, Operations, OperationAssignments) need engine serializer reads plus save parser plus comparator work; 2 deployment fields (custom division name, exile tag) need new recorded fields and therefore format 14, which retires existing captures for the new viewer.
- Phase 1 done 2026-09-11 (committed c73a500a0 and pushed): in-place batching of air registrations and relation tuples with identical rows (engine 499 tests pass); deployment custom-name and exile-tag fields recorded with oracle comparison under format 14 (full workspace 1,040 tests, Clippy and fmt clean; format-13 captures now refused by design); intel serializer reads proved element writer/loader only for Operations (`0x13EF710`/`0x13ED380`) and Assignments (`0x1BD1CD0`), the other four writers unproved with bounded effort; parallel-spool design received (dispatch plus 2 to 4 conversion/diff workers plus single ordered writer, format-neutral, phased slices with live-measurement gates). Follow-ups complete: Operations/Assignments comparators implemented and green (engine 501, installed-content 135, recorder 164); deeper reads found all four remaining writers verified (operative `0xC18FF0`/`0xC11620`, cryptology `0x13E3D30`/`0x13E3950` plus element `0x13E3C70`/`0x13E38B0`, networks `0x11C1BE0`/`0x11C0E70`, autonomy `0x66DEF0`/`0x66C680`); final 4-join implementation landed and the merged tree validates green (full workspace 1,054 tests, Clippy and fmt clean). Gaps addressed 8 of 8: 7 exact comparators, Autonomy census-level with the top-level save root proved absent (0 occurrences in the live save) and each residue limitation-printed. Live oracle re-validation of all new comparators needs a new capture (game required). Stagger tiers approved and implemented 2026-09-11: `replay::cadence` single source of truth; Tier-4 rotated refresh (H0 Characters, H1 Relations, H2 Research, H3 Variants plus Projects plus finished buildings plus LexicalTokens fallback), Tier-H hourly incl. DivisionEquipment and AirBaseRegistrations, off-hours emit `unchanged` with no game-thread walk (format-neutral, no version bump), branch starts and keyframes force full refresh, oracle compares staggered tables on refresh hours only with printed cadence limitations, viewer reads Tier-H only (no change). Full workspace 1,071 tests, Clippy and fmt clean. Projected steady state about 29 to 30 ms average (31 to 33 p95) from 57 to 62 — still above the 16 ms goal; live measurement with the defined protocol (game required) comes next. Parallel spool declined per owner 2026-09-11.

### What is left, in order

1. Finish Phase 2.1 performance and correctness from the takeover measurements (7.16–7.17). Retain the existing four-hour stagger and hourly map tables. The inherited raw-copy count is not an acceptance criterion: 13 faulty intel mirrors were removed, and the remaining families must preserve exact rows and bounded aggregate cost. The latest measured callback is 35.05 ms mean / 39.94 ms p95 including enqueue/bookkeeping tail; paired throughput is 43.94%, not 90%. Index reuse, carried-table spool optimization and batched air pointer copies are implemented and tested. Further work includes cheaper copies and decode, honest raw-payload memory accounting, and carried fresh-save differences. No owner-run tests are pending. Report any proposed cadence/UX changes before implementing them.
2. Milestone 2.2: terrain blend, state borders, localisation names, DDS textures, DLC folder and archive layers, mod layers with `replace_path`, layer hash verification; second reference pair (1936 start map from state history alone).
3. Milestone 2.3: run and record the map-playback check on a 2.1 replay. Custom panels and the generic inspector stay out of scope.
4. Milestone 2.4: the end-to-end `tools/smoke.ps1` action and five reference pairs.
5. Milestone 2.5: `tools/package.ps1` and the unzip-record-view check.
6. Owner-deferred: Milestone 3 items. (Performance now gates 2.1 instead; steady state was about 62 ms and 35% before the redesign, see 7.15.)

### Open failures and limitations (carried, never waived)

- Fresh-save differences: 2026-09-11 saves at hours 60820532, 60822705 and 60822716 differ 4, 21 and 2 with all thirteen allocation counters equal; the 2026-09-11 investigation (7.15) proves each class is a same-hour post-sample mutation (motorization toggles, combat leaves with damage and casualties, one order split), not a table defect. 2026-09-10 saves differ 111 and 25 (recorded before counters existed; audited as post-tick commands in `<local-path>`). Same-hour mutation after the sampled tick remains a documented limitation; the counter witness proves only that nothing was allocated.
- Performance goals now gating 2.1 per owner 2026-09-11 (profile-and-report first): paired 120-second window 31.4% of free-run and 118.6 ms p95 including the double-read phase; steady state about 35% and 62 ms; spool thread 33 to 36 ms CPU per recorded hour.
- Never observed nonempty on the test campaign: VolunteerHosts, TheatreAreas, StationedUnits (no volunteer theatre exists), ProductionLicenseRecords (no licence held), and ten pre-existing tables listed in 7.15.
- Decoded but not comparable against a save: RadarCoverage, AirBaseRegistrations, ThreatCountryAggregates, LexicalTokens, DeploymentLines, DeploymentManpower, DeploymentEquipment, plus the runtime caches and information losses the oracle prints (143 limitation groups).
- Writer and call-order assessments are bounded readings of the decompiled engine; optional debug, script and GUI entrances are not exhaustively closed; a stable double read cannot see a consistently half-updated world.
- Release builds of `collector.dll` and `recorder.exe` are not byte-reproducible on this machine.
- The build ignores synthetic number-key input and camera moves; the smoke driver clicks the speed control and relies on the save's camera.

## 2. Decisions (owner rulings)

| Topic | Decision |
|---|---|
| v1 scope | Capture exact state and replay it over time. The map is the only data visualization for now, with scrub, play, pause, speed, session/branch, pan and zoom. Custom data panels, the country numbers panel and the generic table inspector are deferred (2026-09-10, reaffirmed 2026-09-11). The game-skinned UI is a later phase; nothing in v1 may block it. |
| Approach | Fresh minimal crates. Proven pieces ported by reading the old code. Old tree archived, then deleted once Milestone 1 passed. |
| Fidelity | Data is exact and verified against saves. Visuals come from the game's own files with our own compositing, checked side by side. No byte-identical engine reproduction. |
| Precision | Values are recorded in the engine's representation; fixed-point integers stay integers; rounding only at display; size comes from field masks and compression, never from discarding precision. |
| Evidence corpus | The pre-rewrite tree is at `<local-path>` and tag `archive/pre-rewrite-2026-09-09`. Engine constants replace it: one file per engine struct, one comment per constant. |
| Python | Removed. The viewer parses installed content in Rust; the replay stores content identity, not content. |
| Validation | Local only: section 5 and nothing else. No CI, no test frameworks, no corpus tests, no evidence reverification. |
| Targets | 16 ms per game hour, replay ready within 30 s, campaign under 1 GB with a 50 MB goal. Measured on every capture and reported; the 16 ms p95 and 90% throughput goals gate 2.1 per owner 2026-09-11 (previously reported-only). Both goals are kept: at speed-5 hours this implies about a 4 ms detour. Per owner 2026-09-11, push to 90% as the primary target via the full program (stagger, raw-copy split, deeper measures as measurement dictates); UX-affecting decisions are delegated to the agent, which informs the owner rather than asking. |
| Efficiency | Design for aggregate cost as tables grow: game-thread reads, off-thread work, allocations, copies, pool memory, encoded size. Performance optimization was owner-deferred 2026-09-10; per owner 2026-09-11 it is now required for 2.1, starting with profile-and-report and an agreed approach before optimizing. |
| v1 map set | Clock, country identity, province owner and controller, divisions, land battles, wars and factions, human players, core country numbers, ships and naval battles, air wings, army orders and front lines. First capture priority, not a limit on what is recorded. |
| Recorded tables | Every state domain with a proven walk in the old tree is ported and recorded with its consistency test, its oracle comparison or printed limitation, and its measured cost. Presentation-only and event-hook domains are excluded. Every table must decode through playback; no panel is required. Each table starts with the minimum field set; a field is added only with a measured size cost. Per owner 2026-09-11, all 8 audit gaps will be closed (6 intel joins, 2 deployment fields with a format bump). |
| 2.1 completion | 2026-09-11: finish all remaining Milestone 2.1 work except custom UI panels. This supersedes the 2026-09-10 instruction to finish current work and start nothing new. Per owner 2026-09-11, 2.1 additionally requires both performance goals via structural redesign (about a 4 ms detour; phased with measurement gates) plus closure of all 8 audit gaps. |
| Modded captures | 2026-09-11: a session recorded with a mod enabled still counts as evidence, provided each throughput pair's baseline and capture share the same content roster and save. |
| Mods | Vanilla with all DLC first. The replay records content identity with hashes; the viewer resolves layers by id and verifies them. Mod testing follows the vanilla path. |
| Perspective | Omniscient. Per-country fog and country-specific panels are later phases. |
| Branching | Every game process starts a branch. Within a process, a branch starts on the proven load signal or on an hour discontinuity the collector did not itself record. Parentage is probable or unknown, never asserted. |
| Sound | Dropped. |
| Two games at once | Unsupported; the recorder refuses a second process. |
| Viewer stack | winit, wgpu, egui via eframe. Headless rendering goes through the same wgpu path. |
| Sampling | Exactly one detour on a proven once-per-hour boundary on the game thread, reads within a measured budget, everything else off the game thread, restored on detach. The boundary, its call order, consistency and cost are proved by measurement and by reading the engine. Per owner 2026-09-11, background tables may refresh on a staggered cadence with map tables hourly; tiers approved 2026-09-11 (stagger proceeds, parallel spool declined). |
| Release | Portable ZIP with recorder exe, viewer exe, collector DLL and README. The recorder is a passive console program. |
| Engine files | One file per engine struct; the under-100-file target is superseded with no replacement cap (2026-09-09). |
| Agents | Coordinated helpers on one branch are authorized (2026-09-09). Implementation agents are Opus; each keeps one persistent Fable reviewer and loops with that same reviewer until approved (2026-09-11). |
| History | Git history is kept; the pre-rewrite commit is tagged. |

## 3. Design

### 3.1 Layout

One workspace, six crates: `engine` (pinned build hash, RVAs, struct offsets, sentinels, guarded reads, one walker per table, `PORTING.md`), `replay` (spool and replay formats, tables, frames, deltas, branches, identity, reader and writer), `collector` (cdylib injected into the game), `recorder` (console exe: find, verify, inject, watch, detach, finalize, inspect, oracle), `installed-content` (Rust parsers for installed content, content layers, hashing, text-save parser), `viewer` (eframe app). `tools/smoke.ps1` is the one game-driving script. `reference/` holds game-versus-viewer screenshot pairs. Captures live under `captures\`, never in the repository.

### 3.2 Data flow

The collector's detour, on the game thread, reads every table into a pooled row buffer and pushes it onto a bounded queue. The spool thread diffs against the previous rows, encodes field-masked deltas and appends CRC records to `<out>/<session>/spool.NNNN.h4s`. The recorder watches the process and turns the spool into `<out>/<campaign>.battleplan` on exit or detach. The viewer resolves the recorded content identity, verifies hashes and reads the replay. No game data crosses a pipe.

### 3.3 Engine crate rules

- One file per engine struct; every constant carries a one-line comment naming the decompiled function or struct it came from and how it was confirmed.
- `Build` pins the exe SHA-256, size, version string and hook sentinels; the collector refuses any other build.
- Reads are guarded by an SEH shim: a bad pointer yields `None`, never a crash. Walkers report rows, `Truncated` with the unread count, or `Unavailable` with a reason. They never invent rows, never call engine functions, never retry or wait.
- Native widths and semantics are re-derived from the decompiled writers and serializers; an old reader's width is not evidence.

### 3.4 Replay crate rules

- Rows are typed structs with explicit little-endian fields, engine ids as keys, engine-native numerics. Selected numeric fields encode as zigzag varint differences from the baseline; sorted keys encode as differences from the previous key (format 4 onward). Out-of-range or noncanonical input fails explicitly.
- A frame is one hour on one branch; per table it is exactly one of `snapshot`, `delta`, `unchanged`, `truncated` or `unavailable`. Baselines are per table within a block; after `truncated` or `unavailable` the next frame of that table is a snapshot. Keyframes start every branch and segment and recur every 720 hours.
- Spool records are `magic, length, CRC32, payload`; segments each start with a session or branch record and a keyframe; a write error abandons the segment and starts the next; recovery scans valid prefixes and rebuilds the index. Nothing depends on `DLL_PROCESS_DETACH`.
- Sessions and branches: campaign id from `CGameState+0x728`; every session starts a branch; the load signal (`play_session_started_at`, RVA `0x3317048`) or an unexplained hour discontinuity starts another; parentage is `probable(branch, hour)` when exactly one recorded branch holds the loaded hour, else `unknown`.
- Replay file: header (magic `H4RP`, format version, identity block), zstd blocks of one keyframe plus its deltas, trailer with session, branch and block tables. Appends write a temp file and rename it; an interrupted append leaves the old file untouched. A format change updates writer, reader, collector, recorder and viewer in the same commit and bumps `FORMAT_VERSION` and `RECORD_MAGIC`; older formats are refused.

### 3.5 Collector rules

- The detour does reads and nothing else: hour counter, campaign id, load signal, every table, hour counter again, per-table times, publish. If the hour changed during the read, every table is `unavailable` with that reason. No allocation, diffing, encoding or I/O on the game thread; capacity growth happens off-thread and the next read starts with a snapshot.
- Bounded queue of 256 buffers; overflow drops the frame, counts it and records a gap. The game thread never blocks on the spool.
- Consistency windows: each table is read twice per hour until 200 matching checks complete at stable clock and identity; mismatches are counted and reported. The independent war-matrix census runs 200 checks per attach. Check cost is included in the measured callback.
- Detach: restore and verify the hook bytes, wait for zero active callbacks, drain, write the detach record, join the spool thread, self-unload. The recorder confirms module absence before finalizing.
- Design fallback if game-thread reads miss the budget: copy raw struct bytes on the game thread and decode on the spool thread. Measured on 2026-09-11 the spool thread has no headroom for that (33 to 36 ms per hour), so it is not a lever.

### 3.6 Recorder rules

Passive console program: polls for `hoi4.exe`, refuses a second instance, hashes the exe, builds the content identity with cached layer hashes, injects, prints a status line, detaches on Ctrl+C or `detach`, finalizes from whatever the spool holds. `inspect` prints identity, sessions, branches, gaps, table sizes and cost summary. `oracle` compares a replay hour with a text save through the shared reader and parser, prints every difference under its comparison group with counts, prints its limitations, and prints the thirteen allocation counters against the save header so it can state whether a save was written at the sampled tick.

### 3.7 Gamefiles rules

Content layers in load order (vanilla, DLC in launcher order, mods in launcher order; folders or zip archives; `replace_path` honoured). Layer hash: SHA-256 over sorted (relative path, file SHA-256) under the versioned coverage policy (`map`, `common`, `history`, `localisation`, `interface`, `gfx/flags`, `gfx/interface`, plus descriptors; music, sound, video, loading screens, launcher, crash reporter and logs excluded), cached by path, size and modification time. One Clausewitz parser serves map, colors, localisation, descriptors and text saves.

### 3.8 Viewer rules

First run asks for the game and user folders (`viewer.json`). Opening a replay resolves and verifies every layer; a mismatch is refused; an unverified identity is refused unless `--allow-unverified-content`, which keeps a banner. Map: province id texture, per-hour color lookup, borders by neighbouring ids, pan and zoom. Timeline: play, pause, speed, scrubber with session and branch picker showing parentage. Gaps, truncated and unavailable tables are shown as such. `--dump` renders through the same pipeline offscreen. Overlays (counters, battles, fronts, ships, air) are later map work.

### 3.9 Content identity

Machine-free identity block: game version, exe hash and size, vanilla layer hash, ordered DLC ids with layer hashes, ordered mods with `remote_file_id` or descriptor name and layer hashes, coverage policy version, language, recorder version, collector hash, and verification state (`verified` when the in-memory roster matched the launcher state at injection, otherwise `unverified` with both lists).

## 4. Milestones and acceptance checks

Each phase ends with a check anyone can run; no phase starts before the previous check passes.

### Milestone 1: prove the foundation (done)

- 1.0 Archive: tag, manifest-verified copy of all 30,335 tracked files, new workspace skeleton. Check: root build passes, zero manifest mismatches.
- 1.1 Sampling proof: identify the hourly boundary from the decompiled call order, probe candidates live at every speed, accept only one call per hour on one thread with the hour advancing by one, a stable campaign id and the site after province, country and division updates; 1,000-hour double read with zero mismatches; load signal that fires on every reload and never otherwise; p95 under 5 ms and at least 90% of free-run.
- 1.2 Format and spool: tables, frames, deltas, spool records, recovery, sessions and branches. Check: round-trip and fault tests, a synthetic 2,000-hour three-session campaign with reloads and random faults decodes every readable hour identically and reports every unreadable hour as a gap.
- 1.3 Collector and recorder: attach, verify, one detour, spool, detach protocol; discovery, injection, status, finalize, inspect. Check: ten minutes at speed 5 plus a reload, replay within 30 s, gaps none, p95 under 5 ms, within 90% of free-run; restart appends a session with earlier sessions byte-identical; `taskkill /f` keeps every flushed hour; Ctrl+C detaches with the game running and the DLL confirmed gone.
- 1.4 Oracle v0: refuse binary saves, compare provinces and country scalars against a fresh text save at the recorded hour. Check: zero province differences; every scalar exact or listed as a printed limitation.
- 1.5 Viewer v0: shell, identity verification, headless dump, map with borders, pan and zoom, timeline with session and branch picker. Check: open the 1.3 replay, scrub end to end, pick a branch, dump the last hour, first reference pair.
- 1.6 Knowledge harvest: every constant into the engine crate, one file per struct; `PORTING.md` with one row per old domain and its class. Check: engine builds; 92 registry rows covered.
- 1.7 Delete the old tree. Check: build, tests, strict Clippy pass; tracked file count reported.

### Milestone 2: the v1 product

- 2.1 Port the state domains, in batches. For each domain from `PORTING.md`: (1) walker, table, collector integration, reads only at the hourly boundary; (2) consistency assessment: call-order reading of the structs it touches plus a 200-hour double read with zero mismatches; (3) oracle extension or a printed limitation; (4) a shared capture recording per-table read cost and bytes per hour, aggregate p50, p95 and max, hours per second against free-run, pool memory and spool bytes per hour. Per-table guide 2 ms and 2 KB per hour at speed 5; aggregate goal 16 ms p95 and 90% throughput, reported not gated. Batch 1 (map set) and Batch 2 (extended state) are both implemented; see 7.9 to 7.15.
- 2.2 Gamefiles completeness: terrain blend, state borders, localisation names, DDS textures, DLC folder and archive layers, mod layers with `replace_path`, layer hash verification. Check: a 1936 start map rendered headlessly from state history alone matches the game's 1936 start screen as the second reference pair.
- 2.3 Map playback: preserve the map and controls while extending captured-state playback; show gaps, truncated and unavailable tables honestly. Check: open a 2.1 replay, scrub and play across blocks and branches, confirm the map at the saved hour, verify every recorded table through shared decoding and the oracle within its printed coverage.
- 2.4 Smoke script and reference pairs: `tools/smoke.ps1` checks `save_as_binary`, builds, starts the recorder, launches the game into a chosen save, waits 120 game hours, pauses and saves through its own input, quits, finalizes, runs the oracle on the last hour, dumps it headlessly, scores it against the reference pair and prints hours, p50, p95, throughput, finalize time, size, oracle differences and reference scores. Five reference pairs. Check: the script passes on this machine.
- 2.5 Packaging: `tools/package.ps1` builds, zips the three binaries and README with SHA-256 sums. Check: unzip into a fresh folder, record a session, view it.

### Milestone 3 and later

Politics, focus, research and production panels; naval and air detail; supply; weather; per-country fog; the game-skinned UI from `.gui`, `.gfx`, `.dds` and `.fnt`; custom data panels and the generic inspector. A new captured domain needs a table, walker, consistency assessment and oracle extension; its view can follow later.

## 5. Validation

The whole of validation is:

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
recorder oracle <replay> <save> --hour <n> --branch <id>   # after every capture change, against a fresh text save at the capture boundary
powershell -File tools/smoke.ps1                           # needs the game; before merging recorder or viewer changes
```

- Unit tests cover pure functions: parsers, format round trips, delta and baseline logic, segment recovery, session and branch rules, walkers on synthetic memory. No test spawns the game, reads a capture corpus or embeds evidence.
- The oracle is the correctness check for captured values and prints its own limitations. A difference is never waived.
- The smoke script is the only end-to-end check.
- No test framework beyond `cargo test`, no CI service, no PowerShell test suites.

## 6. Risks

- The double-read test cannot see a world that is consistently half-updated; every table needs its own call-order reading, and same-hour mutations after the sampled tick remain visible only through the oracle (now with the allocation-counter witness).
- Reloads inside a session to the very next hour without a load signal are a documented blind spot; parentage is never asserted.
- A game update changes every RVA; the collector refuses on hash mismatch and a new harvest is required.
- Forced termination skips detach; the spool design keeps everything flushed recoverable.
- Launcher state may differ from what the process loaded; the identity records both and the viewer refuses to call it verified.
- The owner's Documents folder is OneDrive-redirected and holds binary saves; validation uses the isolated text-save user directory.

## 7. Evidence log

Capture artifacts live under `captures\`; the old tree under `C:\hoi4_evidence\`; decompiled engine functions in `<local-path>` (see `PORTING.md`). Numbers below are the acceptance figures; the JSON, oracle and verification files in each capture directory hold the rest.

### 7.1 Milestone 1.0, archive, 2026-09-09

Tag `archive/pre-rewrite-2026-09-09` = commit `901e2ec25`. All 30,335 tracked files (4,486,036,143 bytes) verified by size and SHA-256 against `<local-path>` (manifest SHA-256 `6e9f1929…7711b`). Workspace skeleton built beside the old trees.

### 7.2 Milestone 1.1, sampling proof, 2026-09-09

- Load signal: `play_session_started_at` at RVA `0x3317048` changed on all six in-process loads and never otherwise; one-second resolution, so collisions are a blind spot and the discontinuity rule stays.
- Live: one call per hour on one thread at all five speeds; 6,312 hours with matching double reads of 13,413 provinces and 439 countries; paired 120-second window 39.077 versus 41.272 hours/s (94.7%), total p50/p95/max 3.608/4.882/9.031 ms.
- Corrections to old readers: country scalars are signed i64 fixed-point; CArmy `+0x420/+0x428` strength and organisation are i64; `+0x1F0` is the location province; `+0x490` is play time, not a session id; `+0x338` is a mobilization ratio, not manpower.
- Artifacts: `captures\m11_20260909\` (probes, baselines, static derivations).

### 7.3 Milestone 1.2, format and spool, 2026-09-09

Replay crate with clock, campaign, countries, country links and provinces; field-mask deltas; per-table baselines; spool records, segments, prefix recovery; sessions, branches and parentage. 66 tests including the synthetic 2,000-hour three-session fault campaign.

### 7.4 Milestone 1.3, collector and recorder, 2026-09-09

- Production lifecycle passed: attach and verify, recording, reload branch with `probable` parent, normal exit, restart append with prior blocks byte-identical, `taskkill /F` with every flushed hour recovered, Ctrl+C detach with module absence confirmed.
- Throughput: first ten-minute pair failed at 89.8%; after encoder and worker changes the ten-minute pair passed at 49.090 versus 45.708 hours/s (above 100% is run variance), p50/p95/max 1.346/1.861/9.971 ms. Finalization seconds, not minutes.
- Content identity verified: 24 DLCs, vanilla layer hash `3b232dd2…dd141`, 35,476 covered records.
- Artifacts: `captures\m13_20260909\` (`acceptance\measurement.json`, `forced-verification.json`, `ctrlc-verification.json`).

### 7.5 Milestone 1.4, oracle v0, 2026-09-09

`GER_1936_09_29_19.hoi4` at hour 60765882, branch 2: zero differences across 13,413 provinces, 439 countries and 2,194 country scalars. Binary saves refused with the `save_as_binary=no` instruction. Printed limitations: raw country-link identities, omitted controllers, session time, load timestamp. Artifact: `m13_20260909\acceptance\oracle.txt`.

### 7.6 Milestone 1.5, viewer v0, 2026-09-09

Viewer opens, scrubs, picks branches, renders political and control maps, pans, zooms and dumps headlessly through the same wgpu path (RTX 5090, Vulkan). First reference pair `reference/01.md` at branch 2, hour 60765882, camera `3010,477.5,248.7`. Province bitmap 5632×2048 with 13,413 used ids and 439 authored colors; runtime color overrides are a stated presentation limitation. Artifacts: `captures\m15_20260909\`.

### 7.7 Milestone 1.6, knowledge harvest, 2026-09-09

756 engine source files, 6,867 commented constants, 215 RTTI-resolved vtables; `PORTING.md` covers all 92 registry entries (74 state, 11 presentation-only, 7 event-hook). Material corrections recorded there.

### 7.8 Milestone 1.7, old tree deleted, 2026-09-09

30,329 legacy files removed; 809 tracked files; 150 tests; strict Clippy clean. Milestone 1 complete.

### 7.9 Milestone 2.1, states, 2026-09-10

States table (format 2) from the state-id-indexed `CGameState+0x2C8` array. 26,314-hour session with zero mismatches; clean 600-second pair 92.6% of free-run, p50/p95/max 2.264/2.911/8.175 ms; oracle zero differences (13,413 provinces, 1,081 states, 439 countries, 2,194 scalars); restart, forced termination and Ctrl+C pass. Artifacts: `captures\m21_states_20260909\`.

### 7.10 Milestone 2.1, divisions, 2026-09-10

Divisions table (format 3) from each canonical country's `+0x290` roster with full u64 handles, three owners, authenticated location and template, i64 hit points and organisation. 26,783 hours with zero mismatches; 600-second pair 92.03%; oracle zero differences including 1,337 divisions. Size experiment showed combined difference coding cuts the file 55%, which decided format 4. Artifacts: `captures\m21_divisions_20260910\`.

### 7.11 Milestone 2.1, lossless encoding, 2026-09-10

Format 4: zigzag varint numeric differences and key differences. Two saves with zero differences; one save with one difference (Turkey political power, an event debit after the tick), which established the boundary limitation: the serial session consumer `0x223DDC0` runs later commands after the hourly command `0xEF1A60` that calls the sampled tick, without another hour increment. Artifacts: `captures\m21_encoding_20260910\` (`oracle-boundary.md`).

### 7.12 Milestone 2.1, war relations and factions, 2026-09-10

Format 5 adds WarRelations, Factions and FactionMembers (ten tables). Bounded writer assessment in `PORTING.md`. Consistency windows changed from indefinite double reads to 200 matching checks per table per attach. Final 120-second pair 93.302% of free-run, p50/p95/max 1.2166/2.2766/13.5167 ms; oracle zero differences at hour 60794138; retained failures: BRM and POR command power, Finland war support, Yemen convoys (post-tick commands, unproved). Seek optimization: cold block end 315.6 to 56.3 ms. Generic inspector removed under the owner's deferral. Artifacts: `captures\m21_wars_factions_20260910\`.

### 7.13 Milestone 2.1, remaining map tables, 2026-09-10

Format 6 completes the 20 map-set tables (players, land combats, fleets, task forces, ships, naval combats, air wings, army orders). Third candidate 96.07% of free-run, p50/p95 2.3576/8.8395 ms; oracle zero differences at hour 60814143 (2,576 divisions, 461 wars, 1,951 ships, 768 orders); one failed reload save (a Mexican order created after the tick); lifecycle passes. Artifacts: `captures\m21_map_20260910\`.

### 7.14 Milestone 2.1, extended state, 2026-09-10 (format 7 and 8, not accepted)

- Format 7 registered 124 tables; first capture 25.4% of free-run, p50/p95 75.8/163.8 ms; parser failure on a state-history decimal, then 788 and 878 diagnostic differences on the frozen recording (army upgrades, garrison templates).
- Format 8 corrections and read batching: 29.783% of free-run, p50/p95 56.787/116.882 ms; oracle zero differences across 122,460 comparisons at hour 60818070 with ScriptVariables and LexicalTokens still unavailable; 119 windows at 200 checks.
- Source-enabled build (884 tests): all 121 eligible windows at 200; saves at 60818944 and 60819227 fail 111 and 25 differences (orders, stockpiles, focus, combat; timing unproved); final save `GER_1942_11_01_20.hoi4` at 60819235 zero differences across 136,269 comparisons with all 124 tables available. Lifecycle passes; 850 tracked files.
- Artifacts: `captures\m21_extended_20260910\`.

### 7.15 Milestone 2.1, completion pass, 2026-09-11 (verdict: investigation closed; 2.1 held open for performance and gap audit)

Scope: owner instruction to finish all remaining 2.1 work except custom UI panels. Six branches merged (D102 fog inputs; D11 licences and deployment plus D57 world tension and speed; order of battle, incoming diplomatic actions, scientists and oracle reporting fixes; relation opinions and tuples, country intel lanes and thirteen allocation counters on the clock row; two performance batches). A registry audit found those five extra sub-families required under the recorded-tables rule and classified the 111/25 differences as post-tick command timing (`<local-path>`). Format 13, 148 tables, 1,037 tests, 867 tracked files.

- Consistency: all 145 tables with a window reached 200 checks with zero mismatches in two long sessions; war-matrix census 200/0; zero dropped frames, gaps or damaged segments over 4,656 hours; queue peak 1. 134 tables observed nonempty.
- Oracle: `GER_1942_12_25_20.hoi4` at hour 60820531, branch 1: zero differences across 138,439 comparisons, all 148 tables available, no skipped group, all thirteen counters equal the save header (an exact-boundary comparison). Three later saves at hours 60820532, 60822705 and 60822716 differ 4, 21 and 2 (hub motorization, land-combat attackers, hit points and equipment and casualties, one army order) with counters equal; investigation closed 2026-09-11, see the verdict below.
- Measurement: paired 120-second window 8.483 versus 26.988 hours/s (31.432%; 29.839% by exact common hours), callback p50/p95/max 58.949/118.623/143.248 ms including the double-read phase; steady-state windows p50 57.1 to 58.3 ms, p95 61.7 to 63.2 ms, 34.7 to 35.3% of free-run. The two performance batches cut the twelve previously measured tables from 46.100 to 29.393 ms of p50 (ResearchTechnologies 15.241 to 7.595); the 24 new tables cost 11.693 ms and 3,935 bytes per hour. Pool peak 513,444,832 bytes, 257,923,296 at rest; spool 64,440 to 71,387 bytes per hour, spool CPU 35.85 and 33.17 ms per hour.
- Defects found live and fixed with tests: `CMilitaryDeployment+0xB8` is the owning line, not a template (all four deployment tables had been unavailable; format 12 became 13); the saved theatre `area` field is one array; a country's relation with itself is ordinary state.
- Lifecycle on one campaign file: commanded detach (finalize 1.942 s), recorder-first restart with in-session reload and normal exit (3.619 s), `taskkill /F` recovery (1.068 s), real console Ctrl+C with module absence confirmed and 277 further hours observed without the DLL (0.231 s). Recovery verification 12 then 21 checks pass at every append. Final replay 71,738,541 bytes, four sessions, five branches, 4,656 hours.
- Viewer: headless dumps of hours 60820531 and 60822716; windowed open, scrub within and across blocks, play, session and branch switch with probable parent shown, map modes, fit, pan, zoom.
- Empty on this campaign: VolunteerHosts, TheatreAreas, StationedUnits, ProductionLicenseRecords, CountryOccupationReserveManpower, CountryCollaboration, ActiveAbilities, AirTheatres, AirTheatreGroups, AirTheatreMembers, NavalResults, NavalResultMembers, NavalResultCountries, PeaceConferences.
- Artifacts: `captures\m21_completion_20260911\` (`reports\validation-report.md`, `reports\per-table.md`, `reports\measurement-summary.json`, `reports\oracle-summary.json`, `initial\`, `append\`, `forced\`, `detach\`, `probes\`, `screenshots\`, `builds\`). Per-table results are in the appendix.
- Verdict 2026-09-11, fresh-save investigation closed with no table defect. Four read-only class investigations, each high confidence: (1) hub motorization: explicit `motorization_level` map edits verified in save bytes against the clean hour-1 control (SWE `101+9108` to `383`; SPR `8038` toggle-off; SWE `101+9108` toggle-on), no ID counter covers hub maps; (2) army order instance 1600 of group 5771 (type 53): `creation_date` equals the save hour with `split_from=1553`, instance ids run on an unwitnessed per-country counter so equal `order_index`/`front_index` is expected; (3) land-combat attackers on 16669/17222/17275/17286: replay-superset prefix-identical truncations, save `unit=` proved the full list including reserves, all 11 extra divisions alive in the save; (4) damage triple: 7 hit-point moves are exact raw integers, division 71121 (`305462369063529`) bridges equipment, hit-point and attacker groups, war casualties rise coherently, and combat RNG is side-local so the equal global `multiplayer_random_count` is consistent. Cross-cutting: the clean hour-1 control (0/138,439, same code) rules out systematic offset, width or join defects; 145 tables at 200/0 double reads; complete tables throughout; the current tree re-runs the 60820532 oracle byte-identically (4 diffs, 13 counters equal). Validation on the verdict tree: `cargo build`, 1,037 tests, strict Clippy and `cargo fmt --check` all pass. No code fix was needed; the equal-witness limitation is already printed by the oracle. Per owner 2026-09-11, 2.1 is not recorded complete yet: the 16 ms p95 / 90% throughput goals and a replay-data gap audit are now in scope, profile-and-report first.

### 7.16 Measurements table

Every capture prints the same table. Site is the full-tick return at RVA `0x1DCEF0` for every row after the old design.

| Capture | Site | Hours | Total p50 ms | Total p95 ms | Max ms | Hours/s with | Hours/s free | Pool MB | Spool KB/h | Finalize s | Size MB | Oracle diffs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| e2e_08, old design, 2026-09-05 | polled | 453 | | | | 7.7 | 16.4 | | | 684 | 47 | not run |
| m11 speed-5 probe, 2026-09-09 | full-tick return, 0x1DCEF0 | 6,312 | 1.751 | 2.671 | 11.846 | 52.793 | 57.053 | 220.676 | n/a, probe JSON | n/a | n/a | not run |
| m11 reload probe, speed-5 portion | full-tick return, 0x1DCEF0 | 2,068 | 2.047 | 3.416 | 5.122 | 47.040 | 57.053 earlier, unmatched | | n/a, probe JSON | n/a | n/a | not run |
| m11 paired speed-5 probe, final build | full-tick return, 0x1DCEF0 | 4,693 | 3.608 | 4.882 | 9.031 | 39.077 | 41.272 | 221.285 | n/a, probe JSON | n/a | n/a | not run |
| m13 short production capture, 2026-09-09 | full-tick return, 0x1DCEF0 | 2,017 | 2.204 | 2.824 | 3.926 | 37.211 | not paired | 118.249 | 0.642 | 3.083 | 0.374 | not run |
| m13 main session, includes setup and reload | full-tick return, 0x1DCEF0 | 28,084 | 2.145 | 2.730 | 4.973 | 34.261, includes pauses | n/a | 118.249 | 0.621 | 7.337 | 5.058 | not run |
| m13 main first-branch 600-second window | full-tick return, 0x1DCEF0 | 21,425 | 2.155 | 2.737 | 4.973 | 35.709 | 39.754 | 117.518 | n/a, window | n/a, window | n/a, window | not run |
| m13 optimized first-branch 600-second window | full-tick return, 0x1DCEF0 | 29,449 | 1.346 | 1.861 | 9.971 | 49.090 | 45.708 | 117.518 | n/a, window | n/a, window | n/a, window | not run |
| m21 states first session, 2026-09-10 | full-tick return, 0x1DCEF0 | 26,314 | 2.216 | 2.893 | 8.175 | 36.406, both branches | n/a, full session | 130.854 | 0.649 | 10.094 | 4.930 | 0 |
| m21 states first-branch 600-second window | full-tick return, 0x1DCEF0 | 21,379 | 2.264 | 2.911 | 8.175 | 35.638 | 38.486 | 130.124 | n/a, window | n/a, window | n/a, window | see saved branch 2 |
| m21 divisions first session, 2026-09-10 | full-tick return, 0x1DCEF0 | 26,783 | 2.712 | 3.757 | 9.378 | 35.660, both branches | n/a, full session | 231.540 | 3.491 | 8.272 | 22.819 | 0 |
| m21 divisions first-branch 600-second window | full-tick return, 0x1DCEF0 | 21,192 | 2.721 | 3.768 | 9.378 | 35.327 | 38.386 | 230.810 | n/a, window | n/a, window | n/a, window | see saved branch 2 |
| m21 encoding first session, 2026-09-10 | full-tick return, 0x1DCEF0 | 19,284 | 2.389 | 3.446 | 13.431 | 35.761, both branches | n/a, full session | 231.540 | 2.386 | 6.108 | 8.920 | 0 first save; 1 second save |
| m21 encoding first-branch 120-second window | full-tick return, 0x1DCEF0 | 4,845 | 2.028 | 2.905 | 4.817 | 40.405 | 38.810 | 230.810 | n/a, window | n/a, window | n/a, window | see saved branches |
| m21 encoding appended session, Ctrl+C | full-tick return, 0x1DCEF0 | 5,580 | 2.834 | 3.863 | 10.402 | 19.789, includes save pause | not paired | 231.540 | 2.910 | 2.789 | 12.009, campaign | 0 |
| m21 war/faction initial session | full-tick return, 0x1DCEF0 | 27,334 | 2.240 | 3.190 | 25.156 | see paired window below | n/a, full session | 293.242 | 3.846 | 9.018 | 21.360 | 2 first save; 0 second save |
| m21 war/faction optimized batch, 120-second window | full-tick return, 0x1DCEF0 | 5,484 | 1.217 | 2.277 | 13.517 | 45.699 | 48.980 | 292.518 peak; 210.336 after checks | n/a, window | n/a, window | n/a, window | see saved branch 7 |
| m21 war/faction optimized batch, Ctrl+C session | full-tick return, 0x1DCEF0 | 6,426 | 1.227 | 2.198 | 16.701 | 21.507, includes save pause | not paired | 293.249 peak; 210.336 after checks | 3.044 | 0.768 | 40.588, campaign | 0 |
| m21 format 7 initial, 120-second window | full-tick return, 0x1DCEF0 | 893 | 75.783 | 163.802 | 190.856 | 7.457 | 29.327 | n/a, window | n/a, window | n/a, window | n/a, window | initial invocation failed; later diagnostics below |
| m21 format 8 optimized, 120-second window | full-tick return, 0x1DCEF0 | 1,090 | 56.787 | 116.882 | 133.626 | 9.097 | 30.546 | n/a, window | n/a, window | n/a, window | n/a, window | see saved hour below |
| m21 format 8 optimized, Ctrl+C session | full-tick return, 0x1DCEF0 | 1,097 | 56.788 | 116.882 | 133.626 | includes save pause | not paired, session | 400.742 peak; 201.533 final | 58.415 | 1.430 | 12.926 | 0; incomplete, two sources |
| m21 source-enabled normal/reload session | full-tick return, 0x1DCEF0 | 872 | 50.083 | 104.779 | 120.596 | includes lifecycle pauses | not paired | 450.4 peak | 74.347 | 1.124 | 30.816, campaign | 111 |
| m21 source-enabled forced session | full-tick return, 0x1DCEF0 | 283 | 102.448 | 110.942 | 116.506 | includes lifecycle pauses | not paired | 450.4 peak | 114.059 | 0.485 | 35.894, campaign | 25 |
| m21 source-enabled final Ctrl+C session, startup only | full-tick return, 0x1DCEF0 | 8 | 105.080 | 124.125 | 124.125 | no throughput claim | not paired | 451.5 peak | 2,414.225, startup only | 0.163 | 37.897, campaign | 0; complete within printed limits |
| m21 completion paired 120-second window, 2026-09-11 | full-tick return, 0x1DCEF0 | 1,015 | 58.949 | 118.623 | 143.248 | 8.483 | 26.988 | 513.445 peak | n/a, window | n/a, window | n/a, window | see hour 60820531 below |
| m21 completion steady-state supplement, three windows | full-tick return, 0x1DCEF0 | 1,030 | 57.135-58.312 | 61.709-63.223 | 74.884 | 9.375-9.493 | 26.988 | 258.841 | n/a, windows | n/a, windows | n/a, windows | n/a, supplement |
| m21 completion initial session, commanded detach | full-tick return, 0x1DCEF0 | 1,297 | 58.834 | 117.598 | 143.248 | includes the check phase and pauses | not paired, session | 513.445 peak; 257.923 final | 64.440 | 1.942 | 17.544, campaign | 0 at 60820531; 4 at 60820532 |
| m21 completion append session, normal exit | full-tick return, 0x1DCEF0 | 2,721 | 57.449 | 116.282 | 135.868 | includes the check phase and pauses | not paired, session | 513.445 peak; 258.841 final | 71.387 | 3.619 | 58.885, campaign | 21 at 60822705 |
| m21 completion forced termination | full-tick return, 0x1DCEF0 | 627 | 57.951 | 127.145 | 201.929 | includes the check phase | not paired, session | 514.887 peak | 77.468 | 1.068 | 69.055, campaign | no save taken |
| m21 completion Ctrl+C session, startup only | full-tick return, 0x1DCEF0 | 11 | 116.271 | 132.264 | 132.264 | no throughput claim | not paired | 514.887 peak | 2,399.596, startup only | 0.231 | 71.739, campaign | 2 at 60822716 |

## Appendix: per-table results, 2026-09-11 pass

Read costs are the steady-state window (604 frames, all consistency windows closed). A table read inside a shared traversal shows 0.000 ms because the traversal is timed once against its anchor table. Checks are successful double reads and mismatches from the first session. Oracle coverage is from `GER_1942_12_25_20.hoi4`.

| Table | Checks/mismatches | p50 ms | p95 ms | max ms | Rows | Bytes/h | Rows seen | Oracle |
|---|---|---|---|---|---|---|---|---|
| Clock | n/a | 0.004 | 0.005 | 0.006 | 1 | 24.2 | nonempty | clock/GameSpeed, boundary counters |
| Campaign | n/a | 0.000 | 0.000 | 0.000 | 1 | 5.1 | nonempty | campaign identity |
| Countries | n/a | 0.274 | 0.408 | 0.600 | 439 | 87.7 | nonempty | 439 countries, 2194 scalars |
| CountryLinks | 200/0 | 0.017 | 0.026 | 0.050 | 442 | 8.7 | nonempty | resolved through country links |
| Provinces | 200/0 | 0.710 | 1.171 | 1.485 | 13413 | 382.7 | nonempty | 13413 provinces + victory points |
| States | 200/0 | 0.011 | 0.023 | 0.047 | 1081 | 21.1 | nonempty | 1081 states |
| Divisions | 200/0 | 0.771 | 1.156 | 1.781 | 2843 | 6258.4 | nonempty | 2842 divisions + orders_group |
| WarRelations | 200/0 | 1.400 | 2.155 | 2.568 | 535 | 64.5 | nonempty | 535 war relations |
| Factions | 200/0 | 0.002 | 0.003 | 0.006 | 5 | 5.1 | nonempty | 5 factions |
| FactionMembers | 200/0 | 0.052 | 0.074 | 0.101 | 57 | 6.2 | nonempty | 57 faction members |
| Players | 200/0 | 0.001 | 0.002 | 0.030 | 1 | 5.0 | nonempty | 1 uniquely matched player |
| HumanControllers | 200/0 | 0.019 | 0.027 | 0.033 | 439 | 6.5 | nonempty | player controller census |
| LandCombats | 200/0 | 0.015 | 0.024 | 0.044 | 34 | 314.6 | nonempty | 34 land combats |
| Fleets | 200/0 | 0.069 | 0.099 | 0.131 | 142 | 11.3 | nonempty | 142 fleets |
| TaskForces | 200/0 | 0.066 | 0.100 | 0.145 | 237 | 134.7 | nonempty | 237 task forces |
| Ships | 200/0 | 0.378 | 0.587 | 0.712 | 2036 | 994.4 | nonempty | 2036 ships |
| NavalCombats | 200/0 | 0.003 | 0.004 | 0.054 | 7 | 9.2 | nonempty | 3 naval combats |
| NavalCombatParticipants | 200/0 | 0.009 | 0.015 | 0.021 | 75 | 19.7 | nonempty | 39 participants |
| AirWings | 200/0 | 0.260 | 0.350 | 0.428 | 581 | 89.5 | nonempty | 581 air wings |
| ArmyOrders | 200/0 | 0.464 | 0.668 | 0.902 | 919 | 157.0 | nonempty | 901 army orders |
| Focus | 200/0 | 0.665 | 0.714 | 0.960 | 439 | 60.9 | nonempty | 439 compared |
| CompletedFocuses | 200/0 | 0.000 | 0.000 | 0.000 | 3929 | 189.0 | nonempty | 3929 compared |
| ResearchTechnologies | 200/0 | 7.595 | 8.054 | 10.434 | 242328 | 10535.4 | nonempty | 9758 compared |
| ResearchSlots | 200/0 | 0.262 | 0.285 | 0.325 | 1133 | 67.9 | nonempty | 1133 compared |
| TechnologyDefinitions | 200/0 | 0.000 | 0.000 | 0.000 | 552 | 45.6 | nonempty | 0 compared |
| Politics | 200/0 | 0.799 | 0.864 | 1.050 | 439 | 18.8 | nonempty | 439 compared |
| PoliticalParties | 200/0 | 0.000 | 0.000 | 0.000 | 1756 | 99.0 | nonempty | 1756 compared |
| ActiveIdeas | 200/0 | 0.000 | 0.000 | 0.000 | 2718 | 125.1 | nonempty | 2716 compared |
| DecisionCounts | 200/0 | 0.808 | 0.855 | 1.051 | 439 | 23.9 | nonempty | 439 compared |
| Decisions | 200/0 | 0.000 | 0.000 | 0.000 | 3328 | 319.7 | nonempty | 3320 compared |
| DynamicModifiers | 200/0 | 0.109 | 0.118 | 0.138 | 150 | 43.0 | nonempty | 150 compared |
| PreferredTactics | 200/0 | 0.105 | 0.121 | 0.143 | 439 | 14.0 | nonempty | 439 compared |
| PreferredTacticOperands | 200/0 | 0.000 | 0.000 | 0.000 | 60 | 8.5 | nonempty | 0 compared |
| TechSharingGroups | 200/0 | 0.006 | 0.007 | 0.010 | 7 | 5.2 | nonempty | 7 compared |
| TechSharingMembers | 200/0 | 0.000 | 0.000 | 0.000 | 28 | 5.7 | nonempty | 28 compared |
| PowerBalances | 200/0 | 0.027 | 0.031 | 0.041 | 7 | 7.1 | nonempty | 7 compared |
| PowerBalanceMembers | 200/0 | 0.000 | 0.000 | 0.000 | 5 | 5.3 | nonempty | 5 compared |
| PowerBalanceRanges | 200/0 | 0.000 | 0.000 | 0.000 | 50 | 7.8 | nonempty | 0 compared |
| PowerBalanceOverrides | 200/0 | 0.000 | 0.000 | 0.000 | 7 | 5.8 | nonempty | 7 compared |
| ProductionLines | 200/0 | 0.292 | 0.326 | 0.406 | 768 | 547.7 | nonempty | 763 compared |
| Stockpiles | 200/0 | 0.148 | 0.157 | 0.301 | 1635 | 213.7 | nonempty | 1340 compared |
| DivisionEquipment | 200/0 | 0.653 | 0.714 | 0.887 | 17364 | 1629.6 | nonempty | 14959 compared |
| VariantDefinitions | 200/0 | 2.105 | 2.260 | 3.471 | 6354 | 700.7 | nonempty | 6354 compared |
| VariantModules | 200/0 | 0.000 | 0.000 | 0.000 | 10975 | 662.5 | nonempty | 10975 compared |
| VariantStats | 200/0 | 0.000 | 0.000 | 0.000 | 6354 | 896.2 | nonempty | 0 compared |
| VariantUpgrades | 200/0 | 0.000 | 0.000 | 0.000 | 7480 | 419.5 | nonempty | 7480 compared |
| ArmyUpgradeGroups | 200/0 | 0.839 | 0.911 | 1.100 | 2843 | 68.3 | nonempty | 2842 compared |
| ArmyUpgradeRequests | 200/0 | 0.000 | 0.000 | 0.000 | 3145 | 385.9 | nonempty | 3144 compared |
| ArmyUpgradeDeliveries | 200/0 | 0.000 | 0.000 | 0.000 | 685 | 1741.1 | nonempty | 428 compared |
| ArmyUpgradeProduced | 200/0 | 0.000 | 0.000 | 0.000 | 685 | 401.5 | nonempty | 428 compared |
| CountryFuel | 200/0 | 0.088 | 0.098 | 0.128 | 439 | 171.4 | nonempty | 439 compared |
| ResourceDefinitions | 200/0 | 0.002 | 0.003 | 0.006 | 7 | 5.2 | nonempty | 0 compared |
| CountryResources | 200/0 | 0.353 | 0.389 | 0.740 | 3073 | 419.8 | nonempty | 0 compared |
| ResourceExchanges | 200/0 | 0.229 | 0.248 | 0.349 | 330 | 91.3 | nonempty | 329 compared |
| PurchaseContracts | 200/0 | 0.027 | 0.031 | 0.058 | 42 | 33.8 | nonempty | 40 compared |
| ProvinceSupply | 200/0 | 0.069 | 0.093 | 0.125 | 13413 | 897.7 | nonempty | 0 compared |
| HubMotorization | 200/0 | 0.110 | 0.120 | 0.253 | 353 | 28.1 | nonempty | 341 compared |
| DisruptedSupply | 200/0 | 0.043 | 0.046 | 0.074 | 2 | 6.5 | nonempty | 0 compared |
| ForeignHomebases | 200/0 | 0.054 | 0.060 | 0.111 | 12 | 88.6 | nonempty | 10 compared |
| CountryExperience | 200/0 | 0.058 | 0.064 | 0.113 | 439 | 249.4 | nonempty | 439 compared |
| StateBuildings | 200/0 | 1.612 | 1.778 | 2.932 | 2941 | 183.3 | nonempty | 2941 compared |
| ProvinceBuildings | 200/0 | 0.000 | 0.000 | 0.000 | 5220 | 300.9 | nonempty | 5220 compared |
| StateBaseResources | 200/0 | 0.197 | 0.212 | 0.281 | 718 | 23.8 | nonempty | 718 compared |
| BuildingConstruction | 200/0 | 0.187 | 0.215 | 0.284 | 249 | 120.0 | nonempty | 247 compared |
| RailwayConstruction | 200/0 | 0.000 | 0.000 | 0.000 | 24 | 15.0 | nonempty | 21 compared |
| OccupationSettings | 200/0 | 0.334 | 0.360 | 0.526 | 439 | 31.8 | nonempty | 439 compared |
| StateOccupationLaws | 200/0 | 0.000 | 0.000 | 0.000 | 312 | 22.5 | nonempty | 312 compared |
| StateGarrisons | 200/0 | 0.000 | 0.000 | 0.000 | 292 | 29.6 | nonempty | 292 compared |
| StateAssignedManpower | 200/0 | 0.000 | 0.000 | 0.000 | 291 | 26.5 | nonempty | 291 compared |
| CountryOccupationReserveManpower | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| StateResistance | 200/0 | 0.146 | 0.157 | 0.245 | 1081 | 118.1 | nonempty | 1081 compared |
| CountryCollaboration | 200/0 | 0.075 | 0.081 | 0.116 | 0 | 5.0 | empty all 1297 h | 0 compared |
| Characters | 200/0 | 11.576 | 12.920 | 15.400 | 13127 | 1130.1 | nonempty | 13127 compared |
| CountryLeaderRoles | 200/0 | 0.000 | 0.000 | 0.000 | 2019 | 34.7 | nonempty | 2019 compared |
| AdvisorRoles | 200/0 | 0.000 | 0.000 | 0.000 | 9547 | 1169.8 | nonempty | 9547 compared |
| UnitLeaders | 200/0 | 0.000 | 0.000 | 0.000 | 1285 | 1026.4 | nonempty | 1285 compared |
| UnitLeaderTraits | 200/0 | 0.000 | 0.000 | 0.000 | 3041 | 164.8 | nonempty | 3041 compared |
| UnitLeaderTraitExperience | 200/0 | 0.000 | 0.000 | 0.000 | 19744 | 2843.4 | nonempty | 19742 compared |
| ActiveAbilities | 200/0 | 0.053 | 0.059 | 0.077 | 0 | 5.0 | empty all 1297 h | 0 compared |
| Capitulation | 200/0 | 0.115 | 0.137 | 0.180 | 439 | 24.4 | nonempty | 439 compared |
| Subjects | 200/0 | 0.247 | 0.286 | 0.445 | 29 | 7.2 | nonempty | 29 compared |
| Autonomy | 200/0 | 0.056 | 0.062 | 0.117 | 29 | 16.0 | nonempty | 0 compared |
| IntelligenceAgencies | 200/0 | 0.088 | 0.096 | 0.178 | 439 | 27.0 | nonempty | 439 compared |
| OperativeMemberships | 200/0 | 0.115 | 0.123 | 0.180 | 138 | 57.5 | nonempty | 0 compared |
| Cryptology | 200/0 | 0.076 | 0.081 | 0.168 | 30 | 9.6 | nonempty | 0 compared |
| IntelNetworks | 200/0 | 0.145 | 0.151 | 0.207 | 409 | 14.8 | nonempty | 0 compared |
| ScriptFlags | 200/0 | 0.409 | 0.437 | 0.597 | 1266 | 31.4 | nonempty | 728 compared |
| ScriptVariables | 200/0 | 0.703 | 0.764 | 1.006 | 1521 | 1129.9 | nonempty | 1521 compared |
| Operations | 200/0 | 0.072 | 0.089 | 0.124 | 6 | 6.3 | nonempty | 0 compared |
| OperationAssignments | 200/0 | 0.000 | 0.000 | 0.000 | 11 | 5.3 | nonempty | 0 compared |
| WingEquipment | 200/0 | 0.159 | 0.203 | 0.275 | 1377 | 83.6 | nonempty | 1377 compared |
| AirBases | 200/0 | 0.111 | 0.131 | 0.200 | 396 | 25.2 | nonempty | 396 compared |
| WingCumulative | 200/0 | 0.021 | 0.027 | 0.037 | 581 | 98.4 | nonempty | 581 compared |
| AirRegionHistory | 200/0 | 0.536 | 0.593 | 0.894 | 43 | 4489.8 | nonempty | 1341 compared |
| AirRegionDamage | 200/0 | 0.000 | 0.000 | 0.000 | 34 | 674.6 | nonempty | 210 compared |
| AirAces | 200/0 | 0.070 | 0.075 | 0.136 | 57 | 11.8 | nonempty | 57 compared |
| AirStatistics | 200/0 | 0.561 | 0.617 | 0.909 | 330 | 128.3 | nonempty | 330 compared |
| SunkConvoys | 200/0 | 0.008 | 0.009 | 0.019 | 228 | 11.1 | nonempty | 228 compared |
| RailCooldowns | 200/0 | 0.730 | 0.808 | 1.102 | 3401 | 33.5 | nonempty | 3401 compared |
| RailEdges | 200/0 | 0.000 | 0.000 | 0.000 | 7248 | 94.1 | nonempty | 7248 compared |
| LandTheatres | 200/0 | 0.237 | 0.267 | 0.329 | 93 | 7.6 | nonempty | 93 compared |
| LandTheatreGroups | 200/0 | 0.000 | 0.000 | 0.000 | 139 | 14.5 | nonempty | 138 compared |
| LandTheatreMembers | 200/0 | 0.042 | 0.062 | 0.123 | 728 | 40.8 | nonempty | 715 compared |
| AirTheatres | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| AirTheatreGroups | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| AirTheatreMembers | 200/0 | 0.014 | 0.017 | 0.021 | 0 | 5.0 | empty all 1297 h | 0 compared |
| NavalResults | 200/0 | 0.001 | 0.001 | 0.002 | 0 | 5.0 | empty all 1297 h | 0 compared |
| NavalResultMembers | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| NavalResultCountries | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| SunkShips | 200/0 | 0.090 | 0.096 | 0.158 | 1 | 71.9 | nonempty | 371 compared |
| ProjectPrograms | 200/0 | 0.106 | 0.123 | 0.157 | 21 | 6.5 | nonempty | 21 compared |
| Projects | 200/0 | 1.706 | 1.871 | 2.679 | 21511 | 2005.6 | nonempty | 21511 compared |
| ProjectHistory | 200/0 | 0.000 | 0.000 | 0.000 | 278 | 26.8 | nonempty | 278 compared |
| ProjectBreakthroughs | 200/0 | 0.340 | 0.368 | 0.496 | 1756 | 57.1 | nonempty | 1756 compared |
| Nukes | 200/0 | 0.104 | 0.113 | 0.174 | 878 | 26.8 | nonempty | 878 compared |
| RaidCountries | 200/0 | 0.072 | 0.079 | 0.141 | 440 | 14.5 | nonempty | 440 compared |
| Raids | 200/0 | 0.000 | 0.000 | 0.000 | 93 | 56.3 | nonempty | 93 compared |
| PeaceConferences | 200/0 | 0.000 | 0.001 | 0.004 | 0 | 5.0 | empty all 1297 h | 0 compared |
| WeatherLimits | 200/0 | 0.001 | 0.001 | 0.001 | 1 | 5.0 | nonempty | 0 compared |
| WeatherRegions | 200/0 | 0.007 | 0.008 | 0.019 | 305 | 32.7 | nonempty | 305 compared |
| WeatherProvinces | 200/0 | 0.210 | 0.223 | 0.519 | 13414 | 6743.4 | nonempty | 13414 compared |
| OperationTokens | 200/0 | 0.067 | 0.072 | 0.100 | 9 | 5.3 | nonempty | 9 compared |
| LexicalTokens | 200/0 | 4.357 | 4.665 | 6.241 | 70851 | 3168.9 | nonempty | no save comparison |
| NavalSupremacy | 200/0 | 0.056 | 0.062 | 0.124 | 49 | 27.3 | nonempty | 46 compared |
| RadarCoverage | 200/0 | 0.086 | 0.093 | 0.125 | 18 | 8.9 | nonempty | 0 compared |
| VolunteerHosts | 200/0 | 0.055 | 0.061 | 0.087 | 0 | 5.0 | empty all 1297 h | 0 compared |
| TheatreAreas | 200/0 | 0.064 | 0.074 | 0.098 | 0 | 5.0 | empty all 1297 h | 0 compared |
| StationedUnits | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| UnitIntel | 200/0 | 0.386 | 0.476 | 0.729 | 24 | 9.2 | nonempty | 20 compared |
| AirBaseRegistrations | 200/0 | 2.148 | 2.308 | 3.259 | 5148 | 115.4 | nonempty | 0 compared |
| ProductionLicenses | 200/0 | 0.108 | 0.116 | 0.160 | 8 | 5.1 | nonempty | 8 compared |
| DeploymentConveyors | 200/0 | 0.345 | 0.371 | 0.504 | 89 | 15.0 | nonempty | 89 compared |
| DeploymentLines | 200/0 | 0.000 | 0.000 | 0.000 | 293 | 145.2 | nonempty | partially compared inside Deployment group |
| DeploymentManpower | 200/0 | 0.000 | 0.000 | 0.000 | 586 | 26.4 | nonempty | partially compared inside Deployment group |
| DeploymentEquipment | 200/0 | 0.000 | 0.000 | 0.000 | 2848 | 218.0 | nonempty | partially compared inside Deployment group |
| WorldTension | 200/0 | 0.022 | 0.025 | 0.055 | 1 | 5.0 | nonempty | 1 compared |
| ThreatSources | 200/0 | 0.000 | 0.000 | 0.000 | 127 | 25.3 | nonempty | 127 compared |
| ThreatCountryAggregates | 200/0 | 0.000 | 0.000 | 0.000 | 440 | 11.6 | nonempty | no save comparison |
| ProductionLicenseRecords | 200/0 | 0.000 | 0.000 | 0.000 | 0 | 5.0 | empty all 1297 h | 0 compared |
| OrdersGroups | 200/0 | 0.279 | 0.429 | 0.565 | 728 | 128.1 | nonempty | 715 compared |
| OrdersGroupMembers | 200/0 | 0.000 | 0.000 | 0.000 | 2804 | 123.0 | nonempty | 2801 compared |
| ArmyGroupMembers | 200/0 | 0.000 | 0.000 | 0.000 | 368 | 26.8 | nonempty | 363 compared |
| IncomingDiplomaticActions | 200/0 | 0.070 | 0.075 | 0.141 | 16 | 7.5 | nonempty | 9 compared |
| Scientists | 200/0 | 0.129 | 0.142 | 0.212 | 145 | 9.1 | nonempty | 145 compared |
| ScientistSkills | 200/0 | 0.000 | 0.000 | 0.000 | 580 | 31.4 | nonempty | 580 compared |
| RelationOpinions | 200/0 | 7.697 | 8.369 | 11.650 | 8639 | 86.7 | nonempty | 8639 compared |
| RelationTuples | 200/0 | 0.000 | 0.000 | 0.000 | 1512 | 70.7 | nonempty | 1512 compared |
| CountryIntel | 200/0 | 0.303 | 0.332 | 0.544 | 46640 | 2847.0 | nonempty | 46640 compared |

### 7.16 Milestone 2.1 takeover checkpoint, 2026-09-13 (not accepted)

1. Static gates: workspace build, 1,393 tests, strict workspace/all-target Clippy, formatting and diff checks pass. This is a checkpoint, not a Phase 2.1 completion claim. Format 14, 148 tables and 870 tracked files remain; no new engine-struct files, polling changes, viewer changes or additional workers. Tests and captures are local; the full Smoke action remains a Phase 2.4 task.
2. Repairs: raw payloads are returned to bounded reuse pools; capacity growth and worker startup stay off the callback; publication no longer races frame writes. Corrected growth arithmetic, canonical-address checks, graph authentication and dependencies (including buildings count versus capacity, order successors, wing/theatre sources, disrupted-supply marker and country-link aliases). Deleted the faulty IntelRest raw mirror and restored its typed hourly path. Raw double-read failures now survive pack resets and cannot be published as complete tables. The independent war matrix window stops at 200 without unsigned underflow. Relations no longer performs a second, uncharged typed walk after raw staging.
3. Comparator corrections: operative variant presence follows its native variant discriminator; subnetwork strength uses +0x20 and network joins include the owner; actual country diplomacy Autonomy state is parsed and compared exactly. The earlier “absent top-level autonomy root” census was not sufficient and is superseded. Oracle refresh gates now recognize actually measured forced refresh groups, not just hour modulo four. Failed saves remain failed; matching allocation counters do not prove that no nonallocating post-tick mutation occurred.
4. Fresh saves: `verdicts_oracle_16.txt` at 60827751 has 127,462 exact comparisons and `verdicts_oracle_17.txt` at 60827752 has 140,839; both have zero differences, zero availability issues and zero failed/skipped comparison groups. The adjacent 60827749/60827750 saves fail with 18/4 differences (army orders and one HubMotorization difference); these are carried, not waived. Earlier `shared_oracle_16.txt` also passes (126,375 comparisons); other earlier failures and their artifacts remain. All normal consistency windows reached 200 successful checks; five tables each recorded one earlier mismatch in the latest session (AirTheatreMembers, LandTheatreMembers, OrdersGroups, OrdersGroupMembers, ArmyGroupMembers); the independent war matrix reached 200 with zero failures.
5. Paired speed-5 measurement: same isolated `GER_1943_08_23_19.hoi4`, same camera, vanilla content, autosaves disabled, no builds/heavy analysis in the timed intervals. Free run: 3,283 hours / 119.8731 s = 27.3873 hours/s, no collector loaded. Recording: consistency windows warmed before reloading the same save, then 1,420 hours / 119.9090 s = 11.8423 hours/s (43.24% of free run). No missing clock samples or dropped recorded hours. Branch 2 hours 60826320–60827740: 1,421 frames, 36.1178 ms mean, 35.2804 ms p50, 43.3393 ms p95, 71.9518 ms max. Typed-frame pool peak 814,176,672 bytes; this excludes raw payloads and is not a total-memory bound. Existing callback metric stops just before enqueue; tail accounting remains to be completed. A short diagnostic tail shows about 50.6 ms of summed decode work per job, so decode/index cost also matters.
6. Method: terminal-driven native game controls, screenshots and the existing smoke clock reader; no computer-use plugin, game-memory writes or altered game saves. TeamViewer remained running and untouched; the existing foreground handoff restores it after input. Captures and scratch remain outside the repository. The frozen collector is `builds\verdicts\collector.dll` (SHA256 `52acb18cd5d99ab20ffeb758bf85b04318436eea93ec4ebaA22cb1bc2b647955`, case-insensitive); the forced-refresh oracle recorder SHA256 is `c2925ce148abcd914bceb9c3332a7aa0f5bf4a60a57757c7ac7069c612a869fe`.
7. Artifacts: `captures\m21_takeover_20260913\` — `paired_baseline.jsonl`, `paired_capture.jsonl`, `paired_metrics.txt`, `paired_status_tail_start.json`, `paired_status_end.json`, `verdicts\` replay/spool, `verdicts_oracle_14.txt` through `verdicts_oracle_17.txt`, static gate logs, prior candidate captures and frozen builds. Performance remains an open failure; additional stagger or workers have not been selected.

### 7.17 Milestone 2.1 copy/decode/spool checkpoint, 2026-09-13 (not accepted)

1. Workspace build, 1,406 tests (collector 464, engine 504, installed-content 137, recorder 176, replay 110, viewer 15), strict all-target Clippy and formatting pass. Format 14, 148 tables, 870 tracked files and worker/cadence/UI behavior are unchanged. All artifacts below remain under `captures\m21_takeover_20260913\`; no owner-run tests are pending.
2. Raw indexes retain allocation and exact structural metadata across payload reuse, never game bytes. Repeated reads preserve independent ordered cursors; Characters now uses the shared implementation and gathered header reads. Enqueue/post-publication bookkeeping time is added through a separate atomic tail witness without touching a published frame. Spooling carries an eligible staggered table using its validated baseline without cloning/reconverting rows; branch starts, keyframes, restart and incomplete baselines still materialize explicitly. Air statistics/history pointer arrays use guarded, prefix-preserving 64-element chunks. Raw arena offsets refuse unrepresentable lengths instead of wrapping u32.
3. Paired speed-5 runs reuse 7.16's free-run baseline and the same reloaded save/camera/content after consistency warmup, with no builds or heavy analysis in the timed windows. `index_fast_pair.jsonl`: 42.95% throughput, 41.86 ms p95. `carry_pair.jsonl`: 43.50%, 41.28 ms p95; measured spool CPU 16.30 ms/hour (conversion 11.92, encoder 4.08), improved without a material throughput gain. A process-local thin-LTO/one-codegen-unit experiment reached 44.39% / 41.04 ms; those build-profile settings are not persisted and run variation is not excluded.
4. Latest measured production candidate `array_pair.jsonl`: 1,442 hours / 119.8359205 s = 12.0331199 hours/s, 43.936858% of 27.3873016 free-run hours/s; 888 clock samples, none missing. `array_metrics.txt`: branch 2, hours 60826320–60827762 inclusive, 1,443 frames, no gaps/drops, 35.0457 ms mean / 33.8505 p50 / 39.9353 p95 / 74.3157 max. Typed pool peak 542,915,520 bytes excludes raw/index payloads. AirRegionHistory and AirStatistics mean copy costs fell from 1.1633/1.2839 ms in `carry` to 0.7870/0.9546 ms; both performance goals remain failed.
5. `array_oracle_15.txt`, `_17.txt`, `_18.txt` pass with 118,937 / 140,573 / 118,912 exact comparisons and zero differences, availability issues or failed/skipped groups. `_16.txt` fails: combat type 62/id 22115 has an extra recorded attacker (156203665592937) despite all thirteen counters matching. This is carried, not waived. Paused hub reads immediately before/after save 15 are byte-identical (`array_hubs_before_save.txt`, `array_hubs_after_save.txt`); that observation does not explain other failed saves. All `array` consistency windows and the independent war matrix reached 200 with zero failures.
6. The final `checkpoint2` build adds only the u32 arena guard to measured production code. Collector SHA256 `74a35337ba4b933aa19f9ebfd26091997e43e1f5f092e3b3d89118102815f0c9`, recorder `cd52cab78f9e31d31060900633d9a7c24668b6b10d2c7cf7ea973b1ad166d9f1`; frozen under `builds\checkpoint2`. Session `134337517343784985-1789285809457181600` finalized 5,159 hours with zero drops/write errors; every consistency window reached 200/0 and the independent war matrix 200/0. Fresh May 26 1944 saves 14–17 (`checkpoint2_oracle_14.txt` through `_17.txt`, hours 60832933–60832936) have 1/24/3/1 differences and zero availability/group failures, not passes. Middle two have advanced random/allocation counters proving post-sample work. Outer two have HubMotorization differences with equal counters. Paused native read `checkpoint2_hubs_after_save17.txt` agrees with the save's absent FIN setting, but does not by itself locate the change relative to capture. All previous failing candidate oracles are retained.
7. TeamViewer remains running and untouched. Native terminal-driven game input, fresh text saves, screenshots and read-only probes used the isolated vanilla profile. Static logs are `build_checkpoint2.txt`, `tests_checkpoint2.txt`, `clippy_checkpoint2b.txt`, `build_checkpoint2_release.txt`; the initially failing Clippy test initializer was fixed before the passing run. This remains an implementation checkpoint, not acceptance of exactness or performance.
8. Rejected follow-up: single-pass native record-source validation replaced the Rust preflight, passed 1,407 workspace tests and 200/0 live windows, but did not improve the total. Repeated detached baseline (`singlepass_baseline.jsonl`) was 3,364 hours / 119.9072405 s = 28.0550197 hours/s. `singlepass_pair.jsonl` was 1,450 / 119.9579956 s = 12.0875644 hours/s (43.09%); `singlepass_metrics.txt` has 1,451 frames, no gaps/drops, 35.3825 ms mean / 40.4151 p95. Relations rose from 2.2484 to 2.5374 ms all-hour mean despite cheaper air copies. The production change was reverted; added canonical/fault-order tests remain. Frozen experiment hashes: collector `8c2dfff1607f6fb7073a406ff7d430aa2faae94cefbda00402a4f67f926cdf42`, recorder `03c5f547c3b198c3191d6d0cbe750445087af5bb4d3d7b80ce618837a0324c65`. The finalized session has 2,455 hours, zero drops/write errors, and fresh October 23 23/24, October 24 01/02 saves have 5/14/72/5 differences with no availability/group failures (`singlepass_oracle_*.txt`). They remain failures. The experiment patch and all logs remain outside the repo.
9. Diagnostic wording correction: equal allocation counters no longer claim that the save was written at the sampled instant or that no engine allocation occurred. The oracle now explicitly allows same-hour writes and untracked allocations; it does not waive or alter any field comparison. This is a user-visible CLI diagnostic correction only, not a polling, recording or playback change. Character mirror comments now accurately describe ordered repeated observations and the stable-double-read limitation. Retained-code checks include `tests_singlepass_reverted.txt`, `tests_boundary_wording.txt` and `clippy_after_singlepass.txt`.
