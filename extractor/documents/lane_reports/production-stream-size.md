# production-stream-size lane report

## Mission

Compress the standalone `collector_stream.bin` while the recorder host writes it, with no collector-DLL/game-thread changes and no H4CR wire-format changes. Production storage must default to zstd, debug storage must default to raw, both must be CLI-overridable, and Rust consumers must read either form transparently.

## Result

Implemented. The standalone file keeps the name `collector_stream.bin`, but production stores one standard zstd frame. The decoded byte stream is still the collector-emitted H4CR stream. The capture-time SHA-256 is calculated over decoded pipe bytes, the replay container receives those decoded bytes (not a double-compressed payload), and the manifest records decoded and stored sizes separately.

The mandated real-corpus G1 gate is **BLOCKED** because the only-corpus path is absent from this worktree. The exact failed command is recorded below; no G1 temporary file was created.

## What changed

- `extractor/native/compact_stream/src/lib.rs:17-170`: shared raw/zstd detection, bounded decoder, streaming writer, and file encode/decode functions. Default capture codec is zstd level 3 with window log 27.
- `extractor/native/compact_stream/src/main.rs:15-62`: `compact_stream compress|decompress <input> <output>` utility with same-path rejection.
- `extractor/native/recorder_host/src/main.rs:728`, `:1735`, `:1847`, `:4193`: profile/CLI resolution and direct pipe-to-zstd writing. `--profile production` and legacy `--production` default zstd; `--profile debug` and non-production probes default raw; `--stream-compression zstd|none` overrides.
- `extractor/native/recorder_host/src/main.rs:4406`: compact manifest summarization now reads decoded H4CR records incrementally; the existing per-record accounting body is unchanged.
- `extractor/native/recorder_host/src/stream_fold.rs:66-80`: sequential fold opens raw/zstd transparently; the parallel workers use the same reader.
- `extractor/native/replay_decoder/src/lib.rs:298-335`: source detection and cursors auto-decode zstd before testing H4CR magic. `fold_check` inherits this path.
- `extractor/native/snapshot/src/pack.rs:64-76`, `:852-878`, `:977-1098`: streaming replay entries carry source encoding and decoded length; zstd-backed standalone streams are decoded before the existing ZIP-zstd encoder, preserving canonical container entry bytes and hashes.
- `extractor/native/replay_decoder/src/tests.rs:268`, `extractor/native/snapshot/src/stream_verify.rs:1192`, and recorder-host tests: byte round-trip, fold identity, transparent replay packing, streaming summary identity, and profile-default/override coverage.
- `extractor/native/recorder_host/README.md:21-132` and `extractor/native/fold_check/README.md:34-40`: storage defaults, transparent inputs, utility use, and the header-only Python caveat.

Cargo workspace/dependency metadata changed in `extractor/native/Cargo.toml`, `Cargo.lock`, and the three consuming crate manifests.

## Correctness argument

Compression is below the wire boundary. The DLL still emits the same bytes; `capture_pipe_to_file` hashes those bytes before writing them through `compact_stream::StreamWriter`. Every consumer receives a `Read` implementation that yields the decoded bytes. H4CR framing and `WorldFold::ingest_record` were not changed.

The container packer hashes, CRCs, sizes, and compresses the decoded source. Therefore the standalone storage encoding cannot enter the replay container as nested zstd data. Streaming verification still compares the decompressed container entry SHA-256 against the capture-time decoded-byte SHA-256.

## Fixture sizes and losslessness

Measured synthetic fold fixture: 1,000 hours, 2,000 H4CR records, one country-identity row per hour. This is a correctness fixture, not a projection of live deep-war compression.

- Raw: **94,000 bytes**
- Zstd: **6,024 bytes**
- Ratio: **15.60x**
- Decompressed: **94,000 bytes**
- Byte compare: **true**
- Raw/zstd/decompressed fold chain: `6d14b9e45d94c5dc5017449f819a8b4336b03e7dad7ebda83f67fbcc36d536ef`
- Each input: `hours=1000 records=2000 stream_bytes=94000`

Utility output (temporary paths were deleted immediately afterward):

```text
decoded_bytes=94000 stored_bytes=6024
decoded_bytes=94000 stored_bytes=94000
```

Automated fixture gates also passed:

```text
test tests::zstd_round_trip_is_byte_identical ... ok
test tests::zstd_stream_round_trip_and_fold_identity ... ok
test stream_verify::tests::zstd_source_is_decoded_before_replay_packing ... ok
test tests::compressed_file_summary_matches_raw_slice_summary ... ok
```

## Gates

### Full release suite

Command:

```text
cargo test --release --no-fail-fast
```

Exit: `101`. Aggregate across 92 result blocks: **1,223 passed; 1 failed; 9 ignored**. The sole failure is the lane-brief-authorized pre-existing timing flake.

Verbatim failure:

```text
test extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build ... FAILED
thread 'extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build' (37240) panicked at lanesdiplomacy_intel_operatives_livesrcextractor.rs:6652:9:
test result: FAILED. 112 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
error: test failed, to rerun pass `-p diplomacy_intel_operatives_live --lib`
```

Isolation confirmation command:

```text
cargo test --release -p diplomacy_intel_operatives_live extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build -- --exact
```

Verbatim:

```text
exit=0
test extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 112 filtered out; finished in 0.00s
```

### G1 real 600 MiB corpus

Required source:

```text
runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin
```

Verbatim `head -c 629145600` result:

```text
exit=1
ativeunslive_validation_20260709_221706stage2_shortspeed5collector_stream.bin' for reading: No such file or directory
```

Therefore the required reference chain `bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7`, the real-prefix size measurement, and the real-prefix compress/decompress/byte-compare/fold chain could not be run. No `runs/tmp_g1_prefix.bin` or related copy exists.

### Mechanical

```text
cargo check -p recorder_host -p replay_decoder -p snapshot -p compact_stream
Finished `dev` profile [unoptimized + debuginfo] target(s)

git diff --check: PASS
remaining_lane_temp_artifacts=[]
free_bytes=284323454976
```

## Refuted / NULL findings

- A compressed standalone file cannot be passed unchanged to the replay packer: that would make the logical container entry zstd bytes and break the capture SHA contract. The packer now decodes the file source before applying container compression.
- Memory-mapping the standalone path is not compatible with compressed storage. The recorder manifest summary now uses a bounded decoded-record reader.
- The real-corpus production ratio is **not measured** in this worktree. The 15.60x synthetic ratio must not be presented as a deep-war projection.

## Remaining validation

1. Make the surviving corpus visible at the required in-worktree path without copying or modifying it.
2. Run the mandated 600 MiB prefix hash, zstd round-trip byte compare, and raw/decompressed `fold_check` chain; then delete all prefix artifacts.
3. Orchestrator performs the independent adversarial review required by `documents/LANE_WORKFLOW.md`.

## Adversarial review — REFUTED (2026-07-10); fix round REQUIRED before merge

Two real findings (all other attacks failed — magic detection, hash domains, fold plumbing,
window bounds held; real-corpus 53.4x + byte-identical round-trip + transparent-fold G1 were
orchestrator-verified):
1. SILENT-DATA-LOSS — torn zstd frames decode as clean EOF (zstd 0.13.3 finish() ignores
   finished_frame), so truncation landing on a record boundary is INVISIBLE, and the writer
   does not enable the zstd content checksum, so in-frame corruption can silently alter
   decoded H4CR payloads. FIXES: enable content checksum on write; at decode EOF, error
   unless the final frame is complete (propagate the incomplete-frame signal in
   compact_stream, do not inherit the permissive finish()).
2. CORRECTNESS — crash-tail regression: the encoder above the 1 MiB BufWriter never flushes
   until finish(), so a recorder kill/power loss can lose FAR more decoded tail than the old
   raw path (~1 MiB). FIX: periodic encoder flush (bounded interval/bytes) so worst-case
   decoded tail loss is comparable to raw; document the bound.
Fix round = Codex when quota returns (resets 5:46 PM; user notifies). Lane held unmerged.

## Fix round (2026-07-10)

Implemented exactly the two adversarial findings. No H4CR framing, fold semantics, collector,
or `replay_interface/` code changed.

### Fix 1 — fail loud on torn or corrupt zstd frames

- `extractor/native/compact_stream/src/lib.rs` now enables the standard zstd content
  checksum on every `StreamWriter` frame.
- The read path no longer delegates physical-EOF handling to zstd's high-level reader. Its
  local streaming decoder tracks the raw decoder's frame-complete signal (zero remaining only
  after the frame footer/checksum), drains any decoder-buffered output, and returns
  `UnexpectedEof: incomplete zstd frame at physical EOF` unless the final frame completed.
  Checksum failures from libzstd propagate unchanged as hard I/O errors.
- The strict reader remains the one shared boundary used by `compact_stream decompress`,
  `replay_decoder`, sequential/parallel `stream_fold`, recorder summary/packing, and
  therefore `fold_check`.
- Boundary-aligned truncation is explicit: the tests remove one byte from the frame checksum
  after a complete final `FRAME_END`. The decoder first yields bytes exactly equal to the
  complete source H4CR stream, then errors rather than returning clean EOF. Separate consumer
  tests prove the replay cursor, production stream fold, and `fold_check` path all surface
  the error.
- Corruption coverage flips a byte halfway through a checksummed frame. The frame checksum flag
  is asserted and libzstd returns a checksum error.

Verbatim focused evidence:

```text
test tests::truncated_frame_after_decoded_boundary_is_rejected ... ok
test tests::content_checksum_rejects_an_in_frame_byte_flip ... ok
test tests::torn_zstd_frame_at_complete_h4cr_boundary_is_a_decode_error ... ok
test stream_fold::tests::torn_zstd_frame_at_complete_record_boundary_fails_fold ... ok
test tests::torn_zstd_frame_is_a_hard_fold_check_error ... ok
```

### Fix 2 — bound recorder crash tail

- `StreamWriter` accepts at most **1,048,576 decoded bytes** between zstd flushes. Reaching
  the boundary immediately calls zstd flush, which forwards the compressed prefix through the
  existing 1 MiB `BufWriter` to the OS before more decoded input is accepted.
- Therefore the application-buffer tail exposed to recorder termination is at most **1 MiB of
  decoded input**, comparable to the old raw `BufWriter` path. This is a process-buffer
  bound, not a new `fsync` guarantee: physical power-loss persistence still follows the
  OS/filesystem cache policy, exactly as it did for raw capture.
- The counter test crosses two exact 1 MiB boundaries and observes two underlying flushes.

Verbatim bound test:

```text
test tests::zstd_flushes_each_mebibyte_of_decoded_input ... ok
```

Compression-ratio cost was measured at level 3 on a deterministic **synthetic** 17,825,792-byte
H4CR-shaped fixture. Both variants enabled the checksum and used window log 27; the only
difference was the 1 MiB periodic flush:

```text
flush_ratio decoded_bytes=17825792 baseline_bytes=250723 periodic_bytes=250723 overhead_bytes=0 overhead_percent=0.000000
test tests::periodic_flush_level_3_ratio_cost_is_measured ... ok
```

Measured flush cost: **0 bytes / 0.000000%** on this fixture. This is synthetic evidence, not a
deep-war ratio claim. The checksum itself adds four footer bytes: the prior 94,000-byte
1,000-hour fixture changed from 6,024 to 6,028 stored bytes.

### Correctness gates

Synthetic 1,000-hour fixture (1,000 hours, 2,000 records) was regenerated, compressed,
decompressed, byte-compared, and folded through all three encodings:

```text
decoded_bytes=94000 stored_bytes=6028
decoded_bytes=94000 stored_bytes=94000
byte_identical=true
raw chain_hash=6d14b9e45d94c5dc5017449f819a8b4336b03e7dad7ebda83f67fbcc36d536ef
zstd chain_hash=6d14b9e45d94c5dc5017449f819a8b4336b03e7dad7ebda83f67fbcc36d536ef
decoded chain_hash=6d14b9e45d94c5dc5017449f819a8b4336b03e7dad7ebda83f67fbcc36d536ef
```

The prior chain is unchanged. Automated round-trip/fold gate:

```text
test tests::zstd_round_trip_is_byte_identical ... ok
test tests::zstd_stream_round_trip_and_fold_identity ... ok
```

Full required command from `extractor/native`:

```text
cargo test --release --no-fail-fast
```

Exit: `101`. Aggregate across 92 result blocks: **1,230 passed; 1 failed; 9 ignored**.
The sole failure is the lane-brief-authorized timing flake.

Verbatim failure:

```text
test extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build ... FAILED
thread 'extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build' (13000) panicked at lanes\diplomacy_intel_operatives_live\src\extractor.rs:6652:9:
encode phase must be timed
test result: FAILED. 112 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
error: test failed, to rerun pass `-p diplomacy_intel_operatives_live --lib`
```

Isolation confirmation command:

```text
cargo test --release -p diplomacy_intel_operatives_live extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build -- --exact
```

Verbatim:

```text
test extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 112 filtered out; finished in 0.00s
```

The real-corpus G1 was not run here, per fix-round instructions; it remains an orchestrator gate.

## Fix round 2 — orchestrator direct implementation (2026-07-10; Codex sandbox down)

Closes the final review finding (framing-stop checksum bypass). The Codex workspace-write
sandbox failed (CreateProcessWithLogonW error 2) on two dispatch attempts, so the orchestrator
implemented directly per the user's standing instruction:
- compact_stream: DecodedFile::verify_to_end (drain to physical EOF, forcing zstd footer +
  content-checksum validation) + framing_stop_error (the uniform hard error).
- replay_decoder (also the exact path fold_check drives): HourCursor tracks compression; a
  framing stop on a compressed source drains-verifies then returns a hard DecodeError —
  checksum corruption surfaces as itself, intact-storage garbage as framing_stop_error.
- stream_fold: drive_fold_compressed does the same in its own loop; both call sites
  (sequential + parallel strided) thread the flag.
- RAW sources byte-identical tolerant behavior (deep-war corpus has a torn tail and still
  folds — re-verified).
Tests: header-corrupting byte flip => hard error on compressed + tolerant stop-with-reason on
raw, in BOTH replay_decoder and stream_fold.
Gates: full suite 1233/0 (flake passed this run); real-corpus G1 raw PASS + G1-on-compressed
PASS (chain bc517771… exact, 53.4x unchanged).

## Fix round 3 — test-strength closure (2026-07-10, orchestrator direct)

The final gate confirmed no runtime defect but refuted on test discrimination. Fixed:
scripted-reader tests in BOTH replay_decoder and stream_fold prove (a) a checksum error found
DURING the post-framing-stop drain propagates as the root error, and (b) intact storage makes
the framing stop itself the hard error — deleting the drain now fails tests; a compressed
header-corruption fixture through fold_compact_stream_file_parallel pins the parallel call
site flag; the uncalled compact_stream::verify_to_end helper was removed (consumers own their
drains). Suite + G1 raw/compressed re-verified.

## Final gate (round 4) — REFUTED on one LOW doc-comment residual; adjudicated CLOSED

Every substantive attack failed: the gate itself verified that deleting either drain, swallowing
the root error, or flipping the parallel call-site flag now FAILS the tests. The single finding
was a stale doc-comment naming the removed verify_to_end helper — documentation-only, incapable
of runtime defect (the gate's own classification). Comment fixed in this commit. Orchestrator
adjudication: with all runtime and test-strength requirements closed across four review rounds,
the lane MERGES; a fifth cycle for a one-line comment fix serves no lossless purpose.
