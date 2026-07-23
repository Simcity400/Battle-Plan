# country_economy_production_stockpile_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/country_economy_production_stockpile_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local production stack for country economy, production, deployment queues,
fuel, and stockpile-variant replay records.

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

The frozen D11 production-line base is extended by strict pointer-free joins:
`PVJ1` equipment variant id, `PFR1` requested factories, `PPR1` exact
engine-maintained priority/order from `line+0x44`, `PLS1` runtime subtype,
`PEF1` efficiency values, `PMI1` MIO identity, and `PCP1` completeness.
Every present line-key join is bijective; older records may omit a successor
segment and replay leaves then remain unobtained.

The final D11 segment may be `PDQ1`: a self-sized wrapper around the strict
deployment-queue v2 codec. Its country set is bijective with the admitted D11
country set. Complete empty queues are measured `[]`; unreadable queues carry an
explicit unknown row and never erase an earlier exact queue during folding.

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
- production line fields: exact primary vtable at `+0x00` (military
  `module+0x2965050`, naval `+0x2965238`, railway-gun `+0x2965398`, ship-refit
  `+0x2965858`), legacy type `+0x08`, id `+0x0C`, active factories `+0x18`,
  cost `+0x28`, speed `+0x30`, produced `+0x38`, exact priority/order `+0x44`,
  amount `+0x48`, exact
  equipment-variant pointer `+0x88`, industrial-manufacturer CID `+0x90`, and
  requested factories `+0xE8`; the CID is `{type:u32,id:u32}`, where `{0,0}` is
  proven null and `{79,id>0}` is an assigned industrial organisation
- construction queue vector `CProductionManager + 0x70/+0x78/+0x7C`, pointer
  stride `0x08`, type `+0x08 == 59`, template subobject at line `+0x70`, template
  id at subobject `+0x08`
- `CCountry + 0x1580 -> CCountryFuelStatus*`
- fuel status fields: current fuel `+0x08`, max fuel `+0x10`, gain base `+0x18`,
  fuel cost `+0x38`, all stored as signed `i64 / 32768`
- `CCountry + 0x0F70 -> CDeploymentManager*`; exact manager vtable
  `module+0x2949128`, owner backlink `+0x118`, and conveyor vector `+0x48`
- exact type-64 conveyor vtable `module+0x29A9E18`, type-52 template vtable
  `module+0x2938F70`, and type-63 line vtable `module+0x29A9DC0`
- military-deployment object primary/secondary vtables
  `module+0x29A9D38/+0x29A9D70`; serializer-owned equipment-variant vector
  `+0x40` with `0x10` stride and signed `i64 / 100000` amounts; training and
  max-training at `+0x60/+0x68`; manpower containers at `+0x70/+0x90`; maximum
  manpower at `+0xB0`

## Compact domains

- `10 fuel_live`: country fuel rows, fixed scale `32768`
- `11 production_live`: stockpiles, military/naval production lines, construction
  queue rows, fixed scale `100000`; additive `PVJ1`, `PFR1`, `PPR1`, `PLS1`,
  `PEF1`, `PMI1`, and `PCP1` self-sized segments carry exact variant,
  requested-factory, priority/order, subtype, efficiency, line-owned MIO, and
  vector-completeness state. Final `PDQ1` carries authenticated deployment queues
- `37 stockpile_variant_defs`: stockpile-sourced variant catalog rows and module
  token children
- `40 stockpile_variant_upgrades`: stockpile-sourced variant upgrade rows
- `611 economy_production_gap_badges`: lane-local proof-only badges for remaining
  economy/production gaps. This is not a shared collector domain.

## Completed deployment queues

Deployment queues are no longer a remaining-gap badge. The collector reads the
exact-a729 owner and serializer-backed fields into the strict v2 codec, appends a
final `PDQ1` segment, and poisons the complete row on any unstable read, invalid
vtable/type, duplicate global conveyor/line identity, or malformed token. The
recorder fold enforces D11 elapsed/status/count/tag joins and publishes
`countries.<TAG>.military.deployment_queue` plus the exact authority receipt.
The typed replay model and UI selector preserve queue order and nullable leaves
and reject invalid carriers as a whole. The historical save/rehost route remains
admitted, and the v2 decoder retains strict v1 compatibility for non-null legacy
carriers. Civil `construction_queue` is never substituted for deployment state.

## Remaining gap badges

The accepted production/fuel/stockpile streams do not encode every economy
surface. The lane-local `gap_report` proof fail-closes the remainder:

- `consumer_goods`: source-blocked. Scripted consumer-goods variables exist in
  coverage-audit paths, but no validated numeric country consumer-goods live root
  is in the hot-loop compact path.
- `resource_flows_shortages`: source-blocked. Save paths exist for
  `countries.<TAG>.resources.{produced,exported,to_export}`; state deposits are
  covered elsewhere and this lane must not duplicate market/trade ownership.
- `economy_law_derived_state`: owned-elsewhere badge. Law/idea identity belongs
  to characters/politics; this lane only owns numeric economy effects once rooted.
- `trade_factory_impact`: owned-elsewhere badge. Market/trade owns contract-level
  factory impact; this lane may later consume granted aggregate joins only.

X4-P5 is promoted: `PMI1` now carries one exact assigned or proven-null MIO id
for every admitted production-line key. Missing, unreadable, type-invalid,
duplicate, or non-bijective input suppresses the entire segment.

## Central validation

Run this lane only through the repository's centralized Rust validation runner;
subagents must not invoke Cargo, builds, or tests directly. The root production
pass selects the affected extractor packages, then runs the final all-workspace
validation battery.
