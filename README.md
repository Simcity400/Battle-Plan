# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and opens it later as an interactive map. The current format 13 build has 148 typed tables covering the map, countries, units, politics, production, research, trade, construction, intelligence, characters, projects, weather, deployment queues, world tension, the army order of battle, scientists, diplomatic relations and the fog-derivation inputs, including script variables and the lexical dictionary. The 2026-09-11 live-validation pass is finished with recorded limitations; Milestone 2.1 is not accepted. The viewer has branch-aware playback, pan, zoom and headless image export; a save oracle checks captured values. It supports the pinned Windows build, Operation Postern v1.19.2.0.a729, verified by executable hash.

[PLAN.md](PLAN.md) is the plan of record. States, divisions, war relations and factions have passed Milestone 2.1 acceptance. The owner's 2026-09-11 instruction was to finish all remaining Milestone 2.1 work except custom UI panels; that pass is finished and reported in PLAN section 9. D102's fog-derivation inputs are implemented as six of the 148 tables and were live-captured, with three of them empty on the test campaign for the reasons recorded there. Custom data panels and the generic inspector are deferred. Map overlays for units, battles, fronts, ships and air wings remain future work. Vanilla with all DLC is the first tested content set; mod testing follows. The planned portable ZIP contains `recorder.exe`, `viewer.exe`, `collector.dll` and this README, with checksums. Packaging is still pending.

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

Measured goals are 16 ms per game hour, at least 90% of free-run throughput, finalization within 30 seconds, and a campaign below 1 GB with a 50 MB goal. The 2026-09-11 paired 120-second window, from the same late-1942 German save and camera in the same process as its free-run baseline, reached **31.432% of free-run** (29.839% by exact common hours) with **118.623 ms callback p95** including the startup double-read phase; three later steady-state windows, after every consistency window closed, give about **62 ms p95** and **35% of free-run**. Both goals are missed and further performance optimization is owner-deferred. The completed lifecycle sequence leaves a **71,738,541-byte replay**, with **4,656 recorded hours across four sessions and five branches**; finalization took 1.942, 3.619, 1.068 and 0.231 seconds. Current format 13 preserves native precision and lossless numeric/key differences; older formats are refused. [PLAN.md section 9](PLAN.md#9-measurements) records the separate measurement scopes and all earlier failures.

One fresh save comparison reports **zero differences across 138,439 exact comparisons**, with **all 148 tables available and no skipped comparison groups**, and its boundary witness states that all thirteen recorded engine allocation counters equal the save header, so that save was written at the sampled tick. All **145 eligible table consistency windows reached 200 matching double-read checks with zero mismatches** in each of the two long sessions, alongside a 200-check war-relation matrix census. Normal exit/reload, recorder-first restart, forced termination and a real console Ctrl+C detach preserve replay data; the 21-check verifier confirms all previous frames and compressed blocks unchanged and all new frames exact after every append. Windowed map playback, scrubbing across blocks, session and branch selection, pan and zoom were checked, and the viewer rendered saved hours through shared wgpu. Workspace builds, **1,037 tests**, strict all-target Clippy, the release build and the formatting check pass.

Three further fresh saves from the same pass fail **4, 21 and 2 differences**, in combat aggregates, land-combat participation, war casualties and supply-node motorization. Their thirteen allocation counters are equal too, so the witness bounds object allocation to zero but cannot see a non-allocating command or a continuous combat update, and the exact timing stays unproved. The zero-difference save does not erase those failures. Three defects the live run exposed were fixed with tests: a mislabelled `CMilitaryDeployment+0xB8` pointer that refused all four deployment tables, a saved theatre-area array the parser read as scalars, and a self-relation refusal in the opinion comparator. Native source proofs remain bounded, dictionary publication can report unavailable when its mutation witness changes, and save-information limits remain printed. **Milestone 2.1 is not accepted, and performance is not its only remaining issue.**

Local validation follows [PLAN.md section 5](PLAN.md#5-validation): workspace build, pure-function tests, strict Clippy and an oracle comparison against a fresh text save at the recorded hour. Set `save_as_binary=no` for the oracle; binary saves are refused.

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

`tools/smoke.ps1` currently supplies explicit local game-driving and baseline actions. The complete end-to-end smoke command is pending Milestone 2.4 and reports that when invoked. [reference/01.md](reference/01.md) documents the first game/viewer screenshot pair. [crates/engine/PORTING.md](crates/engine/PORTING.md) indexes the harvested knowledge and the remaining table work.
