# diplomacy_intel_operatives_live domain 53 integration packet

> Historical integration packet, superseded for current a729 operation/intel
> families. Domain 53 is now shared-registered and carries exact OPIN v3
> preparing/prepared/running/finished operation instances, atomic four-lane
> intel levels, INGR v3 network graphs with exact per-state strengths and
> mechanically recomputed network-scoped national coverage, and the
> incoming-action/faction trailers.
> The old `FIXTURE_BLOCKED`, `SHARED_INTEGRATION_PENDING`, and
> `omitted_unresolved` examples below are retained only as implementation
> history and must not be used as current promotion state. Current residuals
> are controller presentation joins and an authenticated same-frame GH04
> runtime fixture, the separate
> unresolved agency/operative labels documented by their `_unvalidated` names,
> centralized validation, and the remaining exact-10 consumer rewrites.

GH04 traversal custody is the exact-ten `SUP-02-D1-P1` receipt, actual job
`20260719T223856Z-wave2-r10-sup-02-d1-p1-ea60087c`, artifact SHA-256
`d50e88d325dc098c7e676de879563e99de6b689d6ab58e5cb094320b1c0903e4`.
The older 20-shard jobs remain discovery aliases only.

This packet is lane-local only. It does not apply shared edits. The serial
integration worker should apply these registrations after re-checking the then
current shared files.

## Validated slice targeted

Integrate only the current validated domain 53 slice:

- country diplomacy rows: `faction_group`, optional at-war count, optional
  casualty projection;
- war tag-pair/casualty rows only when the current war root is present, always
  badged with `STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED`;
- relation opinions, relation flags, subjects/autonomy;
- agency scalar rows and raw agency capacity cluster rows
  `agency+0xE4/+0xF0/+0xF4`;
- active operative identity/detected/skill/XP rows plus rooted numeric
  lifecycle flags;
- cryptology target rows;
- compact validated-state observations for relation flag, relation opinion,
  subject autonomy, country faction group, agency scalar, raw agency capacity,
  operative lifecycle, and cryptology.

Keep these out of the shared fold and replay schema until separately proven:

- war lifecycle grouping/name/start-date scalar;
- running operation detail and assigned operative semantics;
- spy-network graph and intel-map percentages;
- incoming/outgoing diplomacy action queues;
- operative traits, nationality, portrait/gfx token, mission, operation
  assignment, recruitable pool, and captured/captured_by semantics;
- final semantic names for agency capacity values `+0xE4/+0xF0/+0xF4`;
- any full event stream beyond validated-state observations.

## Lane-local files in this packet

- `INTEGRATION_PACKET.md`: this packet.
- `fold_draft.rs`: standalone draft parser/fold logic plus unit tests. It is not
  wired into any build. It can be checked with:

```powershell
$env:TMP=(Resolve-Path .).Path
$env:TEMP=$env:TMP
rustc --test extractor\native\lanes\diplomacy_intel_operatives_live\fold_draft.rs -o extractor\native\lanes\diplomacy_intel_operatives_live\fold_draft_tests.exe
```

Remove the generated executable and `.pdb` after local checking.

## Current source status

The lane crate currently exposes:

- `diplomacy_intel_operatives_live::compact::decode_payload`
- `diplomacy_intel_operatives_live::extractor::build_domain53_payload`
- `diplomacy_intel_operatives_live::extractor::CURRENT_VALIDATED_SOURCE_PROOFS`
- `diplomacy_intel_operatives_live::decoder_gate::decode_domain53_record`

Important risk: this lane does not currently expose a domain 52 style Rust
`Memory` trait plus production `collect_rows_from_memory` function. The serial
worker must verify whether that was added by another lane worker before applying
the collector code below. If it still does not exist, add the reader surface
first or this can only register a decoder/fold, not a live collector source.

## Shared dependency hooks

Files:

- `extractor/native/collector/Cargo.toml`
- `extractor/native/recorder_host/Cargo.toml`

Add the current package name as a dependency:

```toml
diplomacy_intel_operatives_live = { path = "../lanes/diplomacy_intel_operatives_live" }
```

Re-check the crate name before applying. Unlike domains 52 and 55, this lane's
current package name is not suffixed `_lane`.

## Collector wiring

Target file: `extractor/native/collector/src/lib.rs`.

### Import

Add beside the domain 52/55 imports:

```rust
use diplomacy_intel_operatives_live as diplomacy53;
```

### Raw domain string and compact id

Add beside the domain 52/55 constants:

```rust
const RAW_DIPLOMACY_INTEL_OPERATIVES_LIVE_DOMAIN: &str =
    "diplomacy_intel_operatives_live";
const COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE: u16 = diplomacy53::DOMAIN_ID;
```

### Tracker field

Add to the compact tracker bundle beside domain 52:

```rust
diplomacy_intel_operatives_live: CompactDiplomacyIntelOperativesLiveTracker,
```

Add the tracker type near the other compact tracker structs:

```rust
#[derive(Default)]
struct CompactDiplomacyIntelOperativesLiveTracker {
    last_source_proofs: u64,
}
```

The tracker is currently only for diagnostics and future delta/baseline guard
room. Domain 53 is full-snapshot, not a delta domain; if the payload ever sets
`COMPACT_STATUS_DELTA_DATA`, the collector and fold should fail closed until a
baseline contract exists.

### Read result

Add beside `CompactOccupationResistanceComplianceRead`:

```rust
struct CompactDiplomacyIntelOperativesLiveRead {
    payload: Vec<u8>,
    read_time_us: u64,
    status_flags: u32,
    observed_rows: u32,
    changed_rows: u32,
    emitted_rows: u32,
}
```

### Enable flag and read slot

In `build_compact_extraction_gate_records`, add beside the domain 52 enable:

```rust
let diplomacy_intel_operatives_live_enabled = compact_explicit_domain_enabled(
    control,
    RAW_DIPLOMACY_INTEL_OPERATIVES_LIVE_DOMAIN,
);
```

Add beside the domain 52 read variable:

```rust
let mut diplomacy_intel_operatives_live_read = None;
```

Add beside the tracker borrows:

```rust
let diplomacy_intel_operatives_live_tracker =
    &mut trackers.diplomacy_intel_operatives_live;
```

### Worker spawn

Add inside the scoped worker section beside domain 52:

```rust
let diplomacy_intel_operatives_live_handle = if diplomacy_intel_operatives_live_enabled {
    Some(scope.spawn(move || {
        read_compact_diplomacy_intel_operatives_live(
            state,
            status_flags,
            diplomacy_intel_operatives_live_tracker,
        )
    }))
} else {
    None
};
```

### Worker join and panic wrapper

Add beside the domain 52 join:

```rust
if let Some(handle) = diplomacy_intel_operatives_live_handle {
    diplomacy_intel_operatives_live_read = Some(match handle.join() {
        Ok(read) => read,
        Err(_) => compact_diplomacy_intel_operatives_worker_panic_read(status_flags),
    });
}
```

### Concrete in-process memory adapter

Use the same live-memory read shape as domain 52. This assumes the lane exposes a
trait named `Domain53Memory` and a collector function named
`extract_rows_from_current_roots`. If names differ at integration time, keep this
adapter shape and adjust only the trait/function names.

```rust
struct Domain53InProcessMemory;

impl diplomacy53::extractor::Domain53Memory for Domain53InProcessMemory {
    fn module_base(&self) -> u64 {
        unsafe { GetModuleHandleW(ptr::null()) as u64 }
    }

    fn read_bytes(
        &self,
        address: u64,
        len: usize,
    ) -> Result<Vec<u8>, diplomacy53::extractor::ExtractError> {
        let addr = usize::try_from(address).map_err(|_| {
            diplomacy53::extractor::ExtractError::MemoryRead { address, len }
        })?;
        read_bytes(addr, len).ok_or(diplomacy53::extractor::ExtractError::MemoryRead {
            address,
            len,
        })
    }
}
```

### Live-memory validation helper and reader

The reader must not manufacture fixture rows. It should port the validated
`870ebf` roots from the lane probes into a Rust reader and pass only rows backed
by matching source-proof bits to `build_domain53_payload`.

```rust
fn read_compact_diplomacy_intel_operatives_live(
    state: &CollectorState,
    status_flags: u32,
    tracker: &mut CompactDiplomacyIntelOperativesLiveTracker,
) -> CompactDiplomacyIntelOperativesLiveRead {
    let read_started = Instant::now();
    let mem = Domain53InProcessMemory;
    let mut final_status_flags = status_flags;

    let read = diplomacy53::extractor::extract_rows_from_current_roots(&mem)
        .and_then(|extracted| {
            let source_proofs = extracted.source_proofs;
            tracker.last_source_proofs = source_proofs;
            let report =
                diplomacy53::extractor::build_domain53_payload(extracted.rows, source_proofs);
            if report.ok() {
                Ok((report, extracted.observed_rows))
            } else {
                Err(diplomacy53::extractor::ExtractError::Validation(
                    report.errors.join("; "),
                ))
            }
        });

    match read {
        Ok((report, observed_rows)) => CompactDiplomacyIntelOperativesLiveRead {
            payload: report.payload,
            read_time_us: duration_micros_u64(read_started.elapsed()),
            status_flags: final_status_flags | report.gate_summary.status_flags_or,
            observed_rows,
            changed_rows: report
                .gate_summary
                .data_changed_rows_total
                .min(u64::from(u32::MAX)) as u32,
            emitted_rows: report
                .gate_summary
                .data_rows_total
                .min(u64::from(u32::MAX)) as u32,
        },
        Err(_) => {
            final_status_flags |= COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;
            let rows = diplomacy53::extractor::ExtractedDomain53Rows::default();
            let report = diplomacy53::extractor::build_domain53_payload(
                rows,
                diplomacy53::extractor::SOURCE_PROOF_BUILD_SHA_870EBF,
            );
            CompactDiplomacyIntelOperativesLiveRead {
                payload: report.payload,
                read_time_us: duration_micros_u64(read_started.elapsed()),
                status_flags: final_status_flags,
                observed_rows: 0,
                changed_rows: 0,
                emitted_rows: 0,
            }
        }
    }
}

fn compact_diplomacy_intel_operatives_worker_panic_read(
    status_flags: u32,
) -> CompactDiplomacyIntelOperativesLiveRead {
    let status_flags = status_flags | COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA;
    let rows = diplomacy53::extractor::ExtractedDomain53Rows::default();
    let report = diplomacy53::extractor::build_domain53_payload(
        rows,
        diplomacy53::extractor::SOURCE_PROOF_BUILD_SHA_870EBF,
    );
    CompactDiplomacyIntelOperativesLiveRead {
        payload: report.payload,
        read_time_us: 0,
        status_flags,
        observed_rows: 0,
        changed_rows: 0,
        emitted_rows: 0,
    }
}
```

If the lane's current `ExtractError` enum does not have `Validation(String)`,
add an equivalent lane-local error variant before integrating, or map validation
failure to the existing source-unavailable path and log the exact errors in the
collector diagnostics.

### Perf and data emission

Add beside the domain 52 append block:

```rust
if let Some(read) = diplomacy_intel_operatives_live_read {
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE,
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
    append_compact_diplomacy_intel_operatives_data_record(
        &mut out,
        sequence,
        game_hour_raw,
        &read.payload,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

Add the appender beside the domain 52 appender:

```rust
fn append_compact_diplomacy_intel_operatives_data_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    payload: &[u8],
) {
    append_compact_record(
        out,
        sequence,
        game_hour_raw,
        COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        payload,
    );
}
```

Collector acceptance checks after applying:

- scheduled profile can request `diplomacy_intel_operatives_live`;
- `WORKER_PANIC`, `SOURCE_UNAVAILABLE`, and `PARTIAL_DATA` are absent on the
  happy path;
- domain 53 data records use record type `3`, domain id `53`, and the lane
  compact payload bytes exactly;
- unresolved operation/intel-map/action-queue surfaces are omitted or resisted
  with domain-local badges, not compact failure flags.

## Recorder decode arm

Target file: `extractor/native/recorder_host/src/main.rs`.

Add import beside domain 52/55 imports:

```rust
use diplomacy_intel_operatives_live as diplomacy53;
```

Add constant beside 52/55:

```rust
const COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE: u16 = 53;
```

Add the match arm beside domain 52:

```rust
COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE => {
    decode_compact_diplomacy_intel_operatives_live_data_payload(
        offset,
        payload,
        &mut summary,
    )
}
```

Add the decoder merge function beside the domain 52 decoder. This must remain a
generic `data_units_total` merge so v20/v22/v26 counters survive.

```rust
fn decode_compact_diplomacy_intel_operatives_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    let lane_summary = diplomacy53::compact::decode_payload(offset, payload);
    summary.status_flags_or |= lane_summary.status_flags_or;
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.data_rows_total);
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

Expected lane-local v26 fixture counters after decode:

- `diplomacy_intel_rows = 20`
- `agency_capacity_cluster_rows = 1`
- `operative_lifecycle_rows = 3`
- `compact_state_event_rows = 8`
- `compact_state_event_relation_opinion_rows = 1`
- `compact_state_event_country_faction_group_rows = 1`

Do not whitelist only these counters; preserve every lane counter.

## World fold

Target file: `extractor/native/recorder_host/src/world_fold.rs`.

Move/adapt `fold_draft.rs` into `world_fold.rs` or a private module. The draft is
self-contained and intentionally uses plain Rust structs. In shared code, convert
its `CountryPatch`/`DiplomacyWorldDomain53Extra` outputs into `serde_json::Map`
overlays.

### Fold registration

Add domain 53 to the understood folded domain list:

```rust
pub const FOLDED_DOMAIN_IDS: &[u16] =
    &[10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 53, 55];
```

Add the match arm:

```rust
53 => self.fold_diplomacy_intel_operatives_live(payload),
```

### Per-hour overlay fields

Domain 53 should be treated as a full per-frame snapshot overlay, not a carried
delta. Add per-hour fields so failure or absence cannot leak stale values:

```rust
domain53_country_overlays_this_hour: BTreeMap<u32, Map<String, Value>>,
domain53_diplomacy_world_extra_this_hour: Option<Map<String, Value>>,
domain53_known_this_hour: bool,
```

Initialize them in `WorldFold::new` and clear them in `reset_hour_state`.

### Failure/provenance handling

The fold arm should follow current failure conventions:

```rust
fn fold_diplomacy_intel_operatives_live(&mut self, payload: &[u8]) -> Result<(), String> {
    let decoded = decode_domain53_fold_payload(payload)?;
    if compact_status_is_failure(decoded.status_flags) {
        self.record_domain_problem(53, "failure_status");
        self.note(format!(
            "diplomacy_intel_operatives_live failure status {:#x}; domain 53 fold skipped this hour",
            decoded.status_flags & COMPACT_STATUS_FAILURE_MASK
        ));
        return Ok(());
    }
    if decoded.status_flags & COMPACT_STATUS_DELTA_DATA != 0 {
        self.record_domain_problem(53, "unexpected_delta_data");
        self.note(
            "diplomacy_intel_operatives_live emitted delta data before any domain 53 baseline contract; fold skipped this hour"
        );
        return Ok(());
    }

    // Apply only validated rows to per-hour overlays. Keep blocked operation,
    // intel-map percentage, incoming action queue, spy-network, and full
    // agency/operative semantics out of the schema fold.
    self.domain53_known_this_hour = true;
    Ok(())
}
```

Because `apply_domain_data` marks a domain after `Ok(())`, this means domain 53
appears in `provenance.collector.domains` only for successfully decoded,
non-failure payloads. Failure and unexpected-delta cases surface under
`domain_problems` and `notes`, not `unfolded_domains`. Unknown future domains
continue to use the existing `unfolded_domains_this_hour` path.

### Snapshot shape

Use the existing `resolve_country_key`/`resolve_country_value` pattern before
writing any TAG-shaped field. If a target cannot be resolved through domain 18,
keep a clearly named `*_tag_db_id` field under a domain53 extra key rather than
fabricating a TAG.

Recommended overlay locations:

```jsonc
"countries": {
  "GER": {
    "diplomacy": {
      "opinion": { "ENG": -35 },
      "guarantees": ["ENG"],
      "domain53_extra": {
        "faction_group": 42,
        "relation_flags": [
          {"target_tag": "ENG", "target_tag_db_id": 3, "flag_kind": 10, "flag_name": "guarantee", "directed": true}
        ],
        "state_events": []
      }
    },
    "intel": {
      "decryption": {"targets": [{"target": "ENG", "target_tag_db_id": 3, "decryption": true, "amount": 9600.0, "amount_raw": 960000000}]},
      "domain53_extra": {
        "agency_scalars": {"defense_raw": 600000, "upgrade_progress_raw": 2400000, "name_hash": 123, "icon_hash": 456},
        "agency_capacity_cluster": {"value_0xe4": 9, "value_0xf0": 10, "value_0xf4": 9, "proof_hash": 51966},
        "operative_lifecycle_rows": [{"op_id": 11387, "detected": false, "skill": 2, "experience_raw": 71372000, "lifecycle_flag": 47, "state_hash": 221}],
        "state_events": []
      }
    }
  }
},
"diplomacy_world": {
  "wars": {},
  "factions": {},
  "subjects": [],
  "domain53_extra": {
    "war_relation_rows": [{"first_tag_db_id": 1, "second_tag_db_id": 3, "first_casualties": 100, "second_casualties": 250}],
    "subject_autonomy_rows": [{"overlord_tag_db_id": 1, "subject_tag_db_id": 22, "autonomy_state_hash": 43981, "autonomy_ratio_raw": 40000}],
    "omitted_unresolved": ["war_lifecycle_detail", "operation_detail", "intel_map_percentages", "incoming_diplomatic_action_queue", "spy_network_graph"]
  }
}
```

Do not populate `diplomacy_world.wars` until war id/name/group/start-date is
validated. Do not populate `countries.*.intel.operations`,
`countries.*.intel.intel_networks`, or `strategic_operatives.pools` from
resisted domain 53 rows.

## compact_contract descriptor change

Target file: `extractor/native/compact_contract/src/lib.rs`.

Current descriptor:

```rust
domain(
    53,
    "diplomacy_intel_operatives_live",
    "diplomacy_intel_operatives_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

After shared collector/recorder/fold registration lands, change to:

```rust
domain(
    53,
    "diplomacy_intel_operatives_live",
    "diplomacy_intel_operatives_live",
    CompactDomainStatus::ValidatedSliceOpenGaps,
    true,
    true,
    &[],
),
```

Do not set `ProductionComplete` or `ProductionCompleteBase`. This is a validated
slice with open gaps and still needs batched live proof.

## R2 replay schema delta text

Do not edit `replay_interface/` from this lane. When the serial worker owns the
schema update, add text equivalent to:

```text
Domain 53 live diplomacy/intel extras are optional collector-fed overlays under
`countries.*.diplomacy.domain53_extra`, `countries.*.intel.domain53_extra`, and
`diplomacy_world.domain53_extra`. They preserve validated raw/hashy values that
do not yet match the documented save-shaped schema: relation flag kind ids, raw
agency capacity offsets `+0xE4/+0xF0/+0xF4`, operative lifecycle numeric flags,
state-event observation hashes, and war tag-pair/casualty rows without war
grouping/start-date. These extras are present only when the collector provenance
includes `diplomacy_intel_operatives_live`. They do not populate operations,
spy networks, intel-map percentages, incoming action queues, or full war
lifecycle fields.
```

## Contract and registry wording after integration

`extractor/documents/DOMAIN_REGISTRY.md` row:

```markdown
| 53 | `diplomacy_intel_operatives_live` | shared-registered validated relation/opinion/subject/agency/raw-capacity/operative-lifecycle/cryptology/state-event and war tag-pair/casualty slices; operations/spy-network/intel-map/incoming-action/war-lifecycle semantics remain open or fixture-blocked | diplomacy/intel/operatives |
```

`extractor/documents/MASTER_EXTRACTION_CONTRACT.md` domain row:

```markdown
| `diplomacy_intel_operatives_live` / 53 | `SHARED_INTEGRATION_PENDING` | Shared collector/recorder/fold registration preserves the lane-local relation/opinion/subject/agency/raw-capacity/operative-lifecycle/cryptology/validated-state-event rows and the war tag-pair/casualty slice; speed-five/current-equivalent combined proof plus operations, spy-network/intel-map, incoming-action, war-grouping/start-date, and semantic fixture closure remain. |
```

Checklist row deltas:

```markdown
| Relations/opinions | Opinion, tension, guarantees, access, non-aggression, attaches, faction relations. | `SHARED_INTEGRATION_PENDING` | Domain 53 shared registration preserves validated relation/opinion/flag rows and compact observed-state relation rows; full field audit/action semantics remain open. | Shared proof plus production rows or exact rejections for every remaining UI/save-visible relation field. |
| Subjects/autonomy | Puppet/subject relation, autonomy progress, overlord, subject levels. | `SHARED_INTEGRATION_PENDING` | Domain 53 shared registration preserves validated `CSubjectRelation` subject-autonomy rows and state observations. | Shared proof plus subject level/static joins and autonomy-change event semantics. |
| War lifecycle | War ids/names, attacker/defender groups, start/end, participants, casualties. | `ROOT_DISCOVERY_OPEN` | Domain 53 shared registration may preserve validated tag-pair/casualty rows, but war grouping/name/start-date scalar remains unvalidated and badged. | War grouping/name/date closure or exact blocker. |
| Intel agency | Agency scalars, upgrades, cryptology, capacity, recruitment. | `SHARED_INTEGRATION_PENDING` | Domain 53 shared registration preserves validated agency/cryptology scalars plus raw capacity cluster rows `agency+0xE4/+0xF0/+0xF4` without semantic relabeling. | Semantic fixtures for exact capacity labels, upgrades, recruitment, and full cryptology state. |
| Operatives | Operative identity, owner, skill/XP, detected state, lifecycle flags. | `SHARED_INTEGRATION_PENDING` | Domain 53 shared registration preserves active operative identity/stat rows and numeric lifecycle flag cohorts. | Mission/captured/pool/nationality/portrait/trait/operation-assignment closure. |
| Operations | Running operation, target, stage/progress, participants, outcome. | `FIXTURE_BLOCKED` | Domain 53 registration does not accept operation-detail rows; resisted anchors stay out of replay operation state. | Same-tick running-operation fixture and shared rows, or exact rejection. |
| Spy networks/intel map | Network strength, coverage, intel percentages, target states/countries. | `FIXTURE_BLOCKED` | Domain 53 registration does not accept spy-network graph or unresisted intel-map percentage rows. | Same-tick intel-map/spy-network fixture and shared rows. |
| Diplomacy action queues | Incoming/outgoing diplomatic actions and timed requests. | `FIXTURE_BLOCKED` | Domain 53 registration does not accept a pending-action queue root. | Live fixture and compact event rows. |
```

## Post-apply verification

Run after shared changes:

```powershell
cd extractor\native
cargo test -p collector
cargo test -p recorder_host
cargo test -p compact_contract
cargo test -p diplomacy_intel_operatives_live
```

Then run a scheduled compact capture with `diplomacy_intel_operatives_live` in
the production profile. The required acceptance line is:

- hourly-or-better/current-equivalent frames, zero skipped game hours, no domain
  53 `WORKER_PANIC`, `BRIDGE_UNAVAILABLE`, `SOURCE_UNAVAILABLE`, `PARTIAL_DATA`,
  or `CONTROL_INVALID`, recorder manifest includes domain 53 counters, and the
  `.battleplan` container preserves `collector_stream.bin` byte-identically.

## Open and fixture-blocked items that stay out

- War lifecycle detail: no war id/name/grouping/start-date scalar.
- Operation detail: current `COperation` table is definition surface, not a
  running-operation root.
- Intel-map percentages and spy-network graph: no live owner/target graph root.
- Incoming diplomatic action queue: no pending queue root.
- Operative full semantics: no mission/operation assignment, traits,
  nationalities, portrait/gfx, recruitable pool, or captured/captured_by proof.
- Agency full semantics: no upgrade branch levels, current upgrade/building,
  recruitment pool, or final capacity labels.
- Full compact event streams: only validated-state observations are integrated.
