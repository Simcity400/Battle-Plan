# Characters Leaders Cabinet Live Lane

This lane keeps active extractor, decoder, gate, and fixture files here. Historical
plans, proof notes, and registration packets were moved to
`../../../documents/archive/native_lanes/characters_leaders_cabinet_live/`.

Current extraction completion status lives in
`../../../documents/MASTER_EXTRACTION_CONTRACT.md`.

## Validation scope

- `src/stream_gate.rs` is the lane-local certifying parser for complete domain
  `47` streams. Certification requires a sequence-one contiguous stream,
  canonical record flags and types, one exactly matching PERF/DATA pair per
  owned hour, FRAME_END counts bound to the observed frame records, nonfatal
  status, consecutive closed hours, and a final FRAME_END.
- The `replay-verify` developer tools (`replay_fixture_verify`,
  `replay_record_verify`) were retired on 2026-08-13. They were
  non-certifying — their reports read
  `container_integrity_verified_non_certifying` and proved replay-container
  integrity and byte identity only, never source ordering, unique source hours,
  live capture, cadence, or production readiness — so nothing that certifies
  anything depended on them.
