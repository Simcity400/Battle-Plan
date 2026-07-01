# diplomacy_intel_operatives_live domain 53 registration patch packet

This is an exact patch plan, not an applied shared-entrypoint edit. The lane does
not own the shared integration slot.

## Registry State

- `replay_native/DOMAIN_REGISTRY.md`: domain `53` is assigned to
  `diplomacy_intel_operatives_live`.
- `replay_native/domain_registry.json`: domain `53` is assigned to
  `diplomacy_intel_operatives_live`.
- `replay_native/collector/src/domain_registry.rs`: current shared registry
  already has ID `53`, scheduled name `diplomacy_intel_operatives_live`, status
  `Reserved`, requires data record `true`.
- `replay_native/recorder_host/src/compact_domain_registry.rs`: current shared
  registry already has ID `53`, scheduled name
  `diplomacy_intel_operatives_live`, status `Reserved`, requires data record
  `true`.

No ID migration is required. The integration-slot patch should only move status
if the orchestrator asks for a status transition from `Reserved`.

## Workspace Hook

Add this lane crate to `replay_native/Cargo.toml` members:

```toml
"lanes/diplomacy_intel_operatives_live",
```

Mirror the existing domain-46 pattern in `replay_native/recorder_host/Cargo.toml`:

```toml
diplomacy_intel_operatives_live = { path = "../lanes/diplomacy_intel_operatives_live" }
```

If collector-side module compilation is split into collector dependencies during
the slot, add the same path dependency to `replay_native/collector/Cargo.toml`.

## Collector Coordinator Hook

Target file: `replay_native/collector/src/lib.rs`.

Anchor: inside `build_compact_extraction_gate_records`, beside the existing
`market_trade_live_enabled` and `supply_logistics_attrition_live_enabled` booleans.

Add one scheduled-domain boolean:

```rust
let diplomacy_intel_operatives_live_enabled =
    compact_explicit_domain_enabled(control, "diplomacy_intel_operatives_live");
```

Anchor: same function, beside `let mut supply_logistics_attrition_live_read = None;`.

Add one read slot:

```rust
let mut diplomacy_intel_operatives_live_read = None;
```

Anchor: same function, beside `let supply_logistics_attrition_live_tracker = ...`.

Add one tracker only if the shared collector keeps per-domain trackers for this
lane. Otherwise this lane can be stateless because source-proof gating is in the
domain module:

```rust
let diplomacy_intel_operatives_live_tracker =
    &mut trackers.diplomacy_intel_operatives_live;
```

Anchor: thread scope, immediately after the supply/logistics worker spawn.

Add one worker:

```rust
let diplomacy_intel_operatives_live_handle = if diplomacy_intel_operatives_live_enabled {
    Some(scope.spawn(move || {
        diplomacy_intel_operatives_live::collector_module::read_domain53(
            state,
            status_flags,
            diplomacy_intel_operatives_live_tracker,
        )
    }))
} else {
    None
};
```

Required behavior:

- `read_domain53` must build typed rows only from source mask `0x7F` roots.
- It must not emit war rows or country at-war/casualty projections without
  `SOURCE_PROOF_WAR_ROOT_870EBF`.
- It must keep `STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED` set on schema-v1 war
  rows/projections until `SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF` proves
  start-date scalar and war grouping/name extraction.
- It must encode operation detail and intel-map percentage gaps as omitted/badged
  fields, not `SOURCE_UNAVAILABLE` or `PARTIAL_DATA`.
- If `build_domain53_payload(...).ok() == false`, suppress the data record and
  report a source error for this domain.

Anchor: worker join section, beside supply/logistics join handling.

Add one join arm:

```rust
diplomacy_intel_operatives_live_read = match diplomacy_intel_operatives_live_handle {
    Some(handle) => Some(handle.join().unwrap_or_else(|_| {
        diplomacy_intel_operatives_live::collector_module::worker_panic_read(status_flags)
    })),
    None => None,
};
```

Anchor: append section immediately after supply/logistics append.

Add one perf record and one data record:

```rust
if let Some(read) = diplomacy_intel_operatives_live_read {
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: 53,
            read_time_us: read.read_time_us,
            row_count: read.observed_rows,
            changed_rows: read.changed_rows,
            unit_count: read.emitted_rows,
            changed_unit_count: read.changed_rows,
            status_flags: read.status_flags,
        },
        elapsed_wall_ms,
    );
    sequence = sequence.saturating_add(1);
    append_raw_compact_domain_data_record(
        &mut out,
        sequence,
        game_hour_raw,
        53,
        &read.payload,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

If the shared collector does not expose a generic `append_raw_compact_domain_data_record`,
add one equivalent to existing domain appenders: `H4CR`, version `1`, header len
`32`, record type `3`, domain `53`, payload length, flags `0`, payload bytes.

## Rust Recorder Hook

Target file: `replay_native/recorder_host/src/main.rs`.

Add constant near domain constants:

```rust
const COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE: u16 = 53;
```

Add decoder function near the current domain-46 lane decoder:

```rust
fn decode_compact_diplomacy_intel_operatives_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    let lane_summary = diplomacy_intel_operatives_live::compact::decode_payload(offset, payload);
    summary.status_flags_or |= lane_summary.status_flags_or;
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total =
            domain.data_rows_total.saturating_add(lane_summary.data_rows_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.data_changed_rows_total);
        for (name, count) in lane_summary.data_units_total {
            let entry = domain.data_units_total.entry(name.to_string()).or_default();
            *entry = entry.saturating_add(count);
        }
    }
}
```

Anchor: domain data `match domain_id` branch in `summarize_compact_stream`.

Add branch:

```rust
COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE => {
    decode_compact_diplomacy_intel_operatives_live_data_payload(
        offset,
        payload,
        &mut summary,
    )
}
```

Expected semantic counters after the lane-local proof:

- `diplomacy_intel_rows`
- `diplomacy_country_rows`
- `opinion_rows`
- `relation_flag_rows`
- `subject_rows`
- `intel_agency_rows`
- `operative_rows`
- `cryptology_rows`
- `operation_anchor_rows`
- `intel_map_rows`
- `war_relation_rows` must remain `0` until the war root is accepted.
- `STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED` must be present whenever schema-v1
  war rows or country war/casualty projections are emitted.

## Required Shared Proof After Slot Opens

Run a DLL/Rust compact capture with scheduled domain
`diplomacy_intel_operatives_live`, then run the compact replay verification gate
against the integrated run with `.battleplan` verification enabled. The proof must
show:

- hourly-or-lower frames at speed five/current equivalent;
- zero skipped hours;
- no `WORKER_PANIC`, `BRIDGE_UNAVAILABLE`, `SOURCE_UNAVAILABLE`, or
  `PARTIAL_DATA` bits for domain `53`;
- `.battleplan` container opens and preserves `collector_stream.bin` bytes;
- unresolved war-lifecycle/operation/intel-map/incoming-action surfaces are
  omitted/badged, not guessed.
