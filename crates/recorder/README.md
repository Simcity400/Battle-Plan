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

Match the sampled instant, not just the displayed date. Game state can change after the hourly sample but before a same-hour save. Matching allocation counters does not prove that nothing changed. Joined comparisons must respect each input table's availability and sample origin; an old carried value is not a new hourly observation.

## Live checks and performance

[AGENTS.md](../../AGENTS.md) lists local checks. `tools/smoke.ps1` is the existing end-to-end driver; inspect its help before use. It requires coordinated game ownership, the selected local save, an isolated text-save user directory, matching continue selection, calibrated UI settings and a new output directory. It drives the game separately from the passive recorder, then checks finalization, the oracle and reference renders. Keep all outputs out of Git.

Use matched save/content conditions and retained baselines. Keep builds outside timed intervals and cold work separate from steady state. Report actual game-hours per second, callback p95, memory, encoded size and finalization time. The established targets are at least 90% of non-recorder throughput and at most 16 ms p95 per game hour; replay readiness within 30 seconds and a campaign below 1 GB, with 50 MB as a size goal. These are targets, not a statement that this branch meets them.

## Code boundaries

Implementation is under [src](src/). Native reads belong in [engine](../engine/README.md) and [collector](../collector/README.md), the file contract in [replay](../replay/README.md), and content/text-save parsing in [installed-content](../installed-content/README.md).

Each process session starts a branch. In-process load signals and unexplained hour discontinuities also create branches; parentage is probable or unknown, never certain. The load signal's one-second resolution leaves timestamp collisions as a known limitation. Different campaign IDs belong in different replay files.
