# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and opens it later as an interactive map. It records the clock, campaign identity, countries, country-reference links, province and state ownership/control, divisions, war relations, factions and ordered faction membership. The viewer has branch-aware playback, pan, zoom and headless image export; a save oracle checks captured values. It supports the pinned Windows build, Operation Postern v1.19.2.0.a729, verified by executable hash.

[PLAN.md](PLAN.md) is the plan of record. States, divisions, war relations and factions have passed Milestone 2.1 acceptance. The current priority is capturing exact data and replaying it through the map and existing playback controls. Custom data panels and the generic inspector are deferred. Map overlays for units, battles, fronts, ships and air wings remain future work. Vanilla with all DLC is the first tested content set; mod testing follows. The planned portable ZIP contains `recorder.exe`, `viewer.exe`, `collector.dll` and this README, with checksums. Packaging is still pending.

Build from source with the pinned Rust 1.96.0 toolchain, the Windows MSVC build tools, `lld-link` and `sccache`:

```powershell
cargo build --workspace --release
.\.cargo-target\release\recorder.exe --out captures
```

Keep `collector.dll` beside `recorder.exe`. Start the recorder, then play normally. It finds `hoi4.exe` and prints the Paradox user folder it selected; pass `--user-dir <directory>` to select another. Game exit finalizes the replay and resumes waiting for the next process. Ctrl+C detaches, finalizes and exits. `--once` stops after one process session.

```powershell
.\.cargo-target\release\recorder.exe inspect <campaign.battleplan>
.\.cargo-target\release\viewer.exe <campaign.battleplan>
.\.cargo-target\release\viewer.exe --dump <campaign.battleplan> --hour <n> --branch <id> --out <frame.png>
```

On first run the viewer asks for the game and Paradox user folders and stores them beside the executable in `viewer.json`. It offers political and control maps, a session/branch picker, an hourly scrubber and play/pause/speed controls. All ten recorded tables decode through shared playback, including data without a map visualization yet. Windowed playback and headless export use the same wgpu renderer. An hour present on multiple branches requires a branch selection. Both executables provide `--help`.

The recording and playback rules are:

- The recorder is passive: it never pauses, saves, sends input or drives the game. The collector reads game state through exactly one detour at the proven hourly boundary on the game thread, restored and verified on detach. Diffing, encoding and disk writes happen off the game thread. Two simultaneous game processes are unsupported and refused. An unsupported executable records nothing and says which build was required.
- Record state, never pictures or invented data. Values retain the engine's representation and precision; rounding belongs to display. A failed read or dropped hour is named in the file, scoped to the affected table or hours. The viewer shows gaps and unavailable data without interpolation. Save comparisons establish captured-value correctness and print their limitations.
- One `.battleplan` file links recorder and viewer. It stores content identity, including hashes and layer order, rather than game assets or machine paths. The viewer resolves content by id and verifies its bytes under the recorded, versioned coverage policy. Missing or mismatching content is refused. Unverified identity is refused unless `--allow-unverified-content` is explicit; that override keeps a visible warning and does not waive hash checks. The viewer never accesses live game memory.
- Every game process starts a new branch. Within a process, a measured load signal or an unexplained hour discontinuity starts another. Parentage is probable or unknown. The current load timestamp has one-second resolution, so timestamp collisions remain a documented blind spot. A different campaign id creates a different replay file.
- Data is exact; visuals use the game's files with our own compositing and side-by-side reference checks. The game's `.gui`/`.gfx` interface and custom data panels are later work. v1 is omniscient. Fog is later. Sound is dropped.

Measured goals are 16 ms per game hour, at least 90% of free-run throughput, finalization within 30 seconds, and a campaign below 1 GB with a 50 MB goal. Format 5 adds war relations and factions while retaining lossless numeric/key differences and native precision. After read and buffer optimization, the paired ten-table capture reaches **93.302% of free-run** with **2.277 ms callback p95**. Its **6,426-hour** session finalizes in **0.768 seconds**, growing the campaign to **40,588,319 bytes across six sessions and seven branches**. Completed consistency buffers release about **82 MB**; median cold-block seeking falls from **316 ms to 56 ms** in the measured cases, with identical decoded data. Full-campaign size and the remaining v1 tables are unmeasured. [PLAN.md section 9](PLAN.md#9-measurements) retains earlier misses, failures and the diagnostic warning; captures and `inspect` report costs and sizes.

A fresh ten-table save matched all compared values, including 1,911 divisions, 37 war relations, six factions, 36 faction members and 6,015 country scalars. Other comparisons failed and remain recorded in PLAN. Commands can execute after the hourly callback without advancing the hour, so a save at the same displayed hour can contain later state. The oracle prints this boundary limitation and still fails every difference; it does not certify failed comparisons.

Local validation follows [PLAN.md section 5](PLAN.md#5-validation): workspace build, pure-function tests, strict Clippy and an oracle comparison against a fresh text save at the recorded hour. Set `save_as_binary=no` for the oracle; binary saves are refused.

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

`tools/smoke.ps1` currently supplies explicit local game-driving and baseline actions. The complete end-to-end smoke command is pending Milestone 2.4 and reports that when invoked. [reference/01.md](reference/01.md) documents the first game/viewer screenshot pair. [crates/engine/PORTING.md](crates/engine/PORTING.md) indexes the harvested knowledge and the remaining table work.
