# Binary reply readers

This module validates the bounded H4RQ schema-1 reply layout before returning
zero-copy views into one received `ArrayBuffer`. The Rust `wire` crate owns the
writer, shape registry, TypeScript generator and shared synthetic byte fixtures.
The existing dashboard JSON transports remain independent of this module.

`decodeReply` takes a generated `ShapeContract`, the expected query identity and
normalised request bounds, and the generated shape registry for embedded
diagnostics. A failed reply raises `WireError` with its named code and query id;
no partial result escapes. Bounds belong to the complete request and row-unit
positions remain separate from hour, viewport and rank bounds.

Plain numeric arrays, floating point bit patterns, u128/i128 limbs, offsets,
state buffers and byte strings point into the original reply. Packed and
run-length columns expose lazy logical scalar access without expanding rows.
State layers and union gates determine which payloads can be interpreted;
inactive storage remains subject to layout validation. General hours and all
stream positions use exact `bigint` values. Chart run hours use native i32
arrays. `originalIdentity` walks parent offsets and list extents to recover the
original row and every nested element coordinate.

Descriptor graph traversal reads child ranges directly and checks unique
ownership before allocating child views, using storage linear in descriptors
and buffers. Validation propagates coalesced activity ranges once through
descriptor order and list offsets. Run-length payloads are checked only at
encoded runs intersecting those ranges, without expanding logical rows.

Text validation reads bytes without constructing strings. Byte extents may
split UTF-8 scalars at either open boundary; complete values remain strict.
`displayText` decodes reconstructed values and preserves a leading U+FEFF.

Fixed-length list validation compares an extent's total with the contract and
checks its local slice against the extent. Complete values without an extent
use their local length. Extents are indexed by column and occurrence.

`ColumnView.logicalLength` is the exact u64 logical count. Scalar access accepts
`number` or `bigint` indexes. The convenient numeric `length` getter raises
`size-overflow` if a compressed column's count cannot be represented exactly as
a JavaScript number; decoding and BigInt access still work. Ordinary array
indexes are converted only after checking their received byte bounds.

`dictionary.ts` validates BLAKE3-256 shard identities, reference metadata,
contiguous page directories, zero padding, canonical u64 entry coordinates and
UTF-8 for text-based inventories. Page v1 is an entry count followed by u64
offsets and entry bytes; plain entries retain arbitrary bytes. Entry byte
extents are validated independently and retain their original positions and
completion state. `checksum.ts` implements the standard seedless xxh3-64
checksum; its long-input loop uses exact 32-bit limbs.

Generated readers are written by the Rust `export-wire` binary; generated files
are never maintained by hand. Production contracts contain only shape checks,
enums, bounds and dictionary kinds, with shared defaults applied at validation.
Full producer metadata and reference/conformance contracts remain in the test
export. Focused synthetic tests live beside the readers.
The fixture and benchmark tools consume a manifest exported by Rust into an
external scratch directory; they never require game data.

From `ui/`, run `npm.cmd run check:ui` and
`node --experimental-strip-types --test src/wire/*.test.ts`. The shared corpus
test requires the generated `src/generated/wire.ts` and the scratch output of
the Rust `wire-fixtures` tool: set `H4R_WIRE_FIXTURES` to that output directory.
It checks every registered reader, named malformed refusal, physical column
bytes, nested occurrence coordinates and dictionary entries against the same
Rust-authored manifest.

Run `node --experimental-strip-types src/wire/benchmark.ts` with that variable
set to measure the largest registered fixture shapes. Without it, the benchmark
uses clearly labelled synthetic chart, focus and produced-equipment layouts.
The checksum and complete validation are included; byte construction is outside
the timed region. `H4R_BENCH_ITERATIONS`, `H4R_BENCH_WINDOW_RUNS`,
`H4R_BENCH_VIEWPORT_COUNTRIES` and `H4R_BENCH_COMPLETIONS_PER_COUNTRY` configure
benchmark requests and sample counts, and are not application limits.

With the Rust export set, `node --experimental-strip-types
src/wire/focus-benchmark.ts` rebuilds 256/512/1,024/16,384-segment scheduling
units from the registered focus contract and physical fixture. It measures
checksum and complete validation after 200 warmups, with construction outside
the timed region. `H4R_BENCH_RUNTIME` optionally selects an external reader
snapshot for a baseline comparison. Run the shared machine's `wait-quiet.sh`
before timing; Node codec measurements do not establish the WebView2 viewport
budget including the Rust writer and transport.
