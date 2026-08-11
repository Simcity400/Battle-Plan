# `.battleplan` portable container contract

Status: normative for current product publication and playback. The maximum
container version is `2`.

A `.battleplan` is a self-contained ZIP whose manifest authenticates every
payload entry. Product publication uses the stitched v2 profile. Replay also
opens a recorder compact stream through the certified pre-fold boundary below;
compact bytes never become a Replay-owned wire format.

## Stitched v2 profile

A v2 manifest must declare `timeline_segments_v1`. The ZIP uses STORE
compression (`level = none`), orders payload entries lexicographically, and
writes `manifest.json` last. The stitched publication contains:

```text
manifest.json
payload/<game_id>/index.json
payload/<game_id>/segments.json
payload/<game_id>/segments/s<segment-id>.index.json
payload/<game_id>/s<segment-id>.<hour>.snapshot.json
payload/<game_id>/<embedded raw-source files>
```

`index.json` is the visible main-timeline index. Its `index_schema` is
`stitched-main-v1` and its `segments_file` is `segments.json`.
`segments.json` is the authority for reload/relaunch topology, campaign and
segment ownership, transition coverage, retained observations, raw-source
participation, and publication parameters. Per-segment indexes bind every
retained snapshot, gap, coalesced hour, and capture failure. The reader rejects
any graph/index/hash/ownership disagreement.

An embedded raw source appears once and is byte-identical to its declared
stored digest. A source declared standalone is referenced by the graph and
must not appear in the ZIP. Snapshot filenames come from the indexes; readers
never synthesize them from an hour.

Publication is build-beside then atomic replace. The recorder derives every
entry from one sealed plan, copies from retained authenticated handles, verifies
the unpublished container completely, and only then promotes that exact file.
A pre-commit failure leaves the prior destination and all source material
untouched.

## Manifest

`manifest.json` is strict JSON. Its current structural fields are:

```json
{
  "format": "hoi4replay",
  "container_version": 2,
  "required_features": ["timeline_segments_v1"],
  "optional_features": [],
  "schema_version": "<replay schema>",
  "game_id": "<portable game id>",
  "build": { "exe_sha256": "<sha256 or null>", "pinned": false },
  "mod_identity": { "modset_id": "<digest>" },
  "content_manifest": null,
  "created_utc": "<sealed input timestamp>",
  "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
  "counts": { "snapshots": 412 },
  "entries": [
    { "path": "payload/<game_id>/index.json", "sha256": "<sha256>",
      "bytes": 0, "role": "index" }
  ],
  "manifest_digest": "<sha256>",
  "signature": null
}
```

The actual entry list contains every payload file, sorted by path. Each row
binds its decompressed byte length, SHA-256, role, and `hour_index` when
applicable. A finalized portable content manifest is coupled bidirectionally to
the `portable_content_manifest_v2` required feature; either both are present or
both are absent.

`manifest_digest` is SHA-256 over canonical JSON with `manifest_digest` and
`signature` removed. Canonical object keys are lexicographically sorted and
duplicate JSON keys are rejected. A non-null signature is rejected unless the
reader build has a configured verification policy; unsigned containers remain
integrity checked.

## Open and integrity rules

Open is fail closed and bounded:

1. Pin one archive handle, parse its central directory, and reject malformed
   ZIP structure, duplicate entries, unsupported compression, or changed file
   identity.
2. Enforce at most 2,000,000 entries, a 64 MiB manifest, 64 GiB total declared
   uncompressed payload, and the applicable per-entry ceiling.
3. Require normalized relative UTF-8 payload paths. Reject absolute, drive,
   UNC, backslash, `.`/`..`, Windows device-name, non-NFC, and Unicode-caseless
   collisions.
4. Strict-parse the manifest, require version/feature coupling, verify its
   canonical digest, and reject unknown required features.
5. Require exact ZIP/manifest membership: every payload entry is declared,
   every declaration exists, lengths agree, and no stowaway is allowed.
6. Authenticate and strict-parse the main index, stitched graph, all segment
   indexes, and the optional timeline before returning an open v2 container.
7. Recompute the stitched input digest from the serialized graph and segment
   indexes. Validate visible-head selection, source bounds, observation order,
   transition coverage, entry order, compression profile, and all stored
   hashes.

Payload exposure is lazy after open. The reader checks CRC, compression-stream
termination, decoded length, and SHA-256 in the same bounded pass that returns
the requested bytes. A failed check never exposes data. Snapshot provenance
must not contradict manifest build or mod identity.

## Certified compact-stream open

A recorder compact package has an empty snapshot index and exactly one manifest
entry with role `collector_compact_stream`. Replay always reconciles the
selected game, role path, and decoded length/SHA-256. Any declared timeline or
recorder-manifest role entry must use the canonical path. When a
recorder-manifest binding is declared, the timeline/manifest
roles, stream format, and cross-reference must agree; `stream_storage` and
`replay_entries` are joined to the compact role entry when those objects are
present. The product compact-stream ceiling is 2 GiB.

The shipping app opens this route with an explicit
`CertifiedPrefoldSpoolDecoder`:

1. The authenticated role entry is streamed into a private identity-marked
   spool under the caller-owned spool root. Publication uses a unique retained
   temporary handle, non-reparse directory ancestry, flush and sync, same-handle
   length/SHA verification, and a no-replace hard-link commit.
2. The decoder receives only the authenticated source path, exact length and
   SHA-256, selected game identity mode, and a caller-owned snapshot directory.
3. It returns a non-empty, strictly increasing `PrefoldedSpool` plus a receipt
   bound to the same source length/SHA and decoder identity. It cannot change
   the caller-owned snapshot directory.
4. Replay validates every generated hour, schema, filename, length, SHA-256,
   date, and container provenance before exposure. Static artifacts and content
   catalogues are admitted through the same manifest-restricted store.
5. Failure removes the private spool. Success retains it only for the lifetime
   of the lazy replay source.

Without a packaged replay cache or an explicitly supplied certified decoder,
an empty snapshot index is refused. Replay never guesses compact record layouts,
silently drops declared coverage gaps, or depends on Extractor crates.

## Authority

- Container, path, staged-verification, and transactional-spool enforcement:
  `portable_replay/native/snapshot/src/container.rs` and `stream_verify.rs`.
- Stitched v2 planning/publication:
  `extractor/native/recorder_host/src/stitching/`.
- Certified pre-fold contracts:
  `portable_replay/native/snapshot/src/prefolded_decoder.rs`.
- Replay open routing and product bounds:
  `replay_interface/native/replay_core/src/source.rs`.
- Shipping worker boundary:
  `replay_interface/native/replay_app/src/recording_open.rs` and
  `prefold_worker.rs`.
