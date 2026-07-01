# domain 53 operative lifecycle registration patch details

This is a lane-local registration plan for the compact operative lifecycle rows
added in v20. It is not an applied shared-entrypoint edit.

## Scope

Backlog item narrowed: `shared_integrated_compact_event_state_records`.

The v20 lane proof already emits compact operative lifecycle rows. The remaining
registration work for this slice is limited to preserving those row semantics
through the shared collector and recorder once the orchestrator opens domain 53.
No new shared schema is required.

## Current Shared Registry Prerequisites

The shared registries already reserve domain 53:

- `replay_native/collector/src/domain_registry.rs`: `domain(53,
  "diplomacy_intel_operatives_live", "diplomacy_intel_operatives_live",
  CompactDomainStatus::Reserved, false, true, &[])`
- `replay_native/recorder_host/src/compact_domain_registry.rs`: `domain(53,
  "diplomacy_intel_operatives_live", "diplomacy_intel_operatives_live",
  CompactDomainStatus::Reserved, false, true, &[])`

The final `true` is `requires_data_record`. Do not change the ID or wire name.

## Collector Registration Delta

When domain 53 is scheduled, the collector-side `read_domain53` module must feed
the existing lane encoder with typed `OperativeRow` values. The operative
lifecycle requirement is:

```rust
OperativeRow {
    lifecycle_flag: Some(read_u32(identity + 0x110)),
    state_hash: stable_hash32(&format!("lifeflag_{lifecycle_flag}")),
    // existing identity, owner, detected, skill, XP fields unchanged
}
```

Required source proof:

- `SOURCE_PROOF_OPERATIVE_ROOT_870EBF` covers `COperativeLeader`
  `module+0x293F790`, `identity+0x110`, and owner `identity+0x118`.
- Do not add a separate source bit for lifecycle flags unless the orchestrator
  splits operative identity from lifecycle in a later shared registry pass.

Required fail-closed behavior:

- If the operative identity root is stale, omit operative rows and badge/source
  fail the domain rather than emitting guessed lifecycle flags.
- Do not name lifecycle flags `31` or `63`; keep the numeric
  `lifecycle_flag`.
- Do not infer mission, operation assignment, traits, nationality, portrait, or
  captured state from the lifecycle flag.

## Recorder Registration Delta

The recorder should not duplicate the domain 53 schema. It should call:

```rust
let lane_summary = diplomacy_intel_operatives_live::compact::decode_payload(offset, payload);
```

Then it must merge every lane counter:

```rust
for (name, count) in lane_summary.data_units_total {
    let entry = domain.data_units_total.entry(name.to_string()).or_default();
    *entry = entry.saturating_add(count);
}
```

This generic merge is the required hook for `operative_lifecycle_rows`. A shared
recorder patch that whitelists only older counters would silently drop the v20
row proof and is not acceptable.

## Required Acceptance Assertions

For a domain-53 shared proof that includes the current v20 fixture shape:

- `domain_id == 53`
- `data_units_total["operative_rows"] == 3`
- `data_units_total["operative_lifecycle_rows"] == 3`
- `data_rows_total == 11`
- `.hoi4replay_verified == true`
- no `PARTIAL_DATA`, `SOURCE_UNAVAILABLE`, `WORKER_PANIC`, or
  `BRIDGE_UNAVAILABLE` for domain 53

Production completion still additionally requires DLL-only capture,
hourly-or-better speed-five/current-equivalent frames, zero skipped hours, and
the unresolved war/diplomacy/intel/operation/spy-network/agency surfaces either
captured or conclusively blocked.
