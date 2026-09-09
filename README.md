# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and opens it later as an interactive map. The working foundation records the clock, campaign identity, countries and province ownership/control, with a save oracle, branch-aware playback, pan, zoom and headless image export. It supports the pinned Windows build, Operation Postern v1.19.2.0.a729, verified by executable hash.

[PLAN.md](PLAN.md) is the plan of record. Unit counters, battles, fronts, ships, air wings, the country panel and the generic table inspector are the next phase. Vanilla with all DLC is the first tested content set; mod testing follows. The planned portable ZIP contains `recorder.exe`, `viewer.exe`, `collector.dll` and this README, with checksums. Packaging is still pending.

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

On first run the viewer asks for the game and Paradox user folders and stores them beside the executable in `viewer.json`. It offers political and control maps, a session/branch picker and an hourly timeline. Windowed playback and headless export use the same wgpu renderer. An hour present on multiple branches requires a branch selection. Both executables provide `--help`.

The recording and playback rules are:

- The recorder is passive: it never pauses, saves, sends input or drives the game. The collector reads game state through exactly one detour at the proven hourly boundary on the game thread, restored and verified on detach. Diffing, encoding and disk writes happen off the game thread. Two simultaneous game processes are unsupported and refused. An unsupported executable records nothing and says which build was required.
- Record state, never pictures or invented data. Values retain the engine's representation and precision; rounding belongs to display. A failed read or dropped hour is named in the file, scoped to the affected table or hours. The viewer shows gaps and unavailable data without interpolation. Save comparisons establish captured-value correctness and print their limitations.
- One `.battleplan` file links recorder and viewer. It stores content identity, including hashes and layer order, rather than game assets or machine paths. The viewer resolves content by id and verifies its bytes under the recorded, versioned coverage policy. Missing or mismatching content is refused. Unverified identity is refused unless `--allow-unverified-content` is explicit; that override keeps a visible warning and does not waive hash checks. The viewer never accesses live game memory.
- Every game process starts a new branch. Within a process, a measured load signal or an unexplained hour discontinuity starts another. Parentage is probable or unknown. The current load timestamp has one-second resolution, so timestamp collisions remain a documented blind spot. A different campaign id creates a different replay file.
- Data is exact; visuals use the game's files with our own compositing and side-by-side reference checks. The game's `.gui`/`.gfx` interface is a later phase. v1 is omniscient; country selection will choose whose numbers appear. Fog is later. Sound is dropped.

Measured goals are 16 ms per game hour, finalization within 30 seconds, and a campaign below 1 GB with a 50 MB goal. The accepted foundation session recorded 35,984 hours with total callback p95 **1.863 ms**, finalized in **9.666 seconds**, and produced **6,331,988 bytes**. After restart and forced-exit tests, the replay held 44,267 hours across three sessions and four branches in **7,892,420 bytes**. These measurements cover the foundation tables, not the eventual full table set or a full campaign. [PLAN.md section 9](PLAN.md#9-measurements) records the measurements, throughput comparisons and limitations; captures and `inspect` report costs and sizes.

Local validation follows [PLAN.md section 5](PLAN.md#5-validation): workspace build, pure-function tests, strict Clippy and an oracle comparison against a fresh text save at the recorded hour. Set `save_as_binary=no` for the oracle; binary saves are refused.

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

`tools/smoke.ps1` currently supplies explicit local game-driving and baseline actions. The complete end-to-end smoke command is pending Milestone 2.4 and reports that when invoked. [reference/01.md](reference/01.md) documents the first game/viewer screenshot pair. [crates/engine/PORTING.md](crates/engine/PORTING.md) indexes the harvested knowledge and the remaining table work.
