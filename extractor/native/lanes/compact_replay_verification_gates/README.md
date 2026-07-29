# compact_replay_verification_gates
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/compact_replay_verification_gates/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local verification infrastructure for compact replay proof runs. This lane
does not own gameplay extraction. It validates whether another lane's compact
run is merge-ready before shared recorder integration.

## Tool

```powershell
compact_replay_gate.exe `
  --run-dir extractor\native\runs\compact_full_plus_tf_location_870ebf_20260701_v1 `
  --min-game-hours 20 `
  --write-report extractor\native\runs\compact_replay_verification_gates_20260701_v1\gate_report.json
```

The gate requires an explicit positive `--min-game-hours N` and treats it as an
exact advancement contract. The independently scanned physical stream must span
at least `N` game hours. Missing FRAME_END hours are valid only when the next
gap-only frame carries an exact, authenticated H4DG range for every scheduled
domain; the scanner proves `unique_hours + skipped_hours == inclusive_span`.
It opens the replay
container first, and treats its unique `recorder_manifest` entry as the
authority. The disk manifest must be byte-identical to that embedded manifest.
It then decodes the manifest-declared raw/zstd stream, independently scans the
decoded H4CR records, independently hashes those bytes, binds that digest to
the verified container entry, and compares the bytes with the run stream.
Stored and decoded lengths/hashes are reported separately.

For every FRAME_END, production admission requires the exact scheduled domain
roster and exactly one terminal outcome per domain: `DOMAIN_PERF XOR H4DG`.
`DOMAIN_DATA` is nonterminal, appears only beside an accepted PERF outcome, and
can never satisfy cardinality. H4DG target/range witnesses and zero-based
per-`(domain, part)` outcome ordinals are independently decoded and sequenced.
The default profile accepts truthful typed gaps. `--require-zero-loss` adds a
separate promotion claim and rejects every H4DG; it does not redefine valid
capture format.

Production admission also requires all-domain canonical DATA decoding for
accepted outcomes and the exact seven-role static bundle across the bundle report, production
profile, packaging manifest, index, container entries, canonical paths, bytes, and hashes.
The bundle must also carry the pointer-free product-modset key; the gate
recomputes it and joins its executable, mod, DLC, load-order, VFS namespace,
and VFS row components to retained bytes before publication. The embedded
recorder manifest and the container's typed top-level portable-content v2
manifest must be byte-semantically identical and carry that same key.

## What It Rejects

- non-compact or non-`--extraction-perf-gate` runs;
- parse/order errors;
- missing/duplicate frame-end records;
- a missing, duplicate, or unscheduled terminal outcome;
- PERF and H4DG coexistence for one domain/hour;
- DATA used as a terminal outcome, DATA beside H4DG, or missing DATA beside an
  accepted PERF outcome for a DATA-bearing domain;
- malformed H4DG, target/header mismatch, ordinal hole/repeat/regression, or
  overlapping/regressing range;
- a forward FRAME_END discontinuity without one exact H4DG range for every
  scheduled domain;
- any H4DG when `--require-zero-loss` is explicitly selected;
- failed hourly requirement;
- every canonical global failure flag after domain-local status normalization;
  `--allow-partial-or-source-failure` permits only those two named flags and
  never worker panic, bridge failure, invalid control, missing clock, or other failures;
- bridge row counters in a DLL-only proof;
- dirty root-source/debt fields for scheduled root-owning domains;
- byte volume above configured limits;
- stream byte count mismatches;
- missing, corrupt, or byte-mismatched `.battleplan` container output.
- replay/container game-id, role, declared-layout, or manifest identity drift;
- absolute, traversal, symlink-escaped, or missing stream/replay paths;
- `--skip-replay-verify` and other non-certifying weakening profiles.

## Integration Checklist

For a domain lane to be accepted as merge-ready, provide:

- the exact `collector_record` command;
- scheduled domain list and domain id;
- `native_session_manifest.json`;
- `collector_stream.bin`;
- `native_session.battleplan`;
- this gate's JSON report with `passed: true`;
- any lane-local payload decoder report;
- exact root/source/debt fields from the manifest;
- explicit statement that Python/saves/bridges were not in the hot loop.

## Shared Integration Patch Plan

No shared recorder entrypoint was edited in this lane. When the orchestrator
grants an integration slot, wire this gate in one of two ways:

1. Add a `recorder_host` subcommand or flag such as `collector_record verify-run
   --run-dir <dir>` that calls equivalent manifest and compact stream checks
   after recording.
2. Keep this lane crate as the pre-merge verifier and add a CI/dev script that
   runs it against every lane proof directory named by the orchestrator.

Any new compact domain must update both:

- `extractor/native/recorder_host/src/main.rs` domain id/name/data decoder maps;
- the `DOMAIN_CONTRACTS` table here, including semantic counter names emitted under
  `compact_stream.per_domain.<domain>.data_units_total`.
