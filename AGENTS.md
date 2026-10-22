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

## Tests

- Run only the tests for what you changed: `cargo nextest run -p <crate> [filter]`. No full-suite runs, clippy or extra builds unless asked.
- Each test sets up everything it needs. Never rely on another test or on timing.
- No tests behind features. Use `#[ignore]` only for benchmarks and tests that need special hardware.
- Use synthetic data. Tests that need the real game read `BATTLE_PLAN_GAME_DIR`.

## Sharing this machine

- Don't start builds while a `BENCH_RUNNING` file exists under `captures`.
- A fresh checkout can start from the shared build: `python tools/warm_target.py warm --root warm-start\seed`.
- Only stop processes you started. Never touch other sessions' work.
- Put captures and scratch files in `captures\<name>\`, never in Git.
- Never use plain `git stash`; the stash is shared.

## Running the game

- One agent runs the game at a time. Create the folder `%TEMP%\claude\hoi4_game.lock` before launching and delete it when done. A lock older than 90 minutes is stale.
- Load straight into a save with `tools\smoke.ps1 -Action Launch -ContinueLastSave`, never through the main menu.

## Pull requests

- Use Codex for searches, basic research and mechanical work, not Claude agents. Check everything Codex writes. Use claude for more difficult tasks/research.
- One complete PR per task. Never leave residuals unless you are actually blocked and state what and why is blocked.
- After merging, delete your branch, worktree, build folder and captures.

## Reverse engineering

- `hoi4re` looks up decompiled game code and stores what we learn about it (`--help` lists commands). Only record names the code clearly supports, with the evidence.
