# Rust builds and compilation boundaries

Plain `cargo build` and `cargo test` use unoptimized workspace libraries with
incremental compilation and no debug information. Compatible dependency
artifacts are reused. `cargo check --workspace --all-targets` checks the whole
app without generating application machine code; build scripts, procedural
macros and their native helpers still compile. Select a crate for the everyday edit loop:

```powershell
cargo check -p collector-projection --all-targets
cargo test -p replay-frame
```

`cargo build --profile interactive` preserves optimized replay loading, native
readers, content readers and dashboard processing for running the app. The
desktop development launcher (`npm.cmd run dev`) selects this profile for both
the host and viewer, keeping their executables together under
`.cargo-target/interactive`. Third-party dependencies keep the existing settings;
native zstd decompression stays optimized in every development profile.

```powershell
cargo build --profile interactive -p viewer -p replay-desktop
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --profile full --features slow-tests
```

`full` inherits the interactive optimization settings for the long scale suite.
Release and release-incremental builds retain their existing purposes; live
capture acceptance and performance evidence use release builds. Each worktree
can select its own target directory with `CARGO_TARGET_DIR` without changing
tracked configuration. Keep timings, captures and other evidence outside Git.

## Dependency ownership

Consumers name the libraries that own the modules they use. The `engine` and
`installed-content` facades remain component indexes and integration-test hosts; internal
consumers do not depend on them. An edit to an unrelated native or content
reader therefore does not invalidate every consumer of its facade.

The recording contract is layered:

- `replay-identity` owns portable content identity without native readers.
- `replay-data` owns typed rows, table registration, codecs, cadence and terminal
  witness bytes. Table counts belong to the row registry.
- `replay-frame` owns frame encoding, reconstruction, shared update columns,
  provenance, branch continuity, checkpoints and the shared spool envelope.
- `replay-derive` owns playback calculations over recorded operands.
- `replay` owns compressed containers, spool files, recovery and publication.

Frames depend on data. Derivations and compressed recording I/O can compile
independently above frames. Capture helpers depend on data or frames as needed;
only the collector host needs recording I/O. Shared test fixtures are enabled
from dev dependencies through their owning crate's `test-support` feature.

The frame encoder reserves the existing spool envelope in its own buffer. Its
`EncodedFrame::into_spool_record` seals that buffer without a copy; recording
I/O reads frame metadata through accessors. Immutable column APIs keep their
storage private across the frame/container boundary. The split changes neither
recorded bytes nor admission, allocation-failure or baseline rules.

Check both architectural boundaries after dependency changes:

```powershell
python tools/check-build-dependencies.py
python tools/check-content-dependencies.py
python tools/check-build-dependencies.py --impact engine-decisions
```

The impact report includes possible normal-build rebuilds, including optional
and target-specific edges. It excludes dev-only edges. Cargo can reuse more
work when an edit leaves public metadata unchanged; the report is a structural
upper bound rather than a timing prediction. `cargo check --timings` and
`cargo build --timings` show the actual critical path for the selected command.

The restructuring was measured on this Windows workstation with the pinned
compiler. A dependency-generating `cargo build -p collector-projection` took
210 seconds before the change and 56.6 seconds with the fast default profile.
These observed command times include the deliberate change from optimized to
unoptimized development code and more cached third-party artifacts in the second
run; they are not a controlled cold-build comparison or a claim about optimized
release builds. The initial full workspace check took 114 seconds including
third-party work, so later warm-cache checks are not directly comparable.

The manifest graph gives a cache-independent measure of narrower rebuilds:
`engine-decisions` reaches five normal-build crates instead of 163, and
`installed-content-focus` reaches 38 instead of 92. Shared native foundations and row
contracts still correctly reach most of the application.

The dashboard's preparation engine uses borrowed callback interfaces internally.
Its public wrappers still accept closures, but each distinct progress or commit
callback no longer creates another copy of the entire preparation engine. The
callbacks keep their order and error handling and require no heap allocation.

TypeScript contract assembly and declaration callbacks in the product registry
compile only for dashboard tests or its `export-types` feature. Runtime hosts
keep the product loaders without retaining the contract generator. Regenerate
or check the frontend contract explicitly:

```powershell
cargo run -q --profile test -p dashboard --features export-types --bin export-types -- --check
```

## Linker and shared features

Windows builds use Rust's bundled `rust-lld`, without an extra LLVM installation.
It supports the MSVC libraries, resources, DLL exports and x64 SEH this project
uses. Local probes exercised MSVC libraries, DLL exports, SEH and Rust unwinding;
the full workspace build also linked the application executables. Controlled
warm-input linker replays using this app's replay-data library took 317 ms
instead of 693 ms with ordinary dead-code removal, and 583 ms instead of
1,026 ms with its generated code retained. These are linker-only medians,
not whole-build speedups. See [Rust's linker option](https://doc.rust-lang.org/1.96.0/rustc/codegen-options/index.html#linker)
and [LLD Windows support](https://lld.llvm.org/windows_support.html).

Workspace Serde dependencies share `alloc`, `derive` and `rc`. These additive
capabilities keep the library's feature identity consistent when switching
between a collector crate, viewer and desktop; they do not change existing
serialized values. Cargo's resolver still separates host build dependencies.

## Experimental options

Keep the pinned stable toolchain as the default. For isolated experiments,
select a separate target directory so nightly artifacts cannot replace stable
or release evidence. Compare each setting with the same nightly compiler and
the same cache state; changing toolchains alone invalidates artifacts.

Cargo's [workspace feature unification](https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification)
can prevent more feature-dependent rebuilds between package selections:

```powershell
$previousUnification = $env:CARGO_RESOLVER_FEATURE_UNIFICATION
try {
    $env:CARGO_RESOLVER_FEATURE_UNIFICATION = 'workspace'
    cargo +nightly -Zfeature-unification check -p replay-frame --target-dir .cargo-target/nightly-check
} finally {
    $env:CARGO_RESOLVER_FEATURE_UNIFICATION = $previousUnification
}
```

The installed 1.101 nightly also supports a parallel frontend through
`-Zunstable-options --jobs-frontend=4`; stable 1.96 does not. Try two or four
frontend jobs in a separate lane and account for Cargo's concurrent crates and
memory use:

```powershell
$previousRustflags = $env:RUSTFLAGS
try {
    $env:RUSTFLAGS = "$previousRustflags -Zunstable-options --jobs-frontend=4".Trim()
    cargo +nightly check -p replay-frame --target-dir .cargo-target/nightly-parallel
} finally {
    $env:RUSTFLAGS = $previousRustflags
}
```

Neither nightly option has been timed on this workspace. They are experiments,
not promised speedups or release settings.
If `CARGO_ENCODED_RUSTFLAGS` is set, it takes precedence over `RUSTFLAGS`; append
the experimental flags there with its unit-separator delimiter instead.

[Headstart](https://github.com/PowderworksCode/headstart) needs patched Cargo and
rustc, rather than a configuration switch. Its own
[readiness assessment](https://github.com/PowderworksCode/headstart/blob/main/docs/readiness.md)
says it is not ready to merge, records an unenforced metadata-swap invariant and
unbounded paused-compiler memory, and says Windows has not been tested. Keep it
outside the supported toolchain until those requirements are resolved.

[sccache](https://github.com/mozilla/sccache/blob/main/docs/Rust.md) is useful for
repeated nonincremental library builds, particularly CI. It cannot cache checks,
incremental compilation, binaries, proc macros or DLLs. Keep incremental
compilation enabled for local edits; disabling it globally to obtain cache
hits can make that loop slower. Additional linker and codegen settings need
measurements: Cargo already sizes its jobs from logical CPUs and uses 256
codegen units for incremental builds. Cranelift's Windows unwind limitations
make it unsuitable for this recorder's default backend.
