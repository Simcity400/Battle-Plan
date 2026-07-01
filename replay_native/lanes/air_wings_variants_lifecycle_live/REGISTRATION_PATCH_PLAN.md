# air_variant_archetypes_live domain 46 registration patch plan

This lane does not edit shared collector or recorder entrypoints directly.
The orchestrator-owned integration should be a registration hook only.

## Collector hook

Shared file: `replay_native/collector/src/lib.rs`.

1. Keep the existing air compact domains and trackers as the source rows:
   `air_variant_stats`, `air_superiority_inputs`, `air_variant_defs`,
   `stockpile_variant_defs`, and the current shared token dictionary route.
2. Keep the existing shared compact id constant:
   `COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES: u16 = 46`.
3. Reuse the lane crate helper:
   `build_air_variant_archetype_payload_with_token_dictionary`.
4. Build lane input rows from the validated live variant stat rows:
   `variant_id`, `equipment_token_raw`, `equipment_type_token_raw`,
   `interface_type_raw`, and optional air-superiority weight.
5. Pass the shared same-session token dictionary as
   `BTreeMap<u32, String>` keyed by `equipment_token_raw`. The lane helper
   fills missing `equipment_token_name` values before encoding. This is the
   hook that turns raw fallback rows like `equipment_token_raw_20378` into
   semantic rows such as `large_plane_maritime_patrol_plane_airframe_1`.
6. Emit the encoded payload on domain `46` with wire name
   `air_variant_archetypes`.
7. Preserve the existing fail-closed behavior: if the token dictionary is
   unavailable or partial, emit raw-token fallback badges rather than guessing.

## Ace roster residual hook

Shared file: `replay_native/collector/src/lib.rs`.

The new ace residual remains domain `46`, but it must not reuse the existing
variant-archetype compact record shape. Emit it as H4CR domain `46`,
record type `0xACE1`, scope `air_ace_roster_identity`.

1. Reuse the validated current-build roots:
   `hoi4.exe+0x3316E90 -> CGameState*`,
   `hoi4.exe+0x33189C8 -> country database`,
   `CGameState+0x310` country table, `CGameState+0x31C` country count,
   `CCountry` vtable RVA `0x27D0E90`.
2. Add one collector-local memory adapter implementing the lane trait
   `ace_roster_live_roots::AceRosterMemory` over the existing read helpers
   (`read_u32`, `read_u64`, bounded byte/string reads, heap-pointer validation).
   No new Cargo dependency is required; the shared collector already depends on
   `air_wings_variants_lifecycle_live_lane`.
3. Build `Vec<CountryAceRoot>` from the validated live country rows and build
   `Vec<AirWingAceAssignmentRoot>` from the already-enumerated air-wing rows.
   The lane reader performs the bounded root reads:
   `CCountry+0x12E4` ace count, `CCountry+0x12D8` pointer/capacity/size roster
   header, ace object `+0x0C/+0xF0/+0x110/+0x130`, and wing body `+0x2DC`.
4. Call
   `air_wings_variants_lifecycle_live_lane::ace_roster_live_roots::read_ace_roster_from_roots(memory, &country_roots, &wing_roots, assignment_validated)`.
   If `assignment_validated=false`, the lane reader emits identity rows with no
   wing ids and sets the blocked-assignment source bit.
5. Feed `AceRosterRootRead.rows`, `observed_country_rows`, and
   `observed_ace_rows` to `build_air_ace_roster_payload`.
6. Wrap that payload with
   `domain46_air_ace_roster_live::encode_compact_data_record(sequence,
   current_game_date_raw, payload)`; the wrapper writes domain `46` and record
   type `0xACE1`.
7. Append the record with the existing `append_compact_record` helper using
   `domain_id=46` and `record_type=0xACE1`.
8. Schedule this capture hourly-or-better while the game is running. Ace identity
   is mostly stable after creation/death, but wing assignment is mutable, so
   startup-only capture is not sufficient for replay fidelity.

## Recorder hook

Shared file: `replay_native/recorder_host/src/main.rs`.

1. Keep the existing registry mapping for wire domain name
   `air_variant_archetypes` / registry name `air_variant_archetypes_live` /
   compact domain id `46`; no registry or Cargo change is required.
2. Dispatch domain `46` by H4CR `record_type`:
   - existing variant-archetype record type `3` -> existing
     `domain46_air_variant_archetypes_live::decode_payload`;
   - ace roster record type `0xACE1` ->
     `domain46_air_ace_roster_live::decode_compact_data_record`.
3. Merge the lane decoder counters into the existing compact-stream summary:
   `air_variant_archetype_rows`,
   `air_variant_archetype_live_token_dictionary_rows`,
   `air_variant_archetype_static_heuristic_rows`,
   `air_variant_archetype_base_airframe_string_rows`,
   `air_variant_archetype_role_string_rows`,
   `air_ace_roster_rows`,
   `air_ace_roster_identity_rows`,
   `air_ace_roster_wing_assignment_rows`,
   `air_ace_roster_live_layout_validated_rows`,
   `air_ace_roster_live_wing_assignment_validated_rows`,
   `air_ace_roster_wing_assignment_scan_blocked_rows`, and parse errors.
4. Keep `.battleplan` packing unchanged: domain `46` remains another compact
   compact record in the single collector/recorder stream. For replay raw-stream
   preservation, the record is byte-copied by the shared recorder. The
   lane-local proof uses the reference snapshot packer and verifies the embedded
   compact bytes byte-for-byte after reopening the `.battleplan`.

## Cadence decision

Startup-only or bounded-static ace capture is rejected for this residual. The
lane proof under
`replay_native/runs/air_variant_archetypes_live_ace_roster_replay_870ebf_20260701_v1/cadence/`
decoded three domain `46` / record type `0xACE1` samples whose roster rows
changed from `185` to `186` to `188`, adding live ace ids `119585`, `119658`,
and `119788`. A startup snapshot would miss replay-visible ace creation/death
and later wing-assignment changes. Therefore the shared coordinator must schedule
the ace roster record on the same hourly-or-better live frame cadence as the
other compact replay domains.

## Scope line

This patch plan closes the domain-46 shared token/name hook and adds the
validated ace roster identity plus wing-assignment residual. It does not claim
full air operational or combat replay. Sortie/combat pairing, loss cause, exact
displayed air-superiority percentages, and any non-extractable lifecycle fields
still require their own roots/domains or exact blockers.
