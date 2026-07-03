# Factory Totals Derive Spec

Lane: `country_economy_counters_live`  
Build target for live probes: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`  
Status: derive path specified; offmap root still unvalidated.

## Source Evidence

- `extractor/native/lanes/country_economy_counters_live/INTEGRATION_PACKET.md`
  validates the reusable country roots and rejects direct per-country factory
  scalar totals for this build.
- `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` says domain 23 already
  owns state/province building levels, and domain 48 owns the building-health
  sidecar pending shared integration.
- Vanilla building definitions:
  - `<local-path> of Iron IV\common\buildings\00_buildings.txt:55-68`
    defines `arms_factory` with `military_production = 1`.
  - `<local-path> of Iron IV\common\buildings\00_buildings.txt:70-82`
    defines `industrial_complex` with `general_production = 1`.
  - `<local-path> of Iron IV\common\buildings\00_buildings.txt:225-239`
    defines `dockyard` with `naval_production = 1`.
- Damage/repair and occupation are separate from raw level totals:
  - `<local-path> of Iron IV\common\defines\00_defines.lua:750-752`
    defines factory sabotage damage and base repair factors.
  - `<local-path> of Iron IV\common\occupation_laws\occupation_laws.txt:181-182`,
    `204-205`, `241-263`, `452-456`, `631-683`, and `921-922`
    show occupation-law `local_factories` and `local_resources` modifiers.
- Offmap factories are real scriptable state outside state building levels:
  - `<local-path> of Iron IV\common\on_actions\03_wtt_on_actions.txt:49`
    and nearby entries add offsite `arms_factory` levels.
  - `<local-path> of Iron IV\common\on_actions\00_on_actions.txt:6007`
    and `6037` add offsite `industrial_complex` levels for weapon sales.

## Raw Total Formula

For replay-side raw factory totals, derive the state-backed component from
state building levels and add a separately extracted country offmap component:

```text
civilian_state_total(TAG) =
  sum(state.buildings.industrial_complex.level for states attributed to TAG)

military_state_total(TAG) =
  sum(state.buildings.arms_factory.level for states attributed to TAG)

dockyard_state_total(TAG) =
  sum(state.buildings.dockyard.level for states attributed to TAG)

civilian_total(TAG) = civilian_state_total(TAG) + civilian_offmap(TAG)
military_total(TAG) = military_state_total(TAG) + military_offmap(TAG)
dockyard_total(TAG) = dockyard_state_total(TAG) + dockyard_offmap(TAG)
```

The state-backed component is derivable from domain 23 `states[*].buildings`
when those building names are available. If the integration consumes domain 48
numeric building ids instead, it must first use a validated static id-to-building
dictionary. Do not infer that `type_0x...` is an industry building by value
alone.

## Attribution

For the save-oracle factory total used by the 1943 Ireland proof, state-backed
factories attribute by `states.<id>.owner`. This matches the retained oracle
builder and the save fields used by the raw total fields:

```text
states.<id>.owner
states.<id>.controller
states.<id>.buildings.<building>.level
countries.<TAG>.buildings.<building>.level  # offmap
```

This spec therefore defines `*_state_total` as an owner-attributed raw building
level total when comparing to `economy_raw.json` `*_from_state_building_levels`.
The replay's domain-23 state data can also support a controller-attributed view,
but that is a different field. In the 1943 Ireland save, controller attribution
does not match the owner-attributed raw country factory oracle because occupied
states move controller without moving owner.

Occupied-state usable factories are also affected by controller/occupation
state, occupation laws, compliance/resistance, and engine rounding. Those inputs
live in occupation and building-health domains and must be joined separately
before emitting a "usable occupied factories" field.

## Damage And Construction

- Completed raw totals use `level`, not construction progress.
- Under-construction lines do not contribute to raw totals until the building
  level appears in the state/offmap building block. Domain 48 construction queue
  rows can explain progress, target state, active factories, cost, and produced
  progress, but they are not completed capacity.
- Damaged factories remain part of the raw `level` total. Usable/healthy
  capacity requires `healthy_level` from domain 48 building-health rows. If
  domain 48 is absent, the replay must not derive damaged/free/usable splits
  from raw domain 23 levels.

## Not Derivable From State Levels Alone

- `civilian_offmap`, `military_offmap`, and `dockyard_offmap`.
- Trade/export/war-reparation/subject/scripted offsite factory terms.
- Consumer-goods factories and factor.
- Free/used factory counts, because those depend on construction queue,
  production assignments, consumer goods, trade, repair, occupation, and
  possibly current UI policy state.
- Resource import/export/used/deficit flows.
- Fuel/energy/free bonuses and production-speed modifiers; these are modifier
  effects or separate domains, not factory-count levels.

## 1943 Ireland Oracle Verification

Generated inside this lane from
`C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4`:

- `wave2_ire_oracle/economy.json`
- `wave2_ire_oracle/economy_raw.json`
- `wave2_ire_oracle/production.json`
- `FACTORY_TOTALS_DERIVE_VERIFY_1943_IRE.json`

`build_country_economy_oracle.py` melted the save with rakaly and reported save
date `1943.11.9.17`, `439` countries, and `404` construction lines.

`verify_factory_totals_derive.py` then performed the independent check that
matters for replay derive work: it reparsed the raw save states through
`parse_save.py`, summed raw
`states.<id>.buildings.{industrial_complex,arms_factory,dockyard}.level`, and
compared those recomputed values to `economy_raw.json`
`*_from_state_building_levels`. This does not reuse `parse_economy.py`'s state
aggregation.

Independent owner-attributed state-level recomputation:

| Kind | Countries compared | Matches | Mismatches |
| --- | ---: | ---: | ---: |
| Civilian | 439 | 439 | 0 |
| Military | 439 | 439 | 0 |
| Dockyard | 439 | 439 | 0 |

Controller-attributed recomputation is reported separately because it is not the
oracle field's attribution mode:

| Kind | Countries compared | Matches | Mismatches |
| --- | ---: | ---: | ---: |
| Civilian | 439 | 399 | 40 |
| Military | 439 | 398 | 41 |
| Dockyard | 439 | 420 | 19 |

The save contains `222` states with explicit `controller != owner`, explaining
why controller attribution is an occupation/controller view rather than the raw
country factory total compared here.

The old state-plus-offmap row is retained only as an oracle accounting
self-consistency check. It verifies that the oracle's own `total` field equals
the oracle's own `from_states + offmap` fields; it is not the independent
state-level recomputation proof:

| Kind | Countries compared | State only equals total | Nonzero residual | State plus offmap equals total |
| --- | ---: | ---: | ---: | ---: |
| Civilian | 439 | 401 | 38 | 439 |
| Military | 439 | 421 | 18 | 439 |
| Dockyard | 439 | 436 | 3 | 439 |

Residual examples from the self-consistency check:

| Tag | Kind | State levels | Offmap | Total |
| --- | --- | ---: | ---: | ---: |
| IRE | Civilian | 9 | 1 | 10 |
| GER | Civilian | 95 | 4 | 99 |
| ENG | Civilian | 58 | 4 | 62 |
| AST | Civilian | 27 | 3 | 30 |
| AST | Military | 26 | 2 | 28 |
| EST | Civilian | 0 | 2 | 2 |
| EST | Military | 0 | 3 | 3 |
| NOR | Dockyard | 7 | 1 | 8 |

Result: independent raw state-building recomputation proves the
owner-attributed state-backed component exactly. State plus offmap matching
every raw country total remains only an oracle self-consistency check. The
remaining live root work is the country offmap component, not a direct
per-country total scalar.

## Probe Requirement

Run `extractor/memory_reader/economy/offmap_factory_probe_870ebf.py` in the next
live session against the same-tick oracle. A candidate is acceptable only if it:

- passes the `hoi4.exe` SHA gate;
- is not one of the prior rejected total-scalar paths;
- matches offmap values across nonzero and zero-control countries; and
- moves by the exact isolated offmap delta in a before/after fixture while
  unrelated countries remain stable.

Until that happens, the replay should expose state-backed factory totals with
explicit `derived_state_only` provenance and keep offmap totals null or badged.
