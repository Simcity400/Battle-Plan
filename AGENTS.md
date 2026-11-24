# Working on BattlePlan

BattlePlan records campaigns and replays them in a Windows app. The recording is the product: if it is wrong or silently incomplete, everything built on it is wrong.

Prefer simple systems and honest data. Understand the real constraint, then build the smallest model that makes correct behaviour unsurprising. Don't keep complexity because it exists, and don't add machinery because it looks thorough. Fight scope creep: do the task, completely, and no more.

If a rule here fights the task in front of you, say so and get a human sign-off before breaking it.

## Asking the user

- Never put a question in an ordinary reply: it is not seen and will not be answered. Ask only with the question tool (`AskUserQuestion`), and make each question stand on its own.

## Glossary

Use these words for these things.

- **you** is the agent reading this file. **we** and **maintainers** are the people building BattlePlan. **user** is the person recording and watching campaigns.
- **game** is the supported strategy game. **mod stack** is the mods and DLC a recording was made with.
- **collector** is `collector.dll`, loaded into the game; it only reads game memory. **recorder** is the command-line recorder (`recorder.exe`) or the tray app (`battleplan-recorder.exe`) that attaches the collector and writes the recording.
- **spool** is the recoverable file written while recording. **recording** (or **replay**) is the published, indexed `.battleplan` file. **branch** is a continuous stretch of history; a load, a new session or a jump in time starts a new one.
- **viewer** is the map. **dashboard** is the chart, table and timeline overview. **desktop app** is the Tauri shell around both (`replay-desktop.exe`).
- **Boat** is the remote VM that compiles Rust for us (`tools/boat.py`).

## Ways to hurt yourself

1. **Killing by pattern.** Never `Stop-Process -Name`, `taskkill /IM`, or kill a PID you found by matching a name, path or worktree. Your process carries this worktree's path, and other agents' builds, Boat runs, dev servers, the game and the recorder share this machine. Kill only a PID you captured at spawn.
2. **Touching real user data.** `%LOCALAPPDATA%\BattlePlan` (captures, replay library, caches), the game install and the Paradox user folder are the user's. Read or copy them; never write to, clean or delete from them, and never point the recorder at them with its default output: pass your own `--out`. Test against a copy in your own capture folder.
3. **Baking in the machine.** Never commit a machine path, user name, port or host name. Take folders from arguments, settings or the environment (`BATTLEPLAN_CAPTURES` for capture folders). `tools/check-machine-paths.py` fails any path naming this machine; only scripts in `tools/machine-paths.allow` are exempt.
4. **Leaving a mess.** Make worktrees with T3 Code, or `git worktree add` beside the existing ones, not in a temp folder or the repo. A new worktree runs `tools/setup_worktree.py` (`npm ci`) in the background: while `.t3/setup-running` names a live process, wait for it; `.t3/setup-failed` holds the error. When done, delete the worktree, branch, build folder and captures.
5. **Shared state.** Never use plain `git stash`; the stash is shared.
6. **Stale agents.** Never message an agent idle over an hour, except Astra or Sol; brief a fresh one.

## The game

- Recorded facts and numbers match the game exactly, never harsher or guessed. Map visuals may differ for experience or speed: record each in an ADR, and ask when unclear.
- The collector only reads game memory: never call game functions, and copy what you need before the game moves on.
- Keep the game out of the repo: no game data, addresses, decompiler names, or copied game code, shaders, scripts or text. Name offsets for what they are. `tools/check-citations.py` fails addresses, offsets and decompiler names.
- Read game data from the user's own installation at run time, and never write to it.
- A recording carries every game fact it needs (definitions, localisation, executable tables, map layout), so playback never takes facts from the installed game. Art comes from the current installation, labelled as current; missing art is a named placeholder.
- An unrecognised game build is a warning, never a refusal. Each build-specific part verifies itself and is withheld by name if it fails.
- Mods are first-class: read content through the recorded mod stack, hard-code nothing from vanilla, and refuse anything unsupported by name.
- Game sites are `engine_sites::Rva` keys (positions in the reference build) resolved only through the located image (`engine_sites::at`, `hook`, `which`), never as base + offset. After adding, moving or removing a key, run `python tools/sites.py generate`; on a new game build, `python tools/sites.py relocate` reports what still locates.

## Hit every surface

The most common defect here is a change that works on the tested path and is missing elsewhere. Before calling work done, walk this list and say which entries applied:

- **Format ends.** A recording-format change touches the collector, the recorder, the replay reader and writer, the query layer, the viewer, the dashboard and the UI together, in one PR. No upgrader: proven older formats play, new data missing, changed tables and damage withheld; others are refused by name.
- **Entry points.** A stat reachable from one dashboard widget is usually also wanted in the map viewer, other widgets, the tray app and the command-line recorder. Fixing one is not fixing the feature.
- **Missing data.** Every reading has a "not recorded" state, and every consumer shows it as missing, never as zero, blank or a guess. Only a map visual may stand in, noted in debug output.
- **Mods and versions.** Check vanilla against a mod stack and different DLC sets. Every new read has a named refusal for the unsupported content it cannot handle.

## How it works

1. The recorder finds the running game, checks the executable, and loads the collector.
2. Every game hour the collector reads the game's state; hooks catch events between hours. Readings are copied out at once and handed to the recorder.
3. The recorder writes the `.battleplan` as hours come and commits each block; one that dies takes it up again. Continuity gaps become new branches, never invented history.
4. The desktop app opens recordings, and the dashboard and viewer share one playback clock. Every reply is read from the recording's own tables at the request.

## Where code lives

All Rust is in `crates/`; names group by prefix.

- `engine-*`: pinned game layouts, guarded memory reads and the hourly sampling boundary.
- `collector-*`: the readers loaded into the game, one crate per domain.
- `recorder*`: the recorder, the tray app, replay recovery and finalization. `recorder-oracle-*` and `recorder-derived` check recordings offline against text saves and derivations.
- `core-*`, `replay`, `replay-columnar`, `replay-data`: the collector's journal, the recording container and store, and shared rows and codecs.
- `content-reader-*`: read the user's own game and mod installation at run time.
- `viewer-*`, `map-*`, `shader-pipeline`: the map. `dashboard-*`: dashboard products over the contract in `dashboard-core`, which names no product.
- `desktop`, `app-update` and `ui/`: the Tauri shell, its updater and the React UI. `tools/`: Boat, smoke, CI, the release and leak checks, the acceptance suite (`tools/e2e`) and the repo checks.
- `test-unify` has no code. `[workspace.dependencies]` between its markers in `Cargo.toml` is generated by `tools/check-feature-unification.py --fix`: never hand-edit it.

## Building

- Desktop app: `npm.cmd run dev` (`-- -- -- "<replay>"` opens one); release: `npm.cmd run build:desktop`.
- Command-line recorder: `python tools/boat.py build --out <dir> --release -p recorder -p collector`.
- Only Rust compiles go to Boat, never this machine; release builds run in GitHub Actions (ADR 0001). Run `tools/boat.py` from inside your worktree; don't drive `boat` by hand or pass `-j`. Exit 75 means Boat itself failed: run it again.
  - Build: `python tools/boat.py build --out <dir> --release -p <crate> --bin <name>`, only to run that binary, never to confirm what CI or a release shows.
- Everything else runs here: UI checks, Python scripts.

## Verifying

- GitHub CI is the only test runner and the gate: merge only when its `check` passes. Pull requests run the changed crates and direct dependents, main every crate above a change, a nightly the whole workspace. Run `python tools/ci.py check` once, just before a push; it leaves jobs that compile or test to CI. New checks are jobs in `tools/ci.toml`.
- Test behaviour that can break: format readers and writers, recovery, parsing of game and mod files, and the logic a bug fix changed. Leave out what the types already guarantee and tests that mirror the implementation.
- Extend before you add: put the case in an existing test where one fits. Test at the lowest layer that fails on `main`, not again in a wrapper.
- A logic, data or format bug gets a failing test first, then the fix. A recording bug gets a `recorder-oracle-*` case that fails before the fix. A purely visual bug gets before/after captures and no test.
- Tests never wait on sleeps or wall-clock time. Flaky tests are fixed, not retried. A skipped test needs a tracking issue on the line above: `// quarantine: #1234 why`.
- Never run a suite. Run only the test you added or changed, where it needs no Rust compile; leave Rust tests to CI, not Boat.
- A user-visible UI change gets one pass in the running app, done once by the primary agent after integrating: run the Boat-built shell (`tools/e2e/suite.py build --shell`, reused until the Rust changes) with `BATTLEPLAN_UI_DIST` naming a `vite build` output. Subagents don't launch their own.

## Code

- Design every part, new or changed, as the best solution from scratch, not a patch on what is there, as fast and efficient as it can be.
- Design for 1,500 countries. No caps we made up.
- New comments fail the ratchet bar `// SAFETY:` and `# Safety` at unsafe code. Use ADRs.
- Suppressions give `reason = "..."` (`tools/check-suppressions.py`). Clippy and rustc lints fail on the lines you write.

## Pull requests

- Never make a PR unless the developer asks for one. A task that says to land work counts as asking.
- One request is one complete PR. Never leave residuals unless actually blocked; then state what and why.
- Titles are plain language led by the area, as in the log: `Columnar writer: PATCH keeps list runs behind a changed count`. The body gives the problem in a sentence or two, then the fix, and ends with the model and harness that did the work.
- UI changes need before/after images, and motion or timing a short video. Upload them to the PR; never commit PR-only assets.
- Review every PR with the Astra Loop (GPT-6.2 Astra, xhigh, never on a priority tier) before merging. Verify findings against the source: fix real ones, explain dismissals, push back on scope creep.
- To babysit, call `watch_pull_request` and end your turn. Don't poll or sleep. Stop when checks and reviews are green on the latest commit.

## Documentation

- Don't write READMEs or other docs. When a change makes existing doc text wrong, fix or delete that text.
- Plans, research notes and scratch files stay outside the worktree.
- This file has a size budget: cut text before adding it, then run `python tools/check-agent-context.py --write`.

## Running the game

- One agent runs the game at a time. Create the folder `%TEMP%\claude\hoi4_game.lock` before launching and delete it when done. A lock older than 90 minutes is stale.
- Load straight into a save with `tools\smoke.ps1 -Action Launch -ContinueLastSave`, never through the main menu unless you need a new save.

## Reverse engineering

- `hoi4re` looks up decompiled game code and stores what we learn about it (`--help` lists commands). Only record names the code clearly supports, with the evidence.
