# Working on Battle Plan

## The game

- Match the game's behaviour exactly. Never capture or compute more harshly than the game does.
- The collector only reads game memory: never call game functions, and copy what you need before the game moves on.
- Never show missing data as zero, and never fill a gap with a guess.
- Keep the game out of the repo: no installed content, addresses, decompiler names, or copied game code, shaders, scripts or text. Name offsets for what they are.
- Read game data from the user's installation at run time. Installed content are read-only.
- A recording carries every game fact it needs (definitions, localisation, executable tables, map layout), so playback never takes facts from the installed game. Art comes from the current installation, labelled as current; missing art is a named placeholder.
- An unrecognised game build is a warning, never a refusal. Each build-specific part verifies itself and is withheld by name if it fails.
- Mods are first-class: read content through the recorded mod stack, hard-code nothing from vanilla, and refuse anything unsupported by name.
- Game sites are `engine_sites::Rva` keys (positions in the reference build) resolved only through the located image (`engine_sites::at`, `hook`, `which`), never as base + offset. After adding, moving or removing a key, run `python tools/sites.py generate`; on a new game build, `python tools/sites.py relocate` reports what still locates.

## Building

- Desktop app: `npm.cmd ci`, then `npm.cmd run dev` (add `-- -- -- "<replay>"` to open one). Release build: `npm.cmd run build:desktop`.
- Command-line recorder: `python tools/boat.py build --out <dir> --release -p recorder -p collector`.
- Only Rust compiles go to Boat, never this machine. Run `tools/boat.py` from inside your worktree; don't drive `boat` by hand or pass `-j`. Exit 75 means Boat itself failed: run it again.
  - Check: `python tools/boat.py check --all-targets -p <crate>...` for the crates you changed only. Never the whole workspace, never their dependents.
  - Build: `python tools/boat.py build --out <dir> --release -p <crate> --bin <name>`, only when you need to run that binary.
- Everything else runs here: `npm.cmd run check:ui` for the UI, and Python checks and scripts directly.
- Check once, when the work is done, not as you go. Work that passed needs no second check before merging.
- Only run builds when you actually need it.
- Never run checks if you're going to have to build anyway

## Code

- Make everything as fast and efficient as it can be. Design every part, new or changed, as the best solution built from scratch, not a patch on what is there.
- Design for 1,500 countries. No caps we made up.
- Change file-format readers, writers and users together. Recordings stay readable forever: a recording-format change ships a lossless upgrader from the previous format and keeps the original file. Other formats need no backwards compatibility.
- Don't write READMEs or other docs.
- Only comment what reading the code can't tell you: a quirk, or something an agent must know. Keep it to one short line. Licence notices stay.

## Tests

- Don't run tests, not even before merging. Tests run once a day on this machine: the Windows task `Battle Plan daily tests` runs `tools/daily_tests.py` on `main` at 03:00.

## Asking the user

- Ask the user only with the question tool. A question written in an ordinary reply is never seen.

## Sharing this machine

- Never use plain `git stash`; the stash is shared.

## Running the game

- One agent runs the game at a time. Create the folder `%TEMP%\claude\hoi4_game.lock` before launching and delete it when done. A lock older than 90 minutes is stale.
- Load straight into a save with `tools\smoke.ps1 -Action Launch -ContinueLastSave`, never through the main menu unless you need a new save.

## Pull requests

- One complete PR per task. Never leave residuals unless you are actually blocked and state what and why is blocked.
- After merging, delete your branch, worktree, build folder and captures.
- All pull requests must be reviewed by gpt 6.1 sol xhigh in a loop until ready to merge. Don't be afraid to push back and avoid scope creep.

## Reverse engineering

- `hoi4re` looks up decompiled game code and stores what we learn about it (`--help` lists commands). Only record names the code clearly supports, with the evidence.
