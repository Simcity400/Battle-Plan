# diplomacy_intel_operatives_live domain 53 integration patch plan

Shared integration slot status: not owned by this lane. Do not apply these
steps until the orchestrator opens the slot.

Current lane-local production stack:

- `src/extractor.rs` builds domain 53 payloads from typed validated rows and
  source-proof bits. Current accepted source mask is `0x7F`: build SHA,
  country roots, relation roots, agency root, cryptology root, operative
  identity/lifecycle root, and war root.
- `domain44_compact.rs` is the historical filename for the final domain 53
  compact row schema and decoder. It rejects war rows/country war projections
  when `STATUS_WAR_ROOT_UNVALIDATED` is set, requires the current schema to
  badge emitted war rows/projections with
  `STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED`, rejects fabricated operative
  fields, carries rooted operative lifecycle flags as `operative_lifecycle_rows`,
  carries raw agency capacity clusters as `agency_capacity_cluster_rows`,
  carries compact validated-state observations as `compact_state_event_rows`,
  and rejects resisted operation/intel-map rows unless the top-level badges are
  present.
- `src/decoder_gate.rs` wraps and decodes the recorder-facing `H4CR` domain data
  record for compact domain `53`.
- `src/replay_verify.rs` packs and verifies a lane-local `.battleplan` container
  using the same `snapshot::pack` / `snapshot::Container` primitives as the Rust
  recorder.
- `REGISTRATION_PATCH_PACKET.md` contains exact shared registry/coordinator/
  recorder hook anchors for the integration slot.

Exact registration-only port once a slot opens:

1. Collector registry: domain `53` is already reserved in
   `replay_native/collector/src/domain_registry.rs`. Keep the same ID/name;
   only move status if the orchestrator requests it.
2. Collector module: add a domain-owned module equivalent to
   `replay_native/lanes/diplomacy_intel_operatives_live/src/extractor.rs` plus
   the compact schema. The worker should pass only typed validated rows from the
   current `870ebf` roots listed below.
3. Collector emission: call `build_domain53_payload(..., 0x7F)` for the current
   validated subset. If `report.ok() == false`, emit no domain 53 data record
   and surface the errors as source failure. If it is true, append an `H4CR`
   domain-data record for domain `53`.
4. Recorder registry: domain `53` is already reserved in
   `replay_native/recorder_host/src/compact_domain_registry.rs`. Keep the same
   ID/name; only move status if the orchestrator requests it.
5. Recorder decoder: route domain `53`, record type `3`, through the same checks
   as `src/decoder_gate.rs` and `domain44_compact.rs::decode_payload`.
   Preserve the generic `lane_summary.data_units_total` merge from
   `REGISTRATION_PATCH_PACKET.md`; v21 validates that this is what carries
   `operative_lifecycle_rows` into the shared manifest, and v26 requires the
   same merge for `compact_state_event_rows` plus source-specific state-event
   counters.
6. Required combined proof before production-complete: shared collector build,
   shared recorder build, speed-five/current-equivalent hourly-or-lower proof,
   zero skipped hours, no source/worker/bridge failures, and `.battleplan`
   verification.

Current validated roots for source mask `0x7F`:

- `u64[module+0x3316E90] -> game_state`
- `u64[module+0x33189C8] -> country_database`
- `game_state+0x310 -> country_table`, `game_state+0x31C -> country_count`,
  country vtable `module+0x27D0E90`
- `country+0xF88 -> relation manager`, `manager+0x08 -> relation table`,
  `relation+0x300 -> opinion`
- `CSubjectRelation module+0x2949F18`, fields `+0x10/+0x14/+0x58/+0x60`
- `country+0xFC0 -> CIntelligenceAgency`, vtable `module+0x296B338`,
  raw capacity cluster `agency+0xE4/+0xF0/+0xF4` with `+0xE4==+0xF4` invariant;
  keep these as raw values until a UI/save fixture names the semantics
- `agency+0x120 -> CCryptology`, vtable `module+0x29A6540`, decryption records
  at `manager+0x28`, stride `0x38`, state vtable `module+0x29A64F0`
- `COperativeLeader` active vtable `module+0x293F790` plus active identity,
  `identity+0x110` lifecycle flag cohorts `{47,31,63}`, owner, and skill/XP
  stats path
- War relation rows: global row pool with tag ids `+0x00/+0x04`, current date
  RTTI vtable at `+0x08` in `{CGregorianDate module+0x2702190,
  CGameDate module+0x2702388}`, casualties `+0x40/+0x48`, and echo
  `+0x68/+0x6C`; v13 proof warm-revalidated 850/850 rows.

Fail-closed roots still excluded from source mask `0x7F`:

- War lifecycle detail: current war rows are a validated tag-pair/casualty
  slice only; embedded date scalar and war grouping/name remain unvalidated.
- Operation detail: current operation RTTI is definition surface only;
  `accepted_for_operation_detail=false`.
- Intel-map percentages: fixture/root still missing.
- Incoming diplomatic action queue: fixture/root still missing.
- Compact event/state records beyond validated-state observations: war lifecycle, diplomacy queue, running operation, spy-network, and full agency/operative event semantics still need accepted roots/fixtures.
