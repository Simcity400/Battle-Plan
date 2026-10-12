# Binary query wire

`wire` writes and validates the Appendix C `H4RQ` columnar reply schema and immutable `H4RD` dictionary shards. It is independent of recording readers, producers and transports. The existing JSON transport uses its existing contracts. A request router can use this crate for the bounded G1 reply path.

`model.rs` defines every logical type, physical encoding, state layer, descriptor, unit, bounds, extent and dictionary binding. `writer.rs` builds aligned, zero-padded sections, computes the body checksum and runs the validating reader before returning bytes. `reader.rs` returns buffer slices borrowed from the received reply, without converting numeric values through floating point or expanding run-length values. `dictionary.rs` writes content-addressed shards, validates pages and canonical identities, and serves exact entry byte extents. `shapes.rs` owns the registered G1 contracts, name tables, union gates, producer state roles and static enums. `fixtures.rs` owns synthetic conformance replies and malformed mutations.

## Validation and exactness

The reader checks the complete header, checksum, directory, padding, buffer ranges, rooted acyclic descriptor graph, types, encodings, lengths, offsets, dictionary references and codes, state widths and codes, union gates, bounds, extents and embedded diagnostics before exposing a reply. A `WireError` has a stable lowercase name and a contextual explanation. Unknown shapes and shape versions are refused. Before publishing a data reply, the router supplies `ReadContext { expected_bounds: normalized_request_bounds, require_bounds_match: true }`, including `None` for an unbounded request. The default context is for standalone diagnostics and checks that have no request identity. Continuation bytes remain opaque: their authentication and generation lease are the request router's responsibility.

State mappings are authored by producers rather than inferred from Rust `Option`. L1 distinguishes recorded values, NotRecorded and not distinguished; L2 distinguishes native none and computed absence; L3 records knownness; L4 uses zero for no refusal and code minus one for a separately bound refusal dictionary. Validation proceeds outermost first, propagating coalesced activity ranges through descriptor order and list offsets. An inactive struct, list, union payload or outer layer hides its payload and inner state values; structural lengths and buffer ranges remain validated. Independently known operands can therefore remain usable when a related presentation value is refused.

Lists and structs nest without a chosen depth limit. Descriptor graph validation and original occurrence walks use iterative algorithms. `original_occurrence` follows inverse list offsets and extent starts to recover a top-level original row and each original child position. Absolute general hours and native-date ordinals are i64. Chart run hours retain their native i32 values, while original run positions are u64. Safe53 values are physical f64 integers within the inclusive range ±2^53. Other 64-bit integers and 128-bit low/high limbs remain exact. Floating-point values retain their bit patterns, including signed zero and NaN payloads. Byte values remain arbitrary bytes; text is UTF-8 validated. Fixed-point values retain their integer operands, radix and scale.

The document revision called Appendix C v3 uses header schema version 1 in its complete byte walk; `SCHEMA_VERSION` is 1. Reply-kind codes are registered here in Appendix C's listed order, beginning at 1. Dictionary kind codes begin at 0 in the listed order. A new wire kind or a changed layout requires a schema version; changing a registered shape requires its shape version.

## Units and arithmetic

Campaign length does not determine reply allocation. Callers bound work by a requested window, viewport, rank range or configured scheduling unit and continue rows and oversized child/byte values across units. There is no country-count, campaign-duration, nesting-depth, dictionary-lifetime or text-size cap in the wire contract.

At 100 years in the fixed 365-day calendar, 100 × 365 × 24 = 876,000 hours. At 5,000 countries this is 4,380,000,000 country-hour positions, above u32::MAX. General row positions, original run positions, totals, list and byte offsets, child lengths, dictionary counts, canonical dictionary indices and shard page counts are consequently u64; exclusive hour bounds are i64. These fields do not change at a century boundary. Native chart hours alone remain i32: 876,000 < 2,147,483,647, and these hour values never multiply by entity counts. Their existing positive native domain spans about 245,000 game years. Preserving that source domain does not choose a campaign-duration cap; pagination and aggregate positions use their independent 64-bit fields.

The schema's u32 descriptor, section, buffer and reference counts and descriptor indices are local to one serialized reply, whose actual buffers are received together. They do not address a campaign-wide inventory. Writers check each conversion to u32 and return `size-overflow` when another unit is needed. Readers check count multiplication, byte offsets, additions, exclusive ranges and dictionary canonical-index addition before use. Run lengths individually use their specified u32 width; their sums and logical column lengths are u64. Run-length code and safe53 columns validate each encoded value intersecting ancestor and gate activity once, while stateless ordinary numeric columns need only structural validation. Compact repeated columns therefore do not force iteration through a campaign's logical rows.

Text byte extents can split UTF-8 scalars. Validation permits up to three leading continuation bytes at an open first boundary and a valid truncated scalar prefix at an open final boundary, with enough room in a known total to complete it. Complete values remain strict UTF-8. Extents retain exact byte slices until the consumer reconstructs displayable text.

Fixed-length lists can continue across units. Each present value's extent total must equal the contract's fixed length; its local offsets must match the extent's slice length. Values without an extent use their local length. Extents are indexed by column and occurrence for validation.

## Dictionary pages

A shard identity is BLAKE3-256 of its complete immutable bytes. Each page uses `{entry_count: u64, offsets: u64[entry_count + 1], bytes}` and starts on an eight-byte boundary. Page directory first-entry values are canonical dictionary indices; pages cover contiguous entries. Plain dictionaries may contain arbitrary bytes. Text, tag, refusal, image-key and diagnostic-code entries validate as UTF-8. A canonical entry at u64::MAX is valid when the shard contains that single final entry; exclusive arithmetic is checked only where an exclusive position is required.

Appendix C does not define per-entry attributes, so version 1 pages carry no attributes. Oversized entries remain exact bytes in immutable shards and can be fetched by `byte_extent` without changing their canonical identity. The caller chooses page and fetch sizes. Consumers verify shard identity before publishing dictionary values and hold the shard while a cached reply references it; backend lease ownership is outside this codec.

## Synthetic fixtures and checks

`wire-fixtures` exports Rust-written replies, dictionary shards and a manifest into scratch outside Git. Both language suites consume the same bytes and named malformed cases. Rust rewrites every successful reply to identical bytes. `export-wire` emits compact production contracts containing shape checks, enums, bounds and dictionary kinds, with shared defaults applied by the runtime. Producer state mappings and the two Appendix C reference shapes and primitive conformance shape remain in the full fixture export. Generated TypeScript files are regenerated rather than edited. Native chart schemas include prepared runs, segment starts, tree bytes and the series directory.

The crate tests exercise more than u32 logical rows in a small run-length unit, a 6,000-level descriptor/occurrence walk, 128-bit run-length limbs, wide canonical dictionary entries, exact oversized entry extents and every shared malformed fixture. `wire-bench` measures crate-scoped validation cost in a central release build. UI tests and benchmarks do not build Rust. All examples and fixtures are synthetic.

The crate-scoped checks are `cargo test -p wire --all-targets` and
`cargo fmt -p wire -- --check`. Run `cargo run -p wire --bin export-wire` to
write `ui/src/generated/wire.ts`; add `-- --check` to verify the generated
contract. Run `cargo run -p wire --bin wire-fixtures -- <external-scratch>`
to export the shared corpus, and `cargo run --release -p wire --bin wire-bench`
for validating-reader measurements. The [UI reader guide](../../ui/src/wire/README.md)
describes the TypeScript checks against that corpus.
