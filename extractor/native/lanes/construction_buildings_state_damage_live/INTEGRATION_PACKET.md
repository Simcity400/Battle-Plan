# Integration Packet: `construction_buildings_state_damage_live` (compact domain 48)

> **A14 closeout (2026-07-17):** this packet records the original pre-v2
> integration plan. Its statements that exact target province/location and
> repair semantics remain open are superseded by `A14_RE_REPORT.md` and
> `A14_RE_EVIDENCE.json`. Compact v2 and the production fold now carry the
> exact typed location, `repair_levels_remaining`, and `partial_health_raw`.
> Historical damage cause is an event-only enum, not persistent current state,
> so the current-state reader intentionally does not invent a last-cause field.

Prepared for the serial shared-integration worker. This packet is lane-local only;
do not treat it as applied shared wiring.

## Validated Slice Targeted

Integrate only the lane-local validated compact payload:

- construction queue rows from validated shared country roots:
  `tag_db_id`, `line_id`, optional `template_id`, `active_factories`,
  `cost_raw`, `produced_raw`, `priority`, and `target_state_id` only when every
  construction row resolves a target state;
- state and province building rows from shared map topology:
  `scope`, `location_id`, `state_id`, numeric `type_id`, `level`, and optional
  `healthy_level` damage signal;
- state resource rows from validated `CState` definition resources:
  numeric `resource_id` and `amount_raw`;
- fail closed on any worker/source/partial/control failure status. Failed frames
  may remain in raw compact data for audit, but the world fold must not surface
  partial rows.

Do not integrate unresolved fields as canonical schema fields: building type name
resolution, resource name resolution, construction `created_date`, construction
target province/location beyond `target_state_id`, repair-cause semantics, and
production-speed/factory-request details remain out.

## Lane Files In This Packet

- `INTEGRATION_PACKET.md` - this shared-registration plan.
- `fold_draft.rs` - unwired fold reference module with unit tests.

## Shared Cargo Dependencies

Add the lane dependency alias to both shared crates that import domain 48:

`extractor/native/collector/Cargo.toml`

```toml
construction_buildings_state_damage_live_lane = { package = "construction_buildings_state_damage_live", path = "../lanes/construction_buildings_state_damage_live" }
```

`extractor/native/recorder_host/Cargo.toml`

```toml
construction_buildings_state_damage_live_lane = { package = "construction_buildings_state_damage_live", path = "../lanes/construction_buildings_state_damage_live" }
```

## Collector Wiring

Model this on the current domain 52 and 55 blocks in
`extractor/native/collector/src/lib.rs`.

### Import

Inside `#[cfg(windows)] mod attach_log`, add:

```rust
use construction_buildings_state_damage_live_lane as domain48;
```

### Raw Domain And Compact Domain Constants

Near the other `RAW_*_DOMAIN` constants:

```rust
const RAW_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE_DOMAIN: &str =
    "construction_buildings_state_damage_live";
```

Near the other compact domain id constants:

```rust
const COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE: u16 = 48;
```

### Keep Country Roots Available For Lightweight Frames

In `lightweight_root_needs_country_identity`, add the domain 48 scheduled-name
check next to the other domains that need `CountryIdentityCache`:

```rust
            || scheduled_domain_enabled(
                Some(control),
                RAW_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE_DOMAIN,
            )
```

### Tracker Field And Tracker Type

Add a tracker field to `CompactFrameTrackers` near the 51/52/55 fields:

```rust
        construction_buildings_state_damage_live: CompactDomain48Tracker,
```

Add the tracker type near `CompactScriptedStateTracker`:

```rust
    #[derive(Default)]
    struct CompactDomain48Tracker {
        topology: Option<CompactMapControlTopology>,
    }
```

### Read Result Type

Add this near `CompactOccupationResistanceComplianceRead`:

```rust
    struct CompactConstructionBuildingsStateDamageRead {
        frame: domain48::LaneFrame,
        read_time_us: u64,
        status_flags: u32,
        observed_root_rows: u32,
        emitted_rows: u32,
    }
```

### Enable Flag, Read Slot, And Tracker Reference

In the per-frame compact scheduling block, add:

```rust
        let construction_buildings_state_damage_live_enabled = compact_explicit_domain_enabled(
            control,
            RAW_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE_DOMAIN,
        );
```

Add the read slot:

```rust
        let mut construction_buildings_state_damage_live_read = None;
```

Add the mutable tracker binding:

```rust
        let construction_buildings_state_damage_tracker =
            &mut trackers.construction_buildings_state_damage_live;
```

### Worker Spawn

Inside the `thread::scope` worker list, place this next to the domain 52 worker:

```rust
            let construction_buildings_state_damage_live_handle =
                if construction_buildings_state_damage_live_enabled {
                    Some(scope.spawn(move || {
                        read_compact_construction_buildings_state_damage_live(
                            state,
                            game_hour_raw,
                            status_flags,
                            construction_buildings_state_damage_tracker,
                        )
                    }))
                } else {
                    None
                };
```

### Worker Join / Panic Wrapper

Inside the join section, add:

```rust
            if let Some(handle) = construction_buildings_state_damage_live_handle {
                construction_buildings_state_damage_live_read = Some(match handle.join() {
                    Ok(read) => read,
                    Err(_) => compact_construction_buildings_state_damage_worker_panic_read(
                        game_hour_raw,
                        status_flags,
                    ),
                });
            }
```

### Perf And Data Emission

Place this near the domain 52 emission block:

```rust
        if let Some(read) = construction_buildings_state_damage_live_read {
            append_compact_domain_perf_record(
                &mut out,
                sequence,
                game_hour_raw,
                CompactDomainPerfRecord {
                    domain_id: COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
                    read_time_us: read.read_time_us,
                    row_count: read.observed_root_rows,
                    changed_rows: read.emitted_rows,
                    unit_count: read.emitted_rows,
                    changed_unit_count: read.emitted_rows,
                    status_flags: read.status_flags,
                },
                elapsed_wall_ms,
            );
            sequence = sequence.saturating_add(1);
            append_compact_construction_buildings_state_damage_data_record(
                &mut out,
                sequence,
                game_hour_raw,
                &read.frame,
            );
            sequence = sequence.saturating_add(1);
            emitted_domains = emitted_domains.saturating_add(1);
        }
```

### Live Memory Adapter, Root Validation, Read Function, Panic Fallback

Add near the domain 52 memory adapter/read functions:

```rust
    struct Domain48InProcessMemory;

    impl domain48::LaneMemory for Domain48InProcessMemory {
        fn read_u8(&self, addr: usize) -> Option<u8> {
            read_u8(addr)
        }

        fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
            read_bytes(addr, len)
        }
    }

    fn read_compact_construction_buildings_state_damage_live(
        state: &CollectorState,
        game_hour_raw: i32,
        status_flags: u32,
        tracker: &mut CompactDomain48Tracker,
    ) -> CompactConstructionBuildingsStateDamageRead {
        let read_started = Instant::now();
        let mem = Domain48InProcessMemory;
        let mut final_status_flags = status_flags;

        let frame = match domain48_roots(state, game_hour_raw, tracker) {
            Ok(roots) => {
                let mut output = domain48::collector_module::collect_domain48_frame(&mem, roots);
                output.frame.status_flags |= status_flags;
                let gate = domain48::gate_decoded_frame(&output.frame);
                if !gate.ok {
                    output.frame.status_flags |= COMPACT_STATUS_PARTIAL_DATA;
                }
                final_status_flags |= output.frame.status_flags;
                output.frame
            }
            Err(_) => {
                final_status_flags |= COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;
                domain48_empty_frame(game_hour_raw, final_status_flags)
            }
        };

        let emitted_rows = frame
            .construction_rows
            .len()
            .saturating_add(frame.building_rows.len())
            .saturating_add(frame.resource_rows.len())
            .min(u32::MAX as usize) as u32;
        let observed_root_rows = frame
            .observed_country_roots
            .saturating_add(frame.observed_state_roots)
            .saturating_add(frame.observed_province_roots);

        CompactConstructionBuildingsStateDamageRead {
            frame,
            read_time_us: duration_micros_u64(read_started.elapsed()),
            status_flags: final_status_flags,
            observed_root_rows,
            emitted_rows,
        }
    }

    fn compact_construction_buildings_state_damage_worker_panic_read(
        game_hour_raw: i32,
        status_flags: u32,
    ) -> CompactConstructionBuildingsStateDamageRead {
        let status_flags = status_flags | COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA;
        CompactConstructionBuildingsStateDamageRead {
            frame: domain48_empty_frame(game_hour_raw, status_flags),
            read_time_us: 0,
            status_flags,
            observed_root_rows: 0,
            emitted_rows: 0,
        }
    }

    fn domain48_empty_frame(game_hour_raw: i32, status_flags: u32) -> domain48::LaneFrame {
        domain48::LaneFrame {
            game_hour_raw,
            status_flags,
            source_flags: domain48::SOURCE_FLAG_DLL_NATIVE
                | domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
            observed_country_roots: 0,
            observed_state_roots: 0,
            observed_province_roots: 0,
            construction_rows: Vec::new(),
            building_rows: Vec::new(),
            resource_rows: Vec::new(),
        }
    }

    fn domain48_roots(
        state: &CollectorState,
        game_hour_raw: i32,
        tracker: &mut CompactDomain48Tracker,
    ) -> Result<domain48::collector_module::CollectorRoots, &'static str> {
        let country_roots = domain48_country_roots(state)?;
        let topology = domain48_topology(state, tracker)?;
        let state_roots = topology
            .states
            .iter()
            .map(|root| domain48::StateRoot {
                state_id: root.state_id,
                state_ptr: root.state_ptr,
            })
            .collect::<Vec<_>>();
        let province_roots = topology
            .provinces
            .iter()
            .filter(|root| root.state_id != 0)
            .map(|root| domain48::ProvinceRoot {
                province_id: root.province_id,
                province_ptr: root.province_ptr,
                state_id: root.state_id,
            })
            .collect::<Vec<_>>();

        if country_roots.is_empty() || state_roots.is_empty() || province_roots.is_empty() {
            return Err("domain48_roots_empty");
        }

        Ok(domain48::collector_module::CollectorRoots {
            module_base: state.base,
            game_hour_raw,
            country_roots,
            state_roots,
            province_roots,
        })
    }

    fn domain48_country_roots(
        state: &CollectorState,
    ) -> Result<Vec<domain48::CountryConstructionRoot>, &'static str> {
        if !state.country_identity.has_scalar_source() {
            return Err("country_roots_unavailable");
        }
        let country_vtable = state.country_identity.country_vtable;
        let mut seen = BTreeSet::new();
        let mut roots = Vec::new();
        for row in &state.country_identity.rows {
            if row.tag_db_id == 0 || !seen.insert(row.tag_db_id) {
                return Err("country_root_duplicate_or_zero");
            }
            if read_u64(row.object_ptr).map(|value| value as usize) != Some(country_vtable) {
                return Err("country_vtable_mismatch");
            }
            roots.push(domain48::CountryConstructionRoot {
                tag_db_id: row.tag_db_id,
                country_ptr: row.object_ptr,
            });
        }
        if roots.len() < MIN_USABLE_COUNTRY_OBJECT_ROWS {
            return Err("insufficient_country_roots");
        }
        Ok(roots)
    }

    fn domain48_topology(
        state: &CollectorState,
        tracker: &mut CompactDomain48Tracker,
    ) -> Result<CompactMapControlTopology, &'static str> {
        if let Some(topology) = tracker.topology.as_ref() {
            if cprovince_array_still_current(
                state,
                topology.province_array,
                topology.province_count,
            ) {
                return Ok(topology.clone());
            }
        }
        tracker.topology = None;
        let (topology, _, _) = build_compact_map_control_topology(state)?;
        tracker.topology = Some(topology.clone());
        Ok(topology)
    }
```

### Append Function

Add near the domain 52 append function:

```rust
    fn append_compact_construction_buildings_state_damage_data_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        frame: &domain48::LaneFrame,
    ) {
        let payload = domain48::encode_lane_frame(frame);
        append_compact_record(
            out,
            sequence,
            game_hour_raw,
            COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
    }
```

## Recorder Decode Arm

Model this on the current domain 52 and 55 decode arms in
`extractor/native/recorder_host/src/main.rs`.

### Import And Constant

At top:

```rust
use construction_buildings_state_damage_live_lane as domain48;
```

Near the compact domain constants:

```rust
const COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE: u16 = 48;
```

### Match Arm

In the `COMPACT_RECORD_TYPE_DOMAIN_DATA` match:

```rust
                    COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE => {
                        decode_compact_construction_buildings_state_damage_live_data_payload(
                            offset,
                            payload,
                            &mut summary,
                        )
                    }
```

### Decoder Function

Add near the other compact payload decoders:

```rust
fn decode_compact_construction_buildings_state_damage_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    let decoded = match domain48::decoder_gate::decode_domain48_compact(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary
                .parse_errors
                .push(format!("offset {offset}: domain48 decode failed: {error}"));
            return;
        }
    };

    summary.status_flags_or |= decoded.frame.status_flags;
    let row_count = decoded
        .summary
        .construction_rows
        .saturating_add(decoded.summary.building_rows)
        .saturating_add(decoded.summary.resource_rows);

    if !compact_contract::compact_status_is_failure(decoded.frame.status_flags)
        && !decoded.gate.ok
    {
        for error in decoded.gate.errors {
            summary
                .parse_errors
                .push(format!("offset {offset}: domain48 gate failed: {error}"));
        }
    }

    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE)
    {
        domain.status_flags_or |= decoded.frame.status_flags;
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(row_count as u64);
        add_summary_unit(
            &mut domain.data_units_total,
            "construction_queue_rows",
            decoded.summary.construction_rows,
        );
        add_summary_unit(
            &mut domain.data_units_total,
            "construction_queue_target_state_rows",
            decoded.summary.construction_rows_with_target_state,
        );
        add_summary_unit(
            &mut domain.data_units_total,
            "building_rows",
            decoded.summary.building_rows,
        );
        add_summary_unit(
            &mut domain.data_units_total,
            "damaged_building_rows",
            decoded.summary.damaged_building_rows,
        );
        add_summary_unit(
            &mut domain.data_units_total,
            "resource_rows",
            decoded.summary.resource_rows,
        );
    }
}

fn add_summary_unit(units: &mut BTreeMap<String, u64>, name: &str, value: usize) {
    let entry = units.entry(name.to_string()).or_default();
    *entry = entry.saturating_add(value as u64);
}
```

If `add_summary_unit` would duplicate an existing helper by the time this packet is
applied, use the existing helper instead.

## World Fold Arm

Move/adapt `fold_draft.rs` into `extractor/native/recorder_host/src/world_fold.rs`.
The draft is intentionally conservative:

- domain 48 is a full-frame domain, not a delta domain;
- success clears and rewrites only domain-48 extra keys;
- failure, malformed data, or a missing domain 48 record clears those extra keys
  before snapshot build to avoid stale construction targets/building health;
- the fold writes data under the explicit key
  `construction_buildings_state_damage_live` because current
  `REPLAY_SCHEMA.md` does not document numeric building/resource ids or target
  state on live construction queue rows.

### Imports, Folded Ids, And State Flags

Add import:

```rust
use construction_buildings_state_damage_live_lane as domain48;
```

Add domain 48 to `FOLDED_DOMAIN_IDS`:

```rust
pub const FOLDED_DOMAIN_IDS: &[u16] =
    &[10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 48, 55];
```

Add `WorldFold` fields:

```rust
    /// Domain 48 full-frame extras are only valid for the current hour.
    domain48_known_this_hour: bool,
    domain48_unknown_this_hour: bool,
```

Initialize both to `false` in `WorldFold::new` and reset them in
`reset_hour_state`.

Before cloning maps in `finish_hour`, clear stale domain 48 extras unless the
current hour successfully folded domain 48:

```rust
        if !self.domain48_known_this_hour {
            self.clear_domain48_extras();
        }
```

Add a provenance note with the other unknown-hour notes:

```rust
        if self.domain48_unknown_this_hour {
            notes.push(
                "construction_buildings_state_damage_live unknown this hour; domain extras omitted"
                    .to_string(),
            );
        }
```

### Apply Arm

Add to `apply_domain_data`:

```rust
            48 => self.fold_construction_buildings_state_damage(payload),
```

### Fold Function And Helpers

Use `fold_draft.rs` as the reference. In `world_fold.rs`, prefer the lane decoder
instead of the draft-local decoder:

```rust
    fn fold_construction_buildings_state_damage(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded = match domain48::decoder_gate::decode_domain48_compact(payload) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.clear_domain48_extras();
                self.domain48_unknown_this_hour = true;
                return Err(error);
            }
        };

        if compact_status_is_failure(decoded.frame.status_flags) {
            self.clear_domain48_extras();
            self.domain48_known_this_hour = false;
            self.domain48_unknown_this_hour = true;
            self.note(format!(
                "construction_buildings_state_damage_live failure status {:#x}; domain extras unknown this hour",
                decoded.frame.status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        if !decoded.gate.ok {
            self.clear_domain48_extras();
            self.domain48_unknown_this_hour = true;
            return Err(decoded.gate.errors.join("; "));
        }

        self.clear_domain48_extras();
        self.domain48_known_this_hour = true;
        self.domain48_unknown_this_hour = false;

        let target_state_unavailable = decoded.frame.source_flags
            & domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE
            != 0;

        let mut construction_by_country: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
        for row in decoded.frame.construction_rows {
            let mut obj = Map::new();
            obj.insert("id".into(), json!(row.line_id));
            obj.insert(
                "template_id".into(),
                row.template_id.map(|value| json!(value)).unwrap_or(Value::Null),
            );
            obj.insert("active_factories".into(), json!(row.active_factories));
            obj.insert("cost_raw".into(), json!(row.cost_raw));
            obj.insert("produced_raw".into(), json!(row.produced_raw));
            obj.insert("priority".into(), json!(row.priority));
            if !target_state_unavailable {
                obj.insert(
                    "target_state_id".into(),
                    row.target_state_id.map(|value| json!(value)).unwrap_or(Value::Null),
                );
            }
            construction_by_country
                .entry(row.tag_db_id)
                .or_default()
                .push(Value::Object(obj));
        }

        for (tag_db_id, rows) in construction_by_country {
            let country = self.country_mut(tag_db_id);
            let production = domain48_object_entry(country, "production");
            let extra = domain48_object_entry(production, "construction_buildings_state_damage_live");
            extra.insert("construction_queue".into(), Value::Array(rows));
            extra.insert("source".into(), json!("C"));
            if target_state_unavailable {
                extra.insert("target_state_available".into(), json!(false));
            }
        }

        let mut state_buildings: BTreeMap<u32, Vec<domain48::BuildingRow>> = BTreeMap::new();
        let mut province_buildings: BTreeMap<u32, Vec<domain48::BuildingRow>> = BTreeMap::new();
        for row in decoded.frame.building_rows {
            match row.scope {
                domain48::SCOPE_STATE => state_buildings.entry(row.state_id).or_default().push(row),
                domain48::SCOPE_PROVINCE => {
                    province_buildings.entry(row.location_id).or_default().push(row)
                }
                _ => return Err(format!("domain48 building row has invalid scope {}", row.scope)),
            }
        }

        for (state_id, rows) in state_buildings {
            let state = self.states.entry(state_id.to_string()).or_default();
            let extra = domain48_object_entry(state, "construction_buildings_state_damage_live");
            extra.insert("state_buildings".into(), domain48_building_rows_by_type(rows));
            extra.insert("source".into(), json!("C"));
        }
        for (province_id, rows) in province_buildings {
            let state_id = rows.first().map(|row| row.state_id).unwrap_or_default();
            let province = self.provinces.entry(province_id.to_string()).or_default();
            province.insert("state_id".into(), json!(state_id));
            let extra = domain48_object_entry(province, "construction_buildings_state_damage_live");
            extra.insert("buildings".into(), domain48_building_rows_by_type(rows));
            extra.insert("source".into(), json!("C"));
        }

        for row in decoded.frame.resource_rows {
            let state = self.states.entry(row.state_id.to_string()).or_default();
            let extra = domain48_object_entry(state, "construction_buildings_state_damage_live");
            let resources = domain48_object_entry(extra, "resource_amount_raw_by_id");
            resources.insert(domain48_resource_key(row.resource_id), json!(row.amount_raw));
            extra.insert("source".into(), json!("C"));
        }

        Ok(())
    }

    fn clear_domain48_extras(&mut self) {
        for country in self.countries.values_mut() {
            if let Some(Value::Object(production)) = country.get_mut("production") {
                production.remove("construction_buildings_state_damage_live");
            }
        }
        for state in self.states.values_mut() {
            state.remove("construction_buildings_state_damage_live");
        }
        for province in self.provinces.values_mut() {
            province.remove("construction_buildings_state_damage_live");
        }
    }
```

Add free helpers near the existing JSON helpers:

```rust
fn domain48_object_entry<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !matches!(parent.get(key), Some(Value::Object(_))) {
        parent.insert(key.to_string(), Value::Object(Map::new()));
    }
    match parent.get_mut(key) {
        Some(Value::Object(obj)) => obj,
        _ => unreachable!("object entry was just inserted"),
    }
}

fn domain48_building_rows_by_type(rows: Vec<domain48::BuildingRow>) -> Value {
    let mut obj = Map::new();
    for row in rows {
        let mut detail = Map::new();
        detail.insert("level".into(), json!(row.level));
        detail.insert(
            "healthy_level".into(),
            row.healthy_level
                .map(|value| json!(value))
                .unwrap_or(Value::Null),
        );
        detail.insert(
            "damaged".into(),
            json!(row.healthy_level.is_some_and(|healthy| healthy < row.level)),
        );
        obj.insert(domain48_type_key(row.type_id), Value::Object(detail));
    }
    Value::Object(obj)
}

fn domain48_type_key(type_id: u32) -> String {
    format!("type_0x{type_id:08x}")
}

fn domain48_resource_key(resource_id: u32) -> String {
    format!("tid_0x{resource_id:08x}")
}
```

### World-Fold Tests To Move From Draft

Move the three tests in `fold_draft.rs` into
`recorder_host/src/world_fold/tests.rs` or the existing test module:

- success folds construction/building/resource extras;
- target state is emitted only when the target-unavailable source flag is absent;
- failure status clears previous extras and emits an unknown-hour note.

## Compact Contract Descriptor Change

Current `compact_contract/src/lib.rs` descriptor for id 48:

```rust
    domain(
        48,
        "construction_buildings_state_damage_live",
        "construction_buildings_state_damage_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
```

After shared collector/recorder/fold wiring and tests pass, change it to:

```rust
    domain(
        48,
        "construction_buildings_state_damage_live",
        "construction_buildings_state_damage_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
```

Update the production-profile test that currently excludes 48:

```rust
        assert!(
            profile_ids.contains(&48),
            "domain 48 is validated/schedulable"
        );
        for excluded in [1, 18, 49, 50, 56, 59, 1000] {
            assert!(
                !profile_ids.contains(&excluded),
                "domain {excluded} must stay out of the production profile"
            );
        }
```

## R2 Schema Delta Text

Do not inline new fields into existing documented rows until schema ownership is
available. Add this text to the schema update queue after integration:

```md
Runtime additive delta for `construction_buildings_state_damage_live` (compact
domain 48): when the domain is successfully folded, country, state, and province
objects may carry a domain-named extra object
`construction_buildings_state_damage_live`.

- `countries.*.production.construction_buildings_state_damage_live`:
  `{source:"C", target_state_available?:bool, construction_queue:[{id,
  template_id, active_factories, cost_raw, produced_raw, priority,
  target_state_id?}]}`
- `states.*.construction_buildings_state_damage_live`:
  `{source:"C", state_buildings:{type_0xNNNNNNNN:{level, healthy_level,
  damaged}}, resource_amount_raw_by_id:{tid_0xNNNNNNNN:amount_raw}}`
- `provinces.*.construction_buildings_state_damage_live`:
  `{source:"C", buildings:{type_0xNNNNNNNN:{level, healthy_level, damaged}}}`

These keys are omitted on source/worker/partial failure and are full-frame,
current-hour data only. Numeric building/resource ids are intentionally preserved
until a validated static id-name dictionary is wired.
```

## Contract And Registry Wording After Integration

Use wording that matches the validated partial slice and does not overclaim.

`extractor/documents/MASTER_EXTRACTION_CONTRACT.md`

```md
| `construction_buildings_state_damage_live` / 48 | `SHARED_INTEGRATION_PENDING` | Shared registered compact slice for construction queue rows, all-or-none target-state ids, numeric state/province building health rows, and numeric state resource rows; production proof, full name-token/resource-token resolution, construction created dates, and wider repair semantics remain open. |
```

Gameplay checklist row:

```md
| Construction/building damage | Construction target state, building damage/repair/health. | `SHARED_INTEGRATION_PENDING` | `construction_buildings_state_damage_live` / 48 shared registered slice; production proof still pending. | Building/resource token resolution, construction created dates, exact target province/location, and broader repair semantics remain open. |
```

`extractor/documents/DOMAIN_REGISTRY.md`

```md
| 48 | `construction_buildings_state_damage_live` | shared registered validated slice: construction queue, all-or-none target-state ids, numeric building health, numeric state resources; token/name and broader repair semantics open | construction/buildings/state damage |
```

The pre-integration `construction_buildings_state_damage_domain_allocation_gate`
binary is superseded once this packet is applied: domain ownership and
schedulability are derived from `compact_contract`, and recorder/collector gates
validate the integrated shared path. Retire that bin instead of updating its old
"shared hooks absent" assumptions.

## Open / Fixture-Blocked Items That Stay Out

- Target state is emitted only when every construction row resolves it. If the
  lane sets `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE`, no
  `target_state_id` is folded.
- Building `type_id` to canonical building token/name is not resolved here.
- Resource `resource_id` to canonical resource token/name is not resolved here.
- Construction target province/location, `created_date`, requested factories,
  per-line speed, and priority semantics beyond the captured `priority` integer
  remain outside this packet.
- Repair mechanics beyond `healthy_level < level` damage detection stay out.
- Full speed-five/current-equivalent production proof and `.battleplan`
  verification remain the serial worker's acceptance gate.
- Any concurrent first-hour-baseline and unfolded-domain annotation changes must
  be rechecked before applying the world-fold snippets. Domain 48 is full-frame,
  not delta, but it still needs the current unknown-hour/failure conventions.
