# Legacy `.battleplan` Fixture

`pre222_store_deflate.battleplan` is a hand-crafted ZIP32 container fixture for
pre-container-v2 compatibility coverage. It was not produced by the current
`snapshot::pack` writer.

Produced on 2026-07-09 by a one-off byte-level generator that manually wrote
local file headers, central-directory records, and the EOCD:

- STORE entries: `payload/legacy/index.json`, `payload/legacy/timeline.json`,
  and `manifest.json`
- DEFLATE entries: `payload/legacy/snapshots/000016959360.snapshot.json` and
  `payload/legacy/collector_raw/collector_stream.bin`
- no Zip64 records
- no zstd entries
- no data descriptors

The collector stream contains two H4CR records: one domain-18 country-identity
payload for GER and one frame-end record for hour `16959360`.
