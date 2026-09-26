# recorder

The passive command-line controller for recording. It finds and verifies HOI4, injects the collector, monitors recording, detaches safely and finalizes `.battleplan` files. It also inspects recordings and compares them with text saves.

## Record a campaign

Run from the repository root:

```powershell
cargo build -p recorder -p collector --release
.\.cargo-target\release\recorder.exe --out captures
```

Keep `collector.dll` beside `recorder.exe`. The recorder prints the Paradox user directory it selected; use `--user-dir <directory>` to choose another. Play normally. When the game exits, the recorder finalizes the session and waits for the next process. `--once` stops after one process session. Ctrl+C detaches, finalizes and exits.

Unsupported executable hashes and simultaneous game processes are refused. Recording must never pause, save, send input to or otherwise control the game.

## Inspect and compare

```powershell
.\.cargo-target\release\recorder.exe inspect <campaign.battleplan>
.\.cargo-target\release\recorder.exe oracle <campaign.battleplan> <save.hoi4> --hour <n> --branch <id>
```

The oracle compares the selected branch/hour with a fresh **text** save. Set `save_as_binary=no` in the chosen user directory; binary saves are refused. It reports differences, unavailable data and comparison limitations. A difference is never waived.

To compare every Daily, FourHourly, Static and sliced table, take the save at a full-refresh hour. Keyframes are not full-refresh hours: they re-encode the rows the recorder already holds. Two kinds of hour are:

- **A requested refresh.** While paused at the hour you will save, run `recorder refresh --out <dir>`. The collector acknowledges within about a second, and the next hourly sample reads every scheduled table. Make the text save, step one hour so the sample posts, and run the oracle at that hour on the current branch. Nothing is sent to the game. `tools/smoke.ps1` does this at its target hour.
- **A load.** With the recorder attached, load any save from the in-game menu. At the load hour, while still paused, make a fresh text save. Then step one hour so the sample posts, detach, and run the oracle on the new branch (`LoadSignal` in `inspect`) at that hour. Do not compare against the file you loaded: loading reruns the weather passes. [Reference 12](../../reference/12.md) records a run.

The replay derives province neighbour entries, terrain flags and terrain ordinals from the content stack instead of recording them. The movement load oracle checks that derivation against the game once per content stack. With the game owned, a save fully loaded and the game paused, run from the repository root (an attached recorder may stay attached):

```powershell
.\.cargo-target\release\recorder.exe movement-oracle --report captures\movement_oracle\<content>-<date>.json
```

It opens the single `hoi4.exe` read-only, gathers the same content stack a recording would (`--user-dir` as for recording) and refuses to compare unless that stack's load order is proven: the gather reports no ordering limitation (unmeasured mod or DLC dependency order) and the game's successful search path equals the stack's mounts in membership and priority order, as for a verified recording. It then compares every province definition's ordered neighbour entries (ID, crossing kind, midpoint flag, distance, crossing point), terrain flags and terrain ordinal with `MovementMap`. It prints the counts and the first differences (`--show <n>`, default 20) and writes all counts and up to 10,000 differences to the report. Any difference or unreadable province exits nonzero. `MovementMap` derives the neighbour entries of a record the loader invents for an unclaimed colour but not its terrain, so each such record fails as an `unclaimed_terrain` difference. Run it outside timed capture windows; it is not part of recording.

Match the sampled instant, not just the displayed date. Game state can change after the hourly sample but before a same-hour save. Matching allocation counters does not prove that nothing changed. Joined comparisons must respect each input table's availability and sample origin; an old carried value is not a new hourly observation.

## Live checks and performance

[AGENTS.md](../../AGENTS.md) lists local checks. `tools/smoke.ps1` is the existing end-to-end driver; inspect its help before use. It requires coordinated game ownership, the selected local save, an isolated text-save user directory, matching continue selection, calibrated UI settings and a new output directory. It drives the game separately from the passive recorder, then checks finalization, the oracle and reference renders. Keep all outputs out of Git.

Use matched save/content conditions and retained baselines. Keep builds outside timed intervals and cold work separate from steady state. Report actual game-hours per second, callback p95, memory, encoded size and finalization time. The established targets are at least 95% of non-recorder throughput and at most 16 ms p95 per game hour; replay readiness within 30 seconds and a campaign below 1 GB, with 50 MB as a size goal. These are targets, not a statement that this branch meets them.

## Code boundaries

Implementation is under [src](src/). Native reads belong in [engine](../engine/README.md) and [collector](../collector/README.md), the file contract in [replay](../replay/README.md), and content/text-save parsing in [installed-content](../installed-content/README.md).

Each process session starts a branch. In-process load signals and unexplained hour discontinuities also create branches; parentage is probable or unknown, never certain. The load signal's one-second resolution leaves timestamp collisions as a known limitation. Different campaign IDs belong in different replay files.
