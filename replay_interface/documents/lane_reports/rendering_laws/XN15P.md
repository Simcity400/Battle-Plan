# XN15P — production replay handoff

Status: **PROVEN — production handoff promotion-ready**

XN15P converts the frozen XN15 effect/entity/light/weather recording law into a machine-readable writer, container, ingest and renderer handoff. It does not alter the frozen XN15 report, contract, ledger, verifier or successful verification.

## Admission authority

The canonical XN15 evidence manifest is:

`replay_interface/re_evidence/rendering_laws/XN15/evidence_manifest.json`

SHA-256: `86a132e4ab0cd2484d882a37a84f47029473f63b014f4055478eaa20e9fbb114`.

It content-verifies all five frozen XN15 core artifacts and names `XN15/VERIFICATION.md` SHA-256 `9ec7ad09c59780a83a7e694e432e96a56a85fad029d4482dc2fa9a6573fbf173` as the successful 74/74 core verification. The case-distinct lowercase `XN15/verification.json` is explicitly rejected as core authority because it belongs to the earlier XN15B static trigger census.

## Production packet

- `production_handoff.json` defines the `HOI4XN15` v1 container, frozen admission hashes, section order, lossless encodings, authoritative ordering, writer/reader behavior, checkpoints, fail-closed rules, implementation targets and eleven promotion gates.
- `wire_schema.json` defines wire-safe primitive types and exact field catalogs for nine record families: effect lifecycle, particle submit, entity presentation, entity submit, direct visual producer, light lifecycle, tiled-light submit, weather state and checkpoint.
- `verify_xn15p.py` hashes the complete XN15 manifest, validates every frozen file, rejects the verification-path collision, and mechanically compares each production record field list and enum to `recording_contract.json`.
- `verification.json` is the deterministic verifier result.

The verifier passes **50/50** checks with nine record types, eleven production gates and zero residuals.

## Wire and replay laws

Metadata stores u64/i64 values as decimal text and floats as raw IEEE-754 bit strings, avoiding JSON/JavaScript precision loss and decimal float roundtrips. Large exact payloads are content-addressed blobs with raw-byte size and SHA-256 checks. Only identity or deterministic lossless zstd storage is admitted.

Particle record and transform blobs are size-checked against the live count and strides. Skinning palettes are exactly 3,200 bytes. Tiled lights retain raw grid constants, the full 16,384-byte index payload and full 2,048-byte data payload plus semantic slot joins. Weather retains the fixed-point region value, signed absolute game-hour expiry, nullable code-zero token, all ordered position resolutions, primary/fading identities and ordered effect identities with the exact big-nonempty-otherwise-small selected-vector branch and no further class substitution.

Semantic events remain in strict `sequence_u64` order. Render submits preserve submit/pass/draw/packet order. Stable ids are never process pointers or capture-local RenderDoc ResourceIds. A chunk is admitted only after its blobs and canonical metadata are complete and hashed; a partial tail is rejected.

Cold seek loads the nearest valid checkpoint and replays forward. It must equal linear playback byte-for-byte for particles, entity submissions and palettes, direct visual producers, tiled lights and weather membership.

## Implementation handoff

The extractor emits records and blobs only after observed engine operations succeed. The container performs lossless serialization, content hashing, indexing, atomic chunk commit and checkpoint publication. Replay ingest verifies build/catalog/schema identities and complete sequence ranges before reconstructing state. The renderer consumes captured packet, palette and tiled-light bytes without particle simulation, light-animation evaluation, weather reroll or visual substitution.

Promotion requires roundtrip, corruption, lifecycle/order, effective-VFS resolution, cold-seek, packet-size/hash, per-pass palette, complete tiled-light, code-zero weather, exact selected-vector/empty-vector and measured-storage tests. Retention may terminate an exact recording session but may not silently thin render submits inside one.

## Provenance disposition

XN15P makes no new binary-function claim and issued no Ghidra job. It consumes the complete frozen address/job/artifact/manifest ledger at `replay_interface/re_evidence/rendering_laws/XN15/evidence_ledger.md`, SHA-256 `2d48349125008e7db36ab5af887bb7261c8831a44dedae4099e38efe0dd42527`. All observation addresses—including particle create/destroy/submit, entity presentation, direct air/naval/convoy producers, weather state/producer and packet emission—remain recorded there with their exact job ids, artifact paths and manifest hashes.

## Incidental useful findings

- Admission code must follow the manifest's explicit verification path; filename-case conventions are unsafe in this directory.
- Content-address deduplication is legal only for byte-identical decompressed payloads. Ordered references may never be coalesced or reordered.
- JSON number encoding is unsuitable for the full u64 id/index domain; decimal-text metadata is mandatory.
- Storage policy is a product choice, but exact-session submit thinning is not compatible with the proven replay law.

No reverse-engineering, capture, data, tool or human-input residual remains in XN15P. Production code changes are intentionally outside this packet's ownership.
