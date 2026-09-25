# Working on Battle Plan

Read the [project README](README.md) and the README for the component you are changing. The user's current instructions take precedence over old handoffs.

## Keep these guarantees

- Update affected format readers, writers and consumers together. Code owns version numbers, table IDs and cadence; do not duplicate those catalogs in prose.
- Design for native mod compatibility. Resolve every content fact through the recorded content stack in [installed-content](crates/installed-content/README.md) (vanilla, DLC, then mods in recorded order). Never hard-code vanilla tags, IDs, counts, lists, defines, asset names, colours or localisation keys; collector tables size from live game data. Refuse unsupported content forms explicitly instead of assuming vanilla. Modded testing is not yet required.

- Ask questions always using the question tool.

- Feel free to ask questions. It's better to just ask a question when making a decision where there isn't a clear answer than just making an assumption based on incomplete information. Especially when UX is in question.

- Recorder/viewer integration also needs the relevant fresh-save oracle and existing smoke workflow in the [recorder guide](crates/recorder/README.md).

- Builds: while iterating, check only the crates you changed. Run full workspace checks once, when the work is finished, and at most two at a time across agents (`tools\build-slot.ps1`).

Coordinate game ownership. Keep builds outside timed intervals; reuse matched saves/baselines and batch expensive live checks. Never waive a save difference. Report what ran, failed or remains unverified. Documentation-only changes need link and diff checks, not game captures.

## Files and documentation

Use the worktree's `.cargo-target` and shared `C:\sccache_cache`; never delete the shared cache. Put local captures/reports under `captures\`, outside Git. Keep other scratch outside source. Never commit dumps, logs or build output.

Keep the root README as an index, shared rules here, component details beside each crate and field evidence beside code. Preserve approved decisions.
