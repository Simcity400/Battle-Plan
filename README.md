# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and opens it later as an interactive map. The current format 8 build has 124 typed tables covering the map, countries, units, politics, production, research, trade, construction, intelligence, characters, projects and weather, including script variables and the lexical dictionary. The current implementation and validation pass is finished with recorded limitations; Milestone 2.1 is not fully complete. The viewer has branch-aware playback, pan, zoom and headless image export; a save oracle checks captured values. It supports the pinned Windows build, Operation Postern v1.19.2.0.a729, verified by executable hash.

[PLAN.md](PLAN.md) is the plan of record. States, divisions, war relations and factions have passed Milestone 2.1 acceptance. The current pass is finished; no new work is started under the owner's latest instruction. Stopped D102 fog-input additions remain an explicit unimplemented gap. Custom data panels and the generic inspector are deferred. Map overlays for units, battles, fronts, ships and air wings remain future work. Vanilla with all DLC is the first tested content set; mod testing follows. The planned portable ZIP contains `recorder.exe`, `viewer.exe`, `collector.dll` and this README, with checksums. Packaging is still pending.

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

On first run the viewer asks for the game and Paradox user folders and stores them beside the executable in `viewer.json`. It offers political and control maps, a session/branch picker, an hourly scrubber and play/pause/speed controls. All candidate tables decode through shared playback, including data without a map visualization yet. Windowed playback and headless export use the same wgpu renderer. An hour present on multiple branches requires a branch selection. Both executables provide `--help`.

The recording and playback rules are:

- The recorder is passive: it never pauses, saves, sends input or drives the game. The collector reads game state through exactly one detour at the proven hourly boundary on the game thread, restored and verified on detach. Diffing, encoding and disk writes happen off the game thread. Two simultaneous game processes are unsupported and refused. An unsupported executable records nothing and says which build was required.
- Record state, never pictures or invented data. Values retain the engine's representation and precision; rounding belongs to display. A failed read or dropped hour is named in the file, scoped to the affected table or hours. The viewer shows gaps and unavailable data without interpolation. Save comparisons establish captured-value correctness and print their limitations.
- One `.battleplan` file links recorder and viewer. It stores content identity, including hashes and layer order, rather than game assets or machine paths. The viewer resolves content by id and verifies its bytes under the recorded, versioned coverage policy. Missing or mismatching content is refused. Unverified identity is refused unless `--allow-unverified-content` is explicit; that override keeps a visible warning and does not waive hash checks. The viewer never accesses live game memory.
- Every game process starts a new branch. Within a process, a measured load signal or an unexplained hour discontinuity starts another. Parentage is probable or unknown. The current load timestamp has one-second resolution, so timestamp collisions remain a documented blind spot. A different campaign id creates a different replay file.
- Data is exact; visuals use the game's files with our own compositing and side-by-side reference checks. The game's `.gui`/`.gfx` interface and custom data panels are later work. v1 is omniscient. Fog is later. Sound is dropped.

Measured goals are 16 ms per game hour, at least 90% of free-run throughput, finalization within 30 seconds, and a campaign below 1 GB with a 50 MB goal. The last paired measurement, before the final source changes, reached **29.783% of free-run** with **116.882 ms callback p95** including initial checks. Further performance optimization is owner-deferred. The completed lifecycle sequence leaves a **37,896,621-byte replay**, with **2,729 recorded hours across five sessions and six branches**; its final eight-hour append takes **0.163 seconds**. This short append is not a performance benchmark or full-campaign size result. Current format 8 preserves native precision and lossless numeric/key differences; older formats are refused. [PLAN.md section 9](PLAN.md#9-measurements) records the separate measurement scopes and all earlier failures.

The final save comparison reports **zero differences across 136,269 exact comparisons**, with **all 124 tables available and no skipped comparison groups**. All 121 eligible table consistency windows reached 200 matching checks during the longer source-enabled run. Normal exit/reload, process restart, forced termination and Ctrl+C detach preserve replay data; the final 21-check verifier confirms all previous frames and blocks unchanged and all eight new frames exact. Windowed map playback, scrubbing, session selection, pan and zoom were checked, and the frozen viewer rendered the final hour through shared wgpu. Workspace builds, **884 tests**, strict all-target Clippy, the release build and the final formatting check pass.

Two earlier comparisons from this same frozen source still fail **111 and 25 differences**. Their mutable orders, stockpiles, focus and other values are consistent with possible commands after the callback, but the exact timing is unproved. The later zero-difference save does not erase those failures. Native source proofs remain bounded, dictionary publication can report unavailable when its mutation witness changes, and save-information limits remain printed. **D102 fog/knowledge inputs remain unimplemented. Milestone 2.1 is not fully complete, and performance is not its only remaining issue.**

Local validation follows [PLAN.md section 5](PLAN.md#5-validation): workspace build, pure-function tests, strict Clippy and an oracle comparison against a fresh text save at the recorded hour. Set `save_as_binary=no` for the oracle; binary saves are refused.

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

`tools/smoke.ps1` currently supplies explicit local game-driving and baseline actions. The complete end-to-end smoke command is pending Milestone 2.4 and reports that when invoked. [reference/01.md](reference/01.md) documents the first game/viewer screenshot pair. [crates/engine/PORTING.md](crates/engine/PORTING.md) indexes the harvested knowledge and the remaining table work.
