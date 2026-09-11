# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and opens it later as an interactive map. A passive recorder injects one collector into the game, the collector samples the game state once per game hour, and the viewer plays the recording back hour by hour with branch-aware playback, pan, zoom and headless image export. It supports the pinned Windows build, Operation Postern v1.19.2.0.a729, verified by executable hash; any other build is refused with the build it wanted.

The release is a portable ZIP with `recorder.exe`, `viewer.exe`, `collector.dll` and this README, with checksums. Plan, status and evidence live in [PLAN.md](PLAN.md); the engine knowledge index is [crates/engine/PORTING.md](crates/engine/PORTING.md).

## Use

Build from source with the pinned Rust 1.96.0 toolchain, the Windows MSVC build tools, `lld-link` and `sccache`:

```powershell
cargo build --workspace --release
.\.cargo-target\release\recorder.exe --out captures
```

Keep `collector.dll` beside `recorder.exe`. Start the recorder, then play normally. It finds `hoi4.exe`, prints the Paradox user folder it selected (`--user-dir <directory>` selects another), records, and finalizes the replay when the game exits, then waits for the next process. Ctrl+C detaches, finalizes and exits. `--once` stops after one process session.

```powershell
.\.cargo-target\release\recorder.exe inspect <campaign.battleplan>
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
.\.cargo-target\release\viewer.exe <campaign.battleplan>
.\.cargo-target\release\viewer.exe --dump <campaign.battleplan> --hour <n> --branch <id> --out <frame.png>
```

On first run the viewer asks for the game and Paradox user folders and stores them beside the executable in `viewer.json`. It offers political and control maps, a session and branch picker, an hourly scrubber and play, pause and speed controls. Windowed playback and headless export use the same wgpu renderer. An hour present on several branches requires a branch selection. Both executables print `--help`.

## Rules

Recording:

- The recorder is passive. It never pauses, saves, sends input or drives the game.
- The collector reads game state through exactly one detour, at the proven hourly boundary, on the game thread. Diffing, encoding and disk writes happen off the game thread. The detour is restored and verified on detach.
- Two simultaneous game processes are unsupported and refused. An unsupported executable records nothing.
- A failed read or dropped hour costs only the affected table or hours and is named in the file.

Truth:

- Record state, never pictures or invented data. Values keep the engine's own representation and precision; rounding belongs to display.
- The viewer shows gaps, truncated tables and unavailable tables as such. Nothing is interpolated.
- Save comparisons are the correctness check for captured values and print their own limitations. A comparison difference is never waived.

Files:

- One `.battleplan` file links recorder and viewer. It stores content identity (executable, DLC and mod layer hashes, layer order, coverage policy), never game assets or machine paths. Older formats are refused; there are no compatibility shims.
- The viewer resolves content by id, verifies its bytes under the recorded coverage policy, and refuses missing or mismatching content. An identity the recorder could not verify is refused unless `--allow-unverified-content` is given; that override keeps a visible warning and never waives hash checks.
- The viewer never touches live game memory.

Branches:

- Every game process starts a new branch. Within a process, the measured load signal or an unexplained hour discontinuity starts another. Parentage is recorded as probable or unknown, never asserted.
- The load signal has one-second resolution, so a timestamp collision is a documented blind spot.
- A different campaign id is a different replay file.

Scope:

- Data is exact. Visuals come from the game's own files with our own compositing, checked side by side against the game.
- v1 is omniscient. Per-country fog, the game-skinned `.gui`/`.gfx` interface and custom data panels are later work. Sound is dropped.
- Vanilla with all DLC is the first tested content set. Mods are recorded by identity and tested afterwards.

## Validation

Validation is local only: the workspace build, pure-function tests, strict Clippy, and an oracle comparison against a fresh text save at the recorded hour (`save_as_binary=no`; binary saves are refused).

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

`tools/smoke.ps1` drives the game for local tests (launch, keys, clicks, screenshots, free-run baseline); its complete end-to-end action is pending. [reference/01.md](reference/01.md) holds the first game/viewer screenshot pair.
