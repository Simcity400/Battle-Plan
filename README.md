# Battle Plan

Battle Plan records a Hearts of Iron IV campaign while you play and lets you watch it back later. Playback is an interactive map with the game's own UI. You can scrub through time, move around the map, switch countries, and open panels the way you would in the game. It works for single-player and multiplayer, with any mods and DLC.

[AGENTS.md](AGENTS.md) governs agent work.

## How it works

1. Start the recorder, then play HOI4 normally. The recorder finds the game on its own. There is nothing to configure.
2. A small collector inside the game reads world state once per game hour and hands it to the recorder.
3. The recorder writes a `.battleplan` file. That file is the only thing the viewer needs.
4. Open the file in the viewer. It loads the same installed content, mods, and DLC that were active and rebuilds the map and UI from the recorded state.

```text
hoi4.exe -> collector -> recorder -> .battleplan -> viewer
```

## Rules

These rules always apply. Code, schemas, registries, tests, and evidence hold the details. No other document overrides them.

### Recording

- Recording never pauses, slows, or controls the game. It never drives the UI, input, or focus, never freezes the game to take a snapshot, and never makes the game wait on the recorder. Running at maximum speed and auto-pausing are normal play.
- The collector only reads. Hooking the game is a last resort, used only when plain reading cannot get a value, and every hook is registered, bounded, and restored byte-for-byte on exit, including bootstrap hooks.
- An unsupported game version records nothing and says why. Adding support means updating the app, never guessing.
- State is sampled once per game hour. Finer sampling needs evidence that it is worth it.
- A capture failure costs only the affected period, never the whole recording. Anything missed becomes a named gap with its time range, what was missed, and why.

### Truth

- Never fake data. If a value was not captured, the replay says so. It never fills in a plausible guess, another country's view, historical data, or a visual approximation.
- Record world state, not pictures. Screenshots, pixels, audio, and the player's own UI never go into the file.

### Playback

- Draw things exactly the way the game does, based on proven engine behavior. No hand-tuned constants or "close enough" shortcuts.
- Panels, tooltips, and selection are rebuilt from recorded state. Where that is impossible, the viewer shows "unavailable" rather than a guess.
- The replay can be viewed from any country's perspective.
- Sound comes from proven game rules and recorded inputs. Otherwise it is silent and says so.
- The windowed viewer, headless export, and benchmarks all render through the same code path.

### Files

- The `.battleplan` file is the only link between recorder and viewer. Format changes update every reader and writer together, with no compatibility shims left behind.
- The viewer loads the exact base game, DLC, mods, and language that were active, in the same order. If any of it is missing or changed, the viewer refuses and says what is wrong.
- On first run the viewer asks where the game and mods are. It never guesses.
- The viewer never reads live game memory and never launches or injects HOI4.
- Reloading a save or restarting the game stays in the same replay as a branch. Two games running at once are two replays.

## Targets

- Recording adds at most 16 ms per game hour, including the slowest 5% of hours.
- A replay is ready to open within 30 seconds of the game ending.
- A finished replay is under 1 GB. The goal is 50 MB.
- The viewer holds 60 fps on average, with worst frames under 33 ms, at 1920x1080 on an i5-8400 / GTX 1060 class PC.

## Release

One offline portable Windows ZIP: recorder, viewer, collector DLL, instructions, checksums. Unzip and run. No installer, updater, admin rights, or code signing.

Claims about how the game engine works need exact-build Ghidra exports plus live or game-file evidence. See [engine evidence](extractor/native/scripts/AGENT_OPERATIONS.md#engine-evidence).

## Where things are defined

- [Domain registry](extractor/native/domain_registry.json): which pieces of game state get captured, and their status.
- [UI dependencies](replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json): what each UI element needs from the recording and what it shows when that is missing.
- [Map input ledger](replay_interface/re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv): every non-UI visual input and how it is handled.
- Evidence under `extractor/**/re_evidence/`, `extractor/native/runs/`, and `replay_interface/re_evidence/`: build identities and validation results.
