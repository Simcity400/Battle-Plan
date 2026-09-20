# replay

The shared recording contract. This crate owns tables, rows, frames, cadence, sample provenance, spool records, replay containers, branches and codecs. Recording and playback must agree here; consumers must not invent their own interpretation of the bytes.

## Sources of truth

Start with [src](src/), especially [format.rs](src/format.rs) for format support and [cadence.rs](src/cadence.rs) for scheduling. Keep version numbers, table counts, IDs and slot assignments in code rather than copying catalogs into READMEs.

Rows preserve engine-native identifiers and numeric representations. Encoding is explicit and lossless. Reject malformed, out-of-range or noncanonical inputs rather than silently coercing them. Compression and deltas may save space; reduced precision may not.

## Values, availability and freshness

Country references in rows are not necessarily canonical slot numbers. Resolve raw references through the frame's `CountryLinks`; physical country slots can be reused. Preserve the recorded raw ID and do not turn an unresolved link into a guess about the current country.

A frame belongs to one hour on one branch. Tables can be snapshots, deltas, unchanged, truncated or unavailable. Missing data is not an empty successful table. After losing a complete baseline, a later successful table needs a snapshot before deltas can resume.

Value equality and sample freshness are separate:

- A fresh successful acquisition may equal the previous values and encode as `Unchanged`, while advancing its sample origin.
- A carried table retains the original sample hour, even though it appears in a later frame.
- Failed, truncated or skipped acquisition must not be marked as a fresh complete sample.

Keep those distinctions through encoding, decoding, inspection, analytics and oracle joins. Do not infer freshness from a sibling table or the current frame's hour. Startup, branch/load transitions and keyframes must retain the full-refresh and self-contained-baseline behavior required by the format.

Consumers that need only a few tables can use `FrameDecoder::for_tables`. It applies the same baseline and recovery rules to those tables and reports omitted tables as unavailable. The selection is fixed for the decoder lifetime; ordinary `FrameDecoder::new` still reconstructs every table. `Replay::read_block_selected` complements this by retaining only selected updates during parsing. All updates still go through the existing versioned decoder and validation before unselected ones are discarded; frame ordering, bounds, keyframe and compression checks remain shared with the full reader. The decompression buffer reserves the bounded size from the block index once. Neither selection API changes the recording format.

## Compatibility and recovery

A format change updates all affected writers, readers and consumers together. Preserve deliberately supported finalized formats, including their historical cadence and provenance semantics. A table that predates its format is not applicable, not recorded-zero. Unsupported formats, incompatible spool records and unsupported old-file appends must fail explicitly. Removing a live field must not break consumption, alignment or validation of that field in a supported legacy payload.

Spool recovery keeps valid CRC-checked prefixes. Replay finalization and append must not destroy an existing good file when interrupted. Preserve session/branch identity and never join branches through merely probable parentage.

The replay stores content identity, ordered layer hashes and the coverage policy, not game assets or machine-specific paths. Asset resolution belongs in [installed-content](../installed-content/README.md).

## Checking changes

Use existing round-trip, delta/baseline, malformed-input, compatibility, provenance, branch and recovery tests. Validate affected collector, recorder, viewer and analytics consumers in the same change. See [AGENTS.md](../../AGENTS.md) for commands.
