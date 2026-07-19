# Central Rust validation

The repository uses one shared Cargo target directory, `.cargo-target`, for both the extractor and replay workspaces. Cargo uses reduced local debug information. The centralized runner enables `lld-link` and `sccache` when they are installed, without making those local tools mandatory for CI or other clones.

Run Rust validation through `Invoke-CentralRustValidation.ps1`. The script uses a named mutex and rejects a second concurrent invocation, preventing multiple agents from starting competing Cargo builds.

Because the default Windows execution policy blocks unsigned PowerShell files on this machine, invoke it explicitly with `-ExecutionPolicy Bypass`:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Check -Workspace Extractor
```

Available modes:

- `Check`: type-check the selected package/targets without linking.
- `Build`: debug build of the selected package/targets.
- `Fast`: fail-fast library/binary unit tests only.
- `Integration`: fail-fast integration-test binaries only.
- `Test`: fail-fast unit and integration tests, excluding soak and external-evidence tiers.
- `Final`: complete deterministic offline nextest plus Cargo doctests, one workspace at a time; reports every failure but excludes soak and external-evidence tiers.
- `Soak`: serialized replay soak and budget gates only.
- `Evidence`: explicitly selected game-install, capture-corpus, or live-resource tests; includes matching ignored tests.
- `Release`: release build and release-profile nextest run.
- `Stats`: show sccache statistics without compiling.

`Check`, `Build`, `Fast`, `Integration`, `Test`, and `Evidence` use Cargo's normal default targets unless an exact selector is supplied:

- `-Lib`: only the package library target.
- `-Bin <name>`: one named binary; repeat or pass an array for several.
- `-TestTarget <name>`: one named integration-test target.
- `-AllTargets`: every target in the selected package/workspace.

Do not combine `-AllTargets` with exact selectors. `Final` and `Release` intentionally remain complete offline batteries. `Soak` owns its replay targets. `Evidence` refuses an unscoped run: supply a package, target, or filter expression.

Test tiers:

- Fast: deterministic unit tests with no external installation, capture corpus, sleeps, stress loops, or soak budgets.
- Integration: deterministic cross-crate or executable-boundary tests.
- Full offline: all fast and integration tests plus doctests; this is `Final`.
- Soak/performance: long-running memory, seek, stress, or budget evidence; run explicitly with `Soak` or a named ignored performance test.
- Evidence/live: tests requiring the game installation, captures, RenderDoc, hardware, or a live process; run explicitly with `Evidence` after satisfying their prerequisites.

`game_ui`'s shipped-file acceptance probe is a standalone binary rather than a
Rust test harness. Run it explicitly after satisfying its installed-game
prerequisite:

```powershell
cargo run --manifest-path replay_interface/native/Cargo.toml --release -p game_ui --bin acceptance
```

Keeping this target out of the test-harness inventory prevents nextest from
trying to parse the probe's human-readable report as `libtest` enumeration.

Routine modes fail fast. `Final`, `Soak`, `Evidence`, and `Release` retain complete failure reporting. The nextest default filter prevents soak and external-resource tests from being reported as ordinary passing tests when their evidence is absent.

Cache workflows:

- `-CacheMode Iterative` (default): Cargo incremental compilation, best for repeated edits in the current checkout.
- `-CacheMode Reusable`: disables incremental compilation and enables sccache, best for clean builds, branches, and worktrees.

Add `-Timings` to any compile/test mode to generate a Cargo HTML timing report under `.cargo-timings`. Timing generation is folded into the requested build and does not start a second compile.

Useful targeted examples:

```powershell
# One package while iterating
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Extractor -Package recorder_host

# Fast unit-test tier for one package
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Fast -Workspace Extractor -Package compact_contract -Lib

# One integration-test target
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Integration -Workspace Replay -Package game_ui -TestTarget ui09_canvas_admission

# Only one library target
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Check -Workspace Extractor -Package collector -Lib

# Only one integration-test target, with reusable cross-worktree caching
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Replay -Package game_ui -TestTarget ui09_canvas_admission -CacheMode Reusable

# Produce a timing report during the requested build
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Build -Workspace Extractor -Package recorder_host -Lib -Timings

# A nextest filter expression
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Replay -FilterExpression 'test(/xui27/)'

# Complete serialized final battery
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All

# Explicit replay soak tier (includes the long F2 budget gate)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Soak -Workspace Replay

# Explicit external-evidence tier, scoped to one package and test name
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Evidence -Workspace Replay -Package map_viewer -FilterExpression 'test(gate_all_vanilla_effect_files_parse)'
```

The first build into the shared target directory is intentionally a cold build. Subsequent extractor and replay invocations reuse compatible artifacts.

Run the read-only structural audit with `python extractor/native/scripts/Audit-RustTests.py`. Candidates are not deletion instructions: exact duplicates and weak bodies require semantic review before removal. The current reviewed disposition is recorded in `extractor/native/scripts/TEST_SUITE_AUDIT.md`.
