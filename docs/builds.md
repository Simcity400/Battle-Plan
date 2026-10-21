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
what they measure. Run the tests for what you changed; the workspace lints and the
whole suite are there when a change reaches across crates:

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

Check both architectural boundaries and the feature union after dependency changes:

```powershell
python tools/check-build-dependencies.py
python tools/check-content-dependencies.py
python tools/check-build-dependencies.py --impact engine-decisions
python tools/check-feature-unification.py
```

The map viewer's native GUI code (`viewer-foundation`) reaches the dashboard only
through `dashboard-game-window`: no other dashboard crate may depend on it, and
the dashboard depends on it only through that crate, so editing the viewer
rebuilds no other product. The dashboard uses that code's CPU layout, text and
texture decoding only; it never opens a graphics device or a window there.

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

## One test build per crate

Cargo resolves features per command. `cargo test -p A` enables only what A's
dependency graph and A's own dev-dependencies request; `--workspace` enables what
every member requests, including each `test-support` feature another member's tests
need. A crate resolved with other features is another build, so before this rule the
same worktree compiled `engine-native`, `replay-data` and everything above them from
scratch once per package selection: agents' target directories held three to nine
configurations of each crate, and a `-p` check after a workspace check rebuilt
dozens of crates.

Every test build now resolves each crate it builds with the features
`cargo test --workspace` resolves, so `cargo nextest run -p <crate>`,
`cargo clippy -p <crate> --all-targets`, `cargo check --workspace --all-targets` and
the full suite share one build of each crate; after a workspace build, a `-p` test
build of any member compiles nothing but what changed.
[check-feature-unification.py](../tools/check-feature-unification.py) generates the
dev-dependencies that do this and owns them:

- each member's `[target.'cfg(all())'.dev-dependencies]` table names the workspace
  crates and the rarer third-party crates its tests build, with those features;
- `crates/test-unify` depends, with those features, on the third-party crates most
  test builds contain and on every build-script and proc-macro dependency whose
  features vary;
- `[workspace.dependencies]` declares `test-unify` and each rarer third-party crate
  once, between generated markers.

Equal features are not quite enough. Without `--target`, Cargo builds a crate the build
needs for both build scripts and the target, with the same features, once with the
target's profile, and proc-macros and build scripts link that build. A selection that
builds such a crate (`unicode-ident`, for one) only for build scripts builds it with the
build-script profile instead, and so rebuilds every proc-macro above it and everything
that uses them. The check therefore also requires a target build wherever the
workspace's test build has one.

Only test builds read dev-dependencies. `cargo build -p <product>` and checks without
`--all-targets` resolve exactly the features they did before, so shipped binaries
and their release builds are unchanged. Those builds still differ from test builds:
test-only features such as `test-support` and `alloc-fault-injection` stay out of
them by design.

Run the check after changing dependencies or features; it resolves `cargo tree` for
every member and takes a few minutes. When it reports differences, regenerate:

```powershell
python tools/check-feature-unification.py --fix
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

## Warm worktrees

A new worktree's target directory is empty, so its first cargo command compiles the
whole workspace. [tools/warm_target.py](../tools/warm_target.py) starts it from a seed
instead: the build of a clean checkout at a known commit, recorded with that commit,
the toolchain and the lockfile hash.

```powershell
# Once, with `--like`; then to follow main, `--if-moved`, which does nothing when the seed is
# at origin/main already, another refresh runs or the game runs. Fetches, checks out
# origin/main and runs `cargo check --workspace --all-targets`, `cargo test --workspace
# --no-run` (what `cargo nextest run --no-run` builds) and `cargo clippy --workspace
# --all-targets -- -D warnings` in the seed's spare slot, going on past crates that fail.
python tools/warm_target.py seed --root seed --like C:\Users\Example\.t3\worktrees\battle-plan\t3-00000000
python tools/warm_target.py seed --root seed --if-moved
# In a new worktree, before its first cargo command; the profile directory goes to the
# listed directory with the most free space, behind a junction at .cargo-target\debug, and
# a seed behind origin/main is refreshed in the background:
python tools/warm_target.py warm --root seed --target-roots "C:\hoi4-dev;F:\hoi4-dev" --refresh-behind
# Lists references to the seed in a target directory (exit code 1 if there are any):
python tools/warm_target.py check --root seed --target-dir .cargo-target
```

With [one test build per crate](#one-test-build-per-crate), those commands also cover
`-p` test and clippy selections. `seed` keeps only the units its commands used and
drops rustc's incremental caches (`--keep-incremental` keeps them). `warm` copies the
seed's build, about 11 GB, into the worktree's empty target directory (the one cargo would
use, so `CARGO_TARGET_DIR` is followed) with the Windows copy engine, which clones blocks
instead of copying them on ReFS (a Dev Drive). With `--target-roots`, it skips the copy
when the chosen drive would keep less than `--min-free-gb` (60) free, and the next `warm`
deletes profile directories whose worktree junction is gone. Incremental caches and
cargo's uplifted copies stay behind, and nothing is hard-linked: lld rewrites small
executables in place, so a relink in the worktree would change the seed. Then the copy is
made the worktree's own:

- Cargo decides freshness from mtimes, and a checkout gives every file a new one. Once
  the copy is in place, each file cargo may compare (what a seeded unit's dep-info lists,
  including files outside its package, build scripts' `rerun-if-changed` paths, and every
  file a package holds) whose bytes equal the seed checkout's gets the seed file's mtime;
  every other one gets the current time. Committed, uncommitted and untracked changes are
  compared alike, so the crates that differ, and everything that depends on them, rebuild,
  and nothing else does. Files written after `warm` started keep their own mtimes.
- Compiled files name the checkout and target directory they were built in: source paths
  in crate metadata, panic locations, `env!("CARGO_MANIFEST_DIR")`, build-script output,
  object paths in PDBs. Cargo tracks none of these, so a copied unit would go on reading
  the seed's files. `seed` records every place its build wrote either path. `warm`
  rewrites cargo's and rustc's own records (dep-info, fingerprints, build-script output)
  and patches other files in place where the worktree's path has the same length. A unit
  with any other file naming the seed (another length, a path cut across PDB pages, a
  path reached through `..` from the checkout, an environment value rustc read) is
  removed, so cargo rebuilds it and everything above it. Nothing in the copy names the
  seed afterwards: the seed can be rebuilt or deleted at any time.
- `--like` gives the seed checkout the length and depth of the worktrees it serves, which
  is what allows patching in place. A worktree or target directory of another shape
  rebuilds most of the workspace and of its third-party crates.
- The desktop and recorder apps always rebuild: Tauri's context macro embeds their
  configuration and frontend at compile time without listing them in the dep-info.

`warm` does nothing, and says why, when the target directory already holds a build, the
toolchain differs from the seed's or no seed is ready; a different `Cargo.lock` only
rebuilds the units it changes. A seed has two slots, each a checkout of the seed's shape
and its build: `seed` rebuilds the slot warms are not using (a new slot starts as a warm
copy of the other), waits up to `--reader-wait` minutes for warms still copying from it,
and only when its build is indexed points warms at it, so a refresh never leaves new
worktrees without a seed. An interrupted or failed `seed` leaves the slot it was building
unused, and the next `seed` takes over its lock. A copy made while its slot changed is
dropped, and if cargo starts building in the directory before the copy is in place,
cargo's build is kept and no source is backdated; the next `warm` of any worktree deletes
a copy an interrupted `warm` left. Run cargo in a seed checkout only through `seed`:
`warm` refuses a slot whose build changed after it was recorded.

`warm --refresh-behind` (what new threads run) refreshes the seed as main moves. After
warming, if the seed's commit is not the worktree's `origin/main`, no refresh holds the
lock and `hoi4.exe` is not running (benchmarks and oracle runs), it starts
`seed --if-moved -j 8` with the active slot's copy of this tool and returns at once. The
refresh runs detached from the thread's process and console, below normal priority
(cargo and rustc inherit it, so agents' builds come first), and appends its output to
`refresh.log` beside the seed; cargo's output goes to `slots\<slot>\logs`. To stop one,
end its process tree: `taskkill /T /F /PID <pid>` with the pid in the seed's
`updating\pid`. The slot warms use is never touched, and the next refresh starts over.
