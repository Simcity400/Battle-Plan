# Domain 47 Integration Packet

Lane: `characters_leaders_cabinet_live`  
Compact domain: `47`  
Scope target: validated character/leader/cabinet slice only. Anything not listed
under "Validated Slice" stays out of the shared integration and remains
fail-closed.

This packet is lane-local only. It does not pre-edit shared collector,
recorder, contract, document, or replay UI files.

## Validated Slice

Integrate these domain-47 row families from the lane-owned compact codec:

- `character`: `country_tag_db_id`, `char_id`, optional display `name`, optional
  `token`.
- `unit_leader`: army/navy unit leader identity, `char_id`, bounded `skill`,
  raw XP, current stat bytes, optional name/token, and assigned trait count.
- `country_leader_role`: `CCountryLeader -> CCharacter` role edge.
- `active_ruler`: ruling-party leader vector first entry, with party, name,
  token, vector size, and ambiguity bit.

Defined in the lane codec but excluded from shared emission until current-build
root proof is refreshed:

- `unit_leader_trait`: blocked pending `CUnitLeaderTrait` vtable-root
  revalidation on this build.
- `advisor_roster`: blocked pending `CAdvisor` vtable/root revalidation on this
  build.
- `appointed_advisor`: blocked because it depends on a validated live
  `advisor_roster` root set; domain 14 remains the owner of
  `politics.appointed_advisors`.
- `character_portrait`: blocked until a static portrait feed is keyed to
  validated live character identity.

Everything else remains open or blocked: advisor XP/level, advisor availability
or active predicates, direct `CAdvisor` hired bool, country-leader sub-ideology
runtime pointer, army/fleet assignment containers, general-vs-field-marshal
distinction, commander assignment to OOB, broader character relationship graphs,
scripted/effect semantics, and full live nonzero appointed transition proof under
the shared DLL scheduler.

Display names are player-visible data and should preserve bounded valid UTF-8
from MSVC string storage. Identifier tokens stay ASCII-filtered. If a display
name contains invalid UTF-8 bytes, the collector may lossy-decode it for local
diagnostics, but the invalid sequence is counted as a degraded root read and the
shared frame must take the rowless `PARTIAL_DATA` no-advance path.

## Shared Edit Sites

Apply these shared edits serially. Do not apply them concurrently with other
registration workers.

### Cargo Dependencies

`extractor/native/collector/Cargo.toml`

```toml
characters_leaders_cabinet_live_lane = { package = "characters_leaders_cabinet_live", path = "../lanes/characters_leaders_cabinet_live" }
```

`extractor/native/recorder_host/Cargo.toml`

```toml
characters_leaders_cabinet_live_lane = { package = "characters_leaders_cabinet_live", path = "../lanes/characters_leaders_cabinet_live" }
```

### Compact Contract Descriptor

`extractor/native/compact_contract/src/lib.rs`

Current descriptor:

```rust
domain(
    47,
    "characters_leaders_cabinet_live",
    "characters_leaders_cabinet_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

Required descriptor for integration:

```rust
domain(
    47,
    "characters_leaders_cabinet_live",
    "characters_leaders_cabinet_live",
    CompactDomainStatus::ValidatedSliceOpenGaps,
    true,
    true,
    &[],
),
```

Follow-up tests to update in the same serial patch:

- `collector/src/domain_registry.rs`: remove/replace the `registry.by_id(47).is_none()` expectation and assert scheduled lookup returns `Some(47)`.
- `recorder_host/src/compact_domain_registry.rs`: assert `id_for_scheduled_name("characters_leaders_cabinet_live") == Some(47)` and `wire_name_for_id(47) == Some("characters_leaders_cabinet_live")`.
- `compact_contract/src/lib.rs`: add a profile membership assertion for `47` next to the existing `55` assertion if that test still exists.

Do not mark the lane `ProductionComplete`; this is a validated slice with open
gaps until the combined shared proof passes.

## Collector Wiring

File: `extractor/native/collector/src/lib.rs`

### Imports

Add next to the domain 52/55 lane imports:

```rust
use characters_leaders_cabinet_live_lane as domain47;
```

### Raw Domain Token

Add near the other `RAW_*_DOMAIN` constants:

```rust
const RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN: &str =
    "characters_leaders_cabinet_live";
```

### Compact Domain Constant

Add between 46 and 51:

```rust
const COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE: u16 = 47;
```

### Tracker Field

In `CompactPerfGateTrackers` add:

```rust
characters_leaders_cabinet_live: domain47::scheduled::NativeRootCache,
```

The cache must hold current-build vtable roots and stable root pins. Leave the
domain fail-closed when the cache is absent or invalid.

### Read Struct

Add next to the domain 52/55 read structs:

```rust
struct CompactCharactersLeadersCabinetRead {
    frame: domain47::scheduled::NativeScheduledRead,
}
```

### Scheduling Dependencies

Domain 47 consumes current country politics rows for active ideas and ruling
party context. Model this on the domain 45 post-pass dependency injection:

```rust
let characters_leaders_cabinet_live_enabled = compact_explicit_domain_enabled(
    control,
    RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN,
);
if characters_leaders_cabinet_live_enabled {
    for dependency in [CompactCountryDomainKind::Politics] {
        if !enabled_specs.iter().any(|spec| spec.kind == dependency) {
            if let Some(spec) = compact_country_domain_specs()
                .into_iter()
                .find(|spec| spec.kind == dependency)
            {
                enabled_specs.push(spec);
            }
        }
    }
}
```

Add local read storage:

```rust
let mut characters_leaders_cabinet_live_read = None;
```

Borrow the cache after other trackers:

```rust
let characters_leaders_cabinet_live_tracker =
    &mut trackers.characters_leaders_cabinet_live;
```

Prewarm before the scoped worker block, after `state` and `control` are known:

```rust
if characters_leaders_cabinet_live_enabled {
    prewarm_compact_characters_leaders_cabinet_roots(
        state,
        characters_leaders_cabinet_live_tracker,
    );
}
```

Run the read after the country bundle has joined, not inside the scoped block,
so this domain can consume `country_reads` without double-borrowing the politics
tracker:

```rust
if characters_leaders_cabinet_live_enabled {
    characters_leaders_cabinet_live_read =
        Some(read_compact_characters_leaders_cabinet_live(
            &country_reads,
            status_flags,
            characters_leaders_cabinet_live_tracker,
        ));
}
```

### Concrete Memory Trait

Paste near `Domain52InProcessMemory`:

```rust
struct Domain47InProcessMemory;

impl domain47::extractor::LaneMemory for Domain47InProcessMemory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        let addr = usize::try_from(addr).ok()?;
        read_bytes(addr, 1).and_then(|bytes| bytes.first().copied())
    }
}
```

### Current-Build Root Resolution

Use vtable RVAs, not the lane's historical `*_RTTI_RVA` labels unless they
match a validated vtable. Current shared collector constants already provide:

- `POLITICS_CHARACTER_VTABLE_RVA = 0x2967E30`
- `COUNTRY_LEADER_VTABLE_RVA = 0x296A720`
- `ARMY_LEADER_VTABLE_RVA = 0x293F538`
- `NAVY_LEADER_VTABLE_RVA = 0x293F650`

`CAdvisor` and `CUnitLeaderTrait` vtable roots have not been revalidated on the
current build. The shared collector must keep those root fields zero and must
leave `unit_leader_trait_tokens`, `advisors`, and `portraits` empty. The lane
codec still defines those row kinds for lane-local fixtures and future proof,
but shared emission is excluded until revalidation.

Root helper shape:

```rust
fn domain47_resolved_type_roots(
    state: &CollectorState,
) -> Option<domain47::extractor::ResolvedTypeRoots> {
    Some(domain47::extractor::ResolvedTypeRoots {
        ccharacter_vtable: state.base.checked_add(POLITICS_CHARACTER_VTABLE_RVA)? as u64,
        carmy_leader_vtable: state.base.checked_add(ARMY_LEADER_VTABLE_RVA)? as u64,
        cnavy_leader_vtable: state.base.checked_add(NAVY_LEADER_VTABLE_RVA)? as u64,
        ccountry_leader_vtable: state.base.checked_add(COUNTRY_LEADER_VTABLE_RVA)? as u64,
        // Advisor/appointee/portrait rows stay excluded pending CAdvisor
        // vtable revalidation; never substitute the historical RTTI label.
        cadvisor_vtable: 0,
        // Unit-leader trait token rows stay excluded pending
        // CUnitLeaderTrait vtable revalidation.
        cunit_leader_trait_vtable: 0,
    })
}
```

Do not paste `CADVISOR_RTTI_RVA` or `CUNIT_LEADER_TRAIT_RTTI_RVA` into these
vtable fields; they are not proven to equal current-build vtable roots in this
checkout.

### Root Cache Prewarm

Paste this scaffold and fill only the validation helpers that remain true
against the then-current collector constants. It is designed to fail closed:

```rust
fn prewarm_compact_characters_leaders_cabinet_roots(
    state: &CollectorState,
    cache: &mut domain47::scheduled::NativeRootCache,
) {
    if cache.has_stable_roots() {
        return;
    }
    let Some(roots) = domain47_resolved_type_roots(state) else {
        *cache = domain47::scheduled::NativeRootCache::default();
        return;
    };
    let mem = Domain47InProcessMemory;
    let mut next = domain47::scheduled::NativeRootCache {
        roots: Some(roots),
        roots_validated: true,
        ..domain47::scheduled::NativeRootCache::default()
    };
    let mut skipped_root_rows = 0u32;

    let mut character_country_by_unit_leader = BTreeMap::<u64, u32>::new();
    for addr in scan_qword_hits(roots.ccharacter_vtable as usize, HEAP_LO, HEAP_HI) {
        let Ok(raw) = domain47::extractor::read_character_raw_fields(&mem, &roots, addr as u64) else {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        };
        if !domain47_valid_tag_db_id(raw.country_tag_db_id) {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        }
        if raw.unit_leader_ptr != 0 {
            character_country_by_unit_leader.insert(raw.unit_leader_ptr, raw.country_tag_db_id);
        }
        next.characters.push(domain47::adapter::CharacterRoot {
            character_addr: addr as u64,
            name: domain47_read_msvc_display_string(
                addr + domain47::extractor::CCHARACTER_NAME_OFF as usize,
                160,
                &mut skipped_root_rows,
            ),
            token: read_msvc_string(addr + domain47::extractor::CCHARACTER_TOKEN_OFF as usize, 160),
        });
    }

    for (leader_addr, country_tag_db_id) in character_country_by_unit_leader {
        let Some(leader_addr_usize) = usize::try_from(leader_addr).ok() else {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        };
        let Some(vtable) = read_u64(leader_addr_usize) else {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        };
        let leader_kind = if vtable == roots.carmy_leader_vtable {
            domain47::UNIT_LEADER_ARMY
        } else if vtable == roots.cnavy_leader_vtable {
            domain47::UNIT_LEADER_NAVY
        } else {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        };
        next.unit_leaders.push(domain47::adapter::UnitLeaderRoot {
            leader_addr,
            country_tag_db_id,
            leader_kind,
            name: domain47_read_msvc_display_string(
                leader_addr_usize + domain47::extractor::CUNIT_LEADER_NAME_OFF as usize,
                160,
                &mut skipped_root_rows,
            ),
            token: read_msvc_string(
                leader_addr_usize + domain47::extractor::CUNIT_LEADER_TOKEN_OFF as usize,
                160,
            ),
        });
    }

    for addr in scan_qword_hits(roots.ccountry_leader_vtable as usize, HEAP_LO, HEAP_HI) {
        let Ok(character_addr) = domain47_read_nonzero_u64(
            &mem,
            addr as u64,
            domain47::extractor::CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        ) else {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        };
        let Ok(raw) = domain47::extractor::read_character_raw_fields(
            &mem,
            &roots,
            character_addr,
        ) else {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        };
        if !domain47_valid_tag_db_id(raw.country_tag_db_id) {
            skipped_root_rows = skipped_root_rows.saturating_add(1);
            continue;
        }
        next.country_leader_roles.push(domain47::adapter::CountryLeaderRoleRoot {
            country_leader_addr: addr as u64,
            country_tag_db_id: raw.country_tag_db_id,
            name: None,
            token: None,
        });
    }

    // Excluded fail-closed scope: CAdvisor roots are not revalidated in the
    // shared collector, so advisor_roster and appointed_advisor rows are not
    // emitted here. Portrait rows also stay empty until a static portrait feed
    // keyed to validated live characters exists.
    // Excluded fail-closed scope: CUnitLeaderTrait vtable roots are not
    // revalidated in the shared collector, so unit_leader_trait token rows are
    // not emitted from this integration.
    next.debug_assert_shared_collector_excluded_roots_empty();
    if skipped_root_rows != 0 || next.stable_root_count() == 0 {
        *cache = domain47::scheduled::NativeRootCache {
            skipped_root_rows,
            ..domain47::scheduled::NativeRootCache::default()
        };
    } else {
        *cache = next;
    }
}

fn domain47_read_nonzero_u64<M: domain47::extractor::LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
) -> Result<u64, ()> {
    let mut bytes = [0_u8; 8];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = mem.read_u8(base + offset + idx as u64).ok_or(())?;
    }
    let value = u64::from_le_bytes(bytes);
    if value == 0 {
        Err(())
    } else {
        Ok(value)
    }
}
```

The `raw` variable in the country-leader loop intentionally only validates that
the pointed character decodes under current roots. The packet does not use
`raw.char_id` directly because the lane adapter re-reads the same roots and owns
row construction.

Every root skip or early drop in this helper must increment
`skipped_root_rows`. Any nonzero skip count is a degraded root set and must
produce a rowless `PARTIAL_DATA` read with no tracker advance.

The previous mini-stack proof used this same fail-closed shape: a root is
eligible only after vtable + pointed character validation pass.

### Excluded Root Invariant

The lane-level invariant helper
`NativeRootCache::debug_assert_shared_collector_excluded_roots_empty()` must be
called by the shared collector before caching roots. This documents and
debug-asserts that the shared collector never populates `unit_leader_trait`,
`advisor_roster`, `appointed_advisor`, or `character_portrait` roots until
`CUnitLeaderTrait`, `CAdvisor`, and portrait-feed roots are revalidated.

### Read Function

Add next to domain 52/55 read functions:

```rust
fn read_compact_characters_leaders_cabinet_live(
    country_reads: &[CompactBundleRead],
    status_flags: u32,
    cache: &domain47::scheduled::NativeRootCache,
) -> CompactCharactersLeadersCabinetRead {
    let countries = domain47_country_politics_roots(country_reads);
    let frame = domain47::scheduled::read_native_scheduled_frame(
        &Domain47InProcessMemory,
        cache,
        domain47::scheduled::NativeFrameInput {
            elapsed_wall_ms: 0,
            status_flags,
            countries,
        },
    );
    CompactCharactersLeadersCabinetRead { frame }
}

fn domain47_country_politics_roots(
    country_reads: &[CompactBundleRead],
) -> Vec<domain47::adapter::CountryPoliticsRoot> {
    let mut rows = Vec::new();
    for read in country_reads {
        for row in &read.rows {
            let Some(politics) = row.politics.as_ref() else {
                continue;
            };
            rows.push(domain47::adapter::CountryPoliticsRoot {
                country_addr: row.object_ptr as u64,
                country_tag_db_id: row.tag_db_id,
                ruling_party: politics.ruling_party.clone(),
                ruler_name: politics.leader.as_ref().and_then(|leader| leader.name.clone()),
                ruler_token: politics.leader.as_ref().and_then(|leader| leader.token.clone()),
                active_idea_tokens: politics.ideas.clone().unwrap_or_default(),
            });
        }
    }
    rows
}
```

If the then-current scheduled API expects elapsed wall time, pass
`duration_millis_u64(read_started.elapsed())` exactly as domain 52 does.
If compact politics still omits active ideas, `active_idea_tokens` is empty and
the appointed-advisor row family stays out until the shared country politics
read feeds live idea tokens.

### Panic Wrapper

```rust
fn compact_characters_leaders_cabinet_worker_panic_read(
    status_flags: u32,
) -> CompactCharactersLeadersCabinetRead {
    let cache = domain47::scheduled::NativeRootCache::default();
    let frame = domain47::scheduled::read_native_scheduled_frame(
        &Domain47InProcessMemory,
        &cache,
        domain47::scheduled::NativeFrameInput {
            elapsed_wall_ms: 0,
            status_flags: status_flags
                | COMPACT_STATUS_WORKER_PANIC
                | COMPACT_STATUS_PARTIAL_DATA,
            countries: Vec::new(),
        },
    );
    CompactCharactersLeadersCabinetRead { frame }
}
```

This wrapper is only needed if the serial worker chooses a scoped worker. The
post-country read above does not spawn. Keep the same `COMPACT_STATUS_*` flags
used by domain 52; their bit values match the lane-local status constants.

### Perf/Data Emission

Add after domain 46 or near domain 52 in deterministic sequence order:

```rust
if let Some(read) = characters_leaders_cabinet_live_read {
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
            read_time_us: read.frame.read_time_us,
            row_count: read.frame.row_count,
            changed_rows: read.frame.row_count,
            unit_count: read.frame.data_units_total,
            changed_unit_count: read.frame.data_units_total,
            status_flags: read.frame.status_flags,
        },
        elapsed_wall_ms,
    );
    sequence = sequence.saturating_add(1);
    append_compact_characters_leaders_cabinet_data_record(
        &mut out,
        sequence,
        game_hour_raw,
        &read.frame,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

Append helper:

```rust
fn append_compact_characters_leaders_cabinet_data_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    frame: &domain47::scheduled::NativeScheduledRead,
) {
    append_compact_record(
        out,
        sequence,
        game_hour_raw,
        COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &frame.payload,
    );
}
```

## Recorder Decode

File: `extractor/native/recorder_host/src/main.rs`

Import and constant:

```rust
use characters_leaders_cabinet_live_lane as domain47;

const COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE: u16 = 47;
```

Add decode arm next to 52/55:

```rust
COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE => {
    decode_compact_characters_leaders_cabinet_live_data_payload(
        offset,
        payload,
        &mut summary,
    )
}
```

Decoder function:

```rust
fn decode_compact_characters_leaders_cabinet_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    let lane_summary = domain47::decode_payload(offset, payload);
    summary.status_flags_or |= lane_summary.status_flags_or;
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.row_count_total);
        for (name, count) in lane_summary.data_units_total {
            let entry = domain.data_units_total.entry(name.to_string()).or_default();
            *entry = entry.saturating_add(count);
        }
    }
}
```

Because `compact_domain_registry` is descriptor-backed, the scheduled-name and
wire-name mapping should come from the compact contract descriptor change. Do
not add a parallel hardcoded registry unless the then-current file has reverted
away from descriptor-backed lookup.

## World Fold

File: `extractor/native/recorder_host/src/world_fold.rs`

Add import:

```rust
use characters_leaders_cabinet_live_lane as domain47;
```

Add `47` to `FOLDED_DOMAIN_IDS`:

```rust
pub const FOLDED_DOMAIN_IDS: &[u16] =
    &[10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 47, 55];
```

Add match arm:

```rust
47 => self.fold_characters_leaders_cabinet_live(payload),
```

Keep the parser/fold logic in `world_fold.rs`. Preserve these behaviors:

- Full snapshot domain, not a delta domain; do not require a first-hour baseline.
- On `compact_contract::compact_status_is_failure(status_flags)`, do not clear
  previous commander/cabinet values. Record a provenance note and a domain
  problem, then return `Ok(())`.
- Fold documented active ruler data to `countries.*.commanders.leader`.
- Fold all row families that exceed the current replay schema shape under
  `countries.*.extra._r2_characters_leaders_cabinet_live`.
- Add `characters_leaders_cabinet_live` to provenance only after a valid fold.

Provenance/failure hook:

```rust
if compact_status_is_failure(decoded.status_flags) {
    self.record_domain_problem(47, "failure_status");
    self.note(format!(
        "characters_leaders_cabinet_live failure status {:#x}; character/leader/cabinet state retained from prior valid hour",
        decoded.status_flags & COMPACT_STATUS_FAILURE_MASK
    ));
    return Ok(());
}
```

If a concurrent worker has added unfolded-domain annotations or first-hour
baseline guards, keep domain 47 in the non-delta/full-snapshot path and only add
extra annotations if a valid domain-47 record is not folded.

## R2 Schema Delta

`replay_interface/documents/REPLAY_SCHEMA.md` already documents:

- `countries.*.commanders.leader`
- `countries.*.commanders.generals[]`, `field_marshals[]`, `admirals[]`
- `countries.*.commanders.advisors[]`
- `countries.*.politics.appointed_advisors[]`
- `countries.*.politics.appointed_advisor_details[]`
- `countries.*.extra`

Conservative fold:

- Populate `countries.*.commanders.leader` from `active_ruler`.
- Keep domain-specific rows under:
  `countries.*.extra._r2_characters_leaders_cabinet_live`.

Draft schema-delta text:

```md
`countries.*.extra._r2_characters_leaders_cabinet_live` | obj | C |
Domain-47 validated slice sidecar for compact `characters_leaders_cabinet_live`:
`characters[]`, `unit_leaders[]`, `country_leader_roles[]`, and
`active_rulers[]`. Advisor roster/appointee, unit-leader-trait, and portrait row
kinds are codec-defined but excluded from shared emission pending current-build
`CAdvisor`, `CUnitLeaderTrait`, and portrait-feed revalidation. The sidecar
preserves raw live character/leader row identities and fields that are not yet
safely mapped into the save-shaped `commanders` arrays, especially
army-vs-field-marshal distinction and command assignment.
Omitted when domain 47 is absent or status-failed.
```

Do not add fields directly to `commanders.generals[]` or
`commanders.advisors[]` unless the schema owner accepts the exact shape and all
unknowns are represented as documented nulls without adding undocumented keys.

## Post-Integration Document Rows

Draft `extractor/documents/DOMAIN_REGISTRY.md` row after shared registration,
not after production proof:

```md
| 47 | `characters_leaders_cabinet_live` | shared integration pending: registered as a validated slice/open-gap compact domain; active-ruler, unit-leader skill/stat, and role-edge rows fold fail-closed; unit-leader-trait, advisor roster/appointee, and portrait rows remain blocked pending current-build root revalidation; full speed-five proof and broader leader/cabinet closure remain open | characters/leaders/cabinet |
```

Draft `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` row after shared
registration, not after production proof:

```md
| `characters_leaders_cabinet_live` / 47 | `SHARED_INTEGRATION_PENDING` | Shared descriptor/collector/recorder/fold registration is applied for validated character, unit-leader, country-leader-role, and active-ruler rows. Unit-leader-trait, advisor roster/appointee, and portrait rows remain blocked pending current-build root revalidation. Production acceptance still requires full speed-five/current-equivalent proof, zero skipped hours, and no source/partial/worker/bridge failures. Broader leader/advisor/cabinet fields remain open or fixture-blocked. |
```

Only after combined proof passes should this row advance to a production-slice
status, and even then it must remain open-gap scoped.

## Out Of Scope / Fixture-Blocked

Keep these out of the integration:

- Advisor XP, skill, level, political power cost, availability, allowed/visible
  trigger result, or active/hired bool from static definitions.
- Direct `CAdvisor` hired bool. Appointed-advisor rows remain blocked until the
  `CAdvisor` root set is revalidated; domain 14 keeps ownership of current
  politics appointed-advisor fields.
- Country-leader direct runtime sub-ideology pointer. Static sub-ideology joins
  remain static metadata until a runtime pointer is proven.
- Army/fleet/army-group assignment and field-marshal vs general classification;
  those belong to OOB/navy command lanes.
- General scripted relationships and effect execution semantics.
- National spirit/law/idea classification outside advisor-role tokens.
- Portrait rows unless the shared collector receives a static portrait feed
  keyed by validated live character identity.
- Any row produced after a compact status failure. Failure hours are unknown.

## Verification Commands

After applying the serial shared patch:

```powershell
cd extractor/native
cargo test -p characters_leaders_cabinet_live
cargo test -p compact_contract
cargo test -p recorder_host
cargo test --workspace
```

## Three Highest-Risk Rechecks

1. Vtable roots: CAdvisor and CUnitLeaderTrait vtables must be re-verified on
   the current build. The lane file has historical `*_RTTI_RVA` names; some are
   vtables in practice and some are not.
2. Borrow/order model: domain 47 should run after country bundle joins if it
   consumes politics rows. Recheck the then-current collector if another worker
   refactors `country_reads`, `frd_depth_read`, or scoped worker ownership.
3. Fold shape: R2 wants schema-conforming folds. Keep only active ruler in the
   documented `commanders.leader` shape unless the schema owner approves moving
   more rows out of `extra._r2_characters_leaders_cabinet_live`.
