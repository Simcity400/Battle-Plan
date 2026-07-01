# domain 53 compact state-events registration patch details

This is a lane-local registration plan for the compact validated-state event
rows added in v24 and extended in v26. It is not an applied shared-entrypoint
edit and it does not edit the orchestrator-owned master contract directly.

## Scope

Backlog item narrowed: `shared_integrated_compact_event_state_records`.

The v26 lane proof emits compact state-event rows for observations already
covered by validated current-build roots. The remaining registration work for
this slice is limited to preserving those rows and their counters through the
shared collector and recorder once the orchestrator opens domain 53.

These rows do not close unresolved event streams for war lifecycle, diplomacy
queues, running operations, spy networks, or full agency/operative semantics.

## Current Shared Registry Prerequisites

The shared registries already reserve domain 53:

- `extractor/native/collector/src/domain_registry.rs`: `domain(53,
  "diplomacy_intel_operatives_live", "diplomacy_intel_operatives_live",
  CompactDomainStatus::Reserved, false, true, &[])`
- `extractor/native/recorder_host/src/compact_domain_registry.rs`: `domain(53,
  "diplomacy_intel_operatives_live", "diplomacy_intel_operatives_live",
  CompactDomainStatus::Reserved, false, true, &[])`

The final `true` is `requires_data_record`. Do not change the ID or wire name.

## Collector Registration Delta

When domain 53 is scheduled, the collector-side `read_domain53` module may feed
the existing lane encoder with typed `StateEventRow` values only for roots whose
matching source proof is present:

```rust
StateEventRow {
    owner_tag_db_id,
    target_tag_db_id,
    event_kind: STATE_EVENT_KIND_OBSERVED,
    source_kind,
    object_hash,
    value_hash,
    value_raw,
}
```

Required source proofs:

- `STATE_EVENT_SOURCE_RELATION_FLAG` requires
  `SOURCE_PROOF_RELATION_ROOTS_870EBF`.
- `STATE_EVENT_SOURCE_RELATION_OPINION` requires
  `SOURCE_PROOF_RELATION_ROOTS_870EBF`.
- `STATE_EVENT_SOURCE_SUBJECT_AUTONOMY` requires
  `SOURCE_PROOF_RELATION_ROOTS_870EBF`.
- `STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP` requires
  `SOURCE_PROOF_COUNTRY_ROOTS_870EBF`.
- `STATE_EVENT_SOURCE_AGENCY_SCALAR` requires
  `SOURCE_PROOF_AGENCY_ROOT_870EBF`.
- `STATE_EVENT_SOURCE_AGENCY_CAPACITY` requires
  `SOURCE_PROOF_AGENCY_ROOT_870EBF`.
- `STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE` requires
  `SOURCE_PROOF_OPERATIVE_ROOT_870EBF`.
- `STATE_EVENT_SOURCE_CRYPTOLOGY` requires
  `SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF`.

Required fail-closed behavior:

- If the source proof for a state-event source is absent or stale, omit that
  state-event row and fail/badge the domain rather than emitting a guessed
  observation.
- Do not emit state-event rows for war lifecycle, diplomacy queues, running
  operations, spy networks, or full agency/operative semantics until those roots
  are independently accepted.
- Keep `event_kind` and `source_kind` bounded to the current constants; unknown
  source kinds must reject during decode.

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

This generic merge is the required hook for `compact_state_event_rows`. A shared
recorder patch that whitelists only older counters would silently drop the v26
row proof and is not acceptable.

## Required Acceptance Assertions

For a domain-53 shared proof that includes the current v26 fixture shape:

- `domain_id == 53`
- `data_units_total["compact_state_event_rows"] == 8`
- `data_units_total["compact_state_event_relation_opinion_rows"] == 1`
- `data_units_total["compact_state_event_country_faction_group_rows"] == 1`
- `data_units_total["agency_capacity_cluster_rows"] == 1`
- `data_units_total["operative_lifecycle_rows"] == 3`
- `data_rows_total == 20`
- `.hoi4replay_verified == true`
- no `PARTIAL_DATA`, `SOURCE_UNAVAILABLE`, `WORKER_PANIC`, or
  `BRIDGE_UNAVAILABLE` for domain 53

## Proposed master-contract row text

Do not apply this from the lane unless the orchestrator explicitly asks for a
direct master edit. Include this exact text in the lane report instead:

- Domain row current status/evidence proposal:
  `| diplomacy_intel_operatives_live / 53 | SHARED_INTEGRATION_PENDING | Lane-local operative lifecycle, raw agency capacity, relation/opinion, country faction-group, and compact validated-state event slices exist (compact_state_event_rows=8 incl. relation opinion and faction group); shared production integration and operation/intel-map/incoming-action fixtures remain. |`
- Relations/opinions row current status/evidence proposal:
  `| Relations/opinions | ROOT_DISCOVERY_OPEN | Domain 53 v26 lane-local proof verifies compact observed-state relation opinion rows (compact_state_event_relation_opinion_rows=1); base relation flags also remain available. | Shared DLL proof plus any remaining opinion/relation replay fields, tension/action semantics, or exact rejections/blockers. |`
- Factions row current status/evidence proposal:
  `| Factions | ROOT_DISCOVERY_OPEN | Domain 53 v26 lane-local proof verifies compact observed-state country faction-group rows (compact_state_event_country_faction_group_rows=1). | Shared DLL proof plus faction leader/name/join-leave lifecycle/event rows, or exact rejections/blockers. |`

Production completion still additionally requires DLL-only capture,
hourly-or-better speed-five/current-equivalent frames, zero skipped hours, and
the unresolved war/diplomacy/intel/operation/spy-network/agency/operative
surfaces either captured or conclusively blocked under
`extractor/native/MASTER_EXTRACTION_CONTRACT.md`.
