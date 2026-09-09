# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and opens it later as an interactive map. It records the clock, campaign identity, countries, country-reference links, province and state ownership/control, and divisions. The viewer has a generic table inspector, branch-aware playback, pan, zoom and headless image export; a save oracle checks captured values. It supports the pinned Windows build, Operation Postern v1.19.2.0.a729, verified by executable hash.

[PLAN.md](PLAN.md) is the plan of record. States and divisions have passed Milestone 2.1 acceptance. Division rows include full native identities, owners, locations, templates, hit points and organisation; drawing unit counters on the map remains pending, alongside battles, fronts, ships, air wings and the country panel. Vanilla with all DLC is the first tested content set; mod testing follows. The planned portable ZIP contains `recorder.exe`, `viewer.exe`, `collector.dll` and this README, with checksums. Packaging is still pending.

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

On first run the viewer asks for the game and Paradox user folders and stores them beside the executable in `viewer.json`. It offers political and control maps, a session/branch picker and an hourly timeline. The inspector shows all seven recorded tables for the selected hour, with country filtering, scrolling and raw numeric values on hover. Division filtering includes roster, logical and expeditionary owners. Windowed playback and headless export use the same wgpu renderer. An hour present on multiple branches requires a branch selection. Both executables provide `--help`.

The recording and playback rules are:

- The recorder is passive: it never pauses, saves, sends input or drives the game. The collector reads game state through exactly one detour at the proven hourly boundary on the game thread, restored and verified on detach. Diffing, encoding and disk writes happen off the game thread. Two simultaneous game processes are unsupported and refused. An unsupported executable records nothing and says which build was required.
- Record state, never pictures or invented data. Values retain the engine's representation and precision; rounding belongs to display. A failed read or dropped hour is named in the file, scoped to the affected table or hours. The viewer shows gaps and unavailable data without interpolation. Save comparisons establish captured-value correctness and print their limitations.
- One `.battleplan` file links recorder and viewer. It stores content identity, including hashes and layer order, rather than game assets or machine paths. The viewer resolves content by id and verifies its bytes under the recorded, versioned coverage policy. Missing or mismatching content is refused. Unverified identity is refused unless `--allow-unverified-content` is explicit; that override keeps a visible warning and does not waive hash checks. The viewer never accesses live game memory.
- Every game process starts a new branch. Within a process, a measured load signal or an unexplained hour discontinuity starts another. Parentage is probable or unknown. The current load timestamp has one-second resolution, so timestamp collisions remain a documented blind spot. A different campaign id creates a different replay file.
- Data is exact; visuals use the game's files with our own compositing and side-by-side reference checks. The game's `.gui`/`.gfx` interface is a later phase. v1 is omniscient; country selection will choose whose numbers appear. Fog is later. Sound is dropped.

Measured goals are 16 ms per game hour, finalization within 30 seconds, and a campaign below 1 GB with a 50 MB goal. Format 4 adds lossless numeric and key differences; native values retain their full precision. Its first session recorded **19,284 hours across two branches**, with callback p95 **3.446 ms**, finalization in **6.108 seconds**, and an **8,920,329-byte** replay. The paired 120-second interval measured **40.405 hours/s** captured against **38.810 free-running**; the above-baseline result reflects run variation, not a claimed speedup. After a second session and clean Ctrl+C detach, the file holds **24,864 hours in 12,009,034 bytes**, with no dropped hours, incomplete tables or double-read mismatches. These results cover the seven current tables; full-campaign size and the remaining v1 tables are unmeasured. [PLAN.md section 9](PLAN.md#9-measurements) records the measurements, current acceptance status and limitations; captures and `inspect` report costs and sizes.

Two fresh format-4 saves matched all compared values within the printed oracle coverage; the later one included 1,754 divisions and 5,703 fixed-point scalars. Another comparison found one Turkish political-power difference of 25, matching the timing and amount of a scripted event. Commands can execute after the hourly callback without advancing the hour, so a save at the same displayed hour can contain later state. The oracle prints this boundary limitation and still fails every difference; it does not certify the failed comparison.

Local validation follows [PLAN.md section 5](PLAN.md#5-validation): workspace build, pure-function tests, strict Clippy and an oracle comparison against a fresh text save at the recorded hour. Set `save_as_binary=no` for the oracle; binary saves are refused.

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

`tools/smoke.ps1` currently supplies explicit local game-driving and baseline actions. The complete end-to-end smoke command is pending Milestone 2.4 and reports that when invoked. [reference/01.md](reference/01.md) documents the first game/viewer screenshot pair. [crates/engine/PORTING.md](crates/engine/PORTING.md) indexes the harvested knowledge and the remaining table work.
