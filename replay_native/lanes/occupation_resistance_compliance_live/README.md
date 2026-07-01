# Occupation Resistance Compliance Live Lane

Lane-local production stack for `occupation_resistance_compliance_live`.

## Validated production root slice

The live proof module validates this root chain on build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`:

- `hoi4.exe + 0x3316E90 -> CGameState*`
- `CGameState + 0x2B0 -> CProvince**`
- `CGameState + 0x2BC -> province_count`
- `CProvince + 0xC0 -> CState*`
- `CState + 0x58 -> state_id`
- `CState + 0xC8 -> owner tag_db_id`
- `CState + 0xCC -> controller tag_db_id`
- `CState + 0x278 -> resistance_raw`, fixed scale `100000`
- `CState + 0x2A8 -> compliance_raw`, fixed scale `100000`
- `CState + 0x280 -> resistance_speed_raw`, fixed scale `100000`
- `CState + 0x2B0 -> compliance_speed_raw`, fixed scale `100000`
- `CGameState + 0x310 -> CCountry**`
- `CGameState + 0x31C -> country_count`
- `CCountry + 0xFD0 -> CCountryOccupationStatus*`
- `CCountryOccupationStatus` vtables `module+0x296DD50` and `module+0x296DD88`
- `CCountryOccupationStatus + 0x118 -> COccupationLaw*`
- `COccupationLaw` vtable `module+0x2928218`; law id string at `+0x10`
- `CCountry + 0xFD8 -> CCountryCollaborationStatus*`
- `CCountryCollaborationStatus` vtable `module+0x29A5D58`
- `CCountryOccupationData` vtable `module+0x296DD00`, reachable from
  `CCountryOccupationStatus` bounded graph
- `CCountryOccupationData + 0x08 -> CCountryOccupationStatus*` parent
- `CCountryOccupationData + 0x10 -> occupied/original tag_db_id`
- `CCountryOccupationData + 0x20` points to a state-list container with
  `CState*` entries
- `CCountryOccupationData + 0xB0` points to a law-list container with
  `COccupationLaw*` entries
- `CCountryOccupationData + 0xD8` points to a garrison-list container with
  `CStateGarrisonData*` entries
- `CStateGarrisonData` vtable `module+0x296DCB0`
- `CStateGarrisonData + 0x50 -> CState*`
- `CStateGarrisonData + 0x58 -> CCountryOccupationData*`
- Per-state occupation-law assignments are classified for rows where sibling
  `CCountryOccupationData + 0xB0` and `+0xD8` containers carry a
  `COccupationLaw*` and a `CStateGarrisonData*` at the same entry offset. The
  law key is the `CStateGarrisonData + 0x50 -> CState` join; the law value is
  the `COccupationLaw` id string at `+0x10`.
- `CCountryCollaborationData` vtable `module+0x29A5D08` is reachable from the
  collaboration-status graph, but its target semantics are not yet classified

The state/province join keys intentionally match the existing `map_control` and
`state_detail` production lanes: `state_id`, `province_id`, owner tag-db id, and
controller tag-db id.

Rejected/narrowed current-build candidates:

- `CCountryOccupationStatus + 0x78 -> CDivisionTemplate*` is the garrison
  template root. Fresh autosave/live proof matched `102/102` comparable rows
  with `0` mismatches.
- `CCountryOccupationStatus + 0x80` is rejected as the garrison-template root
  for current `870ebf`; the fresh autosave/live proof had `99` mismatches.
- `CStateGarrisonData + 0x48` must not be used as the controller-country join.
  The v5 bounded graph proof found `0` controller-country matches and `0`
  occupied-country matches while validating `203` rows by `+0x50` state and
  `+0x58` occupation-data joins.
- Garrison need/equilibrium/progress scalar offsets inside `CStateGarrisonData`
  remain unclassified, but the v19 bounded scalar proof conclusively rejects
  the pinned inline candidates for the current fixture. The proof compared
  `392` save garrison rows with `225` live rows, `34` key matches, and `18`
  same-law matches. As `strength_ratio`, `i32/u32 @ 0x10`, `0x78`, and `0x7C`
  plus `fixed100000_i32/i64 @ 0xF0` all missed every non-default fixture row.
  As `garrison_required`, `fixed100000_i32/i64 @ 0xF8` matched `0/18`. As
  `army_manpower_need`, `i32/u32 @ 0x88` matched `0/18`. Keep
  need/equilibrium/progress row-badged unavailable until a same-hour or
  one-variable controlled fixture proves a different scalar offset.
- Collaboration/GIE detail remains fixture-gated, and the current save fixture
  rejects the only visible GIE assignment keys as occupation-lane data. The v19
  occupation oracle reports `countries.<TAG>.collaboration` empty for every
  country. The save melt contains `8` `government_in_exile_tag` assignments,
  but all `8/8` are in leader/character context (`POL`, `ROM`, `YUG`), not
  country occupation/collaboration payloads. The bounded live graph still finds
  one reachable `CCountryCollaborationData` object for `GER` at
  `root +0x48 +0x90`, but a fixture with non-empty country collaboration or
  country-scope GIE data is required before wiring detail fields.

## Compact schema

`collector_module.rs` is the lane-owned collector-side Rust module. It contains
the build gate, exact current-build offsets, `OccupationMemory` read trait,
state/country occupation extraction, fail-closed row badges, compact payload
encoding, and a baseline/delta tracker. It is intentionally not registered in
shared collector entrypoints while another lane owns the integration slot.

`domain52_compact.rs` defines the lane-local compact payload for domain id `52`.
`domain52_gate.rs` is a lane-local Rust decoder gate for either the raw domain
payload or the compact collector-style stream. The older `domain44_*` files are
kept only as compatibility sources for the previously generated proof code; new
registration should use the domain-52 named files.
`replay_verify/` is a lane-local Rust verifier that packs the compact stream
into the same native `.battleplan` container shape used by the recorder host,
opens it with the shared container reader, verifies all entry hashes, and
byte-compares the stored compact stream.
It emits one row per validated `CState` root. Resistance, compliance,
resistance speed, and compliance speed are measured raw fixed-point values.
Rows also carry a controller-country default/fallback occupation-law token when
the `CCountryOccupationStatus` root validates. Per-state occupation-law tokens
can be emitted for rows whose `COccupationLaw*` and `CStateGarrisonData*`
same-offset container pair validates; rows without that exact pair must retain
the per-state-law unavailable badge and fall back to the controller default law.
Occupied rows carry the controller-country garrison template id when
`CCountryOccupationStatus + 0x78` validates as a `CDivisionTemplate*`. Garrison
need/equilibrium/progress remain badged unavailable until a fixture or
UI-command flip classifies those scalar values.

The decoder gate fails closed if a row marks garrison signals unavailable while
carrying nonzero garrison scalar values.

Validated but not yet encoded as detailed payload: `CCountry + 0xFD8` gives the
collaboration-status root. Active collaboration/government-in-exile detail still
requires a scenario fixture with non-empty per-target data.

## Registration-only patch plan

Shared entrypoints were intentionally left untouched. The merge-ready integration
hook, after the orchestrator grants the slot, is:

1. Add `RAW_OCCUPATION_RESISTANCE_COMPLIANCE_DOMAIN` and
   `COMPACT_DOMAIN_OCCUPATION_RESISTANCE_COMPLIANCE = 52` in
   `replay_native/collector/src/lib.rs`.
2. Register `collector_module.rs` as the collector implementation and pass it
   the collector's process-memory reader plus the existing cached
   `CProvince -> CState` topology when available.
3. Register the `CompactOccupationResistanceComplianceTracker` from
   `collector_module.rs`, keyed by `state_id`, so the first frame emits a full
   baseline and later frames emit deltas.
4. Add recorder-host domain id/name mapping for `52` and use the decoder
   checks in `domain52_compact.rs`.
5. Run the compact extraction gate with scheduled domains
   `map_control`, `state_detail`, and `occupation_resistance_compliance_live`;
   acceptance requires ordered compact records, `.battleplan` packing, zero
   skipped hours, and no source/partial/worker failures.

## Latest proof

`replay_native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784`
refreshes the lane against current HEAD `bc0797784`:

- `live_probe.py`: current live `hoi4.exe` build
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  `1081` state rows, `319` occupied rows, `279` garrison-template rows,
  `0` invalid state rows, and `69404` compact stream bytes for domain `52`.
- `collector_module.rs` tests: `9 passed`.
- `domain52_compact.rs` tests: `5 passed`.
- `domain52_gate.rs --stream live_probe/collector_stream.bin`:
  `records_seen=3`, `domain_data_records=1`, `rows=1081`,
  `occupation_law_token_rows=1030`, `occupation_garrison_template_rows=279`,
  no parse errors.
- `replay_verify` tests: `2 passed`; `replay_verify --run-dir
  live_probe --game-id orc52` wrote `native_session.battleplan`, verified all
  entries through `snapshot::Container`, and byte-compared `69404` compact
  stream bytes unchanged.
- `remaining_fields_probe.py`: resistance speed `0x280` validated `85/85`
  delta rows and compliance speed `0x2B0` validated `224/224` delta rows.
  `CStateGarrisonData+0x48` remains rejected as country identity.
- `garrison_scalar_probe.py`: pinned garrison scalar candidates remain rejected
  or fixture-blocked; keep need/equilibrium/progress badged unavailable.
- Collaboration/GIE detail remains fixture-blocked because the current save
  fixture has empty country collaboration payloads and all `8/8`
  `government_in_exile_tag` rows are leader-context rows.

`replay_native/runs/occupation_resistance_compliance_live_870ebf_20260701_v17_replay_verify`
compiled the lane-owned collector module, domain-52 decoder gate, and native
replay verifier:

- `collector_module.rs` tests: `9 passed`.
- `domain52_compact.rs` tests: `5 passed`.
- `replay_verify` tests: `2 passed`.
- live proof: `1081` rows, `306` occupied rows, `269` garrison-template rows,
  `0` invalid rows.
- `domain52_gate.rs --stream collector_stream.bin`: `expected_domain_id=52`,
  `records_seen=3`, `domain_data_records=1`, `rows=1081`, no parse errors.
- `replay_verify --run-dir ... --game-id orc52`: wrote
  `native_session.battleplan`, opened it with `snapshot::Container`, verified
  all entries, and byte-compared `69404` compact stream bytes unchanged at
  `payload/orc52/collector_raw/collector_stream.bin`.

`replay_native/runs/occupation_resistance_compliance_live_870ebf_20260701_v19_semantic_gaps`
adds semantic-gap evidence:

- `garrison_scalar_probe.py`: rejected pinned `CStateGarrisonData` candidates
  listed above; remaining garrison scalar search is fixture-blocked by only
  `18` same-law comparable rows in the current live/save pair.
- `remaining_fields_probe.py`: validated the per-state occupation data/garrison
  joins still hold, rejected `CStateGarrisonData + 0x48` as country identity
  again, and classified current collaboration/GIE save data as fixture-blocked
  because collaboration payloads are empty and `government_in_exile_tag` rows
  are leader scope.
