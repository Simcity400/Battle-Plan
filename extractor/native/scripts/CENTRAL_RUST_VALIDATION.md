# Central Rust validation

The repository uses one shared Cargo target directory, `.cargo-target`, for both the extractor and replay workspaces. Cargo uses reduced local debug information. The centralized runner enables `lld-link` and `sccache` when they are installed, without making those local tools mandatory for CI or other clones.

`Release` is a Windows MSVC product build and sets Cargo's encoded Rust flags
to the exact target feature `+crt-static`. The collector C shim reads Cargo's
`CARGO_CFG_TARGET_FEATURE` and selects the same static CRT, so
`collector_record.exe`, `replay_app.exe`, and `collector.dll` do not require the
Visual C++ redistributable. The runner fails unless the active compiler host is
`x86_64-pc-windows-msvc` and rustc reports `crt-static` before building.

The repository toolchain is pinned by `rust-toolchain.toml` to Rust/Cargo
1.96.0. The runner also requires cargo-nextest 0.9.140 and passes `--locked` to
every build and test operation. A missing tool, version drift, or lockfile drift
fails before validation rather than silently selecting a different compiler or
dependency graph. The only exception is the explicit primary-agent `Lock` mode,
which regenerates top-level workspace lockfiles under the same central mutex.

Run Rust validation through `Invoke-CentralRustValidation.ps1`. The script uses a named mutex and rejects a second concurrent invocation, preventing multiple agents from starting competing Cargo builds.

`.github/workflows/rust-production.yml` runs the complete Windows `Final` / `All`
battery for pull requests, pushes, and manual dispatches. It installs the same
pinned toolchain and nextest release, plus exact Python 3.13.14 through an
immutable `actions/setup-python` commit. The Python contract-test step repeats a
fail-closed exact-version check before invoking any test. The workflow then
invokes this script and has no independent Cargo build or test command.

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
- `Lock`: regenerate `Cargo.lock` serially for the selected top-level workspace
  or workspaces; accepts no package, filter, target, or timing selectors and
  never visits standalone workspaces.
- `Stats`: show sccache statistics without compiling.

Full extractor `Final` and `Release` runs also own the two intentionally
standalone Cargo surfaces, `project_progress_live_lane` and the occupation
replay verifier. Both modes also run the deterministic package-builder/receipt-
verifier self-test and static-CRT PE-import self-test before Cargo.
`Test-ProductionCaptureScripts.ps1` performs no Cargo work.
After a successful full `Release -Workspace All`, the
runner atomically writes
`.cargo-target/release/hoi4-production-prebuilt-artifacts.json`. The receipt
binds the current source/compiler-input digest, capture-driver and runner
hashes, workspace/package selection, and exact paths, sizes, and SHA-256s for
the five production capture artifacts plus `replay_app.exe`. Before building,
the runner removes all six canonical binaries and verifies their absence, so a
removed or renamed Cargo target cannot leave an old shared-target artifact that
receives a new attestation. The receipt records that freshness policy. It also
binds the complete `rustc -vV` identity; selected linker path, version, and file
hash; MSVC compiler/toolset; Windows SDK and import-library hashes; GitHub runner
image metadata when present; and a fixed allowlist of build environment values.
The environment is captured before and after the build and must remain byte-
identical. Local and CI releases therefore expose their differences instead of
sharing an ambiguous builder identity. The source digest
includes extractor, portable-container, and replay native inputs. The same
successful full invocation immediately revalidates that attestation, passes
only its `recorder_host`, `replay_app`, and `collector_dll` paths to
`New-PortableProductPackage.ps1`, and requires the builder's complete byte-level
verification of an exact five-entry, two-GUI-executable portable ZIP. It then
durably writes a same-directory candidate receipt binding the ZIP, builder,
artifact attestation, and source-input hashes. The runner then invokes
`Test-PortableProductPackageReceipt.ps1` against those exact candidate bytes,
which revalidates the current source attestation, exact canonical ZIP
identity/hash/shape, and all three packaged binary hashes. Only after that
independent check does the runner atomically move the candidate to
`.cargo-target/release/hoi4-portable-package-receipt.json` as its terminal
publication step. This final verified receipt is the promotion authority; neither
an artifact receipt nor an unreceipted ZIP is a distributable product.
The staged verifier result has a distinct non-promotable status; distribution
uses the verifier's default canonical-path mode and requires
`PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE` with `canonical_receipt=true`.

Static CRT admission uses PE metadata, not filename text scanning or build flags
alone. `PortableProduct.PeImports.ps1` maps RVAs through the section table and
walks both normal import descriptors and delay-import descriptors. A malformed
table or any case-insensitive `VCRUNTIME*.dll` import fails closed. The artifact
attestation records parsed DLL manifests for recorder, viewer, and collector;
the package builder repeats the check on its locked inputs and completed ZIP;
the receipt verifier repeats it on all three ZIP entries.
`Test-PortableProductStaticCrt.ps1` provides deterministic synthetic PE cases
for acceptance, normal-import rejection, delay-import rejection, and malformed
table rejection; central `Final` and `Release` own that test.
`Test-PortableProductPackageSelfTest.ps1` independently builds a deterministic
synthetic package, validates its complete attestation/receipt path, and proves
that altered ZIP and receipt bytes plus a wrong PE role fail closed. Because CI
invokes central `Final -Workspace All`, these package scripts are CI-gated.

Only an unfiltered `Release -Workspace All` may mint those two promotion
receipts and the ZIP. Extractor-only, replay-only, package-filtered, and
expression-filtered release runs never produce or claim a portable product;
an extractor-including release first removes any stale package receipt and its
current-version ZIP. Any attestation, packaging, verification, or receipt-write
failure removes the ZIP and both promotion receipts.

The initial portable product is intentionally unsigned: `signed=false` is part
of the exact receipt contract. The ZIP hash, embedded `SHA256SUMS.txt`, artifact
attestation, and package receipt provide local build/promotion integrity only;
they are not Authenticode, a publisher signature, or independently trusted
distribution provenance. Consumers must obtain this initial release and any
detached receipt through a trusted channel. Do not describe the internal hashes
as publisher authentication.

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

After workspace membership or path dependencies change, the primary agent may
refresh both top-level lockfiles once before locked validation:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Lock -Workspace All
```

The first build into the shared target directory is intentionally a cold build. Subsequent extractor and replay invocations reuse compatible artifacts.

Run the read-only structural audit with `python extractor/native/scripts/Audit-RustTests.py`. Candidates are not deletion instructions: exact duplicates and weak bodies require semantic review before removal. The current reviewed disposition is recorded in `extractor/native/scripts/TEST_SUITE_AUDIT.md`.
