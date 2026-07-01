# diplomacy_intel_operatives_live lane

Private replay extraction lane for diplomacy, intelligence, and operatives. This
directory is intentionally isolated from shared collector and recorder entrypoints
until an orchestrator grants an integration slot.

## Scope

Covered by this lane:

- wars and per-country at-war/casualty joins;
- faction-group membership;
- bilateral opinions and diplomatic relation flags;
- subjects and autonomy;
- intelligence agency scalars;
- active operative roster, detected state, skill, and experience;
- cryptology targets;
- operation anchors where currently known.

Not owned here:

- equipment-market purchase contracts and trade transfer extraction. This lane
  carries only relation flags such as `market_access_rights` and
  `equipment_purchase_contract_relation` as diplomacy join keys.

## Lane-local production stack

- `Cargo.toml` and `src/`: standalone lane-owned Rust crate. It compiles the
  final domain `53` compact schema, production-shaped extractor/admission code,
  and recorder-facing `H4CR` decoder gate without touching shared collector or
  recorder entrypoints.
- `src/extractor.rs`: DLL-collector-side module shape for typed validated rows.
  It admits current `870ebf` source proof mask `0x7F` for country, relation,
  agency, cryptology, operative, and war rows, and fails closed on operation
  detail, intel-map percentages, and incoming action queue.
- `src/decoder_gate.rs`: lane-owned Rust recorder gate for compact domain `53`
  `H4CR` domain-data records.
- `src/replay_verify.rs`: lane-local `.battleplan` pack/verify path using
  `snapshot::pack` and `snapshot::Container`; this verifies the domain 53
  compact stream bytes are preserved in a replay container without claiming a
  live shared-DLL speed-five run.
- `domain44_compact.rs`: reserved production compact domain `53` schema, encoder,
  decoder, and gate tests. The file name is historical from the provisional
  package; the packet constant is `DOMAIN_ID_PROPOSED = 53`, and the
  integration-facing decoder summary type is `Domain53GateSummary`.
- `domain44_slot_ready.rs`: lane-local integration admission layer. It wraps the
  compact encoder/decoder with source-proof bits so a future shared integration
  slot can emit only domain `53` rows backed by current `870ebf` roots, while
  automatically badging unresolved incoming-action, operation-detail, and
  intel-map percentage gaps.
- `live_probe.py`: read-only proof wrapper over existing validated live readers.
  It writes sanitized counts/root evidence for development proof. It is not a
  hot-loop capture path.
- `probe_870ebf_diplomacy.py`: bounded current-build probe for the diplomacy
  relation sub-slice. It gates on the live `870ebf` SHA, derives current
  relation-flag vtables from MSVC RTTI, validates country/faction/opinion/
  subject roots, and records rejected stale candidates.
- `probe_870ebf_intel.py`: bounded current-build probe for the intel sub-slice.
  It gates on the live `870ebf` SHA, derives agency/cryptology/operative RTTI
  vtables, validates agency, cryptology, operative identity, skill, and XP roots,
  and records rejected stale operative descriptors.
- `probe_870ebf_operations.py`: bounded current-build probe for the operation
  detail gap. It derives current operation-related RTTI vtables and rejects stale
  `16022851` operation-definition RVAs. It does not claim running-operation
  detail extraction.
- `remaining_root_fixture_gate.py`: lane-local executable no-go gate for the
  unresolved war, operation-detail, intel-map percentage, and incoming-action
  roots. It consumes current proof artifacts plus an optional same-tick fixture
  JSON and exits `2` with exact blocker evidence until all four surfaces have
  current-build accepted roots.
- `INTEGRATION_PATCH_PLAN.md`: exact registration-only shared patch plan for
  when the orchestrator opens an integration slot.
- `REGISTRATION_PATCH_PACKET.md`: concrete shared registry, collector
  coordinator, and recorder hook packet for the integration owner.

The shared collector already has compact domain `17` for country-level diplomacy
(`faction_group`, at-war tags, and casualties). This lane's schema is wider and
keeps typed rows separate so the final DLL collector can emit per-domain records
without overloading the old country-row payload.

## Exact roots

Current `870ebf` production roots validated by the lane-local probe:

- Country roots: `u64[module+0x3316E90] -> game_state`,
  `u64[module+0x33189C8] -> country_database`, `game_state+0x310 -> country_table`,
  `u32[game_state+0x31C] -> country_count`, `u64[country_database] -> tag_table`,
  `u32[country_database+0x08/+0x0C] -> tag_count/active_count`.
- Country identity: country object vtable `module_base+0x27D0E90` plus
  `u32[country+0x08] == tag_db_id`. The old `16022851` vtable
  `module_base+0x27C0E80` is rejected for current `870ebf`.
- Faction group: `country_object+0x598` i32.
- Relations manager: `country_object+0xF88 -> manager`, `manager+0x08 -> table`,
  `table[target_tag_db_id] -> relation_entry`.
- Opinion: `relation_entry+0x300` i32.
- Relation flags: qword scan of `relation_entry[0..0x340]` for current `870ebf`
  RTTI-derived vtables. Current observed RVAs include
  `CNonAggressionPactRelation=0x2949D98`, `CMilitaryAccessRelation=0x2949918`,
  `CEmbargoRelation=0x294A070`, `CGuaranteeRelation=0x2949CD8`,
  `CImproveRelationsRelation=0x2949A98`, `CAttacheRelation=0x2949B58`,
  `CLicenceProductionAccessRelation=0x2949858`,
  `CAirBaseAccessRelation=0x2A0A9D8`, `CDockingRightsRelation=0x2A0ABC8`,
  `CNavalBlockadeRelation=0x2A0AD70`,
  `CMarketAccessRightsRelation=0x2A0AF20`, and
  `CEquipmentPurchaseRelation=0x2A0AE58`.
- Subject/autonomy: `CSubjectRelation` current vtable `module+0x2949F18`, with overlord
  `+0x10`, subject `+0x14`, autonomy-state DB pointer `+0x58`, autonomy-state
  name at DB `+0x08`, and ratio `+0x60` i32 / `100000`.
- War relation rows: global current-build row pool with the old pair/echo row
  shape and current date RTTI at `+0x08`. Current accepted row layout:
  first tag id `+0x00`, second tag id `+0x04`,
  `+0x08` embedded date vtable in `{CGregorianDate module+0x2702190,
  CGameDate module+0x2702388}`, first casualties `+0x40` i32,
  second casualties `+0x48` i32, and echo tags at `+0x68/+0x6C`.
  `probe_870ebf_war_shape.py` v13 scanned 17,130 heap regions and
  9,189,241,728 bytes with zero read failures, found 850 current-date rows
  covering 715 ordered tag pairs, and warm-revalidated 850/850 row addresses
  after 250 ms.
- Agency: `country_object+0xFC0 -> CIntelligenceAgency`, vtable
  `module+0x296B338`; name `+0x80`, icon `+0xA0`, upgrade progress `+0xC8`,
  max operatives `+0xE4`, usable slots `+0xF0`, usable mirror `+0xF4`,
  defense `+0x128`.
- Cryptology: `agency+0x120 -> CCryptology`, vtable `module+0x29A6540`;
  `manager+0x28 -> CCountryDecryptionState` records with stride `0x38`,
  record vtable `module+0x29A64F0`, target `record+0x08`, decryption byte
  `record+0x12`, and amount `record+0x18` i64 / `100000`.
- Active operative identities: heap identity object with `(type=4713, op_id)`,
  real leader base `L = identity_addr - 0x8`, vtable
  `COperativeLeader=module+0x293F790`, duplicate UTF-8 names at
  `identity+0x18/+0x38`, detected state `identity+0x68`, lifecycle
  `identity+0x110 == 47`, owner `identity+0x118`.
- Operative skill/experience: skill `u32[u64[L+0xE60]+0x1B8]`; experience
  `i64[L+0xE68] / 100000`.

Current `870ebf` candidates rejected or not yet accepted:

- Old country vtable `module+0x27C0E80`: rejected, zero valid country objects
  versus 434 valid objects at `module+0x27D0E90`.
- Old relation flag vtable RVAs from `16022851`: rejected for current relation
  entries, zero hits.
- Old relation flag RVAs shifted only by the country-vtable delta `0x10010`:
  rejected for current relation entries, zero hits.
- Old war date vtable RVAs from `16022851`: rejected for current `870ebf`.
  `CGameDate module+0x26F2398` and `CGregorianDate module+0x26F21A0` have no
  current RTTI name and produced zero current-date rows in the v13 heap proof.
  The current replacement vtables are `CGameDate module+0x2702388` and
  `CGregorianDate module+0x2702190`.
- Old active-operative class descriptor `module+0x292F9E0`: rejected for current
  `870ebf`, zero active hits. The old descriptor shifted by the country-vtable
  delta is also rejected, zero active hits.
- Old operation-definition RVAs from `16022851`: rejected for current `870ebf`
  by module RTTI. The stale RVAs no longer point at the expected operation
  vtables: `COperationsDatabase 0x26F4CB0`, reloadable operations DB
  `0x26F4C70`, `COperationPhase 0x26F4BB0`, `COperationToken 0x26F4DE0`, and
  `COperationResources 0x26F4B60`.
- Old operation-definition RVAs from `16022851`: also rejected by the current
  `870ebf` heap scan in
  `runs/diplomacy_intel_operatives_live_870ebf_20260701_v12_operation_heap`.
  The scan covered 17,375 regions and 9,420,631,012 bytes with zero read
  failures, did not time out, and found zero heap qword hits for every stale
  operation vtable listed above.
- Current `870ebf` operation-related RTTI vtables are derived as definition
  surface only, not running-operation detail roots:
  `COperationsDatabase=0x2704CA0`,
  `TReloadableGameItemDatabaseSpecific<COperationsDatabase,COperation>=0x2704C60`,
  `COperation=0x2928C98`, `COperationPhase=0x2704BA0`,
  `COperationPhasesDatabase=0x2704D90`,
  `COperationToken=0x2704DD0`,
  `COperationTokensDatabase=0x2704ED0`, and
  `COperationResources=0x2704B50`. The same heap scan observed current RTTI
  objects (`COperation=38`, `COperationResources=125`, `COperationPhase=82`,
  `COperationToken=5`, `COperationsDatabase=1`, `COperationTokensDatabase=1`,
  `COperationPhasesDatabase=1`), so the remaining operation blocker is field
  decode plus a running-operation fixture, not current RTTI discovery.

Legacy `16022851` roots retained as research input only:

- War relation rows: pinned `war_relation` record addresses, with
  `record+0x00/+0x04` tag ids, `record+0x08` date vtable in
  `{module+0x26F2398,module+0x26F21A0}`, casualties at `+0x40/+0x48`, and echo at
  `+0x68/+0x6C`.
- Old relation flags: qword scan of `relation_entry[0..0x340]` for
  `16022851` flag vtables.
- Old subject/autonomy: `CSubjectRelation` vtable `module+0x293A168`.
- Old active operatives: `identity-0x08 == module+0x292F9E0` is retained only
  as a rejected old-build candidate.

Still not pinned:

- War start-date/lifecycle detail decode. The current war row root and casualty
  row shape are validated, but the embedded date object's scalar date fields and
  higher-level war lifecycle grouping/name are not decoded yet. Relation-entry
  pointer/range joins were also not observed in v13, so treat the row pool as a
  global pool keyed by tag pair plus warm address validation.
- Running operation full detail. The current `870ebf` operation RTTI definition
  surface is pinned and the stale `16022851` operation-definition RVAs are
  rejected, but the running `COperation` struct fields are still not decoded.
  `final_residuals_live.py` remains old-build research input only.
- Intel-map percentages. The save oracle parses positional
  `countries.<TAG>.intel.intel` rows, but no live owner/target percentage root is
  validated yet.
- Incoming diplomatic action queue. Current validated live surface is relation
  state/flags; no pending-action queue root is accepted by this lane.
  Until a root is validated, compact frames must carry
  `STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED`.

The executable fixture contract for the remaining gaps is
`remaining_root_fixture_gate.py`. Required fixture sections are
`operation_detail`, `intel_map_percentages`, and `incoming_action_queue`; each
must be same-tick current-build evidence before the gate can clear those gaps.
Use `fixture_contract.template.json` for the positive fixture shape.

## Compact schema

`domain44_compact.rs` encodes one payload with:

- header: elapsed wall time, row totals, status flags, schema version, scale, and
  per-kind counts;
- row kinds: country diplomacy, war relation, opinion, relation flag, subject,
  agency, operative, cryptology, operation anchor, and intel-map row;
- fail-closed flags for partial/resisted rows, especially operations detail and
  intel-map values.

The decoder rejects unknown row kinds, unknown relation flag kinds, duplicate
stable keys, invalid tag pairs, fabricated skill/experience fields without
presence flags, and resisted intel-map rows that still emit guessed values. It
also rejects unknown top-level status bits, resisted operation/intel-map rows
without matching top-level badges, and any war rows or country war projections
emitted while `STATUS_WAR_ROOT_UNVALIDATED` is set.

`domain44_slot_ready.rs` adds the source-proof admission contract for the shared
slot handoff:

- this lane must emit production compact domain `53`;
- occupied lower domain IDs are reserved and rejected in the lane packet:
  `44=supply_logistics_attrition_live`,
  `45=focus_research_decisions_depth_live`, `46=air_live`,
  `47=characters_leaders_cabinet_live`,
  `48=construction_buildings_state_damage_live`,
  `49=naval_combat_live`, `50=naval_combat_live_aux`,
  `51=scripted_state_live`, and
  `52=occupation_resistance_compliance_live`;
- current accepted source proofs are build SHA, country roots, relation roots,
  agency root, cryptology root, and operative root;
- war rows and country war/casualty projections require
  `SOURCE_PROOF_WAR_ROOT_870EBF`;
- unresisted operation rows require `SOURCE_PROOF_OPERATION_DETAIL_870EBF`;
- unresisted intel-map rows require
  `SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF`;
- if the incoming diplomatic action queue lacks
  `SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF`, the payload is badged with
  `STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED`;
- unknown source-proof bits hard-fail before integration emission.

## Integration patch plan

1. Add compact domain ID `53` in the shared collector and recorder once the
   orchestrator opens an integration slot. Do not reuse occupied IDs `44..52`.
2. Move `domain44_slot_ready.rs` checks into the shared collector-side domain
   admission path before any compact bytes are emitted.
3. Port the validated root readers into a DLL-side module:
   - reuse shared country/tag bridge;
   - emit war relation rows and country war/casualty projections from the
     current `SOURCE_PROOF_WAR_ROOT_870EBF` row pool; suppress them if the
     current-build proof is absent or stale;
   - add relation-entry table walks for opinion, flags, and subjects;
   - add agency, operative, skill/XP, and cryptology readers.
4. Emit domain `53` `COMPACT_RECORD_TYPE_DOMAIN_DATA` records using the lane row
   layout.
5. Add recorder-host decoder branch and manifest counters mirroring
   `Domain53GateSummary`.
6. Run speed-five/current-equivalent proof requiring hourly-or-lower frames, zero
   skipped hours, no source/worker/bridge failures, and `.battleplan`
   verification.

Until step 5 passes, this lane is not production complete.
