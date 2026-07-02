# compact_replay_verification_gates integration patch plan

This lane did not edit shared collector or recorder entrypoints.

## Current Lane-Local Assets

- `extractor/native/lanes/compact_replay_verification_gates/src/lib.rs`
  implements manifest validation, compact stream header scanning, byte-volume
  gates, bridge/source/partial/worker failure checks, root-debt checks, per-domain
  semantic counter requirements, and `.battleplan` container verification.
- `extractor/native/lanes/compact_replay_verification_gates/src/main.rs`
  exposes the `compact_replay_gate` CLI.
- `extractor/native/lanes/compact_replay_verification_gates/contracts/compact_replay_gate_contract.json`
  is the manifest/domain checklist.
- `extractor/native/runs/compact_replay_verification_gates_20260701_v1/gate_report.json`
  is the green proof report against the current 17-domain compact proof.
- `extractor/native/runs/compact_replay_verification_gates_20260701_v1/negative_partial_projects_report.json`
  proves the failure path catches partial/source/root-debt defects.

## Shared Recorder Slot

When granted, add either:

1. `collector_record verify-run --run-dir <dir>` in
   `extractor/native/recorder_host/src/main.rs`, reusing this lane's checks; or
2. a workspace script/CI target that runs:

```powershell
cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml -- `
  --run-dir <proof-run-dir> `
  --min-game-hours <required-hours> `
  --write-report <proof-run-dir>\compact_gate_report.json
```

## Required Updates For New Domains

- Add the domain id/name to `recorder_host` compact domain maps.
- Add the domain's payload decoder to `recorder_host`.
- Add one shared recorder unit test that rejects malformed payload length and one
  test that accepts a valid typed data record.
- Add the domain to `DOMAIN_CONTRACTS` in this lane.
- Add semantic counter names to
  `contracts/compact_replay_gate_contract.json`.
- Run this gate against the integrated proof run with `.battleplan`
  verification enabled.

Domain 42 `air_combat_losses_live` is lane-known here but intentionally marked
`shared_recorder_known: false` until its recorder integration patch lands.
