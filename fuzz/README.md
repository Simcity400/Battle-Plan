# Recording fuzzing

This independent cargo-fuzz package is excluded from the main workspace. Normal
workspace builds need neither nightly nor libFuzzer. Inputs and generated seeds
are synthetic; the generator reads no game installation. Corpus, crash artifacts,
coverage and build output stay ignored.

| Target | Public reader paths |
|---|---|
| `open_recording` | Header, footer, trailer CRC, metadata and block index admission. |
| `partial_trailer` | `Replay::read_partial_from` over an in-memory seekable recording. |
| `read_blocks` | Whole and selected block reads, frame baseline reconstruction and seeks. |
| `dashboard_metadata` | `Source::from_replay` and its exposed recording metadata. |

Block and dashboard targets also compress arbitrary raw inputs into a valid
synthetic container with a recomputed trailer CRC. This reaches frame and nested
row decoding directly. The raw wrapper derives matching index hour ranges from
complete payloads, and synthetic snapshot/delta sequences reach multi-hour
baseline reconstruction. Container mutations still exercise compressed zstd frames
and their framing checks. Errors are ordinary outcomes; panics, crashes and
sanitizer findings are failures and are not caught by the targets.

## Run

Use a nightly toolchain at least as new as the workspace's Rust requirement, a
supported cargo-fuzz platform and the native build prerequisites of the replay
and dashboard crates. Install cargo-fuzz and generate the small corpus from the
repository root:

```powershell
rustup toolchain install nightly
cargo install cargo-fuzz --locked
cargo +nightly run --manifest-path fuzz/Cargo.toml --bin seed-corpus
cargo +nightly fuzz run open_recording
cargo +nightly fuzz run partial_trailer
cargo +nightly fuzz run read_blocks
cargo +nightly fuzz run dashboard_metadata
```

The generator writes nine container seeds per target (valid, partial, empty,
truncated, wrong magic, wrong CRC, huge raw length, huge trailer count and snapshot/delta sequence), plus
raw single-frame and snapshot/delta seeds for the block and dashboard targets. Regenerate after changing
the current writer's contracts. Preserve readable historical synthetic seeds
when testing compatibility; do not copy shared or installed game recordings into
the corpus.

cargo-fuzz uses AddressSanitizer by default. zstd decoding also runs in native C;
Rust sanitizer flags alone do not instrument that library. For a native-code
campaign on a cargo-fuzz-supported Unix toolchain, rebuild with Clang and C
sanitizer flags, for example:

```sh
CC=clang CXX=clang++ CFLAGS='-fsanitize=address -fno-omit-frame-pointer' \
  CXXFLAGS='-fsanitize=address -fno-omit-frame-pointer' \
  cargo +nightly fuzz run read_blocks
```

Check that the native compiler accepts these flags and that the resulting
zstd-sys objects are instrumented. A Windows desktop build alone does not verify
a sanitizer campaign. Run a short seeded campaign after replay/dashboard reader
changes and longer campaigns before sharing releases. Minimize any crash with
`cargo +nightly fuzz tmin <target> <artifact>` and turn it into a synthetic
regression test.
