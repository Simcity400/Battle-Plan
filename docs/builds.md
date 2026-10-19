# Rust builds and compilation boundaries

There is one build: plain `cargo build`, `cargo test` and `cargo nextest` all
use the `dev` profile. Workspace crates are optimized (`opt-level = 1`) and
compile incrementally, without debug information or cross-unit ThinLTO, with
debug assertions and overflow checks on. Building every workspace library from
scratch took 2,854 CPU-seconds without ThinLTO against 3,892 with it, and the
full suite ran no slower. Third-party crates are built once at `opt-level = 3`. Tests,
the full suite and the desktop app therefore share one set of artifacts under
`.cargo-target/debug`. Release builds are for shipping, live capture acceptance
and performance evidence.

No test is behind a feature: every test compiles in every build, and all but the
`#[ignore]`d ones run. Those are benchmarks or need hardware or inputs a run may not have
(a GPU, the shader compiler, a release build); run those by hand when you change
what they measure. Run the tests for what you changed, then the whole suite
before merging:

```powershell
cargo nextest run -p replay-frame
cargo nextest run -p dashboard event_feed
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
```

[nextest](https://nexte.st) (`cargo install cargo-nextest --locked`) runs each
test in its own process and schedules every test binary at once;
[.config/nextest.toml](../.config/nextest.toml) starts the tests that take tens
of seconds first, so none of them starts near the end of a run. The dashboard,
viewer, content and shader crates' unit tests run on mimalloc, the allocator
the viewer that hosts them ships with. Installation-dependent tests read `BATTLE_PLAN_GAME_DIR`. Because each test is
its own process, a test sets up everything it reads, such as executable
metadata, and never relies on another test having run first.
`cargo check --workspace --all-targets` checks the whole app without generating
machine code. Each worktree can select its own target directory with
`CARGO_TARGET_DIR`. Keep timings, captures and other evidence outside Git.

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
cargo run -q -p dashboard --features export-types --bin export-types -- --check
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
