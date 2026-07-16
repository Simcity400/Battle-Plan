# country_economy_production_stockpile_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/country_economy_production_stockpile_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local production stack for country economy, production, fuel, and stockpile
variant replay records.

> **Warning: `lane_proof` is a slice-only, non-certifying diagnostic.** It checks
> the physical compact stream and the selected lane domains, but it does not bind
> the full production roster, source-capture identity, or commanded duration. Its
> `ok`/`accepted_for_lane_local_slice` result must never be used as production
> certification. Production admission belongs to
> `../compact_replay_verification_gates/` with the authoritative session manifest.
> Optional manifest substring observations emitted by `lane_proof` are explicitly
> untrusted and never affect lane acceptance or production certification.

This lane does not own market/trade, supply/logistics, construction/state damage,
or air/naval/unit extraction. Joins are pointer-free:

- country rows: `tag_db_id`
- equipment/variant rows: `variant_id`
- production lines: `(tag_db_id, kind, line_id)`
- construction queue rows: `(tag_db_id, line_id, priority)`

## Validated roots used by the current compact path

These are the exact roots and offsets this lane gates. They are implemented in
the shared collector today, but this lane keeps its proof and decoder local.

- `CCountry + 0x0F68 -> CProductionManager*`
- `CProductionManager + 0x220/+0x228/+0x22C` stockpile compact vector
- stockpile entry stride `0x10`: `+0x00 equipment_variant*`, `+0x08 i64 amount`
- equipment variant vtable `module + 0x293AFB8` in the current collector profile,
  type field `+0x08 == 70`, id field `+0x0C == variant_id`
- production line vector `CProductionManager + 0x58/+0x60/+0x64`, pointer stride
  `0x08`
- production line fields: type `+0x08` (`56` military, `57` naval), id `+0x0C`,
  active factories `+0x18`, cost `+0x28`, speed `+0x30`, produced `+0x38`,
  amount `+0x48`
- construction queue vector `CProductionManager + 0x70/+0x78/+0x7C`, pointer
  stride `0x08`, type `+0x08 == 59`, template subobject at line `+0x70`, template
  id at subobject `+0x08`
- `CCountry + 0x1580 -> CCountryFuelStatus*`
- fuel status fields: current fuel `+0x08`, max fuel `+0x10`, gain base `+0x18`,
  fuel cost `+0x38`, all stored as signed `i64 / 32768`

## Compact domains

- `10 fuel_live`: country fuel rows, fixed scale `32768`
- `11 production_live`: stockpiles, military/naval production lines, construction
  queue rows, fixed scale `100000`
- `37 stockpile_variant_defs`: stockpile-sourced variant catalog rows and module
  token children
- `40 stockpile_variant_upgrades`: stockpile-sourced variant upgrade rows
- `611 economy_production_gap_badges`: lane-local proof-only badges for remaining
  economy/production gaps. This is not a shared collector domain.

## Remaining gap badges

The accepted production/fuel/stockpile streams do not encode every economy
surface. The lane-local `gap_report` proof fail-closes the remainder:

- `requested_factory_counts`: schema-blocked. Save path exists at
  `countries.<TAG>.production.*_lines.requested_factories`, but current domain
  `11` line rows are only `tag_db_id`, `kind`, `line_id`, `active_factories`,
  `cost`, `speed`, `produced`, and `amount`.
- `consumer_goods`: source-blocked. Scripted consumer-goods variables exist in
  coverage-audit paths, but no validated numeric country consumer-goods live root
  is in the hot-loop compact path.
- `resource_flows_shortages`: source-blocked. Save paths exist for
  `countries.<TAG>.resources.{produced,exported,to_export}`; state deposits are
  covered elsewhere and this lane must not duplicate market/trade ownership.
- `deployment_queues`: source-blocked. Save path
  `countries.<TAG>.deployment.military_deployment_conveyor[]` exists, but no live
  conveyor vector/root is validated.
- `mio_design_variant_production_joins`: schema-blocked. Variant defs/upgrades
  are present, but production line rows do not include `equipment_variant_index`
  or MIO ids.
- `economy_law_derived_state`: owned-elsewhere badge. Law/idea identity belongs
  to characters/politics; this lane only owns numeric economy effects once rooted.
- `trade_factory_impact`: owned-elsewhere badge. Market/trade owns contract-level
  factory impact; this lane may later consume granted aggregate joins only.

## Local commands

```powershell
cargo test --manifest-path extractor\native\lanes\country_economy_production_stockpile_live\Cargo.toml
cargo run --manifest-path extractor\native\lanes\country_economy_production_stockpile_live\Cargo.toml --bin lane_proof -- --stream extractor\native\runs\compact_production_live_speed5_v1\collector_stream.bin --manifest extractor\native\runs\compact_production_live_speed5_v1\native_session_manifest.json --replay extractor\native\runs\compact_production_live_speed5_v1\native_session.battleplan --out extractor\native\runs\country_economy_production_stockpile_live_20260701_v1\production_live_slice_proof.json --expected production_live
cargo run --manifest-path extractor\native\lanes\country_economy_production_stockpile_live\Cargo.toml --bin gap_report -- --production-stream extractor\native\runs\compact_production_live_speed5_v1\collector_stream.bin --badge-stream-out extractor\native\runs\country_economy_production_stockpile_live_20260701_v2\gap_badges_stream.bin --out extractor\native\runs\country_economy_production_stockpile_live_20260701_v2\gap_badges.json
```

## Integration patch plan

When the orchestrator grants a shared integration slot, move this lane from local,
non-certifying slice proof to shared production by:

1. keeping the existing collector domains `10`, `11`, `37`, and `40`;
2. promoting the stricter lane gate checks into `recorder_host` compact coverage
   for this domain group;
3. requiring one data record per scheduled frame for the four lane domains;
4. preserving `.battleplan` roles `collector_compact_stream` and
   `recorder_manifest`;
5. adding manifest counters for `stockpile_rows`, `military_line_rows`,
   `naval_line_rows`, `construction_queue_rows`, `fuel_country_rows`,
   `stockpile_variant_def_rows`, and `stockpile_variant_upgrade_rows`.

The lane-local proof now badges every remaining gap. If an integration slot is
granted later, implement only the proposed lane-owned aggregates and keep
market/trade, characters/politics, state-detail, and unit-lane ownership intact.
