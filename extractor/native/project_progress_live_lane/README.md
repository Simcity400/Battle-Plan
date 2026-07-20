# Project Progress Live Lane

This lane keeps only active build/test files here. Historical plans, fixture
contracts, and proof notes were moved to
`../../documents/archive/native_lanes/project_progress_live_lane/`.

Current extraction completion status lives in
`../../documents/MASTER_EXTRACTION_CONTRACT.md`.

`project_progress_lifecycle_stream_proof` is deliberately a slice-only evidence
tool. A successful exit proves only that the supplied stream contains a clean,
canonical D41 lifecycle transition slice with at least one derived event. Its
JSON reports `production_certifying: false`, `source_capture_verified: false`,
and `certification.claimed: false` because this tool has no manifest-bound
commanded-duration authority. Do not treat its exit status or
`accepted_for_lifecycle_delta_slice` as production-capture certification.

Production certification belongs to the standalone compact production gate in
`../lanes/compact_replay_verification_gates/`, followed by the production-profile
checks in `../fold_check/`; those validators bind the capture manifest, required
domain roster, and requested game-hour duration.

Exact a729 `CProject` lifecycle and fired-reward-history field/writer semantics
are recorded in `SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.md` and its JSON
companion. The reader and downstream `PRHF` contract are integrated fail-closed,
and production emission/fold/replay is active under the exact-10 authenticated
promotion manifest `SPECIAL_PROJECT_LIFECYCLE_REWARD_PROMOTION_MANIFEST_20260720.json`.

The exhaustive retained-source closeout for the remaining d65 owner equality and
breakthrough-handle semantic join is
`A23_X9_RETAINED_SOURCE_EXHAUSTION_20260720.md` with a machine-readable JSON
companion. The retained compact corpus has only two raw breakthrough handles because
the old probe/reader inspected only status record 0. `tools/breakthrough_state_probe.py`
schema v2 now reads all validated status records for the one remaining same-boundary
admission capture; it still refuses to infer a mapping.

The production-prerequisite follow-up is
`H_X9_SPECIALIZATIONS_PREREQUISITE_REAUDIT_20260720.md` with a machine-readable JSON
companion. The collector and lane library now walk all three contiguous a729 status
records, merge identical aliases, reject conflicts/torn trees country-atomically, and
revalidate the d41 country owner at the read boundary. Four raw values already survive
the compact/fold/replay path; semantic names remain blocked on the exact-boundary join.
Unexecuted exact-20 Ghidra follow-up seeds are kept separately in
`H_X9_SPECIALIZATIONS_GHIDRA_SEEDS_PENDING_20_SHARDS_20260720.json`.
