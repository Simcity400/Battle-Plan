# domain-47 shared CollectorState mapping

Status: lane-local mapping, not applied. This is the exact data contract for
populating `scheduled::NativeRootCache` and `scheduled::NativeFrameInput` from
the one shared collector.

## Existing Shared Inputs

The shared collector already has these usable production surfaces in
`replay_native/collector/src/lib.rs`:

- `CollectorState { base, date_ptr, country_database_ptr, country_identity, ... }`;
- `CountryIdentityCache { country_vtable, rows }`;
- `CountryObjectRow { tag_db_id, object_ptr, politics, politics_residuals, ... }`;
- `CountryPoliticsRow { ruling_party, leader, ideas, advisor_ideas, ... }`;
- `read_country_politics(object_ptr, country_vtable, idea_group_filter, compact)`;
- `read_active_ideas(status_ptr, idea_vtable_primary, idea_vtable_secondary)`;
- heap/read helpers: `scan_qword_hits`, `read_msvc_string`,
  `is_map_control_heap_pointer`, `read_u64`, `read_u32`, `read_i32`, `read_u8`.

Those are enough to wire `NativeFrameInput.countries` without new reverse
engineering.

## NativeRootCache Fields

`roots`:

```rust
ResolvedTypeRoots {
    ccharacter_vtable: state.base as u64 + CCHARACTER_RTTI_RVA as u64,
    carmy_leader_vtable: state.base as u64 + CARMY_LEADER_RTTI_RVA as u64,
    cnavy_leader_vtable: state.base as u64 + CNAVY_LEADER_RTTI_RVA as u64,
    ccountry_leader_vtable: state.base as u64 + CCOUNTRY_LEADER_RTTI_RVA as u64,
    cadvisor_vtable: state.base as u64 + CADVISOR_RTTI_RVA as u64,
    cunit_leader_trait_vtable: state.base as u64 + CUNIT_LEADER_TRAIT_RTTI_RVA as u64,
}
```

`characters`:

- scan `scan_qword_hits(ccharacter_vtable, HEAP_LO, HEAP_HI)`;
- keep only rows with `*(addr+0x08)==73`, `char_id=*(addr+0x0C)` in range,
  and `country_tag_db_id=*(addr+0x68)` in `1..=1024`;
- optional static strings: `name=read_msvc_string(addr+0x28, ...)`,
  `token=read_msvc_string(addr+0x48, ...)` if script-token valid;
- store `CharacterRoot { character_addr, name, token }`;
- build side maps:
  `(country_tag_db_id, char_id) -> character_addr`,
  `character_addr -> (country_tag_db_id, char_id, name, token)`, and
  `*(addr+0xA8) -> country_tag_db_id` for unit-leader ownership.

`unit_leaders`:

- scan `CArmyLeader` and `CNavyLeader` vtables separately;
- keep only rows with `*(leader+0x08)==4713` and nonzero `unit_leader_id`;
- resolve `country_tag_db_id` from the character side map where
  `CCharacter+0xA8 == leader_addr`; if absent, reject the root rather than
  guessing;
- store `UnitLeaderRoot { leader_addr, country_tag_db_id, leader_kind, name,
  token }`.

`unit_leader_trait_tokens`:

- scan `CUnitLeaderTrait` vtable;
- read token from `CUnitLeaderTrait+0x10` with `read_msvc_string`;
- store `TraitDefinitionRoot { trait_addr, token }`;
- roots without a readable script token are rejected.

`country_leader_roles`:

- scan `CCountryLeader` vtable;
- read `character_addr = *(country_leader+0x08)`;
- require the character side map to contain that address;
- store `CountryLeaderRoleRoot { country_leader_addr, country_tag_db_id, name,
  token }` from the character side map.

`advisors`:

- scan `CAdvisor` vtable;
- read `character_addr = *(advisor+0x10)` and require the character side map;
- read idea token from `CAdvisor+0x30` with `read_msvc_string`;
- read slot from `CAdvisor+0x80` when available;
- store `AdvisorRoot { advisor_addr, idea_token, slot, ledger }`.

`portraits`:

- source is static `common/characters` metadata, not a hot-loop read;
- populate `CharacterPortraitInput { country_tag_db_id, char_id, role, size,
  gfx }` only after static character token joins to a live `(country_tag_db_id,
  char_id)` identity from the character/role/advisor roots;
- if the shared collector has no static metadata loader available at hook time,
  leave `portraits=[]` and keep `character_portrait_rows=0`; do not set a
  health failure for missing static portrait enrichment. This is narrower than
  ideal replay coverage but avoids blocking live mutable leader/cabinet capture.

## NativeFrameInput Fields

Build this every scheduled frame:

```rust
NativeFrameInput {
    elapsed_wall_ms,
    status_flags,
    countries: state.country_identity.rows.iter().filter_map(...).collect(),
}
```

For each `CountryObjectRow`:

- `country_addr = row.object_ptr as u64`;
- `country_tag_db_id = row.tag_db_id`;
- require `row.tag_db_id != 0`, `row.object_ptr` heap-like, and
  `*(row.object_ptr)==state.country_identity.country_vtable`;
- get active ideas by calling the existing politics reader with non-compact
  ideas enabled:

```rust
let politics = read_country_politics(
    row.object_ptr,
    state.country_identity.country_vtable,
    control.idea_group_filter.as_ref(),
    false,
);
```

Map:

- `ruling_party = politics.as_ref().and_then(|p| p.ruling_party.clone())`;
- `ruler_name = politics.as_ref().and_then(|p| p.leader.as_ref()?.name.clone())`;
- `ruler_token = politics.as_ref().and_then(|p| p.leader.as_ref()?.token.clone())`;
- `active_idea_tokens = politics.and_then(|p| p.ideas).unwrap_or_default()`.

If `read_country_politics(..., false)` is too expensive in the shared frame,
replace it with a thin shared helper that calls only:

- `validated_political_status_ptr`;
- `read_active_ideas`;
- `read_political_party_ideology`;
- optionally `read_active_country_leader`.

That helper should return the same fields above and must not parse or emit
domain-47 compact rows.

## Append Boundary

The shared worker calls:

```rust
let read = read_native_scheduled_frame(&mem, &tracker.cache, frame_input);
```

Then appends:

- one `DOMAIN_PERF` record with `domain_id=47`, `read.read_time_us`,
  `read.row_count`, `read.data_units_total`, and `read.status_flags`;
- one `DOMAIN_DATA` record with `read.payload` through the shared
  `append_compact_record` helper.

The shared collector must not inline row encoding or gate parsing.

## Cadence Budget

Lane-local release proof at live-ish row scale:

- `120` frames;
- `28247` rows/frame;
- `avg_read_time_us=49304`;
- `max_read_time_us=102401`;
- observed live hour wall time from clock probe: `587700us`;
- average cost is `0.0839` of observed hour wall time;
- max cost is `0.1742` of observed hour wall time.

This makes hourly-or-better scheduling plausible after cache population, but it
does not prove shared DLL production capture. The unproven pieces are cache
population inside the shared collector and real scheduled append cadence.
