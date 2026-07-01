# Occupation Resistance Compliance Live Residual Report 2026-07-01

Completion status: `COMPLETE` for this residual-resolution continuation by
candidate rejection / exact fixture blockers. This is not a claim that the
existing emitted domain-52 slice contains garrison scalar/detail or country GIE
payloads; those fields remain fail-closed until a new controlled fixture proves
a production root.

Backlog reduced: yes.

Exact backlog items removed or narrowed:

- Country-scope GIE/collaboration scalar/tag hits: rejected for the current
  fixture. v23 resolved `10/10` target collaboration-status roots for
  `BEL/ENG/FRA/HOL/LUX/NOR/PHI/POL/USA/YUG`, found `1`
  `CCountryCollaborationData` hit and `16` scalar/tag hits, and found `0`
  scopes matching the required host plus target-country tag or unique
  legitimacy contract.
- `government_in_exile_tag` save keys: rejected as occupation/collaboration
  country detail for the current fixture. v23 classifies all `101/101` rows as
  asset scope: `20` leader, `58` ship, `3` deployment, `20` air wing.
- Country collaboration detail: exact fixture blocker. v23 has
  `countries.<TAG>.collaboration` empty for every country.
- Garrison scalar candidates: rejected for the current fixture. v23 compared
  `368` save garrison rows, `305` live garrison rows, `296` key matches, and
  `295` same-law matches; inline and direct/root candidates for
  `strength_ratio`, `garrison_required`, and `army_manpower_need` all had `0`
  threshold scopes.
- Garrison manpower/equipment/detail candidates: rejected or fixture-blocked.
  v23 has `340` manpower-pool rows, `343` equipment-need rows, and `132`
  reinforcement-request rows. The scan rejected manpower-pool value,
  `equipment_need` values for `infantry_equipment`, `support_equipment`, and
  `motorized_equipment`, and reinforcement request count. All `340`
  manpower-pool rows are single-tag and match the already emitted controller
  tag, so the current fixture does not prove an independent manpower vector.
- Occupation-linked damage/events: no separate occupation resistance event
  ledger was validated in this lane. Physical building damage remains domain
  `48` construction/state-damage scope; this lane should not duplicate it
  without a distinct resistance/sabotage ledger root.

## Validated Slice

The current production-shaped slice is domain `52`
`occupation_resistance_compliance_live` compact state rows:

- state owner/controller, occupied flag, occupied-country tag id;
- resistance/compliance raw;
- resistance/compliance speed raw;
- controller default occupation-law token;
- validated per-state occupation-law token where same-offset law/garrison joins
  pass;
- garrison template id from `CCountryOccupationStatus+0x78` when the root
  validates;
- fail-closed badges for unresolved garrison scalar/collaboration detail.

This remains a validated emitted slice. It is not an expanded detail payload for
garrison scalar/manpower/equipment or country GIE detail.

## Roots And Offsets

Validated current-build roots/offsets:

- `hoi4.exe + 0x3316E90 -> CGameState*`
- `CGameState + 0x2B0 -> CProvince**`, `+0x2BC -> province_count`
- `CProvince + 0xC0 -> CState*`
- `CState + 0x58 -> state_id`, `+0xC8 -> owner tag_db_id`,
  `+0xCC -> controller tag_db_id`
- `CState + 0x278 -> resistance_raw`, `+0x2A8 -> compliance_raw`,
  `+0x280 -> resistance_speed_raw`, `+0x2B0 -> compliance_speed_raw`
- `CGameState + 0x310 -> CCountry**`, `+0x31C -> country_count`
- `CCountry + 0xFD0 -> CCountryOccupationStatus*`
- `CCountry + 0xFD8 -> CCountryCollaborationStatus*`
- `CCountryOccupationStatus + 0x118 -> COccupationLaw*`
- `CCountryOccupationStatus + 0x78 -> CDivisionTemplate*` garrison template
  root, based on prior exact autosave/live proof
- `CCountryOccupationData` vtable `module+0x296DD00`
- `CCountryOccupationData + 0x08 -> CCountryOccupationStatus*`
- `CCountryOccupationData + 0x10 -> occupied/original tag_db_id`
- `CCountryOccupationData + 0x20` state-list container
- `CCountryOccupationData + 0xB0` law-list container
- `CCountryOccupationData + 0xD8` garrison-list container
- `CStateGarrisonData` vtable `module+0x296DCB0`
- `CStateGarrisonData + 0x50 -> CState*`
- `CStateGarrisonData + 0x58 -> CCountryOccupationData*`
- `CCountryCollaborationData` vtable `module+0x29A5D08`, reachable but target
  semantics rejected for the current country GIE fixture

Rejected current-build paths/candidates:

- `CStateGarrisonData + 0x48` as controller-country or occupied-country
  identity.
- `CCountryOccupationStatus + 0x80` as the garrison-template root.
- Pinned inline scalar candidates:
  `strength_ratio` `i32/u32 @0x10`, `i32/u32 @0x78`, `i32/u32 @0x7C`,
  `fixed100000_i32/i64 @0xF0`; `garrison_required`
  `fixed100000_i32/i64 @0xF8`; `army_manpower_need` `i32/u32 @0x88`.
- Direct/root scalar candidates reachable from heap pointers in
  `CStateGarrisonData+0x0..0x17F`, sampled for `0x300` bytes per root.
- Detail candidates for `manpower_pool` value, equipment need values, and
  reinforcement request count in the same inline/direct-root scan scope.
- Targeted country GIE scalar/tag hits that do not meet the required
  host-plus-target-or-unique-legitimacy contract.

## Compact And Rust

Compact schema/record changes in this continuation: none. No new field passed
the wireability threshold, so the existing lane-local domain `52` schema remains
unchanged in `domain52_compact.rs`.

Rust decoder/gate changes in this continuation: none. Existing
`domain52_gate.rs` and `replay_verify/` remain the correct gates for the
validated emitted slice.

Lane-local proof classifier changes:

- `remaining_fields_probe.py` classifies `government_in_exile_tag` contexts,
  recovers country-scope GIE owner/host/legitimacy rows, and rejects targeted
  live GIE scalar/tag hits unless they satisfy the host/country/unique-legitimacy
  contract.
- `garrison_scalar_probe.py` now scans inline `CStateGarrisonData`, direct/root
  pointers from that blob, garrison manpower-pool value, equipment need values,
  and reinforcement request count.

## Proof Commands And Results

v23 save/oracle:

```powershell
python save_oracle\parse_occupation.py extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots\autosave_melt_snapshot.txt --out-dir extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots\occupation_oracle
```

Result: save date `1943.5.1.1`, `439` countries with data, `31` active
occupiers, `163` occupation pairs, `443` per-state laws.

v23 garrison residual probe:

```powershell
python extractor\native\lanes\occupation_resistance_compliance_live\garrison_scalar_probe.py --out-dir extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots --save-melt extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots\autosave_melt_snapshot.txt
```

Result: `scanned_garrison_candidates_rejected_current_fixture`; `368` save
garrison rows, `305` live rows, `296` key matches, `295` same-law matches, and
`0` threshold scopes for inline scalar, direct/root scalar, manpower-pool value,
equipment-need value, or reinforcement-count candidates.

v23 remaining-field/GIE probe:

```powershell
python extractor\native\lanes\occupation_resistance_compliance_live\remaining_fields_probe.py --out-dir extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots --seconds 10 --oracle-occupation extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots\occupation_oracle\occupation.json --save-melt extractor\native\runs\occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots\autosave_melt_snapshot.txt
```

Result: resistance speed `92/92`, compliance speed `277/277`,
`CStateGarrisonData+0x50/+0x58` joins validated, `+0x48` country identity
rejected, collaboration/GIE verdict
`fixture_has_country_gie_payload_unjoined_to_live_collaboration_status`, and
GIE live graph verdict
`rejected_targeted_scalar_hits_no_gie_contract_current_fixture`.

Validated emitted-slice proof retained from v21:

- DLL-only compact stream:
  `extractor/native/runs/occupation_resistance_compliance_live_shared_20260701T165935/collector_stream.bin`
- `domain52_gate`: `records_seen=243`, `domain_data_records=81`,
  `rows=1966`, `occupation_garrison_template_rows=1093`,
  `occupation_law_token_rows=1854`, no parse errors.
- `.battleplan` verification through
  `...v21_stable_autosave_residuals/shared_stream_pack`: replay bytes `20552`,
  compact stream bytes `143644`, stream byte-compare unchanged.
- Shared DLL manifest: `80` game hours advanced, `81` unique game-hour samples,
  `0` skipped hours, hourly requirement met, and no source/partial/worker/bridge
  failure flags.

Unit and decoder tests retained:

- `collector_module.rs` tests: `9 passed`.
- `domain52_compact.rs` tests: `5 passed`.
- `replay_verify` tests: `2 passed`.

## Acceptance Fields

- DLL-only: yes for the validated emitted domain-52 slice through the shared
  compact stream; no new residual fields emitted.
- Compact stream: yes for the validated emitted slice.
- Rust decoded: yes for the validated emitted slice.
- `.battleplan` verified: yes for the validated emitted slice.
- Speed-five/current-equivalent hourly-or-lower: yes for the shared DLL proof.
- Zero skipped hours: yes for the shared DLL proof.
- Any partial/source/worker/bridge failures: no for the shared DLL proof.

## Exact Remaining Blockers

1. Garrison scalar/manpower/equipment detail needs a same-hour or controlled
   one-variable fixture that changes one garrison scalar/vector value at a time,
   or a newly validated non-inline root outside the scanned
   `CStateGarrisonData` inline/direct-root scope, before any nonzero detail
   payload can be emitted.
2. Country collaboration/GIE detail needs a fixture with non-empty
   `countries.<TAG>.collaboration`, or a controlled GIE fixture that changes one
   host/legitimacy/exile-leader value at a time. Current v23 scalar/tag hits are
   rejected for the available fixture.
3. Occupation-linked damage/events need a distinct live resistance/sabotage
   ledger root before this lane emits event records; physical building damage
   remains domain `48`.
