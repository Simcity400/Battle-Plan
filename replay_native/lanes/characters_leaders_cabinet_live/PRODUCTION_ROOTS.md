# Characters / Leaders / Cabinet Live Lane

Lane scope: characters, unit leaders, country-leader roles, active rulers,
advisor roster/static closure, appointed cabinet state, and unit-leader skill and
trait membership. This lane deliberately does not own scientists, project
progress, market/trade, naval combat, or air combat losses.

## Production Roots

Build gate: `hoi4.exe` SHA-256
`16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120`.
All offsets below are build-specific and must fail closed when the build gate or
layout guards fail.

Character and unit-leader pools are rooted by RTTI/vtable heap scans:

| Class | RVA | Guards | Production fields |
| --- | ---: | --- | --- |
| `CCharacter` | `0x2957D40` | vtable, `+0x08 == 73`, `char_id @ +0x0C` | `country_tag_db_id @ +0x68`, unit-leader pointer `+0xA8` |
| `CArmyLeader` | `0x292F788` | vtable, `+0x08 == 4713`, `unit_leader_id @ +0x0C` | token `+0x20`, display name `+0x40`, XP raw `+0xE68`, skill-stat bytes `+0xF58/+0xF68/+0xF78/+0xF88`, character id copy `+0xF4C` |
| `CNavyLeader` | `0x292F8A0` | vtable, `+0x08 == 4713`, `unit_leader_id @ +0x0C` | token `+0x20`, display name `+0x40`, XP raw `+0xE68`, skill-stat bytes `+0xF68/+0xF78/+0xF88/+0xF98`, character id copy `+0xF4C` |
| `CCountryLeader` | `0x295A630` | vtable | role character pointer `+0x08 -> CCharacter` |
| `CAdvisor` | `0x2980560` | vtable, linked `CCharacter` | character pointer `+0x10`, advisor idea token `+0x30`, slot `+0x80` |
| `CUnitLeaderTrait` | `0x291F3C0` | vtable | token `+0x10`; unit-leader trait membership elements point at these defs |

## Current Live Proof Note

The active process during this lane pass was build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, so the
old Python memory-reader build gate correctly rejected it. A forced proof using
RTTI-discovered vtables produced these current-build RVAs:

| Class | Current build RVA |
| --- | ---: |
| `CCharacter` | `0x2967E30` |
| `CArmyLeader` | `0x293F538` |
| `CNavyLeader` | `0x293F650` |
| `CCountryLeader` | `0x296A720` |
| `CAdvisor` | `0x2990E60` |
| `CUnitLeaderTrait` | `0x292F160` |

That proof decoded `1213` army leaders, `259` navy leaders, `14161`
characters, `2219` country-leader role objects, and `3687` assigned
unit-leader trait tokens with zero missing skill/trait reads. It is still only a
partial proof because the legacy `characters_live.validate_addrs` warm-pin gate
returned `false`, and politics/cabinet rows were deliberately skipped after the
old save-PP country-array locator failed on this build. Evidence lives under
`replay_native/runs/characters_leaders_cabinet_live_20260701_rtti_units_v1/`.

Follow-up proof
`replay_native/runs/characters_leaders_cabinet_live_20260701_bridge_politics_v2/`
removes those blockers for the current build:

- The warm-pin dependency on `characters_live.validate_addrs` is bypassed by a
  lane-local filtered pin set. Raw aligned vtable hits are first reduced to
  validated object starts, then warm pins are checked with current RTTI vtables,
  type/id guards, role back-links, skill pointer reads, and the assigned-trait
  vector at `leader+0xDC8`. Result: `1232` army leaders, `264` navy leaders,
  `13787` validated character bodies, `2057` country-leader role bodies,
  `3736` assigned-trait rows seen, and zero warm-pin failures. The legacy helper
  still returned `false`, so it is rejected as the lane authority.
- The stale country route is bypassed without save PP. The replacement route is
  `locate.locate` for the live country record array plus
  `country_static_live.bridge_via_tag_table`, which maps each running index by
  `CCountry+0x08` tag-db-id through the live tag table. Result:
  `rec0=0x22BDDB44010`, `count=56`, `55` resolved countries, `revalidate=true`.
  The earlier full `CountryIndexBridge.resolve` object-array fingerprint returned
  `None` in
  `replay_native/runs/characters_leaders_cabinet_live_20260701_bridge_politics_v1/`.
- Active ruler is validated through the replacement bridge for `55` countries
  with `0` ambiguous leader vectors:
  `CCountry+0xF90 -> CPoliticalStatus+0xD0 -> CPoliticalParty+0x70 ->
  CCountryLeader[0]+0x08 -> CCharacter`.
- Advisor roster is validated from current-build `CAdvisor` RTTI roots:
  `9810` advisor addresses, warm-pin validation `true`, `9280` roster rows
  across `365` countries.
- Appointed advisor/cabinet buckets are validated as the active
  `politics.ideas` / advisor-token intersection for the `55` active politics
  countries. The current live frame measured `0` appointed rows and `55` empty
  buckets; this is a real measured empty result, not a skipped cabinet decode.

Compact proof
`replay_native/runs/characters_leaders_cabinet_live_20260701_compact_v2/`
then emitted a lane-local binary compact record from the same current-build
roots and decoded it through the then-provisional Rust gate
`domain44_compact.rs` (now moved to proposed production id
`domain47_compact.rs`):

- compact record:
  `replay_native/runs/characters_leaders_cabinet_live_20260701_compact_v2/domain44_record.bin`
  (`781273` bytes; payload `781241` bytes).
- Rust decode accepted the wrapped record with `parse_errors=[]`.
- Decoded rows: `29491` total, `13920` character, `1481` unit-leader,
  `3732` unit-leader-trait, `1965` country-leader-role, `55` active-ruler,
  `8338` advisor-roster, and `0` appointed-advisor rows.
- Source rows intentionally omitted before compact emission: duplicate advisor
  rows (`12`), advisor rows without usable identity (`5`), roster rows for the
  null/unresolved tag bucket (`917`), duplicate/missing country-leader role
  character ids (`98`), and unit leaders missing required identity (`36`).

Bucket proof
`replay_native/runs/characters_leaders_cabinet_live_20260701_buckets_v1/`
tightens the advisor/cabinet semantics without changing the row size. Advisor
rows now carry a required `cabinet_bucket` string in the existing advisor
`ledger` field, and the Rust gate accepts only:

- `political_advisor`
- `army_chief`
- `navy_chief`
- `air_chief`
- `high_command`
- `theorist`
- `unknown`

The live roster scan in that run found `9272` advisor rows across `365`
countries with slot counts: `political_advisor=3426`, `high_command=2148`,
`theorist=1273`, `army_chief=934`, `air_chief=855`, `navy_chief=633`, and
`unknown=3`. After production compact filtering to resolved country tag ids and
usable identities, the emitted compact roster had `8338` advisor rows bucketed
as `political_advisor=3370`, `high_command=1855`, `theorist=1084`,
`army_chief=788`, `air_chief=735`, and `navy_chief=506`; there were no emitted
`unknown` bucket rows in this frame. Appointed advisor/cabinet rows remained a
measured empty slice: `0` rows across the `55` active politics countries.

The wrapped compact record
`replay_native/runs/characters_leaders_cabinet_live_20260701_buckets_v1/domain44_record.bin`
decoded through the Rust lane gate with `parse_errors=[]`: `29537` total rows,
`13943` character, `1488` unit-leader, `3748` unit-leader-trait, `1965`
country-leader-role, `55` active-ruler, `8338` advisor-roster, and `0`
appointed-advisor rows. Rust tests now include a negative case that rejects
imprecise advisor buckets such as `political`.

Unit-leader overall skill and assigned trait membership are no longer open:

- `skill = *(u32*)(*(u64*)(leader + 0xE60) + 0x1B8)`.
- Assigned traits use `{data* @ leader+0xDC8, capacity u32 @ +0xDD0, size u32 @ +0xDD4}`.
  Iterate exactly `data[0..size)`. Elements are `CUnitLeaderTrait*` definition
  pointers resolved through the trait-def registry.
- `leader+0x288` is rejected for assigned membership. It is a modifier/progress
  bag, not the persistent trait set.

Active country ruler is a politics-side relation:

1. `CCountry + 0xF90 -> CPoliticalStatus`.
2. `CPoliticalStatus + 0xD0 -> ruling CPoliticalParty`.
3. `CPoliticalParty + 0x70 -> vector<CCountryLeader*>`.
4. `vector[0] + 0x08 -> CCharacter`.

`CCharacter` and `CCountryLeader` do not contain a reliable active-ruler flag.
The active-ruler row must be marked ambiguous when the party leader vector size is
not exactly `1`.

Appointed cabinet state should not sweep `CAdvisor` in the hourly DLL hot path.
The production path is active politics idea tokens intersected with a validated
static advisor-token contract from `game_defs` / `common/characters`. This is
the same contract family already used by scheduled `politics_residuals`.

Integration packet
`replay_native/lanes/characters_leaders_cabinet_live/INTEGRATION_PACKET.md`
removes the final shared-slot ambiguity:

- proposed production compact domain id: `47`;
- scheduled-domain token: `characters_leaders_cabinet_live`;
- lane codec: `domain47_compact.rs`;
- proof harness output names: `domain47_payload.bin` and `domain47_record.bin`;
- prior `domain44_*` artifacts are historical lane-local proofs only because
  shared id `44` is now `supply_logistics_attrition_live`.
- candidate id `45` is rejected because current shared collector source already
  defines `COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE: u16 = 45`.
- candidate id `46` is rejected by orchestrator allocation because
  `air_variant_archetypes` owns that proposed shared id.

The `domain47` packet also separates shared health status from lane capability
evidence. Payload header `status_flags` is now shared compact health only; the
schema/capability flags `roots_validated`, `static_name_closure`, and
`appointed_from_active_ideas` live in the header field at byte offset `56`.
Byte offset `60` remains reserved and must be zero.

Packet proof
`replay_native/runs/characters_leaders_cabinet_live_20260701_integration_packet_v2/`
validates the slot-ready shape after rejecting shared ids `44`, `45`, and `46`
for this lane:

- `domain47_record.bin` decoded through `domain47_compact.rs` with
  `parse_errors=[]`;
- `record_domain_id=47`, `status_flags_or=0`, `schema_flags_or=7`;
- `29848` total rows: `14071` character, `1533` unit-leader,
  `3884` unit-leader-trait, `1967` country-leader-role, `55` active-ruler,
  `8338` advisor-roster, and `0` appointed-advisor;
- advisor buckets in emitted compact rows:
  `political_advisor=3370`, `high_command=1855`, `theorist=1084`,
  `army_chief=788`, `air_chief=735`, `navy_chief=506`;
- packet validator result: id `44` rejected as
  `supply_logistics_attrition_live`, id `45` rejected as
  `focus_research_decisions_depth_live`, id `46` rejected as
  `air_variant_archetypes`, id `47` open, shared collector/recorder entrypoints
  untouched.

Appointed advisor transition fixture
`replay_native/runs/characters_leaders_cabinet_live_20260701_appointed_transition_fixture_v1/`
validates the nonzero appointed-advisor compact path without touching shared
entrypoints:

- fixture source is the pinned `politics_residuals` 1946 save-grounded anchor:
  ENG has exactly `ENG_leslie_hore_belisha`, `ENG_maxwell_aitken`, and
  `ENG_philip_kerr`, with live/save validation `46/46` and zero mismatches;
- static game definitions verify all three tokens as `political_advisor` in
  `<the game install>/common/characters/ENG.txt`;
- the fixture pulls the current domain-47 roster identities from
  `domain47_record.bin`: Philip Kerr `char_id=1456`, Maxwell Aitken
  `char_id=1458`, Leslie Hore-Belisha `char_id=1459`, all `country_tag_db_id=2`;
- transition snapshots through `read_appointed_advisors` produce counts
  `before=0`, `after_add=3`, `after_remove=1`;
- `appointed_transition_record.bin` decodes through `domain47_compact.rs` with
  domain `47`, `advisor_roster_rows=3`, `appointed_advisor_rows=3`,
  `status_flags_or=0`, `schema_flags_or=7`, and `parse_errors=[]`.

This closes the lane-local nonzero appointed-row compact/gate proof. It does not
replace the still-needed live hourly transition proof after shared DLL/Rust
integration gets a slot.

Static semantic closure proof
`replay_native/runs/characters_leaders_cabinet_live_20260701_static_semantic_closure_v1/`
adds lane-local replay semantics from game character definitions without adding
hot-loop reads or changing the compact schema:

- source root:
  `<the game install>/common/characters/*.txt`;
- country-leader static ideology/traits join key:
  compact `country_leader_role.token` and `active_ruler.token` ->
  character-definition token with `country_leader={ ideology=... traits={...} }`;
- advisor static slot/bucket/traits join key:
  compact advisor `idea_token` -> static
  `advisor={ slot=... ledger=... idea_token=... traits={...} }`;
- unit-leader static assignment join key:
  compact `unit_leader.token` -> static character token with
  `corps_commander`, `field_marshal`, or `navy_leader` blocks.

Validated counts from `static_semantic_closure.json`:

- `21/55` active-ruler rows have unambiguous static country-leader
  sub-ideology; `14` also have static country-leader traits. Missing or
  multi-role ambiguous rows are fail-closed.
- `417/1967` country-leader role rows have unambiguous static sub-ideology;
  `300` also have static country-leader traits.
- `3472/8338` advisor roster rows have unambiguous static role definitions,
  exact compact slot match, exact compact cabinet-bucket match, and static
  traits.
- `807/1533` unit-leader rows have static corps/field-marshal/navy assignment
  closure; `742` also have static traits.

Rejected/static-only candidates:

- Static advisor `allowed`, `available`, and `visible` blocks are not a live
  active/appointed-status root. The proof found `3468` compact advisor roster
  rows whose joined definition has at least one such scripted predicate; these
  remain definition metadata only. Production appointed status remains the
  active `politics.ideas` / `CAdvisor.idea_token` intersection.
- Non-character national spirits, laws, and generic `politics.ideas`
  classification are excluded from this lane unless the idea token also joins a
  `CAdvisor` role. Their general classification belongs to the politics/ideas
  route.

Parking matrix
`replay_native/runs/characters_leaders_cabinet_live_20260701_parking_matrix_v2/`
inventories the remaining deeper field backlog after static semantic closure:

- generic compact `character` rows currently carry neither name nor token
  (`0/14071`); usable names/tokens are present on role/leader/ruler rows
  (`1242/1533` unit-leader names, `1437/1533` unit-leader tokens,
  `39/55` active-ruler names, `48/55` active-ruler tokens,
  `1751/1967` country-leader role names, `1863/1967` country-leader role
  tokens). Do not claim full character localization closure until generic
  character-row token/name emission is wired.
- portraits are static `common/characters` metadata. The v2 matrix validates
  `4941` static characters with portrait blocks and `6289` portrait entries
  (`army=4737`, `civilian=1492`, `navy=60`) but parks them as a schema/static
  sidecar gap because current domain-47 rows have no portrait field and generic
  character rows still emit `0/14071` tokens.
- unit-leader XP/skill/stat/trait compact coverage is complete for emitted
  unit-leader rows (`1533/1533` skill, XP, and stat rows; `3884`
  unit-leader-trait rows). Runtime army/fleet command assignment remains an
  OOB/navy route, not character roster closure.
- `3765` static advisor roles were scanned; none define `skill`, `level`,
  `experience`, or `xp`. Advisor XP/level is rejected for domain 47 unless a
  real game mechanic/root appears.
- active ruler party is covered for `55/55` rows; deeper runtime sub-ideology
  beyond the static definition join remains unvalidated and fail-closed.
- appointed cabinet edge cases are blocked on either live integrated hourly
  capture or another nonzero live fixture. The lane-local ENG fixture validates
  transition counts `before=0`, `after_add=3`, `after_remove=1`.
- broader relationship closure is intentionally limited to validated role edges:
  `CCharacter+0xA8` unit-leader pointer, `CAdvisor+0x10 -> CCharacter`,
  `CCountryLeader+0x08 -> CCharacter`, and the politics-side active-ruler chain.
  Runtime command assignment remains an OOB/navy concern, and scripted/effect
  relationships require a separate decoder.
- shared adapter closure is hard-blocked until the one injected collector
  populates the lane-owned `LaneFrame` from `CollectorState`, country identity,
  politics active ideas, and validated character/advisor/leader roots. The exact
  lane-local adapter plan is
  `replay_native/lanes/characters_leaders_cabinet_live/LANEFRAME_ADAPTER_PLAN.md`.

## Compact Payload

Lane-local codec: `domain47_compact.rs`.

Proposed production compact domain id: `47`.

The payload is pointer-free and uses a per-record string table. Row kinds:

- `character`
- `unit_leader`
- `unit_leader_trait`
- `country_leader_role`
- `active_ruler`
- `advisor_roster`
- `appointed_advisor`

The row split is intentional:

- static-ish identity/name closure can be emitted as character and role rows;
- mutable hourly values stay in `unit_leader`, `active_ruler`, and
  `appointed_advisor` rows;
- trait membership is explicit row data keyed by `unit_leader_id`, so the decoder
  can prove declared trait counts match decoded trait rows.
- advisor roster and appointed advisor rows carry both the raw game `slot`
  string and a required compact `cabinet_bucket` enum in the existing `ledger`
  string field. The enum is the recorder-facing cabinet grouping; the raw slot is
  retained as source evidence.

## Integration Plan

Collector integration, once granted a shared-entrypoint slot:

1. Add `characters_leaders_cabinet_live` to the collector scheduled-domain parser
   and compact domain map with final domain id.
2. Add a lane-local collector module that cold-locates the RTTI/vtable pools,
   stores address pins plus semantic ids, and warm-rereads fields on scheduled
   frames. It must cache addresses, not decoded values, because XP, skill, and
   appointed state are mutable.
3. Validate every warm pin with vtable, type, stable id, character back-link, and
   trait-set count/def-pointer guards. On any miss, fail closed or cold-rescan
   outside hook callbacks.
4. Reuse the existing country object / politics roots for active rulers and
   appointed state. Do not duplicate the politics lane's active-ideas decode.
5. Emit one compact data record per scheduled frame using the row schema above,
   plus a perf record with row/unit counts and source-unavailable/partial flags.

Recorder integration:

1. Import the lane decoder/gate logic or mirror it in `recorder_host`.
2. Add domain id/name mapping and scheduled-domain coverage checks.
3. Expose semantic counters for character rows, unit-leader rows, trait rows,
   active rulers, advisor roster rows, and appointed advisor rows.
4. Pack the compact stream into `.battleplan` through the existing compact raw
   stream role; do not add JSON root frames as a production dependency.

Acceptance for production extraction remains: DLL-only capture, compact stream,
Rust decode, `.battleplan` verification, hourly-or-better at speed five, zero
skipped hours, and no partial/source/worker/bridge failures.
