# domain 53 agency capacity registration patch details

This is a lane-local registration plan for the compact raw agency capacity rows
added in v22. It is not an applied shared-entrypoint edit.

## Scope

Backlog item narrowed: `shared_integrated_compact_event_state_records`.

The v22 lane proof emits compact raw agency capacity-cluster rows. The remaining
registration work for this slice is limited to preserving those raw row semantics
through the shared collector and recorder once the orchestrator opens domain 53.
No shared semantic relabeling is required or allowed by this patch.

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
the existing lane encoder with typed `AgencyCapacityClusterRow` values. The raw
capacity-cluster requirement is:

```rust
AgencyCapacityClusterRow {
    owner_tag_db_id,
    value_0xe4: read_u32(agency + 0xE4),
    value_0xf0: read_u32(agency + 0xF0),
    value_0xf4: read_u32(agency + 0xF4),
    proof_hash: stable_hash32("agency_capacity_offsets_e4_f0_f4"),
}
```

Required source proof:

- `SOURCE_PROOF_AGENCY_ROOT_870EBF` covers
  `country_object+0xFC0 -> CIntelligenceAgency`, agency vtable
  `module+0x296B338`, and the raw `agency+0xE4/+0xF0/+0xF4` capacity cluster.
- Do not add a separate source bit for this raw row unless the orchestrator
  later splits agency scalar roots from agency capacity roots.

Required fail-closed behavior:

- If the agency root is stale, omit agency capacity-cluster rows and badge/source
  fail the domain rather than emitting guessed raw values.
- Keep the row raw. Do not call `value_0xf0` max, or `value_0xe4/value_0xf4`
  usable/current, until the same-tick UI/save fixture confirms the labels.
- Do not mutate existing `AgencyRow.max_operatives` or
  `AgencyRow.usable_slots` labels from this raw row.

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

This generic merge is the required hook for `agency_capacity_cluster_rows`. A
shared recorder patch that whitelists only older counters would silently drop
the v22 row proof and is not acceptable.

## Required Acceptance Assertions

For a domain-53 shared proof that includes the current v22 fixture shape:

- `domain_id == 53`
- `data_units_total["agency_capacity_cluster_rows"] == 1`
- `data_units_total["intel_agency_rows"] == 1`
- `data_units_total["operative_lifecycle_rows"] == 3`
- `data_rows_total == 12`
- `.hoi4replay_verified == true`
- no `PARTIAL_DATA`, `SOURCE_UNAVAILABLE`, `WORKER_PANIC`, or
  `BRIDGE_UNAVAILABLE` for domain 53

Production completion still additionally requires DLL-only capture,
hourly-or-better speed-five/current-equivalent frames, zero skipped hours, and
the unresolved war/diplomacy/intel/operation/spy-network/agency semantic
surfaces either captured or conclusively blocked.
