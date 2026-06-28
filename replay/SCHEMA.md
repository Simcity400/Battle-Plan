# Replay snapshot schema (canonical) — v1

> **This is the contract.** The native app, recorder/container path, diff/event
> engine, and validation tools all build against the shapes defined here. Field
> names and nesting are stable; new fields are added optional/null, never renamed in place. The Python
> mirror of this document is `replay/schema.py` (dataclasses + validators).
>
> **v1 revision log (additive only — no renames):** added the four SAVE-side deep
> domains — per-country `commanders` (§3.9) and `mios`/`scheduled_variants`/
> `mio_policies` (§3.10); deepened `intel` (§3.8); top-level `designs` (§11) and
> `order_of_battle` (§12). Each is schema-present and `null`/`[]`/`{}` until its
> parser feeds it.
> **Later additive batch (still `schema_version` `"1"`):** equipment-market / trade
> (`countries.*.diplomacy.lend_lease` carries the directed trade contracts; the
> canonical world list is `diplomacy_world.trade_contracts`; per-country
> `economy.market_stockpile`/`market_automation`) from `market.json`; per-state
> `occupation_law` (§4) from `occupation.json`; and two top-level PROXY/region LIVE
> sidecars promoted to first-class confidence-badged fields — `naval_air` and
> `supply` (§13). `schema_version` stays `"1"` because every change is additive.
> **Tier-1 EXTRACT batch (still `schema_version` `"1"`):** nine save domains wired —
> per-country `economy.logistics` (supply_system_2) from `supply.json`; per-country
> `extra` (nukes / dynamic_modifiers / aces / special-projects / misc) from
> `country_extra.json` (§3.11); per-country `human_name` (persona, MP-only) from
> `players.json` (§3.12); the additive per-division body (`dig_in`/`acclimatization`/
> `army_manpower`/`max_supply`/`supply_gain`/`killed`/… §6) MERGED from
> `divisions_full.json`; and five top-level world blocks — `strategic_air` (§14),
> `strategic_operatives` (§15), `raids` (§16), `world_events` (§17), `world` (§18).
> Every one is save-sourced (**S**), folded under `provenance.save`, schema-present
> and `null`/`[]`/`{}` until its parser feeds it. New diff events: `event_fired`,
> `convoy_sunk`, `nuke_count_changed`, `ace_gained`, `project_completed`.
> **Runtime-source revision:** in-process collector capture is the selected runtime
> source. Existing save-sourced schema rows remain valid fixtures and development
> evidence, but `S` is not an acceptable runtime fallback. Collector-fed snapshots
> should use additive provenance fields; `schema_version` can remain `"1"` while
> those fields are optional.

A **replay** of one game = an ordered series of **SNAPSHOTS** (full point-in-time
world state) plus a derived **EVENT timeline** (what changed between snapshots). This
file specifies the SNAPSHOT. The event records the diff engine emits are specified in
§9.

---

## 0. Conventions

### 0.1 Source tags (per DESIGN §4b)

Every leaf field carries an implied **source** so consumers know its reliability and
cadence. The source is documented per-field in the tables below using these tags:

| tag | meaning | cadence |
|-----|---------|---------|
| **C**  | in-process collector live source | collector layer cadence |
| **M**  | legacy external live memory, already cracked & validated | observed cadence; not runtime-accepted unless hourly-or-better |
| **M\*** | live memory/collector crackable but not yet pinned | future collector cadence |
| **S**  | save fixture/oracle parser | development/offline only |
| **D**  | derived (diff / join / compute) | layer of its inputs |

`C` is the target runtime source. `S` rows are useful schema evidence and test
fixtures, but a shipped live replay must not require save input or silently fill
collector gaps from saves. A row counts as `C` only when the snapshot provenance
claims the corresponding collector domain; preflight/cache success by itself is
not captured replay state.

A snapshot does **not** repeat the tag on every value at runtime — that would bloat
every file. Instead each snapshot records, **once**, which domains were populated and
from which channel, in `snapshot.provenance` (§2). Per-field source tags live in this
document and in `FIELD_SOURCES` in `schema.py` so a consumer can look up the canonical
source of any field by dotted path.

### 0.2 No fabrication (DESIGN §1, §4)

A field that is **schema-present but unobtained** is `null`, never guessed. A consumer
distinguishes "known to be zero" from "not measured" by: value `0`/`0.0` = measured
zero; value `null` = not captured this snapshot (channel absent or field unpinned).
Aggregates over null members skip them and report a coverage count, never impute.

### 0.3 Time

- **`date`**: Clausewitz date string `"YYYY.M.D.H"` (e.g. `"1940.9.10.13"`), exactly as
  the save/memory emit it. Not zero-padded (matches the oracle output).
- **`hour_index`**: integer hours-since-epoch (Clausewitz `((year*365 + day_of_year)*24
  + hour)`, 365-day years, no leap) — the **canonical sort/diff key**. Two snapshots are
  "adjacent" iff no snapshot exists with an `hour_index` strictly between them. Derived
  from `date` by `schema.date_to_hour_index`; `null` only if `date` is malformed.

Canonical live recordings are hourly-or-better. The recorder may preserve raw
sub-hour collector samples in a session sidecar, but daily-only rows are rollups,
not replay coverage.

### 0.4 Identity keys

- **country**: 3-letter TAG string (`"GER"`). The primary key for all country data.
- **state**: integer state id as string key in JSON objects (`"23"`), int in Python.
- **province**: integer province id (same string-key convention).
- **division**: integer `div_id` (stable per division; matches save `id.id` & memory).
- Tag-db-id vs TAG: memory emits a 1-based tag-db id for owner/controller/div-owner;
  the writer resolves it to TAG before it reaches a snapshot (DESIGN §2 caveat). A
  snapshot **never** stores raw numeric tag-db ids in a `*_tag` field.

### 0.5 Units & encodings (carried through from the oracle, unchanged)

- Fractions `stability`/`war_support`: float `0.0..1.0`.
- `political_power`, `command_power`: float (already `/100000`-decoded by the source).
- `manpower`/`available_manpower`/`manpower_ratio`: raw integer manpower (people).
- `fuel`/`max_fuel`: float (engine fixed-point already decoded).
- `organisation`/`strength`: float (memory decodes `/100000`; the save's `strength` is
  a percent-of-template float, kept as-is — see §6 note).
- Equipment amounts, resource produced/exported: float as the source emits.

---

## 1. Top-level snapshot object

```jsonc
{
  "schema_version": "1",          // string, this document's version
  "game_id": "ger_1941",          // string, stable per game/library entry
  "date": "1940.9.10.13",         // Clausewitz string  (§0.3)
  "hour_index": 17000... ,        // int | null         (§0.3, canonical key)
  "save_version": "Operation Postern v1.19.1.0.b49d (d4f6)",  // str | null
  "provenance": { ... },          // §2 — which domains came from which channel
  "countries":  { "GER": {Country}, ... },   // §3, keyed by TAG
  "states":     { "23":  {State},   ... },   // §4, keyed by state-id string
  "provinces":  { "11596": {Province}, ... },// §5, keyed by province-id string
  "divisions":  [ {Division}, ... ],         // §6, list (div_uid = "id:type" is identity)
  "combats":    [ {Combat}, ... ],           // §7, list (active at snapshot instant)
  "diplomacy_world": { "wars": {war_id:War}, "factions": {id:Faction}, "subjects": [] }, // §7b world-scoped diplomacy
  "map":        { MapLayers },               // §8, derived/raster-ready aggregates
  "designs":    { Designs },                 // §11, world design catalogs (templates + equipment-variant catalog)
  "order_of_battle": { OrderOfBattle },      // §12, world command structure (armies/fleets/wings/plans)
  "naval_air":  { NavalAir },                // §13, live PROXY/region naval+air sidecar (badged)
  "supply":     { Supply },                  // §13, live PROXY supply sidecar (badged)
  "strategic_air": { StrategicAir },         // §14, world air-base capacity table (save)
  "strategic_operatives": { StrategicOperatives }, // §15, operative pools + intel-network graph (save)
  "raids":      { Raids },                   // §16, strategic-bombing/naval-raid targets (save)
  "world_events": { WorldEvents },           // §17, fired events + threat + sunk-convoy ledgers (save)
  "world":      { World },                   // §18, scenario meta + global vars/flags/tech-share/balance (save)
  "events":     [ {Event}, ... ]             // §9, per-snapshot marker events (usually
                                             //     empty; the diff engine fills the
                                             //     cross-snapshot timeline separately)
}
```

Every top-level container is **always present** even if empty (`{}` / `[]`), so a
consumer never has to null-check the container itself — only leaf values.

---

## 2. `provenance` — per-domain channel record

Records, once per snapshot, what was actually captured and from where. This is how a
consumer reasons about coverage without per-value tags.

```jsonc
"provenance": {
  "collector": {                 // null for legacy/offline snapshots
    "captured": true,
    "backend": "in_process_collector",
    "collector_version": "0.1.0",
    "exe_sha256": "16022851...",
    "mod_checksum": "...",
    "domains": ["clock", "country_identity", "economy"],
    "capture_boundary": "scheduled_hour_snapshot",
    "quality_status": "complete"
  },
  "memory": {                    // legacy/external research path; optional
    "captured": true,
    "exe_sha256": "16022851...", // build gate (DESIGN §6); null if unknown
    "domains": ["country_scalars", "state_control", "divisions"],
    "save_date_validated_against": "1940.9.10.13"  // last save it was checked vs; null
  },
  "save": {                      // optional fixture/offline import; never runtime fallback
    "captured": true,
    "source_filename": "GER_1941_melted.txt",   // basename only, never a full path
    "save_date": "1940.9.10.13",
    "domains": ["economy","production","tech","focus","state_ownership",
                "province_control","divisions","country_snapshot"]
  },
  "notes": []                    // free-form strings (e.g. "collector omitted
                                 // state_control: layout sentinel failed")
}
```

`domains` values are the canonical domain ids in `schema.DOMAINS`. A domain absent from
all captured-domain lists means every field in that domain is `null` in
this snapshot — by design, not by error.

Collector session artifacts live beside the snapshot stream, not inside every
snapshot. The current recorder writes `collector_frames.jsonl` with every
validated DLL frame and records manifest counters such as
`raw_collector_frame_count`, duplicate same-hour frame count, payload row-count
summaries, transport diagnostics, and preflight bridge status. The canonical
snapshot stream still stores one selected frame per unique game hour.

### 2.1 `provenance.mod` — the per-replay MOD IDENTITY (additive, §K MOD AWARENESS)

Records the **active mod-set** the snapshot was captured under, read at **capture
time** from the host's the game **launcher config** (`dlc_load.json` + the `.mod`
descriptors it enables) — **not** from game memory and **not** from the melted save.
This is the identity the native replay UI auto-matches against so it loads the correct
mod skin and warns on a mismatch (vanilla-UI-over-a-mod-replay is structurally
prevented). Written capture-side by `live_capture/mod_identity.py`; consumed natively by
`replay_native::assets::modset` (`match_replay`).

```jsonc
"provenance": {
  "memory": { ... },
  "save":   { ... },
  "mod": {                            // null when NO launcher config was found
    "modset_id": "e23fc38a4e83b612",  // stable, order-SENSITIVE digest (sha1[:16] of
                                      // canonical-JSON [[ident,version],...], ident
                                      // NAMESPACED "workshop:<id>"/"name:<name>");
                                      // "vanilla" for a measured no-mod run
    "source": "dlc_load.json",
    "load_order": ["test.mod"],       // ordered enabled-mod descriptor BASENAMES
    "mods": [                         // in LOAD ORDER
      { "name": "AMSB - DEV",         // str|null
        "workshop_id": null,          // Steam Workshop id (remote_file_id) | null
        "version": "1.18",            // str|null
        "supported_version": "1.19.*",// str|null
        "descriptor": "test.mod" }    // .mod basename | null
    ]
  },
  "notes": []
}
```

- **NO FABRICATION.** `mod` is **`null`** whenever no launcher config was read (a
  vanilla/SP run, or capture without launcher-dir access) — never a guessed mod-set. A
  launcher config that enables **zero** mods records a *measured* vanilla mod-set
  (`modset_id="vanilla"`, `mods=[]`), which is **distinguishable** from `null` (not
  read): "known vanilla" vs "unknown".
- **NO PERSONAL PATHS.** A `.mod` descriptor's absolute `path="…"` is **dropped**; only
  the mod name, workshop id, version, and the descriptor **basename** are kept.
- **Identity is metadata-only** (id + version + load order). Two *local* mods with no
  workshop id, identical name, and identical version are indistinguishable by
  `modset_id`; the native matcher therefore also diffs the **full mod list** (add /
  remove / version / order) to explain a mismatch, and recomputes the digest from
  content rather than trusting a stored `modset_id`.

**Path redaction rule (HARD):** `source_filename` is a **basename only**. The writer
never stores an absolute path, a `~`-path, or anything containing a user directory. The
same rule applies to every `provenance.mod` string (descriptor basenames only).

---

## 3. `Country` object (keyed by TAG)

Grouped by domain to mirror DESIGN §4b. Every sub-object is present; unobtained leaves
are `null`.

### 3.1 `economy` — economy & industry
| field | type | source | notes |
|-------|------|--------|-------|
| `factories.civ` / `.mil` / `.dockyard` | int\|null | S / M\* | effective totals incl. offmap |
| `factories.civ_from_states` / `.mil_from_states` / `.dockyard_from_states` | int\|null | S | cross-check vs map |
| `factories.offmap.{civ,mil,dockyard}` | int\|null | S | |
| `factories.under_construction` | int\|null | S | count of build slots active |
| `construction_queue[]` | list | S | each: `template`, `location_state`, `active_factories`, `produced`, `cost`, `priority`, `created_date` |
| `resources.produced.{steel,oil,aluminium,tungsten,chromium,rubber,coal}` | float\|null | S | |
| `resources.exported.{...}` | float\|null | S | sparse (only exported resources) |
| `resources.to_export.{...}` | float\|null | S | |
| `resources.available_derived.{...}` | float\|null | S/D | produced − exported |
| `resources.imported.{...}` | float\|null | S | reserved; null until oracle emits |
| `fuel.current` / `fuel.max` | float\|null | C / M / S | scheduled collector `fuel_live` reads validated `CCountry+0x1580` fuel status; saves remain the backstop |
| `gross_ic_by_equipment` | obj\|null | S/D | `{equip_archetype: produced_delta}`; null until production-delta pass runs |
| `market_stockpile[]` | list | S | equipment this country placed ON the equipment market (sellable). Each: `equip_id`, `equip_type`, `archetype`, `amount`. From `market.json`. |
| `market_automation` | obj\|null | S | market-request automation flags (`auto_accept_purchase`, `auto_accept_market_access`, …); null when none set |
| `logistics` | obj\|null | S | supply_system_2 per-country logistics (from `supply.json`). `{priority, buffer, wanted_supply_trucks, capital, last_supply_capital_move, daily_losses_index, last_lost_train_province, truck_count, truck_damage_avg, trucks[{equip_id,equip_type,count,damage}], node_count, node_motorization[{node_province,levels{TAG:lvl}}], disrupted_count, disrupted_supply[{node_province,value}], foreign_homebase_nodes[{province,supply,penalty,source}], lost_railways(+_total), lost_trains(+_total), lost_trucks_attrition(+_total), lost_trucks_killed(+_total)}`. `lost_*` are ~30-day ring buffers; `*_total` is the sum. Only the keys the parser emitted are present; a genuine `0`/`0.0` is preserved. |

### 3.2 `military`
| field | type | source | notes |
|-------|------|--------|-------|
| `num_ships` | int\|null | M / S | |
| `division_count` | int\|null | M/D | count of this tag's entries in `divisions[]` |
| `manpower.available` | int\|null | S / M\* | deployable reserve |
| `manpower.deployed` | int\|null | S/D | null until computed |
| `manpower.army` / `.navy` / `.air` | int\|null | S | split; null until oracle emits |
| `xp.army` / `.navy` / `.air` | float\|null | S | |
| `equipment_stockpile[]` | list | S | each: `equip_id`, `equip_type`, `archetype`, `amount` |
| `convoys[]` | list | S | same shape as stockpile entries (convoy archetype) |
| `ships_by_hull` | obj\|null | S/D | `{hull_type: {count, tonnage}}`; null until parsed |
| `casualties.killed` / `.captured` | int\|null | S/D | feeds K/D |
| `command_power` | float\|null | C / M | duplicated under `politics` for convenience |

### 3.3 `politics`
| field | type | source | notes |
|-------|------|--------|-------|
| `stability` | float\|null | C / M | 0..1; `C` through the country-object preflight bridge |
| `war_support` | float\|null | C / M | 0..1; `C` through the country-object preflight bridge |
| `political_power` | float\|null | C / M | `C` through the country-record preflight bridge |
| `command_power` | float\|null | C / M | `C` through the country-object preflight bridge |
| `ruling_party` | str\|null | S | e.g. `"fascism"` |
| `party_popularities` | obj\|null | S | `{ideology: fraction}`; null until parsed |
| `laws.economy` / `.trade` / `.conscription` | str\|null | S | token or null |
| `ideas_all[]` | list[str] | S | every active idea token |
| `ideas_other[]` | list[str] | S | spirits/advisors/traits (laws stripped) |
| `cabinet` | obj\|null | S | `{leader, advisors[], theorists[]}`; null until parsed |
| `is_human` | bool\|null | S/M | `is_ai = no` at start (DESIGN §4 — humans flagged) |
| `is_major` | bool\|null | S | |
| `capital_state` | int\|null | S | state id |
| `autonomy.level` / `.overlord` | str\|null | S | subject status; null until parsed |
| `faction.name` / `.leader` / `.is_leader` | str\|str\|bool\|null | S | faction membership |

### 3.4 `research`
| field | type | source | notes |
|-------|------|--------|-------|
| `tech_count` | int\|null | S | |
| `technologies[]` | list | S/D | each: `name`, `level`, `date` (completion, Clausewitz), `research_points` |
| `in_progress[]` | list | S/D / M\* | each: `name`, `points_factor`; derived fields may include `progress_points`, `required_points_base`, `remaining_points_base`, `progress_pct`, `eta_days`, `progress_basis`, or an unavailable reason when the active progress row / static cost / speed input is absent |
| `in_progress_live[]` | list\|null | M/D | live raw slots: `slot_index`, raw tech handles, `progress_raw`, optional `tech_name` / `progress_units`; derived `progress_pct` only when live progress + static cost are safe; live ETA requires live `points_factor` and otherwise gets `eta_unavailable_reason` |
| `doctrine` | obj\|null | S/D | optional save-backed doctrine manager fold from `doctrine.json`; TAG rows expose `{raw, completed[], completed_count, in_progress[], source}` under the country research surface. Numeric/non-TAG save keys stay in top-level `doctrine_save` without an inferred country bridge. No live doctrine-manager root is claimed. |
| `research_slots` | int\|null | S | |
| `original_research_slots` | int\|null | S | |
| `research_bonuses[]` | list | S | each: `name`, `categories[]`, `bonus`, `kind` |

### 3.5 `focus`
| field | type | source | notes |
|-------|------|--------|-------|
| `focus_tree` | str\|null | S | id of the active tree |
| `continuous_focus_palette` | str\|null | S | |
| `completed[]` | list[str] | S/D | completed focus ids (diff → completion timeline) |
| `completed_count` | int\|null | S | |
| `current` | str\|null | S / M\* | focus in progress |
| `progress` | number\|null | S / M\* | progress on `current` |
| `paused` | bool\|null | S | |

### 3.6 `production`
| field | type | source | notes |
|-------|------|--------|-------|
| `stockpile` | obj\|null | C / M | equipment-variant id string -> signed amount; scheduled collector `production_live` reads validated `CCountry+0xF68` production manager |
| `stockpile_total` | float\|null | C / M / D | sum of measured `stockpile`; absent when stockpile vector was unavailable |
| `military_lines[]` / `naval_lines[]` | list\|null | C / M | each: `id`, `active_factories`, `cost`, `speed`, `produced`, `amount`; measured-empty `[]` is distinct from null |
| `military_factories` / `naval_factories` | int\|null | C / M | sum of active factories over measured lines |
| `construction_queue[]` | list\|null | M | live construction-order slice; not emitted by collector production manager |

### 3.7 `decisions`
| field | type | source | notes |
|-------|------|--------|-------|
| `active[]` | list | S | active decisions/categories; null until parsed → `[]` ok |
| `timed[]` | list | S | timed decisions + cooldowns |
| `missions[]` | list | S | ongoing missions |

> One-shot decision **clicks** are not stored here (DESIGN §4 — only recoverable when
> they persist state or fire a log marker; surfaced as derived events §9, not country
> state).

### 3.8 `diplomacy`
| field | type | source | notes |
|-------|------|--------|-------|
| `wars[]` | list | S | each: `against[]`, `war_goal`, `war_score`; null→`[]` until parsed |
| `guarantees[]` / `non_aggression[]` / `embargoes[]` | list | S | |
| `opinion` | obj\|null | S | `{other_tag: value}`; null until parsed (large) |
| `lend_lease[]` | list | S | the game 1.19 **equipment market** = modern lend-lease. Each entry is a directed trade contract attached to **both** parties: `contract_id` (`"id:type"`), `direction` (`"outgoing"` for the seller, `"incoming"` for the buyer), `counterpart`, `seller`, `buyer`, `equipments[]`, `prices[]`, `convoys{assigned,total}`, `delivery_country`, `delivery_efficiency`, `days`, `delivery_state`, `completed`. Canonical world list: `diplomacy_world.trade_contracts`. |
| `volunteers[]` / `attaches[]` / `expeditionary[]` | list | S | reserved; null→`[]` |

### 3.9 `intel` (deepened from `intel.json`)
| field | type | source | notes |
|-------|------|--------|-------|
| `agency` | obj\|null | S | `{name, icon, is_created, upgrade_progress, current_upgrade, upgrades{branch:level}, capacity{max_operatives,usable_slots,defense}}` |
| `operatives[]` | list | S | each: `op_id`, `name`, `gfx`, `skill`, `experience`, `traits[]`, `nationalities[]`, `state` (`on_operation`/`on_mission`/…), `operation_id` (→ `operations.running`), `mission` (in-field task obj), `captured`, `captured_by` |
| `recruitable_operatives[]` | list | S | recruitable pool, same operative shape |
| `intel_networks[]` | list | S | reserved; null→`[]` (graph summarized in diplomacy) |
| `operations` | obj\|null | S | `{priority, running[], finished[]}`. A running op: `op_id`, `type`, `date`, `duration` (days), `target`, `phases[]`, `operatives[]` (`{operative_id, mission}`). Progress derives from `save_date` vs `date`+`duration`. |
| `decryption` | obj\|null | S | cryptology: `{targets[{target, decryption(bool), amount}], intel_source}`. `amount` = accumulated decryption points; `decryption=true` = actively decrypting that target. |
| `peace_conference` | obj\|null | S | active peace-conference state (`{active}`; empty/inactive in most saves) |

> `gfx` on an operative is a portrait sprite **name**, not image data (rendering is a
> separate asset-extraction step). `operative_id`s inside `operations.running[].operatives`
> join back to this country's `operatives[].op_id`.

### 3.10 `commanders` (from `characters.json`)
The the game 1.19 unified CHARACTER system, resolved to names + assignments by the parser.
| field | type | source | notes |
|-------|------|--------|-------|
| `leader` | obj\|null | S | active country leader (ruling party): `{name, token, ideology, traits[], expire, portrait_gfx, char_id}` |
| `party_leaders[]` | list | S | all ideology-group leaders + `party`, `popularity` (the political picture) |
| `ruling_party` | str\|null | S | ideology group currently in power |
| `generals[]` / `field_marshals[]` / `admirals[]` | list | S | military leaders. Each: `name`, `token`, `role`, `level` (`skill`), `experience`, `attack`/`defense`/`planning`/`logistics` (army) or `maneuvering`/`coordination` (navy), `traits[]`, `preferred_tactic`, `penalty`, `captured`, `unit_leader_id`, `assignment` (`{kind: army_group/army/fleet, name}` or `null` = reserve), `portrait_gfx` |
| `advisors[]` | list | S | cabinet / high command / theorists / chiefs: `{name, token, slot, ledger, idea_token, political_power, traits[], portrait_gfx}` |
| `counts` | obj\|null | S/D | `{generals, field_marshals, admirals, advisors, roster_size}` |

> A military leader's `assignment` is the join from its `unit_leader_id` (type 4713) to
> the formation that commands it (theatre `field_marshal_group`→army group, `orders_group`→
> army, navy `fleet`→fleet). `assignment=null` = unassigned/reserve. `portrait_gfx` is a
> sprite name (asset-extraction step, not image data).

### 3.11 `mios` / `scheduled_variants` / `mio_policies` (from `designs.json`)
Per-country military-industrial picture (the world variant CATALOG is top-level §11).
| field | type | source | notes |
|-------|------|--------|-------|
| `mios[]` | list | S | military-industrial organisations. Each: `key`, `id`, `type`, `name`, `icon`, `level` (`size`=org level), `funds` (development progress), `points`, `research_bonus`, `task_capacity`, `unlocked_traits[]` (taken), `allowed_policies[]` (eligibility only), `assigned_equipment` (`[id,type]`), `assigned_equipment_archetype` |
| `scheduled_variants[]` | list | S | named designer PRESETS: `{archetype, family, name, icon, obsolete, modules, upgrades, industrial_organisation([id,type]\|null), industrial_organisation_mio}` |
| `mio_policies` | obj\|null | S | `{policy_name: {cost, cooldown}}` — MIO-policy cost/cooldown state |

### 3.12 `extra` (from `country_extra.json`)
Country-level extras not covered by the other domains. `null`-but-present until ingested;
when present it carries only the keys the parser emitted (no fabrication; a genuine `0`
stays `0`, e.g. a nuke RECORD with `amount=0` yields `nukes_total=0`).
| field | type | source | notes |
|-------|------|--------|-------|
| `nukes_total` / `nukes_ready_total` | int\|null | S | summed across the country's nuke records |
| `nukes[]` | list | S | each: `{amount, nukes_ready}` (a real all-zero list ⇒ `nukes_total=0`) |
| `dynamic_modifiers[]` | list | S | each: `{modifier, enabled, value[]}` (the value array is the modifier's stored coefficients) |
| `ace_count` | int\|null | S | number of `aces[]` |
| `aces[]` | list | S | each: `{name, surname, callsign, modifier, alive, kill_type, killer_country}` (post-death fields null while the ace is alive) |
| `reinforcement_priority` | int\|null | S | |
| `deployment_blocks[]` | list | S | active deployment toggles |
| `special_projects` | obj\|null | S | `{projects_started, projects_completed, active_programs[{building,province,country}], breakthrough{specialization_*:points}}` |
| `experience_status` | obj\|null | S | `{army_experience, navy_experience, air_experience}` |
| `script_flags` | obj | S | per-country script flags from `countries.<TAG>.flags`, keyed by flag name with `{value, date}`; omitted when the save has no flags block for that country |
| `strategic_navy` | obj | S | save-persisted `countries.<TAG>.strategic_navy` block, preserved under `extra` with nested maps/repeated blocks intact; save-backed only, no live memory offset claim |
| `convoys_destroyed` | int\|null | S | |
| `accidents_score` | float\|null | S | |
| `cosmetic_tag` | str\|null | S | active cosmetic tag (`""` when none) |
| `pride_of_the_fleet` | obj\|null | S | `{id, type}` of the flagship ship |
| `collaboration_present` | bool\|null | S | whether a collaboration government exists |

### 3.13 `human_name` (from `players.json`)
| field | type | source | notes |
|-------|------|--------|-------|
| `human_name` | str\|null | S | the human player's lobby/persona DISPLAY name (`player_countries.<TAG>.user`). Populates in MP; `null` for AI and SP. **Not** a Steam id / OS account — those remain a hard wall (§10). |

---

## 4. `State` object (keyed by state-id string, ×~1081)

> VP and manpower are **split** to match the oracle's actual shapes (a scalar total
> vs a per-province map; a manpower *pool* object vs a scalar). Occupation is an
> object, not a lone scalar.

| field | type | source | notes |
|-------|------|--------|-------|
| `owner` | str\|null | C / M / S | TAG; `null` = genuinely ownerless. `C` only when a collector `state_ownership` frame/domain supplied it. |
| `controller` | str\|null | C / M / S | TAG. `C` only when a collector `state_control` frame/domain supplied it. |
| `is_core_of[]` | list[str] | S | cores; null until parsed → `[]` ok |
| `is_claimed_by[]` | list[str] | S | |
| `buildings` | obj\|null | S / M\* | state-scoped levels (`infrastructure, industrial_complex, arms_factory, dockyard, air_base, land_fort, ...`) |
| `province_buildings` | obj\|null | S | province-scoped buildings folded to the state (`naval_base, coastal_fort, ...`) |
| `victory_points_total` | int\|null | S | sum of the state's province VP |
| `victory_points_by_province` | obj\|null | S | `{province_id: vp}` (static) |
| `resources` | obj\|null | S | `{resource: amount}` (raw deposits) |
| `manpower_pool` | obj\|null | S | `{available, locked, total}` |
| `category` | str\|null | M/S | state category id (`town`, `large_city`, …) |
| `occupation` | obj\|null | S | `{resistance, resistance_speed, compliance, compliance_speed, occupied_country_tag, operational_status}` |
| `occupation_law` | str\|null | S | |
| `demilitarized` | bool\|null | S | |
| `supply` | float\|null | S / M\* | |

---

## 5. `Province` object (keyed by province-id string, ×~10k–13k land)

| field | type | source | notes |
|-------|------|--------|-------|
| `controller` | str\|null | C / S/D / M\* | TAG. `C` only when a collector `province_control` frame/domain supplied it; the live dense-array root is still not a closed normal-recording path. |
| `state_id` | int\|null | D | join from province→state map |
| `supply` | float\|null | S | |
| `rail_usage` / `route_usage` | float\|null | S | |
| `naval_base_usage` / `port_usage` | float\|null | S | |

> The oracle emits `province_control.json` as a flat `{province_id: TAG}` map. The
> writer normalizes each into `{"controller": TAG}` and adds `state_id` if the
> province→state map is available, else `null`. Provinces absent from the oracle map
> (sea/lake) are simply omitted — only land provinces with a controller appear.
>
> Collector frames may also populate this table directly. In that path the frame
> must explicitly claim `province_control`, emit normalized TAG controllers (no
> raw tag-db ids), and the adapter mirrors the result into
> `map.control_by_province`.

---

## 6. `Division` object (list element; **identity = `div_uid` = `"div_id:div_type"`**, ×~1600)

> The Clausewitz object id is a `(id, type)` pair. The diff engine keys divisions by
> `schema.div_uid(d)` (`"div_id:div_type"`, falling back to bare `div_id` when
> `div_type` is absent) so an id **reused after destruction** is a destroy+create, not
> a spurious move+retemplate. Both snapshots being diffed should carry `div_type` for
> this to bite; mixing a typed and an untyped channel falls back to bare-id matching.
>
> **Collector contract (`divisions`):** the first in-process collector slice for this
> domain is a scheduled hourly-or-better snapshot of active division identity/status
> rows. It is not a 10 ms worker-poll payload and it does not include division equipment,
> command hierarchy, template bodies, or battle-plan orders. The collector must write
> only measured leaves, require `div_id` + `div_type`, reject raw pointers/tag-db ids,
> and append `divisions` to `provenance.collector.domains` only after the fold succeeds.
> `division_created` / `division_destroyed` / `division_moved` /
> `division_template_changed` remain snapshot-diff events at this cadence; sub-hour
> movement/order deltas are a later sibling raw delta/event stream, not extra
> same-hour snapshots.

| field | type | source | notes |
|-------|------|--------|-------|
| `div_id` | int | C / M / S | bare id (matches save `id.id` & memory); required for collector `divisions` |
| `div_type` | int\|null | C / M / S | id `type` half; pair with `div_id` for identity; required for collector `divisions` |
| `owner` | str\|null | C / M / S | TAG; raw tag-db ids rejected |
| `location_province` | int\|null | C / M / S | current province |
| `state_id` | int\|null | C / M / S / D | province→state join |
| `template_id` | int\|null | C / M / S | |
| `template_type` | int\|null | C / M / S | template id `type` half |
| `template_name` | str\|null | C / M / S | resolved name |
| `template_role` | str\|null | S | `infantry`/`armor`/`motorized`/… |
| `organisation` | float\|null | C / M | exact live value (memory `/100000`) |
| `strength` | float\|null | C / M / S | see note |
| `equipment[]` | list | M / S | each: `archetype`, `equip_id`, `equip_type`, `amount` |
| `experience` | float\|null | C / M\* | |
| `moving` | bool\|null | C / M\* | |
| `move_target_province` | int\|null | C / M\* | |
| `move_progress` | float\|null | C / M\* | engine progress metric, not a normalized percentage unless a collector profile proves that |
| `in_combat` | bool\|null | C / M\* | live combat flag; cross-checked vs §7 |
| `encircled` | bool\|null | M\* | |
| `supply_status` | float\|null | C / M\* | |
| `script_id` | int\|null | S | save script id (`-1` = none) — **divisions_full** |
| `division_name` | obj\|null | S | `{type, name_order}` (name-list ref, not the resolved string) — **divisions_full** |
| `max_supply` | float\|null | S | division supply capacity — **divisions_full** |
| `army_current_supply_ratio` | float\|null | S | current supply ratio — **divisions_full** |
| `supply_gain` | float\|null | S | supply gain rate (a genuine `0.0` exists and is kept) — **divisions_full** |
| `motorization_level` | int\|null | S | — **divisions_full** |
| `move_priority` | str\|null | S | e.g. `front_order` — **divisions_full** |
| `last_combat_date` | str\|null | S | Clausewitz date of last combat — **divisions_full** |
| `dig_in` / `dig_in_cap` | float\|null | S | entrenchment + cap — **divisions_full** |
| `acclimatization` | obj\|null | S | `{hot_climate, cold_climate, max_acclimatization}` — **divisions_full** |
| `army_manpower` | obj\|null | S | `{value, need}` — **divisions_full** |
| `killed` | int\|null | S | manpower killed — **divisions_full** |
| `killer_definition` | str\|null | S | template that destroyed it (`no` if alive) — **divisions_full** |
| `equipment_archetypes` | int\|null | S | count of distinct equipment archetypes — **divisions_full** |
| `has_officer` | bool\|null | S | — **divisions_full** |

> The **divisions_full** rows are LITERAL save fields (not derived), MERGED onto each
> division by its full `(div_id, div_type)` identity. The 9 fields above them
> (`div_id`…`equipment`) come from `divisions.json` and are authoritative; the merge
> never overwrites them. A division present only in `divisions_full.json` is appended
> (carrying its identity + the additive body); the per-country `division_count`
> recounts to include it.

> **`strength` note:** the memory reader emits a manpower-scaled strength figure; the
> save's per-division `strength` is a fraction-of-template. The writer preserves
> whatever the source provided and records the channel in `provenance`. The diff engine
> treats `strength` as **monotone-comparable within one channel only** (don't diff a
> memory strength against a save strength). Same caution for `organisation`.

---

## 7. `Combat` object (list element; active at snapshot instant — Tier C)

Active combats are a **snapshot of currently-running engagements**, not history. The
diff engine reconstructs each engagement's lifecycle (start/end/casualties) by tracking
a combat's `combat_id` across the appear→persist→disappear sequence (§9, `combat_*`).

| field | type | source | notes |
|-------|------|--------|-------|
| `combat_id` | int\|null | M\* | stable-ish id used to track lifecycle; null until cracked |
| `kind` | str | M\* | `"land"` \| `"naval"` \| `"air"` |
| `location_province` | int\|null | M\* | land/naval anchor province |
| `attackers[]` | list | M\* | TAGs and/or div_ids/ship-ids engaged |
| `defenders[]` | list | M\* | |
| `combat_width` | float\|null | M\* | land |
| `terrain` | str\|null | M\* | land |
| `progress` | float\|null | M\* | combat bar `-1..1` |
| `modifiers` | obj\|null | M\* | |

> The entire `combats[]` domain is **M\*** (not yet cracked). Until then snapshots carry
> `"combats": []` and `provenance` omits the `combats` domain. Schema-present, never
> faked.

---

## 7b. `diplomacy_world` (world-scoped diplomacy)

The per-country view of diplomacy lives under `countries.*.diplomacy`. This top-level
block holds the **shared, world-scoped** objects so a consumer (and a future war-id
event lifecycle) has a single source of truth. Always present (`{}`-filled).

| field | type | source | notes |
|-------|------|--------|-------|
| `wars` | obj | S | `{war_id: {a, b, start_date, a_casualties, b_casualties, war_score_a_vs_b, war_score_b_vs_a}}`; `war_id` = `"A-B@start_date"` |
| `factions` | obj | S | `{faction_id: {id, ideology, members[], leader, ...}}` |
| `subjects` | list | S | `[{overlord, subject, autonomy_state, value, start_date}]` |
| `trade_contracts` | list | S | canonical equipment-market (lend-lease) contracts, world-scoped (single source of truth; the per-country view is `countries.*.diplomacy.lend_lease`). Each: `contract_id`, `seller`, `buyer`, `speed`, `equipments[]`, `prices[]`, `delivery_country`, `convoys`, `delivery_efficiency`, `days`, `delivery_state`, `completed`. Absent (key omitted) when no contracts exist. From `market.json`. |

## 8. `map` layers (derived aggregates; raster-renderer convenience)

| field | type | source | notes |
|-------|------|--------|-------|
| `owner_by_state` | obj\|null | C / M / S / D | `{state_id: TAG}` (mirror of `states[*].owner` for the raster) |
| `control_by_province` | obj\|null | C / M / S / D | `{province_id: TAG}` (mirror of `provinces[*].controller`) |
| `vp_by_state` | obj\|null | S(static)/D | computed VP per state |
| `controlled_industry_share` | obj\|null | D | `{TAG: fraction}`; null until computed |
| `front_lines[]` | list | D | derived from control adjacency; null→`[]` until computed |
| `strategic_regions` | obj\|null | S(static) | static; usually carried once, not per-snapshot |

> `map` is **redundant with** `states`/`provinces` — it exists so the raster renderer
> can recolor without walking every state object. The writer fills `owner_by_state` and
> `control_by_province` directly from the state/province domains; the rest stay null
> until their compute pass exists.

---

## 11. `designs` (world design catalogs; from `designs.json`)

World-scoped because the equipment-variant catalog is the join target for divisions,
production lines, ships and air wings across **all** tags. Per-country MIOs / presets /
policies live on each `Country` (§3.10). Always present (`[]`/`{}`-filled).

| field | type | source | notes |
|-------|------|--------|-------|
| `division_templates[]` | list | S | each: `id`, `type`, `name`, `country` (owning TAG), `role`, `regiments[{type,x,y}]` (the grid; key repeats → ordered list), `support[]`, `regimental_support[]`, `priority`, `division_cap`, `obsolete`, `is_locked`, `is_army_hq`, `n_regiments`, `n_support` |
| `equipment_variants` | obj | S | keyed `"id:type"`. Each: `id`, `type`, `archetype`, `family` (`land_vehicle`/`ship`/`plane`/`land_other`), `name` (custom or null), `version`, `parent_id`, `creator`, `obsolete`, `modules{slot:mod}`, `upgrades{name:lvl}`, `design_team([id,type])`, `design_team_mio` (resolved `{tag,key,name,size}` or null) |

> The save does **not** store a template's derived combat stats (soft/hard attack, org,
> HP, combat width) — those are runtime-computed; only the grid layout is here (DESIGN
> hard limit, not a gap). A division's / ship's / wing's equipment `(id,type)` refs join
> the `equipment_variants` catalog by the `"id:type"` key.

## 12. `order_of_battle` (world command structure; from `oob.json`)

Command structure is world-scoped (a tag's army can hold another tag's expeditionary /
lend-lease divisions). Canonical lists live here; each element carries its `owner` TAG.
The per-country count rollup is `by_country` (also mirrored to `Country.order_of_battle`).
Always present (`[]`/`{}`-filled).

| field | type | source | notes |
|-------|------|--------|-------|
| `army_groups[]` | list | S | each: `id` (`id:type`), `owner`, `name`, `leader` (FM `id:type` → §3.9), `army_refs[]` (member-army id refs), `army_refs_resolved`, `cached{plan_value,our_power,enemy_power}` (RUNTIME, non-authoritative), `icon` |
| `armies[]` | list | S | each: `id`, `owner`, `name`, `leader` (general id), `members[]` (division `id:type` refs), `member_count`, `members_resolved` (joined to a real division), `orders[]` (battle-plan `order_instance`: `type`, `instance_id`, dates, `operation`, `path[]`, `states[]`, `scheduled_members[]`, `root_front`), `field_marshal_group`, `cached{…}` |
| `fleets[]` | list | S | each: `id`, `owner`, `name`, `leader` (admiral id), `task_forces[{id,name,location_province,owner,mission,ships[{id,definition,organisation,strength,experience,name,equipment[[id,type,amount]]}]}]` |
| `navy_theaters[]` | list | S | `{owner, id, name, fleet_refs[]}` (fleet `id:type` refs) |
| `air_wings[]` | list | S | each: `id`, `owner`, `name`, `count` (planes), `experience`, `manpower`, `mission`/`mission_type` (cached), `strategic_region`, `plane_definition`, `air_base` (`id:type`), `state_id` (resolved via air_base→state; null for carrier/mobile), `equipment[[id,type,amount]]`, `effective_planes` (RUNTIME) |
| `theatres[]` | list | S | `{owner, id, area_states[], n_armies, n_army_groups}` |
| `by_country` | obj | S/D | `{TAG: {armies, army_groups, divisions_in_armies, fleets, task_forces, ships, air_wings, planes}}` |

> `cached.*` (plan/power values) and air mission `effective_planes` are CACHED runtime
> snapshots — surfaced verbatim, **not** authoritative (live combat targeting / fronts /
> supremacy / pathfinding are memory-only, deferred). `members[]` resolve to `divisions[]`
> by the `id:type` key; ship/wing equipment refs join `designs.equipment_variants` (§11).

## 13. `naval_air` + `supply` (live PROXY/region sidecars, first-class)

Promoted from untyped assembler sidecars into first-class top-level fields so a
consumer has a stable home for them — but they are **PROXY / region-level** LIVE reads,
**not** per-province / per-ship ground truth. Each carries its own `source`/`confidence`
badge that travels WITH the data; a UI must render them as estimates, never authoritative.
Always present; `null`-but-present until a live read folds them.

### 13.1 `naval_air`
| field | type | source | notes |
|-------|------|--------|-------|
| `region_control` | obj\|null | M | `{strategic_region_id: [ {controller(TAG\|null), …} ]}` naval sea-zone dominance (region granularity) |
| `air_superiority` | obj\|null | M | `{strategic_region_id: {controller(TAG\|null), …}}` air-dominance per region |
| `source` | str\|null | — | `"M"` when a live read filled it, else `null` |
| `confidence` | str\|null | — | a PROXY label (e.g. `"region_proxy"`); **never** `"exact"` |

### 13.2 `supply`
| field | type | source | notes |
|-------|------|--------|-------|
| `by_province` | obj\|null | M | `{province_id: {current, capacity, gain, divisions}}` — the validated division-province supply PROXY (where troops are). The exact tooltip supply NUMBER is a documented wall; see `memory_reader/supply_solver_offsets.draft.json` and `RESEARCH_logistics_intel.md`. |
| `source` | str\|null | — | `"M"` when a live read filled it, else `null` |
| `confidence` | str\|null | — | e.g. `"division_province_proxy"` |

> Both are M-channel LIVE; the assembler records them under `provenance.memory.domains`
> (`naval_air` / `supply`) AND keeps the raw plug payload in a `_live_naval_air` /
> `_live_supply` sidecar for back-compat. No diff-engine events are derived from them
> (region-proxy data is not eventized — it would be noisy and non-authoritative).

---

## 14. `strategic_air` (world air-base table; from `strategic_air.json`)

World strategic-air picture. Save-sourced (**S**). Always present (`[]`/`null`-filled).

| field | type | source | notes |
|-------|------|--------|-------|
| `air_bases[]` | list | S | each: `{id, type, state, capacity, base, level, countries[{country, capacity_penalty}]}` — per-state air-base capacity + the per-country capacity penalty list |
| `indices` | obj\|null | S | `{air_theatre_index, air_group_index}` (engine running indices) |
| `per_country_region_blocks` | obj\|null | S | `{TAG: count}` of strategic-air region sub-blocks (coverage indicator) |

## 15. `strategic_operatives` (operative pools + intel-network graph; from `strategic_operatives.json`)

| field | type | source | notes |
|-------|------|--------|-------|
| `pools` | obj\|null | S | `{operative_index: {subversive_activity_level, danger_level, networks[{target, vertices}]}}`. Index is the save operative slot (string key). `networks` may be empty (the operative runs no intel network) — a genuine empty list, not null. |

## 16. `raids` (strategic-bombing / naval-raid targets; from `raids.json`)

| field | type | source | notes |
|-------|------|--------|-------|
| `counters` | obj\|null | S | `{next_state, existing_target_num, detectable_target_num, next_target}` |
| `targets[]` | list | S | each: `{existing_target_num, detectable_target_num, valid, dynamic, target_province, target_building, target_location}` |
| `countries[]` | list | S | each: `{tag, priority}` (per-country raid priority) |

## 17. `world_events` (event + convoy ledgers; from `events.json`)

Persisted ledgers. Drives the `event_fired` / `convoy_sunk` diff events (§9.2).

| field | type | source | notes |
|-------|------|--------|-------|
| `fired_events[]` | list[str] | S | persisted scripted-event ids that fired this game |
| `pending_events[]` | list | S | scheduled-but-unfired events (empty in most saves; future-proofed) |
| `threat[]` | list | S | world-tension CONTRIBUTOR ledger (per threat-action). Each: `{threat, final_threat, daily, tag, date, label}`. **NOT** the aggregate tension scalar (that is not stored — see `world.world_tension_inputs`). |
| `sunk_convoys[]` | list | S | per-month convoy-loss ledger. Each: `{month (hours-since-epoch bucket), convoys, killer_country, owner}` |
| `convoy_losses_by_killer` / `convoy_losses_by_owner` | obj\|null | S | `{TAG: n}` rolled-up tallies |

## 18. `world` (scenario meta + globals; from `world.json`)

| field | type | source | notes |
|-------|------|--------|-------|
| `scenario` | obj\|null | S | `{start_date, difficulty, historical, tutorial, dlcs, mods[], session, speed, save_version, ideology, gameplaysettings{}, game_rules_empty, difficulty_settings_empty}`. `difficulty_settings_empty` records only the explicit save block's empty/non-empty/absent state; the selected difficulty remains `difficulty`. |
| `variables` | obj\|null | S | `{name: value}` global script variables (array members keyed `name^i` / `name^num`) |
| `flags` | obj\|null | S | `{name: {value, date}}` global flags |
| `tech_sharing_groups[]` | list | S | each: `{id, bonuses[], countries[]}` |
| `power_balances[]` | list | S | each: `{template, value, left_side, right_side, trending_side, countries[]}` |
| `faction_influence[]` | list | S | per-faction internals (the parser's `faction_system`): each `{key, index, initiative, contribution, war_score_breakdown, member_upgrades[]}` |
| `world_tension_inputs` | obj\|null | S | `{average_major_ic, tension_scaling_base_country}` — the inputs to world-tension scaling (the aggregate tension VALUE is not stored) |
| `saved_event_targets[]` | list | S | each: `{country, state, name}` named saved scopes |

---

## 9. Event records (diff engine output)

Two places events live:

1. **`snapshot.events[]`** — exact-timed **marker** events the in-game mod logged at this
   timestamp (DESIGN §8): capitulation, war declared, focus completed, faction join.
   Usually empty until the mod log channel is wired. Source **S/marker**.
2. **The replay event timeline** — the chronological, cross-snapshot feed the **diff
   engine** derives from adjacent snapshots and merges with the markers. This is the
   primary product; it is stored alongside snapshots (see README) and is what the
   native event/log views read.

### 9.1 `Event` record shape (both places use this shape)

```jsonc
{
  "type": "state_owner_change",   // §9.2 enum
  "hour_index": 17000300,         // int | null — when it happened (the LATER snapshot)
  "date": "1940.9.11.13",         // Clausewitz string of the later snapshot
  "subject": "GER",               // primary actor TAG, or null
  "object": "POL",                // affected party TAG / id, or null
  "key": "23",                    // entity id the event is about (state/province/div/tech/focus), or null
  "from": "POL",                  // prior value (TAG/level/law/...), or null
  "to": "GER",                    // new value, or null
  "source": "D",                  // C / M / S / D / marker (where the event was derived)
  "confidence": "exact",          // "exact" | "interval" | "inferred"  (§9.3)
  "detail": { ... }               // type-specific extras (optional)
}
```

`confidence`:
- **exact** — a marker event, or a diff on a collector/live memory layer
  (`C`/`M`) at that layer's cadence.
- **interval** — diff on a coarser (save) layer: the change happened *somewhere between*
  the two snapshots; `hour_index` is the later bound. `detail.interval_start_hour` gives
  the earlier bound.
- **inferred** — reconstructed (e.g. combat lifecycle, casualty attribution).

`source` is the channel the diff was derived from, computed per event-domain:
- **C** — both snapshots captured the domain from the in-process collector.
- **M** — both snapshots captured the domain from **memory** (⇒ `confidence` exact).
- **S** — both snapshots backed the domain from the **save** channel (a folded save
  backstop) ⇒ honest save-sourced provenance (`confidence` interval, not exact).
- **D** — mixed / synthetic derivation (neither pure-memory nor pure-save both sides).
- **marker** — an exact mod-log marker event.

### 9.2 Event `type` enum (what the diff engine derives)

Ownership / control (subject = `to or from`, so a TAG→null change is filterable by
the losing tag):
- `state_owner_change` — `key`=state, `from`/`to`=TAG, `object`=old owner.
- `state_controller_change` — `key`=state.
- `province_controller_change` — `key`=province.

Country lifecycle (emitted **only** when both adjacent snapshots captured the country
domain — otherwise a missing country is "not captured", not "ceased to exist"):
- `country_created` — `subject`/`key`=TAG (released/formed).
- `country_removed` — `subject`/`key`=TAG (annexed/merged; reason left to a future
  capitulation/peace correlation pass).

Divisions (`key`=`div_uid`):
- `division_created` — `to`=owner; `detail`=template.
- `division_destroyed` — `from`=owner.
- `division_moved` — `from`/`to`=province; `detail.from_state/to_state`.
- `division_template_changed` — `from`/`to`=template_id.

Politics / focus / research:
- `focus_completed` — `subject`=TAG, `key`=focus id.
- `law_changed` — `subject`=TAG, `key`=law category, `from`/`to`=law token.
- `idea_gained` / `idea_lost` — `subject`=TAG, `key`=idea token.
- `government_changed` — `subject`=TAG, `from`/`to`=ruling_party.
- `tech_researched` — `subject`=TAG, `key`=tech name, `to`=level (new tech, or a level
  **increase**).
- `tech_lost` — `subject`=TAG, `key`=tech name (level **decrease** or tech
  disappeared — save rollback / civil war).
- `research_slot_gained` — `subject`=TAG, `from`/`to`=slot count.

Diplomacy:
- `war_declared` — `subject`=attacker, `object`=defender. (Set-diff of each country's
  `wars[].against`; an A↔B war therefore surfaces from both sides — see §9.4.)
- `peace` — `subject`/`object`=former belligerents.
- `faction_joined` / `faction_left` — `subject`=TAG, `object`=faction name (a switch
  emits both).
- `subject_created` — `subject`=puppet TAG, `object`=overlord, `to`=autonomy level.
- `subject_freed` — `subject`=former subject TAG, `object`=overlord.
- `autonomy_changed` — `subject`=subject TAG, `from`/`to`=autonomy level.
- `capitulated` — `subject`=TAG (marker preferred; future inference from mass
  owner-flip; **not yet emitted by the diff engine** — reserved in the enum).

Economy / production:
- `production_milestone` — `subject`=TAG, `key`=equipment archetype, `detail`=threshold
  (cumulative gross produced crossing a configured step).
- `factory_milestone` — `subject`=TAG, `key`=`civ`/`mil`/`dockyard`, `to`=new total.

Commanders (from `commanders` domain):
- `commander_assigned` — `subject`=TAG, `key`=commander ident (`token`/unit_leader_id),
  `from`/`to`=formation name, `detail`={commander, role, kind}. Emitted only when a
  commander gains or switches a formation (formation→reserve is **not** emitted).

Order of battle (from `order_of_battle` domain):
- `army_formed` — `subject`=owner TAG, `key`=army `id:type`, `to`=name.
- `army_disbanded` — `subject`=owner TAG, `key`=army `id:type`, `from`=name.

Designs (from `designs` domain):
- `design_created` — `subject`=creator TAG, `key`=variant `id:type`, `to`=design name.
  Only player-authored designs (custom name OR modules) — stock archetypes appearing in
  the catalog at game start are not events.
- `mio_leveled` — `subject`=TAG, `key`=MIO `key`, `from`/`to`=org level (increase only).

Intel / operations (from `intel` domain):
- `operation_started` — `subject`=TAG, `object`=target TAG, `key`=`op_id`, `to`=op type.
- `operation_completed` — `subject`=TAG, `object`=target TAG, `key`=`op_id`, `from`=op
  type. Emitted when a running operation leaves the running set (finished/aborted/expired).

Trade / equipment market / lend-lease (from `market` domain; keyed by contract id):
- `lend_lease_started` — `subject`=TAG, `object`=counterpart TAG, `key`=contract id,
  `to`=lead equipment archetype. A trade contract appears for this country.
- `lend_lease_ended` — `subject`=TAG, `object`=counterpart TAG, `key`=contract id,
  `from`=lead equipment archetype. A contract disappears (completed/cancelled).
  > Directed contracts surface from **both** parties (seller `outgoing` + buyer
  > `incoming`), indexed by `contract_id|direction`. Group by `contract_id` for one
  > event per contract.

Occupation governance (from `state_detail` for resistance/compliance; `occupation`
for the law; keyed by state id):
- `resistance_changed` — `key`=state, `subject`=occupier TAG, `object`=occupied TAG,
  `from`/`to`=resistance value. Emitted only on a **band crossing** (the in-game
  15/25/40/60/80 milestones) — sub-band daily drift is suppressed as noise.
- `compliance_changed` — same shape, on a compliance band crossing.
- `occupation_law_changed` — `key`=state, `from`/`to`=occupation-law token.

World events / convoys (from `world_events` domain):
- `event_fired` — `key`=`to`=scripted-event id. A new id appeared in the persisted
  `fired_events` list.
- `convoy_sunk` — `subject`=killer TAG, `object`=owner TAG, `key`=`month:killer:owner`,
  `to`=convoy count (`from`=prior count when an existing month's tally grew). Emitted
  for a new per-month ledger entry, or an existing `(month,killer,owner)` whose tally
  increased between two saves.

Country extra (from `country_extra` domain):
- `nuke_count_changed` — `subject`=TAG, `from`/`to`=`nukes_total`. Only on an actual
  change (a measured `0`→`0` does not fire).
- `ace_gained` — `subject`=TAG, `key`=ace ident (`name|surname|callsign`), `to`=callsign,
  `detail`={name, surname, callsign, modifier}. A new ace appears.
- `project_completed` — `subject`=TAG, `from`/`to`=`special_projects.projects_completed`,
  `detail.completed_delta`. Emitted on an INCREASE only (a rollback does not fire).

Combat (Tier C, inferred):
- `combat_started` — `key`=combat_id; `confidence`="inferred".
- `combat_ended` — `key`=combat_id; `detail`=casualties/outcome; `confidence`="inferred".

### 9.3 Ordering & dedupe

The merged timeline is sorted by (`hour_index`, `type`, `key`). The dedupe key is
(`type`, `key`, `from`, `to`, `subject`) — including `from` so an A→B flip and a later
B→A flip are **not** collapsed.

Marker events (source `marker`) **supersede** an equivalent diff-derived event, with an
**interval-aware** window (so imported fixture intervals work, and collector-hourly
diffs do not eat real repeated flips):
- an **interval** diff is superseded iff a matching exact marker's hour lies in
  `(interval_start_hour, hour_index]` — exactly the window the change is known to have
  happened in;
- an **exact** (collector/live) diff is superseded only by a marker at the **same tick**.

This is how the cheap mod-log markers "sharpen" the diff feed (DESIGN §3, §8) without
double-counting.

**Same-hour coalescing.** Before diffing, `build_timeline` merges any snapshots that
share a (non-null) `hour_index` into one canonical snapshot (later wins on scalars;
domain containers shallow-merged; provenance domains unioned). This prevents a
live collector snapshot and an offline fixture import at the *same instant* — each
carrying complementary domains — from being diffed into fabricated null↔value transitions.

### 9.4 War events are pairwise (known limitation)

`war_declared`/`peace` are derived by set-diffing each country's `wars[].against`. An
A↔B war therefore surfaces from **both** A's and B's perspective. This is intentional
for now; a future war-id-scoped lifecycle (`war_started`/`war_ended`/
`war_participant_joined`) keyed off `diplomacy_world.wars` will replace it. Consumers
that want one event per war should group by the unordered TAG pair.

---

## 10. What this schema deliberately does NOT contain

Per DESIGN §4 hard-impossible — never add fields for these, never fake them:
- Steam / player real names. Humans are identified by TAG and the `is_human` flag.
- Sub-sample-transient micro-events (a combat that begins *and* ends between two
  samples; a one-shot decision click leaving no persistent state and no log marker).
- Pure-UI display numbers that exist only in the renderer.

A consumer that needs one of these must treat its absence as a hard limit, not a TODO.
