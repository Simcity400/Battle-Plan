# Working on Battle Plan

## The game

- Match the game's behaviour exactly. Never capture or compute more harshly than the game does.
- The collector only reads game memory: never call game functions, and copy what you need before the game moves on.
- Never show missing data as zero, and never fill a gap with a guess.
- Keep the game out of the repo: no installed content, addresses, decompiler names, or copied game code, shaders, scripts or text. Name offsets for what they are.
- Read game data from the user's installation at run time. Installed content are read-only.
- Mods are first-class: read content through the recorded mod stack, hard-code nothing from vanilla, and refuse anything unsupported by name.
- Ask the owner before recording any new data.

## Building

- Needs Rust 1.96+ with the x64 MSVC toolchain, Visual Studio C++ Build Tools with a Windows SDK, and Node.js 22.12+.
- Desktop app: `npm.cmd ci`, then `npm.cmd run dev` (add `-- -- -- "<replay>"` to open one). Release build: `npm.cmd run build:desktop`.
- Command-line recorder: `cargo build -p recorder -p collector --release`.
- Compile on Boat, not this machine, with `tools/boat.py` from inside your worktree: `python tools/boat.py check --all-targets -p <crate>...` (or `--workspace`), `python tools/boat.py ui`, and `python tools/boat.py build --out <dir> --release -p <crate> --bin <name>` for release binaries. It uploads the worktree's current source, picks, isolates and stops the VM, and passes Cargo's exit code through. Don't drive `boat` by hand or pass `-j`. Check only the affected crates while iterating.
- Exit 75 means Boat itself failed: run the same work locally.
- Compare benchmarks only between binaries built the same way, both on Boat or both locally.

## Where things are

- `crates/engine*`: reading the game's memory.
- `crates/collector*`: the DLL loaded into the game.
- `crates/recorder*`: the recorders, recovery and offline tools.
- `crates/replay*`: the recording format.
- `crates/installed-content*`: game, DLC and mod files.
- `crates/viewer*`: the map.
- `crates/dashboard*`: the dashboard's data.
- `crates/desktop` and `ui/`: the desktop app and its interface.

## Code

- Design for 1,500 countries. No caps we made up.
- Change file-format readers, writers and users together. No backwards compatibility.
- Depend on the specific crate you use, not the `engine` or `installed-content` bundles.
- After changing dependencies, run `python tools/check-build-dependencies.py`, `python tools/check-content-dependencies.py` and `python tools/check-feature-unification.py --fix`.
- After changing types the UI reads, regenerate them with `cargo run -q -p dashboard --features export-types --bin export-types`.
- Don't write READMEs or other docs.
- Only comment what reading the code can't tell you: a quirk, or something an agent must know. Keep it to one short line. Licence notices stay.

## Tests

- Don't run tests, not even before merging. Tests run once a day on this machine: the Windows task `Battle Plan daily tests` runs `tools/daily_tests.py` on `main` at 03:00.
- Before merging, compile what the PR changed, without running tests, on top of current `main`: `cargo check --all-targets -p <crate>...` for every crate it changes and every crate depending on those, directly or not, tests included (`cargo tree -i <crate> -e normal,build,dev --prefix none` lists them), and `npm.cmd run check:ui` when it changes the UI or the types the UI reads.
- Read the last run in `captures\ci\latest.md`. If it fails in something you changed, fix it in a new PR; the next daily run checks the fix. No clippy.
- Each test sets up everything it needs. Never rely on another test or on timing.
- No tests behind features. Use `#[ignore]` only for benchmarks and tests that need special hardware.
- Use synthetic data. Tests that need the real game read `BATTLE_PLAN_GAME_DIR`.

## Sharing this machine

- Only recorder benchmarks need a quiet machine. Each one holds a `BENCH_RUNNING` file under `captures` while it runs. Don't start local builds, tests or other benchmarks while one exists; `tools/boat.py` runs elsewhere and needn't wait.
- Viewer, dashboard and playback measurements don't need a quiet machine: run them in parallel, without a `BENCH_RUNNING` file, and pause them while one exists.
- Don't start a recorder benchmark while `captures\ci\TESTS_RUNNING` exists: the daily tests hold it, and wait for running recorder benchmarks to end first. One older than 20 hours is stale.
- If a single test holds up the daily run for more than an hour, stop the run.
- A fresh checkout can start from the shared build: `python tools/warm_target.py warm --root warm-start\seed`.
- Otherwise, only stop processes you started. Never touch other sessions' work.
- Put captures and scratch files in `captures\<name>\`, never in Git.
- Never use plain `git stash`; the stash is shared.

## Running the game

- One agent runs the game at a time. Create the folder `%TEMP%\claude\hoi4_game.lock` before launching and delete it when done. A lock older than 90 minutes is stale.
- Load straight into a save with `tools\smoke.ps1 -Action Launch -ContinueLastSave`, never through the main menu.

## Pull requests

- One complete PR per task. Never leave residuals unless you are actually blocked and state what and why is blocked.
- After merging, delete your branch, worktree, build folder and captures.

## Reverse engineering

- `hoi4re` looks up decompiled game code and stores what we learn about it (`--help` lists commands). Only record names the code clearly supports, with the evidence.
