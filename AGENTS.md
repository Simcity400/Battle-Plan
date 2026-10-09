# Working on Battle Plan

Read the [project README](README.md) and the README for each component you change; component rules such as capture, cadence, performance and text live there. The user's current instructions take precedence over old handoffs.

## Keep these guarantees

- Port native behaviour exactly. Copy the game's logic rather than approximating it, using hoi4re (`hoi4re fn|asm|data|callers <addr>`), and cite the addresses beside the code. Names the executable hard-codes are native facts: cite them, resolve them through the content stack, and refuse names the content does not define.
- Never capture or compute more harshly than the game does: where the game is stale, the recorder and viewer need not be fresher.
- Update affected format readers, writers and consumers together. Code owns version numbers, table IDs and cadence; do not duplicate those catalogs in prose. A branch's format version and new table IDs are provisional: set them to main+1, keep table IDs dense, and renumber at merge. An older recording that today's readers decode correctly must still open; refuse only layouts or meanings that changed, and add no migration layers.
- Design for native mod compatibility. Resolve every content fact through the recorded content stack in [installed-content](crates/installed-content/README.md) (vanilla, DLC, then mods in recorded order). Never hard-code vanilla tags, IDs, counts, lists, defines, asset names, colours or localisation keys; collector tables size from live game data.
- Implement every content form that vanilla or an installed mod (`<local-path>`) reaches. Refuse an unreached form by name and count it; never drop anything silently. Game and mod folders are read-only.
- Design for 1,500 countries and test at 5,000 synthetic countries. Capacities size from the live count; no caps we chose.
- Keep code minimal: no dead code, duplicated logic, needless abstraction or bloated tests, and concise comments that match the surrounding style. Tests must fail on the old behaviour.
- Recorder/viewer integration also needs the relevant fresh-save oracle and existing smoke workflow in the [recorder guide](crates/recorder/README.md).

- Ask questions always using the question tool.

- Feel free to ask questions. It's better to just ask a question when making a decision where there isn't a clear answer than just making an assumption based on incomplete information. Especially when UX is in question.

- If an action is denied, stop and report it; never work around it. Never print or share API keys.

## Builds and merging

- Only the agent writing code builds; reviews read the diff. While iterating, including fix rounds, run `cargo fmt` plus clippy and the tests your change affects, for only the crates you changed. Never run whole test suites or workspace-wide builds: the wrapper refuses them unless `GATE=1`.
- Before merging, merge current main into the branch, then at that head run `cargo fmt --all -- --check` and clippy on the crates you touched.
- The full workspace gate runs only at checkpoints the user or coordinator calls.
- Merge commits only: never rebase, force-push or push to main. Work only in your own worktree and branch. A PR body states what changed, the evidence, mod reach, what ran and what is unverified. After merging, delete the branch, the worktree and its `.cargo-target`.
- Every cargo run, and the npm entry points that build Rust (`dev`, `build:desktop`), uses the wrapper below from the worktree root.

```bash
# While iterating, for each changed crate
bash tools/build-slot.sh "dev/<task>" "cargo fmt -p <crate>"
bash tools/build-slot.sh "dev/<task>" "cargo clippy -p <crate> --all-targets -- -D warnings"
bash tools/build-slot.sh "dev/<task>" "cargo test -p <crate> <filter>"
# Before merging, also
bash tools/build-slot.sh "dev/<task>" "cargo fmt --all -- --check"
# Checkpoint gate, only when called
GATE=1 bash tools/build-slot.sh "dev/<task>" "cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast && cargo run -q --profile test -p dashboard --bin export-types -- --check"
```

Changes to dashboard data types also run `cargo run -q --profile test -p dashboard --bin export-types -- --check`; UI changes also run `npm.cmd run check:ui` and `npm.cmd run test:ui`. From PowerShell without Git Bash, use `tools\build-slot.ps1 -Owner "dev/<task>" -Command "<command>"`. The wrapper leaves `CARGO_INCREMENTAL` unset (sccache refuses to run when it is set); the Cargo profiles keep incremental compilation on.

## Live checks

Drive the game only while holding the game lock, and never stop another agent's game or recorder. Don't build during timed measurements; reuse matched saves/baselines and batch expensive live checks. Never waive a save difference. Report what ran, failed or remains unverified. Documentation-only changes need link and diff checks, not game captures.

## Files and documentation

The game is installed at `<local-path> of Iron IV` (tests that read it are `#[ignore]`d and take its path from `BATTLE_PLAN_GAME_DIR`). Python is `C:\Program Files\Python311\python.exe`, on PATH as `python`.

Use the worktree's `.cargo-target` and shared `C:\sccache_cache`; never delete the shared cache. Put local captures/reports under `captures\`, outside Git. Keep other scratch outside source. Never commit dumps, logs or build output.

Keep the root README as an index, shared rules here, component details beside each crate and field evidence beside code. Preserve approved decisions.

A README describes its area for agents working in it: purpose, layout, key concepts and invariants, rules and pitfalls, and how to check a change. Describe what exists now. Change history, status or acceptance records, test results, native evidence dumps and prose copies of code catalogues stay out.

Keep game installation files, copied game source/shader/GUI fixtures, captures and private audit reports outside Git. Use synthetic inputs for tests. Before sharing source or artifacts, follow [publication guidance](docs/publication.md) and run `python tools/check-publication.py` on the staged index; this guard does not audit history or establish rights.
