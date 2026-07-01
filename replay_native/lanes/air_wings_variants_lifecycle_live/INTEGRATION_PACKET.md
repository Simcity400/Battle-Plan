# air_wings_variants_lifecycle_live lane mini stack

The old slot-ready packet workflow is canceled. The orchestrator registry now
assigns this lane final compact domain `46 = air_variant_archetypes_live`.
Shared collector/recorder entrypoint edits should be registration hooks only.

Lane-owned production files:

- `Cargo.toml`: standalone lane crate for isolated tests.
- `src/domain46_air_variant_archetypes_live.rs`: compact schema, encoder, and
  Rust decoder/gate for domain 46.
- `src/domain46_air_ace_roster_live.rs`: lane-local ace roster compact schema,
  encoder, decoder/gate, and H4CR record-type discriminator for domain 46
  residual ace identity/wing assignment rows.
- `src/ace_roster_live_roots.rs`: lane-local bounded root reader contract for
  ace roster capture. The shared collector only needs to implement
  `AceRosterMemory`, pass `CountryAceRoot` and `AirWingAceAssignmentRoot`, and
  call `read_ace_roster_from_roots`.
- `src/extractor.rs`: lane-owned extractor core contract, validated live roots
  and offsets, same-session token-dictionary classification helpers.
- `src/bin/ace_roster_record_gate.rs`: compact binary decoder proof for the ace
  roster residual record.
- `ace_roster_replay_store.py`: lane-local snapshot-store build/verify proof
  for packing ace compact records into `.battleplan` through the repo reference
  packer.
- `ace_root_candidate_probe.py`: bounded live RE/proof extractor for the direct
  ace roots.
- `ace_roster_cadence_probe.py`: lane-local repeated live compact-capture
  cadence proof.
- `src/lib.rs`: lane module exports.
- `REGISTRATION_PATCH_PLAN.md`: exact shared collector/recorder hook plan; no
  shared entrypoints are edited by this lane packet.

Validated roots and offsets:

- `CGameState+0x690 -> StrategicAirManager`
- owner containers `+0x30/+0x3C`
- wing pool `+0x58/+0x64`, pool `+0x28`, object `+0x10`
- wing fields `id +0x0C`, count `+0x6C`, mission mask `+0x90`,
  mission region `+0xB0`, effective planes `+0x160`,
  equipment bank `+0x1E8`
- variant token/stat offsets `+0x18/+0x1C/+0x38`
- same-session token dictionary route:
  `CCountry+0x230 CFlagManager`, `CFlagManager+0x18` token-record vtable,
  token record `+0x08` id, token record `+0x10` text pointer
- ace roster residual roots:
  `hoi4.exe+0x3316E90 -> CGameState*`,
  `hoi4.exe+0x33189C8 -> country database`,
  `CGameState+0x310` country table, `CGameState+0x31C` country count,
  `CCountry+0x12E4` ace count, `CCountry+0x12D8` ace roster vector,
  ace object `+0x0C` id, `+0xF0` name, `+0x110` surname, `+0x130` callsign,
  assigned ace id at `wing_body+0x2DC` / `wing_object+0x2EC`

Compact schema:

- Domain: `46 = air_variant_archetypes_live`
- Header length: `48`
- Row length: `32`
- Header carries elapsed time, row/change counts, status flags, observed active
  variant count, classified/unresolved counts, static catalog hash, and fixed
  scale.
- Row carries variant id, row flags, equipment token raw, equipment type token
  raw, interface type raw, optional air-superiority weight, source mask, and
  string-tail tokens for archetype/base-airframe/role.
- Source bits include live stats, live definition, static catalog,
  rejected dev-save join, static heuristic, live token dictionary, and static
  source-enum closure.
- Ace roster residual record:
  - Domain: `46 = air_variant_archetypes_live`
  - H4CR record type: `0xACE1`
  - Scope: `air_ace_roster_identity`
  - Header length: `48`
  - Row length: `32`
  - Row carries tag db id, ace id, country ace index, optional wing id, source
    mask, and compact strings for name/surname/callsign.
  - Source bits distinguish live country ace vector, live ace object layout,
    save-backed layout validation, live wing ace-id assignment, and blocked
    assignment scan.

Production proof:

- Final proof run:
  `replay_native/runs/air_wings_variants_lifecycle_live_modular_domain46_20260701_v3/`
- Command used the single DLL collector and Rust recorder with compact stream,
  `--stream-format compact`, `--extraction-perf-gate`,
  `--lightweight-root-frame`, and the non-loss air bundle plus domain 46.
- `.battleplan` was written:
  `native_session.battleplan` (`78933` bytes).
- Rust compact decode succeeded, domain coverage was OK, ordering was OK, and
  source/worker/bridge/partial failures were all false.
- Cadence gates passed:
  `hourly_requirement.met=true`, `game_hours_advanced=102`,
  `unique_game_hour_count=103`, `skipped_game_hours=0`.
- Domain 46 decoded `103` records, `112` rows, `112` classified rows, and
  `0` unresolved rows.
- The previous unresolved row is now intentionally fail-closed:
  first domain-46 data record row `97`, `variant_id=5085`,
  `equipment_token_raw=20378`, `equipment_type_token_raw=2`,
  `interface_type_raw=27`, `flags=2`, `source_mask=17`,
  archetype string `equipment_token_raw_20378`, empty base-airframe/role.
- Domain 46 read timing after amortization:
  avg `24104us`, max `42029us`, p95 `34741us`; frame max `89316us`,
  p95 `53643us`.

Follow-up validated-slice closure:

- Fresh lane proof
  `replay_native/lanes/air_wings_variants_lifecycle_live/proofs/domain46_live_token_lifecycle_20260701_v2/`
  validates the live token dictionary route on pid `45880` and resolves raw
  equipment token `20378` to
  `large_plane_maritime_patrol_plane_airframe_1`.
- The lane-local classifier now maps that token family to
  `base_airframe=large_plane_maritime_patrol_plane_airframe` and
  `role=maritime_patrol_plane`, backed by `common/script_enums.txt`,
  `common/technologies/bba_air_techs.txt`, and
  `common/units/equipment/x_plane_airframes.txt`.
- The Rust encoder now exposes
  `build_air_variant_archetype_payload_with_token_dictionary`, the exact
  shared-hook adapter expected from the orchestrator: pass
  `equipment_token_raw -> token_name` and domain 46 resolves dictionary-backed
  rows before falling back to raw token badges. The lane tests prove token
  `20378` resolves through this adapter without a static-heuristic fallback.
- Base-airframe/role closure covers the current modular airframe families and
  legacy air equipment names such as `fighter_equipment_*`,
  `CAS_equipment_*`, `nav_bomber_equipment_*`, `tac_bomber_equipment_*`,
  `strat_bomber_equipment_*`, jet variants, carrier variants, scouts, and
  transports.
- The integrated v3 compact stream remains the proof artifact for fail-closed
  raw fallback rows (`112` static-heuristic rows, `0` unresolved rows). Emitting
  the semantic maritime-patrol token in domain 46 still requires the
  orchestrator-owned shared hook to pass live dictionary names into the lane
  encoder.

Ace roster residual replay proof:

- Fresh lane proof
  `replay_native/runs/air_variant_archetypes_live_ace_roster_replay_870ebf_20260701_v1/`
  validates direct ace identity and wing assignment roots, emits domain `46`
  H4CR record type `0xACE1`, and packs the raw compact record into
  `air_ace_roster.battleplan`.
- The compact record gate decoded `176` ace rows, `175` identity rows, `128`
  wing-assignment rows, `176` live-layout-validated rows, and `0` parse errors.
- The lane-local replay pack verifier reopened the `.battleplan`, ran
  the reference `pack_replay verify` byte-identity check for the snapshot store,
  extracted the compact bytes embedded in the packed snapshot, and proved the
  compact record byte-for-byte identical to the source
  (`sha256=fa48f67113cae0da538cfff97c7ce44eee29498c984f3538bf6368565cf01313`).
- Cadence probe emitted three live ace compact records with valid domain `46`
  record type `0xACE1` headers. Sample times were `11621ms` to `14452ms`, all
  with validated assignment rows. This rejects Python `ReadProcessMemory` probe
  cadence as a production speed-five hourly proof; the DLL scheduler hook is
  still required. Startup-only/static ace capture is also rejected by decoded
  sample mutation: the three samples advanced from `185` to `186` to `188`
  rows and added live ace ids `119585`, `119658`, and `119788`.

Registration-only patch plan:

See `REGISTRATION_PATCH_PLAN.md`.

1. Keep domain ID `46` from `replay_native/domain_registry.json`.
2. Shared collector hook passes the shared same-session token dictionary into
   `build_air_variant_archetype_payload_with_token_dictionary`.
3. Shared collector hook implements `AceRosterMemory` over existing read
   helpers, passes live country/wing roots to
   `ace_roster_live_roots::read_ace_roster_from_roots`, and appends domain
   `46` record type `0xACE1`.
4. Shared recorder hook maps domain `46`/wire name `air_variant_archetypes` to
   the lane decoder/gate and only merges lane counters into the recorder
   summary.

Scope warning: this remains a validated lifecycle/archetype slice, not full air
operational or combat replay. Ace identity and current wing assignment now have
validated roots and lane-local replay proof, but they are not complete until the
single DLL collector/Rust recorder path emits and verifies record type `0xACE1`.
Sortie/combat pairing, exact displayed air-superiority percentages, and loss
cause/source remain outside domain 46 unless a future root/domain captures them
or rejects them with exact proof.
