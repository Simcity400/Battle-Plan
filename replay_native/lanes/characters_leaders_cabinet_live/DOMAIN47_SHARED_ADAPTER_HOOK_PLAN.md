# domain-47 shared adapter hook plan

Status: not applied. This is the smallest shared collector action remaining
after the lane-local adapter implementation in `src/adapter.rs`.

The existing `DOMAIN47_REGISTRATION_ONLY.patch` covers Cargo dependency,
domain-registry activation, scheduled-name mapping, and recorder decoder/gate
registration. This file covers the collector hot-loop hook that patch
deliberately does not include.

## Shared blocker

The lane-local adapter expects validated roots, not broad scans during the
scheduled frame. The shared collector still needs a root cache that can supply:

- `ResolvedTypeRoots` for `CCharacter`, `CArmyLeader`, `CNavyLeader`,
  `CCountryLeader`, `CAdvisor`, and `CUnitLeaderTrait`;
- warm-pinned `CharacterRoot`, `UnitLeaderRoot`, `CountryLeaderRoleRoot`, and
  `AdvisorRoot` inputs;
- `TraitDefinitionRoot` token mappings;
- `CountryPoliticsRoot` rows with live `CCountry*`, tag database id, ruling
  party metadata, and country-scoped active politics idea tokens;
- `CharacterPortraitInput` static portrait metadata keyed by live character
  identity.

Until that cache exists, a shared patch can register id `47` but cannot
truthfully produce production frames without emitting source/bridge/partial
flags.

The live-refresh v2 proof proves why this hook must be native/scheduled rather
than another Python full-root scan: one lane-local compact record verified
through `.battleplan`, but the root walk took `507.542` wall seconds while the
running game advanced `745` game hours. The separate clock probe validated
`10` advanced hours with `0` skipped hours at `0.05s` polling, so the blocker is
domain-root capture cadence, not an unreadable game clock.

The lane-owned scheduled API now has release-mode native proof at live-ish row
scale:

```powershell
cargo run --release --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --bin scheduled_adapter_benchmark -- --frames 120 --out-dir replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1
```

Result: `28247` rows per frame, `avg_read_time_us=49304`,
`max_read_time_us=102401`, `status_flags_or=0`, `schema_flags_or=7`,
`parse_errors=[]`, `appointed_rows=3`, and `portrait_rows=128`. The last
native scheduled frame was packed through `replay_record_verify` and
independently verified with `pack_replay verify`. This does not replace shared
DLL capture; it proves the lane-owned hot-loop work is no longer the blocker.

The cadence budget proof compares that scheduled benchmark to the current live
clock cadence probe: `observed_hour_wall_us=587700`,
`avg_fraction_of_observed_hour=0.0839`, and
`max_fraction_of_observed_hour=0.1742`. This says full domain-47 cached-root
read/encode/gate cost is below the observed hourly budget; it still leaves
shared cache population, scheduler invocation, and coordinator append unproven.

The shared collector also needs a tiny memory wrapper implementing the lane
`LaneMemory` trait:

```rust
struct CharactersLeadersCabinetLiveMem;

impl CharactersLeadersCabinetLiveMemory for CharactersLeadersCabinetLiveMem {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        read_u8(addr as usize)
    }
}
```

Use the shared collector's actual safe read helper names if they differ at
integration time.

## Collector imports

Add after other lane imports in `replay_native/collector/src/lib.rs`:

```rust
use characters_leaders_cabinet_live_lane::{
    adapter::{
        AdvisorRoot, CharacterPortraitInput, CharacterRoot, CountryLeaderRoleRoot,
        CountryPoliticsRoot, TraitDefinitionRoot, UnitLeaderRoot,
    },
    extractor::{
        LaneMemory as CharactersLeadersCabinetLiveMemory, ResolvedTypeRoots,
        CADVISOR_CHARACTER_PTR_OFF, CADVISOR_IDEA_TOKEN_OFF, CADVISOR_RTTI_RVA,
        CADVISOR_SLOT_OFF, CARMY_LEADER_RTTI_RVA, CCHARACTER_COUNTRY_TAG_DB_ID_OFF,
        CCHARACTER_ID_OFF, CCHARACTER_NAME_OFF, CCHARACTER_RTTI_RVA,
        CCHARACTER_TOKEN_OFF, CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF,
        CCHARACTER_UNIT_LEADER_PTR_OFF, CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        CCOUNTRY_LEADER_RTTI_RVA, CNAVY_LEADER_RTTI_RVA, CUNIT_LEADER_ID_OFF,
        CUNIT_LEADER_NAME_OFF, CUNIT_LEADER_TOKEN_OFF, CUNIT_LEADER_TRAIT_RTTI_RVA,
        CUNIT_LEADER_TRAIT_TOKEN_OFF, CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
    },
    scheduled::{read_native_scheduled_frame, NativeFrameInput, NativeRootCache},
    UNIT_LEADER_ARMY, UNIT_LEADER_NAVY,
};
```

## Constants

Add with the other raw scheduled-domain names and compact ids:

```rust
const RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN: &str =
    "characters_leaders_cabinet_live";
const COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE: u16 = 47;
```

Add `RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN` to the scheduled-domain
enabled expression near the existing raw lane domains.

## Tracker/root cache

Add to `CompactPerfGateTrackers`:

```rust
characters_leaders_cabinet_live: CompactCharactersLeadersCabinetLiveTracker,
```

The tracker should cache roots and static metadata only:

```rust
#[derive(Default)]
struct CompactCharactersLeadersCabinetLiveTracker {
    cache: NativeRootCache,
    country_politics_roots: Vec<CountryPoliticsRoot>,
}
```

Populate `cache` outside hook-held callbacks, using the same cold-scan /
warm-pin discipline as other compact root caches. `NativeRootCache` contains:

- `roots: Option<ResolvedTypeRoots>`;
- `roots_validated`;
- stable `CharacterRoot`, `UnitLeaderRoot`, `TraitDefinitionRoot`,
  `CountryLeaderRoleRoot`, `AdvisorRoot`, and `CharacterPortraitInput` vectors.

Refresh `country_politics_roots` from live country/politics data for each
scheduled frame, because active ideas and ruling-party leader edges are mutable.
Do not decode volatile domain-47 row values into the tracker; the lane scheduled
API rereads mutable values from memory on each scheduled frame.

## Read object

Add a private read result type:

```rust
struct CompactCharactersLeadersCabinetLiveRead {
    payload: Vec<u8>,
    read_time_us: u64,
    row_count: u32,
    data_units_total: u32,
    status_flags: u32,
}
```

## Scheduled read

Add an enabled flag in `build_compact_extraction_gate_records`:

```rust
let characters_leaders_cabinet_live_enabled = compact_explicit_domain_enabled(
    control,
    RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN,
);
```

Add a read slot:

```rust
let mut characters_leaders_cabinet_live_read = None;
```

Borrow the tracker:

```rust
let characters_leaders_cabinet_live_tracker =
    &mut trackers.characters_leaders_cabinet_live;
```

Spawn the worker in the existing `thread::scope`:

```rust
let characters_leaders_cabinet_live_handle =
    if characters_leaders_cabinet_live_enabled {
        Some(scope.spawn(move || {
            read_compact_characters_leaders_cabinet_live(
                state,
                status_flags,
                control.idea_group_filter.as_ref(),
                characters_leaders_cabinet_live_tracker,
                elapsed_wall_ms,
                game_hour_raw,
            )
        }))
    } else {
        None
    };
```

Join it with the other handles:

```rust
characters_leaders_cabinet_live_read =
    join_compact_read(characters_leaders_cabinet_live_handle);
```

Use the shared collector's actual join helper/panic wrapper name if it differs
at integration time.

## Adapter call

Add the read helper:

```rust
fn read_compact_characters_leaders_cabinet_live(
    state: &CollectorState,
    status_flags: u32,
    idea_group_filter: Option<&BTreeMap<String, String>>,
    tracker: &mut CompactCharactersLeadersCabinetLiveTracker,
    elapsed_wall_ms: u64,
    game_hour_raw: i32,
) -> CompactCharactersLeadersCabinetLiveRead {
    let started = Instant::now();
    refresh_characters_leaders_cabinet_live_root_cache(state, tracker);
    let frame_input = build_characters_leaders_cabinet_live_frame_input(
        state,
        status_flags,
        idea_group_filter,
        tracker,
        elapsed_wall_ms,
    );
    let mem = CharactersLeadersCabinetLiveMem;
    let read = read_native_scheduled_frame(&mem, &tracker.cache, frame_input);
    let read_time_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    CompactCharactersLeadersCabinetLiveRead {
        payload: read.payload,
        read_time_us: read_time_us.saturating_add(read.read_time_us),
        row_count: read.row_count,
        data_units_total: read.data_units_total,
        status_flags: read.status_flags,
    }
}
```

The only required shared translation helpers are:

- `refresh_characters_leaders_cabinet_live_root_cache(...)`, which cold-scans or
  warm-validates stable roots into `NativeRootCache`;
- `build_characters_leaders_cabinet_live_frame_input(...)`, which supplies
  `NativeFrameInput { elapsed_wall_ms, status_flags, countries }` from existing
  country identity and politics active-idea paths.

Neither helper should parse domain-47 rows, encode compact bytes, or gate the
payload. That work is lane-owned by `read_native_scheduled_frame(...)`.

Exact helper skeletons:

```rust
fn refresh_characters_leaders_cabinet_live_root_cache(
    state: &CollectorState,
    tracker: &mut CompactCharactersLeadersCabinetLiveTracker,
) {
    let roots = ResolvedTypeRoots {
        ccharacter_vtable: state.base as u64 + CCHARACTER_RTTI_RVA as u64,
        carmy_leader_vtable: state.base as u64 + CARMY_LEADER_RTTI_RVA as u64,
        cnavy_leader_vtable: state.base as u64 + CNAVY_LEADER_RTTI_RVA as u64,
        ccountry_leader_vtable: state.base as u64 + CCOUNTRY_LEADER_RTTI_RVA as u64,
        cadvisor_vtable: state.base as u64 + CADVISOR_RTTI_RVA as u64,
        cunit_leader_trait_vtable: state.base as u64 + CUNIT_LEADER_TRAIT_RTTI_RVA as u64,
    };

    let mut cache = NativeRootCache {
        roots: Some(roots),
        roots_validated: true,
        ..NativeRootCache::default()
    };
    let mut character_by_addr = BTreeMap::<usize, (u32, u32, Option<String>, Option<String>)>::new();
    let mut unit_owner_by_leader = BTreeMap::<usize, u32>::new();

    for addr in scan_qword_hits(roots.ccharacter_vtable as usize, HEAP_LO, HEAP_HI) {
        if read_u64(addr) != Some(roots.ccharacter_vtable)
            || read_u32(addr + CCHARACTER_TYPE_OFF as usize) != Some(CCHARACTER_TYPE_ID)
        {
            continue;
        }
        let Some(char_id) = read_u32(addr + CCHARACTER_ID_OFF as usize) else { continue };
        let Some(country_tag_db_id) = read_u32(addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF as usize) else { continue };
        if char_id == 0 || !(1..=1024).contains(&country_tag_db_id) {
            continue;
        }
        let name = read_msvc_string(addr + CCHARACTER_NAME_OFF as usize, MAX_POLITICS_TOKEN_LEN);
        let token = read_msvc_string(addr + CCHARACTER_TOKEN_OFF as usize, MAX_POLITICS_TOKEN_LEN);
        cache.characters.push(CharacterRoot {
            character_addr: addr as u64,
            name: name.clone(),
            token: token.clone(),
        });
        character_by_addr.insert(addr, (country_tag_db_id, char_id, name, token));
        if let Some(leader_addr) = read_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF as usize)
            .map(|value| value as usize)
            .filter(|value| is_map_control_heap_pointer(*value))
        {
            unit_owner_by_leader.insert(leader_addr, country_tag_db_id);
        }
    }

    refresh_domain47_unit_leader_roots(
        &mut cache,
        roots.carmy_leader_vtable as usize,
        UNIT_LEADER_ARMY,
        &unit_owner_by_leader,
    );
    refresh_domain47_unit_leader_roots(
        &mut cache,
        roots.cnavy_leader_vtable as usize,
        UNIT_LEADER_NAVY,
        &unit_owner_by_leader,
    );

    for trait_addr in scan_qword_hits(roots.cunit_leader_trait_vtable as usize, HEAP_LO, HEAP_HI) {
        if read_u64(trait_addr) != Some(roots.cunit_leader_trait_vtable) {
            continue;
        }
        if let Some(token) =
            read_msvc_string(trait_addr + CUNIT_LEADER_TRAIT_TOKEN_OFF as usize, MAX_POLITICS_TOKEN_LEN)
        {
            cache.unit_leader_trait_tokens.push(TraitDefinitionRoot {
                trait_addr: trait_addr as u64,
                token,
            });
        }
    }

    for role_addr in scan_qword_hits(roots.ccountry_leader_vtable as usize, HEAP_LO, HEAP_HI) {
        if read_u64(role_addr) != Some(roots.ccountry_leader_vtable) {
            continue;
        }
        let Some(character_addr) = read_u64(role_addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF as usize)
            .map(|value| value as usize)
        else {
            continue;
        };
        let Some((country_tag_db_id, _char_id, name, token)) = character_by_addr.get(&character_addr) else {
            continue;
        };
        cache.country_leader_roles.push(CountryLeaderRoleRoot {
            country_leader_addr: role_addr as u64,
            country_tag_db_id: *country_tag_db_id,
            name: name.clone(),
            token: token.clone(),
        });
    }

    for advisor_addr in scan_qword_hits(roots.cadvisor_vtable as usize, HEAP_LO, HEAP_HI) {
        if read_u64(advisor_addr) != Some(roots.cadvisor_vtable) {
            continue;
        }
        let Some(character_addr) = read_u64(advisor_addr + CADVISOR_CHARACTER_PTR_OFF as usize)
            .map(|value| value as usize)
        else {
            continue;
        };
        if !character_by_addr.contains_key(&character_addr) {
            continue;
        }
        let Some(idea_token) =
            read_msvc_string(advisor_addr + CADVISOR_IDEA_TOKEN_OFF as usize, MAX_POLITICS_TOKEN_LEN)
        else {
            continue;
        };
        let slot = read_msvc_string(advisor_addr + CADVISOR_SLOT_OFF as usize, MAX_POLITICS_TOKEN_LEN);
        cache.advisors.push(AdvisorRoot {
            advisor_addr: advisor_addr as u64,
            idea_token,
            slot,
            ledger: None,
        });
    }

    tracker.cache = cache;
}

fn refresh_domain47_unit_leader_roots(
    cache: &mut NativeRootCache,
    leader_vtable: usize,
    leader_kind: u8,
    unit_owner_by_leader: &BTreeMap<usize, u32>,
) {
    for leader_addr in scan_qword_hits(leader_vtable, HEAP_LO, HEAP_HI) {
        if read_u64(leader_addr) != Some(leader_vtable as u64)
            || read_u32(leader_addr + CUNIT_LEADER_TYPE_OFF as usize) != Some(CUNIT_LEADER_TYPE_ID)
            || read_u32(leader_addr + CUNIT_LEADER_ID_OFF as usize).unwrap_or(0) == 0
        {
            continue;
        }
        let Some(country_tag_db_id) = unit_owner_by_leader.get(&leader_addr).copied() else {
            continue;
        };
        cache.unit_leaders.push(UnitLeaderRoot {
            leader_addr: leader_addr as u64,
            country_tag_db_id,
            leader_kind,
            name: read_msvc_string(leader_addr + CUNIT_LEADER_NAME_OFF as usize, MAX_POLITICS_TOKEN_LEN),
            token: read_msvc_string(leader_addr + CUNIT_LEADER_TOKEN_OFF as usize, MAX_POLITICS_TOKEN_LEN),
        });
    }
}

fn build_characters_leaders_cabinet_live_frame_input(
    state: &CollectorState,
    status_flags: u32,
    idea_group_filter: Option<&BTreeMap<String, String>>,
    tracker: &mut CompactCharactersLeadersCabinetLiveTracker,
    elapsed_wall_ms: u64,
) -> NativeFrameInput {
    tracker.country_politics_roots.clear();
    for row in &state.country_identity.rows {
        if row.tag_db_id == 0
            || !is_map_control_heap_pointer(row.object_ptr)
            || read_u64(row.object_ptr) != Some(state.country_identity.country_vtable as u64)
        {
            continue;
        }
        let politics = read_country_politics(
            row.object_ptr,
            state.country_identity.country_vtable,
            idea_group_filter,
            false,
        );
        tracker.country_politics_roots.push(CountryPoliticsRoot {
            country_addr: row.object_ptr as u64,
            country_tag_db_id: row.tag_db_id,
            ruling_party: politics.as_ref().and_then(|value| value.ruling_party.clone()),
            ruler_name: politics
                .as_ref()
                .and_then(|value| value.leader.as_ref().and_then(|leader| leader.name.clone())),
            ruler_token: politics
                .as_ref()
                .and_then(|value| value.leader.as_ref().and_then(|leader| leader.token.clone())),
            active_idea_tokens: politics.and_then(|value| value.ideas).unwrap_or_default(),
        });
    }
    NativeFrameInput {
        elapsed_wall_ms,
        status_flags,
        countries: tracker.country_politics_roots.clone(),
    }
}
```

## Perf/data append

Append after domain `46` / `air_variant_archetypes` and before later air
variant domains:

```rust
if let Some(read) = characters_leaders_cabinet_live_read {
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
            read_time_us: read.read_time_us,
            row_count: read.row_count,
            changed_rows: read.row_count,
            unit_count: read.data_units_total,
            changed_unit_count: read.data_units_total,
            status_flags: read.status_flags,
        },
        elapsed_wall_ms,
    );
    sequence = sequence.saturating_add(1);
    append_compact_record(
        &mut out,
        sequence,
        game_hour_raw,
        COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &read.payload,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

This keeps the domain-47 payload lane-owned while letting the shared coordinator
own compact sequence numbers. The lane-local `LaneCollectorModule` remains valid
for isolated fixture/probe proof, but shared production capture should use the
shared `append_compact_record` helper.

## Recorder summary

`DOMAIN47_REGISTRATION_ONLY.patch` already adds the recorder-side decoder
adapter. The summary fields to merge are:

- `character_rows`
- `unit_leader_rows`
- `unit_leader_trait_rows`
- `country_leader_role_rows`
- `active_ruler_rows`
- `advisor_roster_rows`
- `appointed_advisor_rows`
- `character_portrait_rows`

## Proof after applying shared hook

Run the shared compact extraction gate with only this domain scheduled. Accept
only if domain `47` emits one perf record and one data record on every scheduled
game hour, Rust decode has zero parse errors, `.battleplan` verifies, the
cadence is hourly-or-better at speed five/current equivalent, zero game hours
are skipped, and no worker/source/partial/bridge flags are set.
