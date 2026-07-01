# compact_replay_verification_gates

Lane-local verification infrastructure for compact replay proof runs. This lane
does not own gameplay extraction. It validates whether another lane's compact
run is merge-ready before shared recorder integration.

## Tool

```powershell
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml -- `
  --run-dir replay_native\runs\compact_full_plus_tf_location_870ebf_20260701_v1 `
  --min-game-hours 20 `
  --write-report replay_native\runs\compact_replay_verification_gates_20260701_v1\gate_report.json
```

The gate reads `native_session_manifest.json`, scans `collector_stream.bin`
independently at the compact-record header layer, opens
`native_session.battleplan` through the existing `snapshot::Container`, runs
`verify_all`, and byte-compares the container's `collector_compact_stream`
against the raw compact stream.

## What It Rejects

- non-compact or non-`--extraction-perf-gate` runs;
- parse/order errors;
- missing/duplicate frame-end records;
- missing per-hour scheduled domain perf records;
- missing per-hour typed domain data records;
- skipped game hours;
- failed hourly requirement;
- worker-panic, bridge-unavailable, source-unavailable, or partial-data flags;
- bridge row counters in a DLL-only proof;
- dirty root-source/debt fields for scheduled root-owning domains;
- byte volume above configured limits;
- stream byte count mismatches;
- missing, corrupt, or byte-mismatched `.battleplan` container output.

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

- `replay_native/recorder_host/src/main.rs` domain id/name/data decoder maps;
- `contracts/compact_replay_gate_contract.json` and the `DOMAIN_CONTRACTS`
  table here, including semantic counter names emitted under
  `compact_stream.per_domain.<domain>.data_units_total`.
