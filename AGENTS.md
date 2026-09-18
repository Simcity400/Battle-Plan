# Working on Battle Plan

Read the [project README](README.md) and the README for the component you are changing. The user's current instructions take precedence over old handoffs.

## Keep these guarantees

- Update affected format readers, writers and consumers together. Code owns version numbers, table IDs and cadence; do not duplicate those catalogs in prose.

## Validate locally

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

For frontend or analytics data-type changes:

```powershell
npm.cmd run check:ui
npm.cmd run build:ui
cargo run -p analytics --bin export-types -- --check
```

Unit tests use pure functions and synthetic memory, not the game or a capture corpus. Recorder/viewer integration also needs the relevant fresh-save oracle and existing smoke workflow in the [recorder guide](crates/recorder/README.md).

Coordinate game ownership. Keep builds outside timed intervals; reuse matched saves/baselines and batch expensive live checks. Never waive a save difference. Report what ran, failed or remains unverified. Documentation-only changes need link and diff checks, not game captures.

## Files and documentation

Use the worktree's `.cargo-target` and shared `C:\sccache_cache`; never delete the shared cache. Put local captures/reports under `captures\`, outside Git. Keep other scratch outside source. Never commit dumps, logs or build output.

Keep the root README as an index, shared rules here, component details beside each crate and field evidence beside code. Preserve approved decisions.
