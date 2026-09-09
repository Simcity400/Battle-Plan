# Battle Plan rewrite plan

Owner decisions taken 2026-09-09, amended the same day after two review rounds. The owner subsequently authorized parallel agents in this conversation: "you dont have to work alone". Work stays on one branch with coordinated tasks. Nothing is backward compatible. This document is the plan of record until the owner changes it.

## 1. Where we are

- The repo is 30,335 tracked files and 4.3 GB. 4.1 GB is captured evidence under `extractor/native/runs`. There are 2.26 million lines of Rust in about 100 crates, 18,500 test functions, 88,000 lines of Python, and 4 MB of PowerShell validation scripts. 15,100 commits landed in eleven weeks from multi-agent lanes.
- Both Rust workspaces compile. The installed game (`<local-path> of Iron IV`, Operation Postern v1.19.2.0.a729) matches the build pinned in the collector byte for byte.
- The recorder produces files. A 60-second speed-5 capture on 2026-09-05 recorded 453 game hours into a 47 MB replay, wrote 382 MB of raw stream in that minute, and took 11 minutes to finalize.
- That capture's own clock baseline shows the cost of the old sampling design. It polled engine memory from outside the game thread and retried torn reads. Cost: 395 ms of CPU and 82 ms of wall time per game hour, about 4.4 cores. The game ran at 7.7 hours per second with the collector attached against 16.4 free-running, a 53% reduction in throughput (the file's own `loss_fraction` field reports 45%, computed against a different baseline). The README target is 16 ms per hour.
- The viewer at HEAD refuses that recording: `unknown required feature "portable_content_manifest_v2"`. The capture after it failed on a stream-format mismatch between a DLL from one worktree and a recorder from another. There is no working end-to-end path.
- Nearly all of the code exists to certify, gate, cite, or measure the small part that does the work. The reverse-engineering knowledge is small and concentrated: about 300 layout constants in `compact_contract`, about 650 inline offsets in the collector, and in each lane crate the reading logic is 2 to 5 percent of the lines. The parts worth keeping are those constants, the injection and detour code, and the file-level ideas behind the container.

## 2. Decisions

| Topic | Decision |
|---|---|
| v1 scope | Map replay: political and control map over time, unit counters, battle markers, scrub, pan, zoom, country picker with a plain numbers panel. The full README vision (the game's own UI rebuilt from its .gui files) is a later phase, so nothing in v1 may block it. |
| Approach | Fresh minimal crates in this repo. Port proven pieces by reading the old code. Delete the old tree once Milestone 1 has proved the foundation. |
| Fidelity | Data is exact and verified against saves. Visuals are drawn from the game's own files (provinces.bmp, definition.csv, colors, terrain atlas, .gui, .gfx, .dds, .fnt) with our own compositing. Reference screenshots from the real game are checked side by side. No byte-identical engine reproduction. |
| Precision | Values are recorded in the engine's own representation. Fixed-point integers stay fixed-point integers. Rounding happens only at display. Size comes from field masks and compression, never from discarding precision. |
| Evidence corpus | The tracked tree is copied by manifest to `<local-path>` and tagged in git, then deleted from the tree at the end of Milestone 1. One offsets file with a comment per constant replaces it. |
| Python | Removed from the product and the repo. The replay stores content identity, not content. The viewer parses installed content in Rust. |
| Validation | Local only. `cargo build`, `cargo test` on pure functions, `cargo clippy`, a save oracle that grows with every captured table, and one smoke script. No cloud CI, nextest profiles, PowerShell suites, citation checks, or evidence reverification. |
| Targets | 16 ms per game hour, replay ready within 30 s, campaign under 1 GB with a 50 MB goal. Measured on every capture, reported, not gates. |
| v1 map set | Clock, country identity, province owner and controller, divisions, land battles, wars and factions, human players, core country numbers, ships and naval battles, air wings, army orders and front lines. This is what the v1 viewer draws and therefore what is ported first. It is not a limit on what is recorded; see the next row. |
| Recorded tables | Every state domain with a proven walk in the old tree is ported and recorded once its oracle check, its consistency test, and its cost are measured, in priority order, even before it has a dedicated view. A recording is the scarce artifact; the viewer can catch up. Presentation-only domains and event-hook domains are excluded (section 4, Milestone 1.6). Every recorded table is visible in the viewer's generic inspector. The aggregate per-hour cost is reported after every addition and the owner decides what to defer when it exceeds the goal. |
| Mods | Vanilla with all DLC first. The replay records the content identity with content hashes. The viewer resolves layers by id and verifies the hashes. Mods are tested after the vanilla path works. |
| Perspective | Omniscient. Picking a country chooses whose numbers appear. Fog is a later phase. |
| Branching | Every new game process starts a new branch. Within a process, a branch starts on a proven load signal, or on an hour discontinuity the collector did not itself record as a gap. Parentage is recorded as probable or unknown, never asserted. |
| Sound | Dropped. |
| Two games at once | Unsupported. The recorder refuses a second game process and says so. |
| Viewer stack | winit, wgpu, egui via eframe. Headless rendering goes through the same wgpu path with an offscreen target. |
| Sampling | Inject the DLL. Exactly one detour, on a proven once-per-hour boundary on the game thread. Read inside a measured budget. Everything else happens off the game thread. Restored on detach. The boundary, its place in the engine's update order, its consistency, and its cost are proved by measurement and by reading the engine before anything is built on them. |
| Release | Portable ZIP with recorder exe, viewer exe, collector DLL, README. The recorder is a console window and is passive: it never pauses, saves, or drives the game. |
| History | Keep git history. Tag the pre-rewrite commit `archive/pre-rewrite-2026-09-09` before deleting. |

## 3. Target architecture

### 3.1 Repository layout

```text
Cargo.toml                 one workspace, six crates
rust-toolchain.toml        1.96.0 pinned (unchanged)
.cargo/config.toml         per-worktree target dir, lld, sccache (trimmed)
README.md                  what it is, how to use it, the rules that survive
AGENTS.md                  how to work here (short)
PLAN.md                    this file
crates/
  engine/                  the pinned game build: exe hash, version string, RVAs, struct offsets, sentinels, read helpers, table walkers, PORTING.md
  replay/                  the .battleplan format and the spool record format: tables, frames, deltas, branches, identity, reader and writer
  collector/               cdylib injected into hoi4.exe
  recorder/                console exe: find the game, inject, watch, detach, finalize, inspect, oracle
  installed-content/               Rust parsers for the installed content the viewer needs, content layers, content hashing
  viewer/                  eframe app
tools/
  smoke.ps1                the one end-to-end script; it drives the game for the test, the recorder never does
reference/                 5 to 10 screenshot pairs (game vs viewer) with the save and camera used
```

During Milestone 1 the old `extractor/`, `replay_interface/`, and `portable_replay/` trees remain on disk, excluded from the new workspace, so old code can be read in place. They are deleted in one commit when Milestone 1 passes.

### 3.2 Data flow

```text
hoi4.exe ──(one detour on the hourly boundary)──> collector.dll
                                          │ game thread: read tables into a pooled row buffer, push it onto a bounded queue, return
                                          │ spool thread: diff against the previous rows, encode field-masked deltas, append uncompressed CRC records, flush
                                          ▼
                              <out-dir>/<session>/spool.0000.h4s, spool.0001.h4s, ...
                                          │
recorder.exe (watches the process, detaches on request, finalizes on exit) ──> <out-dir>/<campaign>.battleplan
                                                  │
viewer.exe ──(reads game content by id, verifies hashes, reads replay)──┘
```

The collector writes its own spool. The recorder never carries game data over a pipe. Its jobs are: find the game, verify the build, gather content identity, write a small config next to the DLL, inject, watch, detach cleanly when asked, and turn the spool into a replay when the game exits or the user asks. That removes the shared-memory ring, the control pipe, the duplex session, and the fold worker of the old design.

### 3.3 The engine crate

One file per engine struct. Every constant carries a one-line comment naming the engine function or struct it came from and how it was confirmed. Example shape:

```rust
/// CGameState singleton data pointer. Confirmed 2026-06-30 by RTTI vtable match.
pub const CGAMESTATE_SINGLETON_RVA: usize = 0x3316E90;
/// CGameState +0x468, current hour counter, incremented once per game hour by the hourly update.
pub const CGAMESTATE_CURRENT_HOUR: usize = 0x468;
/// CGameState +0x728, the campaign's unique id string, written when a game is created and preserved by saves.
pub const CGAMESTATE_GAME_UNIQUE_ID: usize = 0x728;
```

Also in this crate:

- `Build` with the exe SHA-256, byte size, version string, and the byte sentinels for the hooked and referenced functions. The collector refuses to run on any other build and says which build it wanted.
- `read::<T>(addr)` and `read_slice` guarded by an SEH shim (port `seh_read.c`, 75 lines) so a bad pointer yields `None` instead of a crash.
- Typed walkers, one per recorded table. Milestone 1 lands clock, campaign identity, countries, and provinces. Milestone 2.1 lands the rest in batches. Constants for every domain are kept from Milestone 1.6 onwards whether or not their walker exists yet. Each walker reports its own outcome: rows, `truncated` with the count it could not fit, or a reason it could not read.
- `PORTING.md`: the index of every old domain, written in Milestone 1.6.

Sources of the constants: `extractor/native/collector/src/attach_log/offsets.rs` (1,756 lines), `extractor/native/compact_contract/src/build_profile.rs` (including the daily update sentinel, the campaign id offsets at `+0x720` and `+0x728`, and the session offset at `+0x490`), the 302 layout constants across `compact_contract/src`, and the inline constants in `attach_log.rs` near the `append_compact_*_data_record` functions. The old division reader at `attach_log.rs` around line 39,800 shows the representation to preserve: strength and organisation are raw 32-bit fixed-point values.

Hourly boundary candidates known from the old tree: the daily update site `C_GAME_STATE_DAILY_UPDATE_OFFSET` with its sentinel, and `HOURLY_UPDATE_RVA` at `0x6EFE10`. The second is used by the old delayed-event lane and is not proven to be a once-per-hour, whole-world boundary. Neither is adopted until Milestone 1.1 proves one, or finds the function that increments `CGameState+0x468` and hooks a point after the hourly work it triggers.

### 3.4 The replay crate

One crate owns both on-disk formats: the spool records the collector writes during play, and the replay file the recorder finalizes. The viewer and the recorder read replays through the same code the collector's tests use.

#### Tables and rows

Tables in the map set: provinces (owner, controller), states (owner, controller), countries (identity and core scalars), divisions, land combats, ships, naval combats, air wings, army orders, wars, factions, players. Every other state domain becomes a table in the same way as it is ported in Milestone 2.1. Rows are plain structs with explicit little-endian serialization and fixed-width fields. Row keys are engine ids. Every numeric field is stored in the engine's own representation: a 32-bit fixed-point organisation stays a 32-bit integer. Each table starts with the minimum field set the map needs; a field is added only with a measured size cost.

#### Frames and deltas

- A frame is one game hour on one branch. It carries, per table, exactly one of: `snapshot` (all rows), `delta` (changed rows, deleted keys, and the baseline hour it applies to), `unchanged` (nothing changed since the baseline hour), `truncated` (a snapshot or delta that stopped at the walker's capacity, with the count of rows not read), or `unavailable` (a reason string, no data).
- A delta row is the key, a bitmask of changed fields, and only the changed fields' bytes. A division whose organisation ticks up is about nine bytes; a resting division at full strength is nothing. One generic mechanism implements it for every table.
- The baseline is tracked per table, not per frame. The baseline of a table is the last hour on this branch, inside this block, at which that table was `snapshot`, `delta`, or `unchanged`. After a table is `unavailable` or `truncated`, its next available frame is a `snapshot` of that table. So a dropped hour or a failed walk never invalidates later hours; the reader is never asked to apply a delta to a baseline it does not have.
- A keyframe is a frame in which every table is either a full `snapshot` or explicitly `unavailable`. It has no dependency on anything outside its block. One at the start of every branch and every segment, then every 720 game hours (30 days). The interval is written in the header.
- The reader refuses to apply a delta whose baseline it lacks and reports the hour as a gap with the reason recorded in the frame. It never guesses.
- The first optimisation if measurement shows the size goal is missed: encode changed numeric fields as the difference from the baseline value in zigzag varint form, and keys as differences from the previous key. About fifty lines, decided by the Milestone 2.1 numbers, not before.

#### Spool records and segments

- Each record is `magic, length, CRC32, payload`. The payload is one uncompressed frame or one control record: session start (process id, process start time, collector hash, identity block), branch start, gap note, detach, clock witness. The spool is not compressed: it is temporary, writing it must never wait on a compressor, and a full campaign spool is on the order of a few hundred megabytes.
- Records are appended by the spool thread and flushed after every write. `FlushFileBuffers` runs at least every five seconds and at detach.
- A spool is a sequence of segment files, `spool.0000.h4s`, `spool.0001.h4s`, and so on. Every segment begins with a session start or branch start record and a keyframe, so each segment decodes on its own.
- On any write error the spool thread abandons the current segment, never appends to it again, opens the next segment number, and starts it with a keyframe. Hours lost in between are written as a gap note. A damaged segment therefore never hides good records that came after it.
- Recovery scans each segment's valid prefix. A record whose length or CRC does not match ends that segment's prefix and is discarded. Segments are ordered by number. The index is rebuilt from the prefixes at finalize, never read from the spool.
- If the game is terminated, whatever the spool thread flushed is recoverable. `DLL_PROCESS_DETACH` is not relied on for anything, because Windows stops other threads before it runs and skips it entirely on forced termination.

#### Sessions and branches

- Campaign identity is the game unique id read from `CGameState+0x728`, confirmed in Milestone 1.1. A replay file belongs to one campaign id. A different campaign id is a different replay file.
- A session is one game process. Every session start is recorded, and every session starts a new branch, whatever its first hour is. That covers a restart into a save at the exact next hour, which hour arithmetic alone cannot see.
- Within a session, a branch starts when the load signal fires. The load signal is found in Milestone 1.1: the candidates are the game state singleton pointer changing, the session field at `+0x490` changing, and the engine's own load path. If no load signal is proven, the plan says so in section 9 and the fallback below is the only in-session detection.
- Within a session, a branch also starts on any hour discontinuity that the collector did not itself record: the hour went backwards, repeated, or jumped forward by more than one without a gap note or dropped-frame counter explaining the missing hours. The collector always knows when it dropped or failed a frame, so an unexplained jump is the game's doing.
- Parentage is evidence-limited. The loaded hour is looked up across the branches already recorded for this campaign. If exactly one branch contains that hour, the parent is recorded as `probable(branch, hour)`. Otherwise, or if that hour was never recorded, the parent is `unknown`. Recording order is not ancestry and is never used as it.
- A branch starts with a keyframe.

#### Replay file

- Header: magic `H4RP`, format version, and the identity block from 3.9.
- Blocks in write order, one branch's blocks contiguous per session. A block is one keyframe followed by the deltas up to the next keyframe, compressed together as a single zstd frame. Compressing across the hours in a block is what makes the small deltas compress at all.
- Trailer: session table, branch table, block index (branch, first hour, last hour, offset, length), and a fixed-size footer pointing at the trailer.
- Appending a session never modifies the existing file in place. The recorder writes `<campaign>.battleplan.tmp` with the old blocks, the new blocks, and a new trailer, flushes it, and renames it over the old file with `MoveFileEx` and `MOVEFILE_REPLACE_EXISTING`. An interrupted append leaves the old file untouched.
- Reading an hour: find its block from the index, decompress that one block, decode the keyframe, apply the deltas up to the hour.

Size expectation, from the row shapes and not yet from measurement: a peacetime hour is under a kilobyte raw; a wartime hour with several thousand divisions, of which one to three thousand move, fight, or regenerate, is roughly 10 to 40 KB raw with plain field masks; a keyframe is about 250 KB raw. A 720-hour wartime block is therefore 7 to 30 MB raw before compression and decompresses in tens of milliseconds, which is still a fast seek. Over a 1936 to 1948 campaign of about 105,000 hours, that is roughly 6 MB of keyframes and 60 to 150 MB of deltas after block compression with plain field masks, and 30 to 60 MB with the numeric difference coding described above. The 1 GB cap is safe on paper. The 50 MB goal most likely needs the difference coding, and Milestone 2.1 measures both before it is added.

### 3.5 The collector

- `DllMain` does nothing except spawn a worker thread on attach.
- The worker reads `collector.json` next to the DLL (out dir, session id, detach event names), verifies the build against `engine::Build`, writes the session start record, opens the first segment, installs the one detour, and starts the spool thread.
- The detour, on the game thread, does reads and nothing else: increment an active-callback counter, take a preallocated row buffer from the pool, read the hour counter, read the campaign id and the load-signal fields, walk the tables into the buffer, read the hour counter again, record the per-table times, push the buffer onto the queue, decrement the counter, return. If the two hour reads differ, every table in the buffer becomes `unavailable` with reason `hour changed during read`. No allocation, no diffing, no encoding, no I/O on this thread.
- A walker that reaches its buffer capacity stops and marks the table `truncated` with the count it could not read. The spool thread then grows that table's capacity in the pool, off the game thread, so the next hour fits and starts with a snapshot.
- The queue is bounded at 256 buffers and non-blocking. On overflow the buffer is returned to the pool, a dropped-frame counter increments, and the next frame that gets through carries a gap note with the dropped hour range. The game thread never blocks on the spool.
- The spool thread diffs each buffer against the previous rows per table, encodes field-masked deltas, appends records, flushes, and returns the buffer to the pool. It has about 60 ms per hour at speed 5 and needs about one. On a write error it abandons the segment as described in 3.4. Disk trouble never reaches the game thread.
- Every frame records the total callback time and the per-table times. The recorder's status line and `inspect` report the total, not only the parts.
- Detach protocol, in this order: the recorder signals the detach event. The worker restores the original bytes at the hook site and verifies them. It then waits until the active-callback counter is zero, so no callback is still executing collector code. It stops the queue, drains it through the spool thread, writes the detach record, flushes, closes the segment, and joins the spool thread. It signals the done event and ends itself with `FreeLibraryAndExitThread`, which is the documented way for a DLL's own thread to unload the DLL without racing its exit. The recorder waits for the done event, then confirms by enumerating the process's modules that the DLL is gone before it finalizes.
- Detour implementation: try the `retour` crate first. If it misbehaves on this build, port the core of `extractor/native/inline_detour/src/lib.rs`. The `minhook_h4` fork is not carried forward.

Reading on the game thread is the design, and Milestone 1.1 is where it earns its place. The budget goal is 16 ms per hour in total, and the Milestone 1 acceptance for the first tables is a 95th percentile under 5 ms. If measurement misses that, the fallback is to copy the raw struct bytes on the game thread and decode on the spool thread, then external reads as a last resort.

### 3.6 The recorder

A console exe. Start it, leave it open, play. It is passive: it never pauses, saves, or sends input to the game. Anything that drives the game for a test lives in the smoke script.

- Polls the process list every two seconds for `hoi4.exe`. Refuses a second instance while one is being recorded and says so.
- Hashes the exe once, compares with `engine::Build`, prints a clear refusal if it differs.
- Builds the content identity (3.9) from the launcher state, computing content hashes on a background thread with a local cache, and cross-checks the layer list against the in-memory DLC and mod roster after injection once Milestone 1.1 confirms that read. A mismatch marks the identity `unverified` with both lists recorded; the viewer then refuses to claim verified content (3.8).
- Writes `collector.json`, injects via `OpenProcess`, `VirtualAllocEx`, `WriteProcessMemory`, `CreateRemoteThread` on `LoadLibraryW`. Port from `extractor/native/recorder_host/src/main.rs`, roughly 200 lines.
- Prints one status line per minute: hours recorded, last hour's total cost and p95 so far, hours per second, spool size, dropped frames, truncated tables.
- On `Ctrl+C` or the `detach` command it runs the detach protocol and finalizes. On game exit it finalizes from whatever the spool holds.
- Finalize: recover each segment's prefix, group frames into 720-hour blocks, compress each block with zstd (blocks in parallel), write the temp file with any existing sessions for that campaign id plus the new one, rename it into place, print the path. For a full-campaign spool this is a single read and compress pass, seconds rather than the old design's eleven minutes.
- `recorder inspect <file>` prints the identity and its verification state, sessions, branches with parentage, hour ranges, gaps with reasons, table sizes, and the per-hour cost summary.
- `recorder oracle <file> <save.hoi4> [--hour n]` is the save comparison from section 5.

### 3.7 The installed-content crate

- Content layers, in load order: the vanilla install, then each enabled DLC in the launcher's order, then each enabled mod in the launcher's order. A DLC descriptor names either a folder (`path`) or a zip (`archive`); 24 of the 25 installed DLCs are folders and one is a zip. A mod descriptor names a folder or a zip too. Both kinds are read through one layer abstraction backed by the `zip` crate for archives. `replace_path` entries in a mod descriptor remove the listed vanilla directories before overlay. Override is by relative path, last layer wins.
- Content hashing under a defined coverage policy: within each layer, every file under `map`, `common`, `history`, `localisation`, `interface`, `gfx/flags`, and `gfx/interface`, plus the descriptor. Music, sound, video, loading screens, the launcher, the crash reporter, and logs are excluded by name. The layer hash is a SHA-256 over the sorted list of (relative path, SHA-256 of the file's bytes). For the vanilla install the covered set is about 400 MB, which hashes in a few seconds; the recorder and the viewer each keep a cache keyed by path, size, and modification time in their own local data folder so it is paid once per install. The coverage list is versioned in the header so a later widening does not invalidate old files silently.
- Clausewitz text parser: a small recursive-descent parser for `key = value`, blocks, lists, and `hsv`/`rgb` colors. One implementation shared by every file type below.
- Map: `provinces.bmp` (24-bit), `definition.csv`, `terrain.bmp`, `adjacencies.csv`, `unitstacks.txt` for counter positions, `history/states/*.txt` for state membership and 1936 owners. `heightmap.bmp` and `rivers.bmp` later.
- Colors: `common/countries/colors.txt` and `common/countries/*.txt`.
- Localisation: `localisation/<language>/*.yml` for country and state names.
- Textures: DDS with BC1, BC3, and uncompressed RGBA. Uploaded as compressed formats when wgpu reports the feature, decoded on the CPU otherwise.
- Later phase: `.gui` and `.gfx` parsers and BMFont `.fnt` for the game-skinned UI.

### 3.8 The viewer

eframe with the wgpu backend.

- First run asks for the game folder and the Paradox user folder, stores them next to the exe in `viewer.json`. A file picker opens a replay.
- Opening a replay resolves every layer in its identity block against the configured folders and verifies each layer hash under the recorded coverage policy. A missing or mismatching layer is named and the file is refused. An identity the recorder marked `unverified` is refused too, with the reason, unless the user passes `--allow-unverified-content`, in which case a banner stays on screen and nothing is described as verified.
- Map: the province ID image becomes an `R32Uint` texture. A one-dimensional lookup texture maps province id to color and is rewritten when the displayed hour changes. The fragment shader samples the id, looks up the color, and draws province, state, and country borders by comparing neighbouring ids. Terrain from `terrain.bmp` and the atlas is blended under the political color at a fixed ratio. Camera is pan and zoom on a flat map with the same aspect as the game's map.
- Overlays: division counters at `unitstacks.txt` positions grouped per province, battle markers, front lines from army orders, ship and air wing counters at sea zone and airbase positions.
- egui: timeline with play, pause, speed, and a scrubber over absolute game hours with a session and branch picker that shows parentage as probable or unknown. Country picker. Panel showing that country's identity and core numbers, division count, wars, and faction. A generic table inspector for every recorded table.
- Gaps, truncated tables, and unavailable tables are shown as such on the timeline and in the panel. Nothing is interpolated.
- Playback state: the decoded rows of the current hour plus the decompressed current block. Playing forward applies the next delta; seeking loads at most one new block.
- Headless: `viewer --dump <replay> --hour <n> --out frame.png` renders through the same wgpu pipeline into an offscreen texture and reads it back. There is no second renderer. The smoke script and the reference pairs use this.

### 3.9 Content identity

The identity block is machine-free. It contains:

- Game: version string, exe SHA-256 and size, and the vanilla layer hash under the coverage policy.
- DLC: ordered list of enabled DLC ids (the folder name, such as `dlc034_no_step_back`), each with its layer hash. The checksum a descriptor declares is recorded but is not treated as verification of the installed bytes.
- Mods: ordered list of enabled mods, each with `remote_file_id` (or the descriptor file name for local mods), display name, declared version, and its layer hash. No absolute paths are stored.
- The coverage policy version.
- Language, as the launcher reports it.
- Recorder version and the collector DLL's SHA-256.
- Verification state: `verified` when the in-memory roster matched the launcher state at injection, `unverified` with both lists when it did not, `unchecked` until the roster read exists.

The viewer resolves DLC by folder under its configured game folder and mods by `remote_file_id` or descriptor name under its configured user folder, then recomputes each layer hash before opening. What the viewer loads is therefore byte-equal to what the recorder saw, within the coverage policy, and the policy itself is printed by `inspect`.

## 4. Milestones and phases

Each phase ends with a check anyone can run. No phase starts until the previous check passes.

### Milestone 1: prove the foundation

Goal: a measured sampling boundary, a proven load signal or a written statement that none exists, a recoverable spool, one branch-aware format, one verified capture of clock, countries, and province control, and one frame rendered through the new viewer. The old tree is deleted only when this milestone passes.

#### 1.0 Archive

1. Tag HEAD as `archive/pre-rewrite-2026-09-09`.
2. Write a manifest from `git ls-files` with each tracked file's size and SHA-256. Copy those files to `<local-path>`. Verify every file in the manifest against the copy by size and hash, and confirm the count is 30,335. Untracked and ignored files are build output and are not archived.
3. Add the new root `Cargo.toml` with `crates/*` as members and the old trees excluded. Add the six empty crates.
4. Check: `cargo build --workspace` passes at the root. The old workspaces still build on their own. The manifest verification prints zero mismatches.

#### 1.1 Sampling proof

1. Identify the function that increments `CGameState+0x468` and the site that runs the hourly update, using the Ghidra project outside the repo and the two candidates from 3.3. Write down, from the decompiled call order, which per-hour updates run before and after each candidate site, and which of the tables in scope are written there. Identify whether any worker thread writes those tables outside the main tick, from the engine's job dispatch sites.
3. Build a spike DLL that installs a detour on each candidate and logs, per call, the QPC timestamp, the hour counter before and after, the thread id, the campaign id string, the game state singleton pointer, and the session field. Inject it with a spike recorder into a running game and run two minutes at speed 5, two at speed 1, a pause and unpause, a reload of an earlier save, a reload of a later save, and a reload to the immediately following hour.
4. Accept a site only if: calls equal hour increments one to one at every speed; every call is on the same thread; the hour counter equals the previous plus one at every call; the campaign id reads stably; and the call-order reading places the site after the province, country, and division updates for the hour.
5. Consistency: inside the accepted callback, read the province table twice and the country table twice and compare. Run 1,000 hours. Accept only zero mismatches. Also read the hour counter before and after the walk and confirm it never changes. State explicitly that this test cannot see a partially updated world that is stable within the callback, which is why step 1's call-order reading is required as well.
6. Load signal: from the three reloads, record which of the logged fields changed and when relative to the next hourly call. Accept a signal only if it fired on all three reloads and never otherwise. If none does, write that down; in-session detection then rests on the discontinuity rule alone and the reload-to-next-hour case within one session is a known blind spot that the new-session rule does not cover.
7. Cost: time the walks of the clock, country, and province tables per call. Report p50, p95, p99, max. Accept p95 under 5 ms. Report game hours per second with and without the spike DLL at speed 5. Accept no worse than 90% of free-run.
8. Check: a short report in section 9 with the numbers, the accepted site, the call-order justification, the load signal outcome, and the overall outcome. If no site is accepted, the fallbacks in 3.5 are chosen here, in writing, before continuing.

#### 1.2 Format and spool

1. Tables for clock, campaign identity, countries, provinces. Frame with per-table `snapshot`, `delta`, `unchanged`, `truncated`, `unavailable`. Field-masked delta rows with engine-native values. Per-table baselines scoped to a block.
2. Spool records with length and CRC, segments, prefix recovery per segment, session start, branch start, gap, detach, and clock witness control records. Replay blocks with zstd, the session and branch tables, the block index, and temp-and-rename append.
3. Session and branch rules, parentage as probable or unknown.
4. Check: round-trip tests and fault tests pass. A synthetic 2,000-hour campaign across three sessions, with reloads to an earlier hour, the same hour, a later hour, and the immediately following hour, reads back with the right sessions, branches, and parentage. After random dropped hours, random unavailable and truncated tables including at keyframes, a write error mid-segment, and mid-record truncation of the last segment, every readable hour decodes identically to the direct read and every unreadable hour is reported as a gap with its reason. An interrupted append leaves the previous file byte-identical.

#### 1.3 Minimal collector and recorder

1. Collector: attach, verify build, session start record, one detour on the accepted site, walks for clock, campaign id, load-signal fields, countries, provinces, bounded queue, spool thread with segments, detach protocol with the callback counter and self-unload, restore.
2. Recorder: discovery, refusal messages, identity gathering with background hashing, injection, status line, detach with module-absence confirmation, finalize with temp-and-rename, inspect.
3. Check: start the recorder, start the game, play ten minutes at speed 5, reload an earlier save, play two more minutes, quit. The replay appears within 30 seconds. `inspect` shows one session, two branches with the second's parent recorded as probable, no gaps, per-hour total p95 under 5 ms, and hours per second within 90% of free-run. Start the game again into the same campaign, play two minutes, quit: the file gains a second session and a third branch, and the previous sessions are byte-identical inside it. Then repeat with `taskkill /f` on the game instead of quitting: the replay contains every hour flushed before the kill. Finally, detach with `Ctrl+C` while the game keeps running: the game continues, the DLL is confirmed gone, and the replay is complete to the detach record.

#### 1.4 Oracle v0

1. `settings.txt` must have `save_as_binary=no`. The recorder's oracle command checks the save header and refuses binary saves with that instruction.
2. Text save parser using the installed-content Clausewitz parser. Compare province owner and controller for every province, and every country's identity and core scalars in their engine representation, against the replay frame at the save's hour.
3. The capture boundary: the smoke script, not the recorder, pauses the game, saves, and then compares the last recorded hour. The save's date and hour must equal that hour, otherwise the oracle refuses to compare.
4. Check: zero differences on provinces. Every country scalar either matches exactly or is listed in the oracle's printed limitations with the reason the save cannot carry it.

#### 1.5 Viewer v0

1. eframe shell, configuration, file picker, identity resolution and hash verification against the configured folders, offscreen headless dump through the same wgpu pipeline.
2. Map renderer with the province id texture and the color lookup, pan and zoom, country borders.
3. Timeline scrubber over hours with session and branch picker.
4. Check: open the 1.3 replay, scrub end to end, pick a branch, dump the last hour headlessly, and place that PNG beside a screenshot of the game at the saved hour as the first reference pair in `reference/`.

#### 1.6 Knowledge harvest

This phase moves the knowledge, not the scaffolding, while the old tree is still on disk.

1. Copy every engine offset, RVA, vtable, stride, and sentinel for every domain into the engine crate, one file per engine struct, each constant with its one-line comment. Constants for tables that have no walker yet are still kept; they cost nothing at runtime.
2. Write `crates/engine/PORTING.md`: one row per domain with its old status, the archive file and line range of its walk, the structs it reads, the representation of each numeric field, its known quirks (partial data, torn-read handling, dependence on another domain, which threads write it), and which of the three classes below it falls in.
3. Classify each domain. State domains, read at the hourly boundary, are portable and are the Milestone 2.1 backlog. Presentation-only domains served the abandoned exact-UI approach and are dropped: resolved tooltip instances, production render submits, hud surface activation, production map content snapshot, character presentation, scripted gui runtime, selected designer identity, national focus presentation, dynamic collection trace, design catalog context, runtime text inputs. Event-hook domains depended on detours the new design does not install: fired event popups, decision and focus completion hooks, delayed events, the tech, character, and rail change gates, land combat detail events. They are excluded; if a later view needs an event, it is derived from hourly state where that is honest and otherwise recorded as unavailable.
4. Check: `cargo build -p engine` passes and `PORTING.md` covers all 92 registry domains.

#### 1.7 Delete the old tree

1. `git rm -r extractor replay_interface portable_replay tools runs .circleci .config pyproject.toml codex_r2.thread_id r3.threadid` and the 177 KB `.gitattributes`. Replace `.gitattributes` with five lines and trim `.gitignore` to the new layout.
2. Write the new `README.md` and `AGENTS.md` per section 6.
3. Check: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` pass. `git ls-files | wc -l` is under 100.

### Milestone 2: the v1 product

#### 2.1 Port the state domains, in batches

Batch 1, the map set: states, divisions, wars and factions, players, land combats, ships, naval combats, air wings, army orders. Batch 2, the panel set: politics, fuel, production, focus, research, decisions, division equipment, air variants and losses, projects, market and trade, supply and logistics, characters and cabinet, construction and building damage, occupation and resistance, intel and operatives, naval missions, hub motorization, dynamic modifiers, tech sharing, convoy and ship history, power balance, nukes, raids, preferred tactics, upgrade requests, peace conferences, theatre selector, division combat stats, weather and snow, victory points, rail state. Batch 2's order follows the panels planned for Milestone 3 and can be reordered by the owner.

For each domain, from its `PORTING.md` row:

1. Write the walker against the harvested constants, add the table to the replay crate, extend the collector. The walk is a read at the hourly boundary; the old code's polling, bracketing, and retry logic is not ported.
2. Repeat the consistency assessment for the table: the call-order reading for the structs it touches, and the double-read test over at least 200 hours with zero mismatches. Proving provinces and countries does not prove a table that a different engine subsystem writes.
3. Extend the oracle to the table, or record the explicit limitation in its printed output.
4. Re-run the 1.3 capture with the new table and record in section 9: the table's own read cost and bytes per hour, and the aggregate for the whole callback: total p50, p95, and max, hours per second against free-run, pool memory, and spool bytes per hour. The per-table guide is 2 ms and 2 KB per hour at speed 5; the aggregate goal is 16 ms at p95 and 90% throughput. When the aggregate is exceeded, the owner decides what to defer or rework before the next table lands.

The work is mechanical for most domains, half a day or less each, and a full day for the large ones (diplomacy and intel, naval combat, characters). Batch 1 is required for v1. Batch 2 proceeds as time allows and does not block Milestones 2.2 to 2.5.

Oracle coverage expected from a text save: divisions (location, template, strength, organisation), ships and fleets, air wings and bases, wars and factions, player countries, army battle plans and orders, politics, research, production lines, stockpiles, characters, buildings, occupation, and most country scalars. Land and naval combats appear in saves in a reduced form; the oracle compares locations and participants only and prints that limitation.

#### 2.2 Gamefiles completeness

Terrain blend, state borders, localisation names, DDS textures, DLC folder and archive layers, mod layers with `replace_path`, layer hash verification. Check: a 1936 start map from state history alone, rendered headlessly, matches the game's 1936 start screen as the second reference pair.

#### 2.3 Viewer overlays, panel, and inspector

Counters, battles, fronts, ships, air. Country picker and panel. Gap, truncated, and unavailable display. A generic table inspector: pick any recorded table and see its rows for the current hour, filtered by the selected country, as a plain egui grid, values shown rounded with the engine representation available on hover. Every table ported in 2.1 is visible here the day it lands, so nothing is recorded that cannot be looked at. Check: open a Milestone 2.1 replay, scrub at 60 fps, pick three countries, confirm the panel numbers against the oracle output for the saved hour, and open every recorded table in the inspector.

#### 2.4 Smoke script and reference pairs

1. `tools/smoke.ps1`: check `save_as_binary`, build release, start the recorder, launch the game into a chosen save, wait for 120 game hours, pause and save through the script's own input to the game, quit, finalize, run the oracle on the last hour, dump that hour headlessly, score it against the matching reference pair, and print hours recorded, per-hour total p50 and p95, hours per second against free-run, finalize time, replay size, oracle differences, and reference scores. The recorder stays passive throughout.
2. Five reference pairs at fixed saves and cameras.
3. Check: `tools/smoke.ps1` passes on this machine.

#### 2.5 Packaging

`tools/package.ps1`: release build, copy the three binaries and README into a ZIP with SHA-256 sums. Check: unzip on this machine into a fresh folder, record a session, view it.

### Milestone 3 and later

Politics, focus, research, and production panels. Naval and air detail. Supply. Weather. Per-country fog. The game-skinned UI from `.gui`, `.gfx`, `.dds`, and `.fnt`. Each is one new table in the replay crate, one walker in the engine crate, one consistency assessment, one oracle extension, and one view in the viewer. None of them changes the file format's framing.

## 5. Validation

The whole of validation is:

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
recorder oracle <replay> <save> --hour <n>    # after every capture change, against a fresh save at the capture boundary
powershell -File tools/smoke.ps1               # needs the game; before merging recorder or viewer changes
```

Rules:

- Unit tests cover pure functions: parsers, format round trips, delta and baseline logic, segment recovery, session and branch rules, walkers on synthetic memory. No test spawns the game, reads a capture corpus, or embeds evidence with `include_str`.
- The oracle is the correctness check for captured values and grows with every table from Milestone 1.4 onwards. It prints its own limitations.
- The smoke script is the only end-to-end check. It runs locally, drives the game itself, and prints a short table.
- A change to the replay format updates writer, reader, collector, recorder, and viewer in the same commit. There are no compatibility shims and no feature flags.
- No test framework beyond `cargo test`. No nextest, no CI service, no PowerShell test suites.

## 6. README and AGENTS rules after the rewrite

README keeps: what the product is, the three-file release, the recording rules (never pauses or controls the game, reads only, an unsupported build records nothing and says why, hourly sampling, a failure costs only the affected hour and table and is named in the file), the truth rules (never fake data, record state not pictures, values kept in the engine's own precision), and the file rules (one file links recorder and viewer, viewer loads the same game content by id and verifies it by hash, refuses if it differs, first run asks for paths, viewer never touches live game memory).

README changes: the exact-engine rendering rule is replaced by the two-layer rule from section 2. "Hooking is a last resort" becomes "exactly one detour, on a proven hourly boundary, restored on detach". Sound is removed. Perspective becomes a later-phase note, with v1 stated as omniscient. Two games at once is stated as unsupported and refused. Branching is kept and its contract summarised: every game process is a new branch, in-session reloads are detected by the load signal or by unexplained hour discontinuities, and parentage is probable or unknown. Targets stay as measured goals with the current numbers printed by every capture.

AGENTS.md becomes:

- Read `PLAN.md` first. Work the current phase. Do not start the next phase before its check passes.
- One branch; parallel agents may handle coordinated tasks. No lanes, no landing branches, no owner rulings in code comments.
- Replace, never shim. When you change the format, update every reader and writer in the same commit.
- Validation is section 5 of `PLAN.md` and nothing else. Do not add test infrastructure.
- Per-worktree `.cargo-target`, shared sccache at `C:\sccache_cache`, never delete it.
- Captures go under `captures\`, never in the repo. Keep scratch outside the repo.
- Only the owner changes `README.md`, `AGENTS.md`, or `PLAN.md` decisions.

## 7. Risks and how the plan handles them

- The old sampling design cost 82 ms of wall time per hour and halved game speed. The new design is only adopted after Milestone 1.1 measures it, with written fallbacks if it fails.
- Neither known hourly candidate is proven to be a whole-world boundary, and a double-read test cannot see a world that is consistently half-updated. Milestone 1.1 pairs the measurement with a reading of the engine's call order and thread writers, and Milestone 2.1 repeats that for every added table.
- In-session reloads may have no clean signal. Every new process is a branch regardless, unexplained discontinuities are branches, and the one blind spot (a reload to the very next hour inside one session, with no load signal) is written down rather than papered over.
- Ancestry cannot be proven from the game. Parentage is recorded as probable or unknown and shown that way.
- The offsets for the older domains (divisions, map control, ships, air wings, army orders, diplomacy) never reached production status in the old tree. Each is proved by the consistency test and the oracle as it lands in Milestone 2.1, and wrong ones are re-derived in Ghidra outside the repo.
- Dozens of small tables can add up past the 16 ms goal. The aggregate is reported after every addition and the owner decides what to defer.
- A game update changes every RVA. The collector refuses on hash mismatch and says which build it wants. Updating is a Milestone 1.1 and 2.1 harvest against the new exe.
- Forced termination skips DLL detach. The spool is written so that whatever was flushed is recoverable and nothing depends on detach running. A write error starts a new segment rather than corrupting the current one.
- Launcher state may not equal what the process loaded. The identity block records both when they differ and the viewer refuses to call such content verified.
- Content hashing costs a few seconds per install on first run and is cached by path, size, and modification time afterwards.
- The user's Documents folder is OneDrive-redirected. The recorder resolves the Paradox user directory from the registry and the known OneDrive path and prints what it chose.
- The owner's saves are binary. The oracle refuses binary saves with the instruction to set `save_as_binary=no`.

## 8. Rough effort

| Phase | Sessions |
|---|---|
| 1.0 Archive | 1 |
| 1.1 Sampling proof | 3 to 4, needs the game running and Ghidra |
| 1.2 Format and spool | 2 to 3 |
| 1.3 Minimal collector and recorder | 3, needs the game running |
| 1.4 Oracle v0 | 1 |
| 1.5 Viewer v0 | 2 |
| 1.6 Knowledge harvest | 2 to 3 |
| 1.7 Delete the old tree | 1 |
| 2.1 Batch 1, the map set | 5 to 7, needs the game running |
| 2.1 Batch 2, the panel set | 10 to 14, needs the game running, interleaved with 2.2 to 2.5 |
| 2.2 Gamefiles | 2 |
| 2.3 Viewer overlays, panel, inspector | 2 to 3 |
| 2.4 Smoke and references | 1 to 2, needs the game running |
| 2.5 Packaging | 1 |

## 9. Measurements

Filled in as phases complete. Every capture prints the same table. Site, call-order justification, and load-signal outcome are recorded here by Milestone 1.1.

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

### Milestone 1.0 completed 2026-09-09

- The existing `archive/pre-rewrite-2026-09-09` tag points to the pre-rewrite commit `901e2ec25fb8a2dd7f12e8d0a541b6c8b197ddcc`.
- Verified all 30,335 tracked files (4,486,036,143 bytes) against the existing copy under `<local-path>`: zero size or SHA-256 mismatches, zero missing files. The exact tagged-tree manifest is `MANIFEST.pre-rewrite-901e2ec25.json`; its SHA-256 is `6e9f1929dc7fd40e84f3c82f57c18b5c8a2c61e18f953716e13bf2ea7037711b`. The result is in `VERIFICATION.pre-rewrite-901e2ec25.json` beside it. The earlier `MANIFEST.json`, which also lists a later `PLAN.md` and therefore has 30,336 entries, was preserved.
- The root workspace has six empty crates and excludes the three old trees. It uses Rust 1.96.0, the worktree's `.cargo-target`, lld, and shared sccache at `C:\sccache_cache`. Root `cargo build --workspace --locked`, `cargo test --workspace --locked` (zero tests in the scaffold), and `cargo clippy --workspace --locked -- -D warnings` pass.
- Both old workspaces also pass `cargo build --workspace --locked`, selected with `--manifest-path extractor/native/Cargo.toml` and `--manifest-path replay_interface/native/Cargo.toml`. Their existing warnings remain. The old trees have not been deleted.

### Milestone 1.1 completed 2026-09-09

Accepted for the pinned build and the current clock, campaign, country and province reads: sample after the full tick at RVA `0x1DCEF0` returns. Use a change in `play_session_started_at` at image RVA `0x3317048` as the measured in-process load signal, with the collision limits below and the discontinuity fallback. The final paired comparison passed at 94.7% of free-run and p95 4.882 ms. No fallback sampling design was needed. Milestone 1.2 may begin; this is not acceptance of any unported table or the production recorder.

- Its return follows hourly, daily, weekly, monthly and yearly updates when due, idler callbacks, deferred occupation-bundle drains, and queued unit deletion. The identified country, state, unit, AI, decision and monthly worker jobs are joined or explicitly waited before return. Province-controller writes run through hourly country/army updates; state-owner writes run through joined daily state updates and drained occupation bundles. Country command/political power run through daily country updates, stability/war support through weekly updates, and the convoy cache's identified writer is joined hourly AI. Background border work writes draw buffers and is waited by its caller. Prior-frame rendering and icon construction can overlap the tick; the inspected normal paths write presentation storage, with no identified unjoined writer to these captured fields. Optional debug/script/GUI virtual entrances are not exhaustively closed. This bounded dispatch assessment is not a claim that every engine worker is stopped, and new tables require their own assessment.
- RVA `0x1D4390` is daily and also runs during initialization. RVA `0x6EFE10` handles delayed events for one country. Neither is a once-per-hour world boundary.
- Pinned engine instructions corrected the old readers: command power, political power, stability, war support and the convoy cache are signed 64-bit fixed-point values. The old `CCountry+0x338` manpower label was wrong; that field is a mobilization ratio, so it is omitted from the probe. The convoy cache is explicitly unavailable when its validity byte is clear. Country aliases are validated and deduplicated through the engine's link array.
- The same audit found that division strength and organisation are 64-bit at primary `CArmy+0x420/+0x428`; the old 32-bit reader is not evidence for the representation. Future walkers must retain all 64 bits. Primary `+0x1F0` is the location province pointer; the old `+0x28` is a supply cache.
- A zero-mismatch double read can still observe a stable partially updated world or an invalid cache. The call-order and field-semantics assessment is required alongside the live comparison.
- The first short live probe (`m11_20260909/probes/40848-1788936789715759100`, under the external capture directory) attached and restored the hook but did not complete self-unload. Its log contains only the session start, so it supplies no hourly measurements. A noninvasive debugger located the worker in the post-restore thread-context check with zero active callbacks. The release assembly passed a Windows x64 `CONTEXT` buffer aligned to eight bytes instead of the required sixteen. An explicitly aligned wrapper and post-resume error reporting fixed the observed failure: the six-hour retry and the 6,312-hour run both restored and verified the hook bytes, drained, self-unloaded, and were confirmed absent by module enumeration.
- The 120-second speed-5 run is `m11_20260909/speed5_probe/77760-1788938709912305400/probe.jsonl`. Hours 60759397 through 60765708 each have one call on thread 45408; every original tick advanced its entry hour by exactly one. Both reads of all 13,413 provinces and 439 canonical countries matched on every call. No walk clock changed, and no tables were unavailable or truncated; no calls were dropped. Total p99 was 3.267 ms. The total includes entry/clock reads, both complete table walks, row comparisons and queue publication; it excludes the original engine routine and the final atomic decrement/epilogue.
- First-pass province latency p50/p95/p99/max was 1.015/1.577/1.847/10.975 ms; first-pass country latency was 0.104/0.174/0.214/0.340 ms. The no-DLL 120-second baseline recorded 6,840 hours in 119.889 seconds (57.053 hours/s); the probe achieved 92.5% of that throughput. Both used the same vanilla campaign from the saved January 1936 start; the launcher requested all installed DLC and no mods. The subsequent in-memory roster check confirmed that membership. These are sampling-proof numbers, not replay size or finalization measurements.
- The campaign UUID and seed in the live reads exactly match `game_unique_id` and `game_unique_seed` in the subsequent text save. The old `+0x490` session label was also corrected: this is persisted unsigned 64-bit play time. A separate signed 64-bit Unix timestamp at image RVA `0x3317048`, written by the engine's play-session-start routine on in-game load completion, is being tested as a passive load signal. Its seconds resolution is retained; it is not presented as a monotonic generation counter.
- The first speed-5 run used DLL SHA-256 `95db8b7ac3af0198035def078ecbaa5ef44756a7553f5f968b944edc21d09668`; the reload/clock-timing run used `54355a05ee5fddc70f626c42a253cbafc76baa09b5671112aa4d6da26d6cbe90`. The latter's 2,068 speed-5 calls took 43.941 seconds (47.040 hours/s), only 82.45% of the earlier free-run baseline. A new 120-second no-DLL baseline from `GER_1936_01_02_17.hoi4` measured 4,952 hours over 119.985 seconds (41.272 hours/s). The completed paired probe is reported below; the changed free-run rate is retained rather than assuming the earlier rate still applies.
- The reload run, `m11_20260909/reload_probe/77760-1788939547060579300/probe.jsonl`, recorded 59 speed-1 hours over a 120.492-second unpaused input interval; the first-to-last callback rate was 0.493 hours/s, total p50/p95/max 2.349/3.324/4.013 ms. A 20.029-second pause produced no new records. All 2,135 calls across that run, including its later speed-5 portion, had complete matching province/country double reads and an unchanged hour during the walks.
- All six in-process loads changed `play_session_started_at` before the next hourly callback: earlier save, later save, setup loads, the immediately following saved hour, and a load whose first callback advanced exactly one hour from the last recorded hour. The last case changed the timestamp from 1788940174 to 1788940221 while hours advanced 60759399 to 60759400. Campaign UUID and game-state pointer stayed unchanged throughout. Saving and then resuming advanced the hour with that timestamp unchanged. The signal can collide if two starts have the same Unix second, including after a system-clock adjustment; the discontinuity fallback remains necessary and this is not an unconditional generation counter.
- The one-time in-callback content roster double read took 0.0309 ms and matched exactly. It found all 24 currently installed DLC descriptors active and zero mods. The earlier installation count of 25 was inaccurate. The manager's `+0x50` vector combines initial descriptor requests with later active display names, so the old disabled-DLC label was wrong; it is now named `dlc_filter_strings`. These diagnostics establish membership, not VFS mount order or content hashes. Content identity remains unchecked in this spike.
- Final paired capture: both no-DLL and probe runs reloaded `GER_1936_01_02_17.hoi4` in process 77760 before their 120-second speed-5 window. DLL SHA-256 `9d18744c2f3ba36ff14600715766f5f5231a34bacc0ff8798f0b4e144786f2a3`, capture `m11_20260909/paired_probe/77760-1788941023316005700/probe.jsonl`. The probe recorded hours 60759401 through 60764093 with zero gaps, mismatches, unavailable/truncated tables or changed walk clocks; the load timestamp stayed stable. Its 4,692 continuous intervals took 120.071 seconds: 39.077 hours/s against 41.272 free-run, or 94.7%. Total p50/p95/p99/max was 3.608/4.882/5.389/9.031 ms. First-pass province p50/p95/p99/max was 1.982/2.581/2.867/7.519 ms; first-pass countries 0.281/0.368/0.429/0.873 ms. Clock entry/pre-walk/post-walk p95 was 0.001/0.003/0.001 ms. All values are wall-clock QPC measurements; the one-time roster read is included in that frame's total. Restored hook bytes and DLL absence were confirmed again. No fallback is needed for this measured pair; the variation and narrow p95 margin remain visible.
- The remaining speed matrix (`m11_20260909/speed_matrix/77760-1788941311073522900`) recorded 38 calls at speed 2, 98 at speed 3 and 183 at speed 4, over roughly 20 seconds at each speed. All 319 were consecutive hours on thread 45408, with zero clock changes, mismatches, drops or incomplete tables, and stable campaign/load fields. Total p50/p95/p99/max was 3.139/4.516/5.221/6.479 ms. Together with the 120-second speed-1 and speed-5 runs, this checks all five speeds.
- Live rejection probes were stopped after their cadence disqualified them. Daily RVA `0x1D4390` produced 27 calls over 15 seconds, all separated by exactly 24 game hours (`daily_candidate/77760-1788941497639713100`). Country-event RVA `0x6EFE10` produced 495 calls over 12 seconds at speed 1: exactly 99 calls with 99 distinct country arguments in each of five hours (`country_candidate/77760-1788941602332754400`). Both restored and unloaded cleanly. Neither received further acceptance testing once falsified; the full speed/reload/consistency/performance matrix applies to the accepted full-tick site.
- Final local checks pass: root workspace build, all 16 pure-function tests, clippy for all targets with warnings denied, and release collector/recorder build. Captures, saves, screenshots and the detailed static reports remain outside the repository. The short static consolidation is `C:\Users\Example\AppData\Local\Temp\hoi4-rewrite-23d29856\sampling-boundary-conclusion.md`; load and roster derivations are `load-signal-static.md` and `content-roster-static.md` under `captures\m11_20260909\`.

### Milestone 1.2 completed 2026-09-09

The new `replay` crate owns the spool and replay formats. Root workspace build, all 66 pure-function tests (50 in `replay`), and clippy for all targets with warnings denied pass. Milestone 1.3 may begin. These are synthetic format checks; production capture, finalization time and replay size remain to be measured there.

- Clock, campaign, countries, country links and provinces use explicit little-endian row fields and one field-mask mechanism. The separate link table preserves raw province country references while resolving them through canonical country slots. Country fixed-point fields retain the proven signed 64-bit representation; a single such field change costs 13 bytes before compression, including its key and mask. No difference coding or quantization was added.
- Baselines advance independently for each table, including unchanged tables. Unavailable and truncated tables clear only their own baseline. The next complete read becomes a snapshot. Truncation at a keyframe is explicitly unavailable with the missing count. Clock/hour mismatches and ambiguous country slots become named table failures. Keyframes follow elapsed game hours, branch starts and segment starts.
- The integrated test generates 2,000 source hours across three sessions and seven branches, including earlier, same-hour, later and next-hour reloads. Independent source rows are compared against indexed replay reads after deterministic dropped hours, unavailable/truncated tables including at keyframes, a partial write in an early segment, and a torn final record. All readable data matches; missing hours retain reasons. The test takes about 9.5 seconds in the local development test build; this is not a capture or seek performance target.
- Spool fault tests cover every byte truncation, invalid CRC/length/payload, failed create/write/flush/sync, and independent later segments. A complete CRC-valid frame recovered after an uncertain flush remains readable even if a loss note covers that hour. Tail damage without a trustworthy hour retains its session when known, segment number, valid-prefix byte count and reason. The spool worker must service the five-second sync timeout while the game is paused, and repeat both session and branch context when restarting a segment.
- Parentage is calculated only from exactly recorded hours. The loaded hour is optional when its first callback was lost; unknown origin cannot supply a probable parent. Recovery preserves a recorded probable assessment even if its supporting branch or hour was subsequently lost, so missing earlier evidence does not hide later data. Consumers must display supporting-data availability separately.
- Blocks are single checksummed zstd frames with bounded decompression and a CRC-protected indexed trailer. Append copies existing compressed blocks byte-for-byte. The actual publication sequence is shared with pure fault tests: interruption at every write-byte position and every create/write/sync/replacement checkpoint leaves the prior file byte-identical.
- Each session retains its collector identity, verification state and QPC frequency. Appending an unverified session cannot inherit a verified claim from the first session's header. The current container holds compressed file bytes and finalization blocks in memory; seeking decompresses only the selected block. Production memory and finalization cost are measured in Milestone 1.3.

### Milestone 1.3 completed 2026-09-09

Production captures passed startup discovery/attach, content verification, recording, recovery/finalization, inspection, live reload, normal exit, restart/append, forced termination and Ctrl+C detach. The first ten-minute throughput comparison failed; the optimized full-duration repeat passed, with the variance and earlier failure retained below. Root build, all 112 pure-function tests and clippy with warnings denied pass. Milestone 1.4 may begin. No save-oracle claim is made yet, and the old trees remain until all of Milestone 1 passes.

- Capture `captures\m13_20260909\short\sessions\134334192318154539-1788945633718865400` recorded 2,017 consecutive hours (60759401 through 60761417) in process 59172, with zero gaps, damaged segments, unavailable/truncated tables, dropped frames or country-link double-read mismatches. The hook was restored and the collector confirmed absent while the game continued. The replay contains the detach record at its final hour.
- The replay is 373,545 bytes from a 1,294,525-byte spool (641.807 bytes per hour). Total callback p50/p95/max is 2.204/2.824/3.926 ms. Province p95 is 2.550 ms, countries 0.274 ms and country links 0.032 ms. Continuous clock witnesses measure 37.211 hours/s; no matching baseline was taken for this short functional run. Initial finalization took 3.083 seconds; detach through the reporting command took 6.516 seconds.
- Removing redundant decoder copies and reusing the finalizer's validated rows and encoded sizes reduced finalization of that same spool to 0.742 seconds, with the complete command returning in 0.903 seconds. The resulting replay is byte-identical. Compression was already parallel across blocks. Root build, all 105 pure-function tests and clippy with warnings denied pass after these changes.
- The live identity is verified: both observations of the active DLC roster and all 31 successful content search roots match the resolved files and order. There are 24 selectable DLCs (23 folders and one archive), four integrated DLC roots, the game and user roots, and the Tweaker UI root. Integrated DLC and Tweaker content are included in the vanilla hash under their install-relative namespaces. The actual policy-1 covered set is 35,476 records and 1,761,361,427 bytes, substantially larger than the plan's initial estimate. Recorder hashing with an empty hash cache took 4.437 seconds in this run. Vanilla SHA-256 is `3b232dd23ad750df112a72c3eb2dea6b6a325d636069d04ceb826d0dc42dd141`.
- Main capture `m13_20260909/main/sessions/134334192318154539-1788946895678764500` used DLL SHA-256 `2b400af3f1d7a39e0aeff3863e322d91e3f56c0a3752cf3263de78eaa87d1bc4`. It recorded 28,084 hours across one session and two branches, with zero gaps, damaged segments, unavailable/truncated tables, dropped frames or country-link mismatches. Reloading the newly saved hour 60759402 produced branch 2 at hour 60759403, with `probable(1, 60759402)` parentage and supporting data present. After normal game exit, finalization took 7.337 seconds; exit confirmation through the completed report took 15.211 seconds. The replay is 5,057,907 bytes from 17,439,398 spool bytes.
- The matched throughput check failed. Both runs used process 59172 and the January 2 save, with the same campaign seed and stable load witness during each window. Free-run baseline `baseline-20260909-193047-821-59172.jsonl` covered 600.127 wall seconds. Its native summary is 39.752 hours/s after excluding three unreadable samples; its complete valid-endpoint interval, including those missing samples' time, is 39.754 hours/s. The first-branch capture window contains 21,425 frames over 599.957 seconds, at 35.709 hours/s: **89.825% of free-run**. Exact common endpoint hours 60759432 through 60780828 take 599.173 seconds captured versus 532.082 seconds free: **88.803%**. No endpoint interpolation was used. Callback p50/p95/max for the window is 2.1552/2.7367/4.9725 ms. Thus the throughput miss is not an artifact of setup pauses, different hour ranges or the missing baseline samples. Detailed results are in `m13_20260909/main/measurement-windows.json`, generated with the replay crate's own recovery/decoder APIs.
- Loading the starting save resets speed to 1. An initial baseline attempt at that speed was stopped and excluded; the completed pair explicitly selected speed 5 after loading. Worker timing and buffer reuse are being measured before choosing a throughput fix. No sampling-boundary or precision change has been made.
- Restart/append passed with process 74672 and session `134334217303806388`, using the instrumented FIFO DLL `ad6017731a2dcc20d6e8398c213a3917eae8b5ece7ab1dce88f66b1eff55cc15`. The new process recorded 6,245 hours without gaps or incomplete tables. Finalization took 1.847 seconds and produced a 6,185,509-byte replay containing two sessions and three branches. All 40 compressed blocks from the first session compare byte-identical against the retained previous replay. Branch 3 correctly starts a new session; its load origin and parent are unknown.
- The instrumented FIFO pair also exposes run variance: its selected 120-second window recorded 4,925 frames at 41.089 hours/s against a fresh 40.565 free-run baseline, or 101.293%; exact common hours give 101.373%. Window callback p50/p95/max is 1.9456/2.7690/5.0889 ms. Only worker timing instrumentation changed; this is not evidence that a throughput fix has landed. The failed longer pair remains above. At 1,773 recorded hours the spool's inclusive buffer service averaged 1.196 ms/hour: conversion 0.323 ms, frame encoding 0.617 ms and record writes 0.094 ms; spool-thread CPU averaged 1.481 ms/hour including other worker work. These components overlap with the inclusive service time and must not be summed with it. The observed queue peak was two. FIFO versus immediate buffer reuse will be compared within one process before adoption.
- The buffer experiment used process 58464, reloading `GER_1936_01_02_19.hoi4` before each 120-second run. The free-run baseline was 59.201 hours/s. FIFO measured 54.824 hours/s (92.606%; exact common hours 92.368%), with callback p50/p95/max 1.3285/1.8644/15.0422 ms. Immediate reuse measured 54.784 hours/s (92.539%; exact common hours 92.288%), with 1.3506/1.8738/3.1850 ms. Both had zero gaps, incomplete tables, dropped frames, link mismatches or write errors and restored/unloaded cleanly. The reuse DLL was `e382e9be3544f8a328788dcb7c48df8931da34ea2e904d33fd41f3620d569d7d`. No throughput or median/p95 improvement was demonstrated, so the experiment was removed and FIFO retained. Evidence remains under `m13_20260909/buffer-pair/{baseline,fifo,hot}`. The next measured change retains unchanged encoder baselines instead of cloning their row vectors; it does not change recorded values or wire bytes.
- Retaining unchanged encoder baselines reduced mean frame encoding from 0.605 to 0.149 ms/hour and spool-thread CPU from 1.086 to 0.664 ms/hour in the same-process comparison. DLL `2076e2cbb0d58b745a4b0b93761f2ed9f763ad59d36325297654c8d3afc6ef5d` recorded 6,660 frames in the selected 120-second window at 55.500 hours/s, 93.749% of free-run (exact common hours 93.639%). Callback mean/p50/p95/max was 1.2824/1.2332/1.7271/2.9674 ms. It retained FIFO allocation, full precision and identical format semantics. This is a measured worker-cost improvement; the longer acceptance repeat is still required.
- Actual Ctrl+C was sent to that recorder's console with the game unpaused. The hook was restored, module absence confirmed, and all 6,730 recorded hours finalized in 2.422 seconds into a 1,232,112-byte replay. There were no gaps, damaged segments, incomplete tables or drops; the final detach record names hour 60766132 and `completed.json` reports detached. The surrounding PowerShell console returned status 1 after cancellation, while recorder finalization completed. A subsequent no-DLL clock check in the same game process recorded another 564 hours over 9.862 seconds, confirming continued simulation. Files are under `m13_20260909/buffer-pair/unchanged` and `after-ctrlc`. Root build, all 106 pure-function tests and clippy with warnings denied pass.
- Forced termination passed using the same optimized DLL in `main/sessions/134334227054091720-1788950418701613100`. After 120 seconds at speed 5, `taskkill /PID 58464 /F` stopped the test process. Its last saved status acknowledged 6,692 hours through 60766094; all 6,739 recoverable hours survived into the replay with exact frame and control equality. No detach record was fabricated, and this run happened to end between valid records: zero damaged segments, gaps, incomplete tables or drops. Finalization took 1.818 seconds; callback p50/p95/max was 1.218/1.630/2.570 ms. The campaign file now holds three sessions and four branches in 7,413,385 bytes. All 49 old compressed blocks, their indexes, the old header-and-block prefix and prior metadata compare unchanged. The shared recovery/reader API verification is recorded in `main/forced-verification.json`; its 22 checks passed.
- Final source review found a detach reporting race: the recorder checked errors before waiting for the collector's final drain. It now reads terminal status and error evidence after module absence or process exit, finalizes recoverable prefixes even on a drain failure, and records/reports partial completion explicitly. Successful recovery of an earlier transient error remains a warning; ordinary game exit does not require a detach witness. Collector and recorder agree on an explicit terminal `drain_complete` result. Pure outcome tests and independent review cover these distinctions. Root build, all 112 pure-function tests, clippy for all targets and release builds pass. The full-duration repeat uses DLL `6a8d859279327eac0b1667f12aaed569271cbc5a663f85f296e3f06d8b04e836`; its game-thread sampling and unchanged-baseline optimization are unchanged.
- The optimized ten-minute window passed the measured targets in process 71292, with both runs reloading `GER_1936_01_02_17.hoi4`. Baseline `acceptance/baseline/baseline-20260909-204522-148-71292.jsonl` ran 600.039 seconds, with one unreadable clock sample and no load changes. Its complete valid-endpoint rate is 45.708 hours/s (native summary 45.704). Capture `acceptance/sessions/134334242296068413-1788951507610823300` contains 29,449 selected frames, hours 60759402 through 60788850 over 599.877 seconds: 49.090 hours/s, or 107.400% of the baseline. Exact common hours 60759440 through 60786858 took 555.833 seconds captured versus 599.853 seconds free, or 107.920%. Callback mean/p50/p95/p99/max was 1.3808/1.3464/1.8609/2.2192/9.9711 ms. All selected hours and tables were complete and consecutive, with stable campaign/load fields and no dropped frames. The above-100% ratio demonstrates run variation, not that recording makes the engine faster; the earlier failed pair remains part of the record. The measured encoder and worker CPU reduction is separately established above. Details are in `acceptance/measurement.json`.
- The complete optimized session recorded 35,984 hours across two branches. Branch 2 starts at 60759403 after loading hour 60759402 and records `probable(1, 60759402)` with supporting data present. After normal exit, finalization took 9.666 seconds and produced a 6,331,988-byte replay from 21,935,490 spool bytes (609.590 bytes/hour). Total callback p50/p95/max was 1.338/1.863/9.971 ms. There were zero gaps, damaged segments, unavailable/truncated tables or drops. A fresh text save, `GER_1936_09_29_19.hoi4`, was written while paused at the final recorded hour 60765882; the save remains in the isolated test user directory and matching game screenshots are under `acceptance/`. This is the M1.4 oracle fixture, not an oracle result.
- The final build also passed recorder-first startup, restart/append and actual Ctrl+C. Process 58964 loaded that final-hour save; its first frame is exactly the next hour, 60765883, but still starts session `134334261769361984` and branch 3. It recorded 7,228 hours over 146.075 seconds of clock witnesses; callback p50/p95/max was 1.401/1.999/3.867 ms. Ctrl+C confirmed module absence and `drain_complete=true`, with no partial outcome or warnings. Finalization took 2.670 seconds. All new frames/controls match the spool and all 50 earlier blocks remain byte-identical (`acceptance/ctrlc-verification.json`). The game then advanced 537 more hours without the DLL before being paused and closed. The surrounding PowerShell host returned cancellation status 1 after recorder completion, as in the earlier console test.
- The final-build forced-exit regression used process 35936 and recorded 1,055 hours before `taskkill /F`, with zero gaps or incomplete tables. An initial test launch caught the previous process during shutdown and refused with `game exited while content identity was being gathered`; its logs were retained, and a fresh recorder attached to process 35936. The actual recorded witness interval was 19.571 seconds. Recovery/finalization took 0.321 seconds; all recoverable frames and controls survived, there is no invented detach record, and all 61 prior blocks are unchanged (`acceptance/forced-verification.json`). Natural process exit is correctly reported with `drain_complete=null`, `partial=false`. The final acceptance replay is 7,892,420 bytes and holds 44,267 hours, three sessions, four branches and 63 blocks. All game processes are now closed.

### Milestone 1.4 completed 2026-09-09

The text-save oracle passes at the captured save boundary. Root workspace build, all 130 pure-function tests, strict clippy for all targets and the release recorder build pass. Milestone 1.5 may begin. The oracle uses the same replay reader and Clausewitz parser as the product; no corpus or game process is used by unit tests.

- `recorder oracle <acceptance replay> <GER_1936_09_29_19.hoi4> --hour 60765882 --branch 2 --user-dir captures\m11_20260909\userdir` returns success with **zero differences across 13,413 provinces, 439 countries and 2,194 available country scalars**. The replay is the final M1.3 campaign under `captures\m13_20260909\acceptance\`; output is `oracle.txt` there. Command elapsed time is 2.100 seconds with a warm content cache. The saved hour occurs on two branches, so omitting `--branch` correctly refuses ambiguity. Wrong-hour and binary-header checks also return failure; the binary refusal instructs `save_as_binary=no`.
- Province membership is independently reconstructed from the recorded content's `definition.csv` and state history. The save contains 1,081 states and 4,013 sparse province records, including 37 explicit controllers. All 13,413 non-sentinel map provinces are compared, including 3,141 without a state. Country references are resolved through the replay's link table; the save's omitted controller defaults to the saved state owner, as proven by its writer and post-load path. Both missing and extra rows are differences. An incomplete table, unresolved reference or missing required comparison input refuses the comparison.
- Political power, command power, stability and war support are exact signed i64 values at scale 100,000. The save formatter preserves all five digits and trims only trailing zeroes. Convoy stockpiles resolve their saved typed equipment ids through verified content definitions, filter the convoy classifier and reproduce the engine's per-entry clamp in saved order. The one unavailable scalar is AFA's invalid captured convoy cache; all 438 available convoy totals match. No display rounding or tolerance is used.
- Printed limitations are explicit: rendered country names and raw country-link identities are not preserved by the save; omitted controllers lose the distinction between owner-equivalent and nonpositive references; the save's session time includes elapsed wall time and cannot equal the persisted clock field directly; the process-local load timestamp is not serialized. These do not substitute values into the replay. The 1–24 save-hour convention and fixed 365-day calendar are confirmed against the engine's date parser and formatter.
- Content resolution follows the chosen session's recorded ids and order, independently of current launcher selections, and verifies executable, vanilla, DLC and mod hashes under the recorded policy. The resolver measured 35,477 hashed records and 1,817,042,059 bytes including `hoi4.exe`; the content-only counts remain those in M1.3. Its verification took 1.245 seconds. Save extraction alone took about 545 ms and peaked at 542 MiB while holding the shared syntax tree for the 65,774,563-byte save; the tree is released after typed extraction.
- Independent review found and fixed a false-success path that treated malformed convoy inputs as a save limitation. Missing pools, dangling equipment definitions and unresolved equipment classification now refuse comparison when a captured convoy value exists. Synthetic tests cover that refusal, one-unit differences beyond floating-point exact range, aliases, sparse controller rules, missing/extra provinces, failed tables, branch ambiguity and wrong hours. Static derivations remain outside the repository in `country-save-semantics.md`, `province-save-semantics.md` and `save-extraction-report.md` under the rewrite scratch directory.

### Milestone 1.5 completed 2026-09-09

The new viewer opens the accepted replay, scrubs recorded hours and switches sessions/branches, renders political and control maps, pans and zooms, and dumps the saved hour through the same wgpu map pipeline. The first game/viewer reference pair is in `reference/01.md`. Root workspace build, all 150 pure-function tests, strict clippy for all targets and release recorder/viewer builds pass. The save oracle still reports zero differences. Milestone 1.6 may begin; the old trees are retained until the knowledge harvest and final foundation check pass.

- The viewer uses eframe 0.36.2, winit and wgpu 30.0.1 on Rust 1.96. First-run folder entry, saving `viewer.json` beside the executable and opening a replay through the native file picker were exercised in the actual window. UI checks covered the first/middle/last hours, selecting branch 2 with probable parent support present, switching to session 2/branch 3 with unknown parentage, playback/pause, political/control selection, pan, zoom and fitting the map. Screenshots remain under `captures\m15_20260909\`. No game process was needed or touched by the viewer.
- Verified map parsing reads the 5632×2048, 24-bit province bitmap in top-left row order: 11,534,336 pixels, 13,414 definition ids including the zero sentinel and 13,413 used province ids. All pixels resolve, and all 439 authored country colors resolve without invented fallbacks. Country file colors are overridden by the political `.color` entries in `common/countries/colors.txt`; UI colors are separate. Runtime/cosmetic color overrides are not captured and remain an explicit presentation limitation. The initial renderer draws country borders and plain political colors; terrain and overlays remain scheduled for Milestone 2.
- GPU rendering was exercised on an NVIDIA GeForce RTX 5090 through Vulkan. Both GUI and dump use the same province-id texture, country lookup and offscreen render method. The output is registered with egui in its required unorm format; headless readback handles padded rows. The 2560×1440 reference dump at branch 2/hour 60765882 uses camera `3010,477.5,248.7`. The camera fit, original screenshot, save identity/hash and visual limitations are recorded with the pair. No pixel-equality or similarity-score claim is made at this phase.
- Playback retains one decoded block and its per-table baselines. Forward play applies deltas; backward seeks inside the block reset the decoder without decompressing again. Synthetic tests cover exact gaps, unavailable/truncated tables, corrupted blocks, missing initial/trailing hours, ambiguous parent support and cache reuse. A gap or failed required table hides the map with its reason; previous colors are not displayed as a new hour. UI review fixed a date label that could lag the newly selected frame, and actual interaction fixed initial fit sizing and drag-origin handling.
- Cold first-open file reads took approximately 134 seconds in one release run while creating the separate viewer hash cache. A subsequent empty application cache with warm filesystem data read all 1,817,042,059 bytes in 3.429 seconds, with document load/seek/palettes complete in 3.746 seconds. With the hash cache populated, content resolution took 867 ms and the complete load/seek/palettes took 1.162 seconds. These measurements point to cold storage/cache state as the source of the long first read; content loading stays on a worker so the UI remains responsive. No faster first-ever load is claimed.
- Synthetic one-hour replay files outside the repo verified content refusal paths. Unverified roster identity returns failure normally; `--allow-unverified-content` loads it with persistent GUI warnings and an amber `UNVERIFIED CONTENT` banner in the headless PNG. An actual layer-hash mismatch is refused even with that override and names the layer and both hashes. The unverified-banner addition leaves verified PNG bytes unchanged: `05ddfb187e2b3318e2d419295a1d54839c2b4aa278f3af1b733f71f8afe6d7d1` for the reference image. All test viewer windows were closed after validation.
