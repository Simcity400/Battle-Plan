# Domain 56 Registration Patch Plan

This is an exact plan only. The lane did not edit shared collector or recorder
entrypoints.

## Collector registry

In `extractor/native/collector/src/domain_registry.rs`, insert the assigned domain
descriptor between ids 54 and 1000:

```rust
domain(
    56,
    "naval_missions_status_live",
    "naval_missions_status_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

When the lane-owned collector module is wired into the one shared DLL
coordinator, change status/scheduling in the same patch to the project-standard
active value for newly integrated lanes.

## Recorder registry

In `extractor/native/recorder_host/src/compact_domain_registry.rs`, insert the
matching descriptor:

```rust
domain(
    56,
    "naval_missions_status_live",
    "naval_missions_status_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

## Collector hook

Add one lane-owned domain module registered into the existing injected DLL
collector/coordinator. Do not create a second collector or independent DLL.

Required hot-path inputs:

- current validated naval OOB root cache or equivalent:
  `CGameState -> CCountry -> CFleet -> CTaskForce -> CShip`;
- `CTaskForce+0x18` for current location only;
- province-to-region derivation must fail closed when a province is zero or
  missing from the map;
- `CShip+0x3C` may populate aggregate sunk-convoy count only, not route context.

Fields that must remain badged fixture-required until exact fixtures pass:

- `mission_enum`
- `mission_state`
- `home_base_province_id`
- `repair_base_province_id`
- `repair_progress_milli`
- `retreat_state`
- `convoy_route_context_id`

## Recorder hook

Use `domain56_compact.rs` as the wire contract:

- 44-byte payload header;
- 64-byte rows keyed by `task_force_id`;
- data-unit keys:
  - `naval_mission_status_rows`
  - `naval_mission_observed_task_forces`
  - `naval_mission_validated_location_rows`
  - `naval_mission_derived_region_rows`
  - `naval_mission_fixture_required_rows`
  - `naval_mission_sunk_convoy_count_rows`
  - `naval_mission_location_rows`
  - `naval_mission_region_rows`
  - `naval_mission_rows_with_mission`
  - `naval_mission_home_base_rows`
  - `naval_mission_repair_rows`
  - `naval_mission_retreat_rows`
  - `naval_mission_convoy_route_rows`
  - `naval_mission_sunk_convoy_rows`
  - `naval_mission_fixture_rows`

Preserve the raw compact stream in `.battleplan` through the existing recorder
container path.

## Required fixture gates before production-complete claim

1. Mission diff: stable `task_force_id` before/after UI mission change selects
   mission enum/state offsets.
2. Home-base diff: stable task force or fleet before/after base change selects
   home-base province or `CProvince*` offset and proves it is distinct from
   current location.
3. Repair/retreat fixture: damaged/repairing and retreating task-force snapshots
   select repair base/progress and retreat state offsets.
4. Convoy route fixture: active convoy battle/loss route proof links losses to
   route/mission context, not just aggregate `CShip+0x3C` counters.
