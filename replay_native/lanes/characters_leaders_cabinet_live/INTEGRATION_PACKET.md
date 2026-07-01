# characters_leaders_cabinet_live Integration Packet

Status: lane-ready, not merge-ready. This file is retained as lane-local schema
and patch-plan context. Current source of truth is
`replay_native/DOMAIN_REGISTRY.md`, `replay_native/domain_registry.json`, and
`replay_native/lanes/characters_leaders_cabinet_live/MERGE_READY_BLOCKER.md`.

This packet is lane-local. Do not apply shared collector/recorder edits from a
subagent unless the shared integration owner serializes the hook.

## Final Domain Choice

- Domain name: `characters_leaders_cabinet_live`.
- Scheduled-domain token: `characters_leaders_cabinet_live`.
- Proposed production compact domain id: `47`.
- Prior lane-local proof id: `44`, now rejected for integration because shared
  id `44` is already `supply_logistics_attrition_live`.
- Candidate id `45` is rejected for this lane because the current shared
  collector already defines `COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE`
  as `45`.
- Candidate id `46` is rejected for this lane by orchestrator allocation:
  `air_variant_archetypes_live` is production-complete on shared id `46`.

The lane codec is now `domain47_compact.rs`. The proof harness writes
`domain47_payload.bin` and `domain47_record.bin`.

## Compact Contract

Record wrapper:

- magic `H4CR`
- version `1`
- record type `3` (`DOMAIN_DATA`)
- domain id `47`
- payload header length `64`

Payload header:

- `status_flags` is reserved for shared compact health/failure bits only:
  worker panic, bridge unavailable, source unavailable, partial data, and future
  shared health bits.
- `schema_flags` is the header field at byte offset `56`. Current values:
  `1=roots_validated`, `2=static_name_closure`,
  `4=appointed_from_active_ideas`.
- byte offset `60` remains reserved and must be zero.

Rows are unchanged from the accepted bucket proof:

- `character`
- `unit_leader`
- `unit_leader_trait`
- `country_leader_role`
- `active_ruler`
- `advisor_roster`
- `appointed_advisor`

Advisor rows carry raw `slot` plus required compact `cabinet_bucket` in the
existing `ledger` string field. Valid buckets are `political_advisor`,
`army_chief`, `navy_chief`, `air_chief`, `high_command`, `theorist`, and
`unknown`.

## Shared Patch Plan

Collector target: `replay_native/collector/src/lib.rs`.

1. Add `RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN =
   "characters_leaders_cabinet_live"` next to the existing raw compact domain
   tokens.
2. Add `COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE: u16 = 47` after the
   reserved `air_variant_archetypes` domain id `46`.
3. Add `CompactCharactersLeadersCabinetTracker` to `CompactPerfGateTrackers`.
   The tracker owns warm pins only; decoded values are read per scheduled frame.
4. Add `prewarm_compact_characters_leaders_cabinet_roots` beside the division
   and market prewarm helpers. It should cold-locate RTTI/vtable pools and the
   country/politics bridge outside hook-held callbacks.
5. Add `characters_leaders_cabinet_live_enabled =
   compact_explicit_domain_enabled(control,
   RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN)` in the compact scheduled frame
   path.
6. Spawn `read_compact_characters_leaders_cabinet(...)` as an independent compact
   worker. Reuse existing country/politics roots for active ruler and appointed
   active-idea intersection; do not duplicate focus/research/decisions lane work.
7. Emit one `DOMAIN_PERF` record and one domain `47` `DOMAIN_DATA` record when
   scheduled. Put it after the reserved id `46` slot in deterministic sequence
   order.

Recorder target: `replay_native/recorder_host/src/main.rs`.

1. Add `47 => decode_compact_characters_leaders_cabinet_live_data_payload(...)`
   in the domain-data match.
2. Add `characters_leaders_cabinet_live => Some(47)` in `compact_domain_id`.
3. Add `47 => "characters_leaders_cabinet_live"` in `compact_domain_name`.
4. Import or inline the decoder gate from `domain47_compact.rs`, preserving
   required cabinet-bucket validation and duplicate-row guards.
5. Add a compact summary test with a synthetic perf/data/frame bundle for id
   `47`, plus a negative test for a bad `cabinet_bucket`.

## Integration Blocker

The lane-local nonzero appointed advisor transition fixture is now validated:
ENG transitions `0 -> 3 -> 1` through the pinned active-idea/advisor-roster
intersection and the generated domain `47` compact record decodes with
`appointed_advisor_rows=3` and `parse_errors=[]`.

The remaining production blockers are shared registration and the missing
collector-frame adapter for integrated DLL/Rust hourly capture. Current live
domain-47 proof still has measured-empty appointed rows, so the nonzero
transition proof is fixture-backed rather than an integrated hourly
`.battleplan` proof.
