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
- `replay_fixture_verify` creates a synthetic closed fixture stream, and
  `replay_record_verify` wraps one DATA record or payload in a synthetic closed
  envelope. Their reports use
  `container_integrity_verified_non_certifying`: they prove replay-container
  integrity and byte identity only. They do not certify source ordering,
  unique source hours, live capture, cadence, or production readiness.
