# Occupation Resistance Compliance Live Lane
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/occupation_resistance_compliance_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


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
- `CCountryCollaborationStatus + 0x28/+0x34` is the bounded relation-pointer
  array/count; `+0x40` is the owning `CCountry*`; `+0x48/+0x54` is the
  reciprocal incoming-owner tag-id array/count.
- `CCountryCollaborationData` vtable `module+0x29A5D08`; `+0x08` is the target
  country tag-db id and `+0x10` is signed i64 fixed-x100000 collaboration
  progress, clamped to `[0, 100000]` by the exact add/set producers.

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
- The three per-state garrison scalars are wired and closed by the wave-3
  dynamic proof plus the 2026-07-19 serializer/deserializer RE recorded below.
  The earlier v23 bounded proof compared `368` save garrison rows, `305` live
  rows, `296` key matches, and `295` same-law matches but rejected those scalar
  candidates because that fixture did not discriminate them. It remains valid
  negative evidence for detail candidates: `3374` direct/root pointer scans
  from `CStateGarrisonData+0x0..0x17F` did not identify `manpower_pool` value,
  `equipment_need` values (`infantry_equipment`, `support_equipment`,
  `motorized_equipment`), or reinforcement request count.
  The current fixture has `340` manpower-pool rows, all single-tag and all
  matching the already emitted controller tag, so it does not prove an
  independent manpower vector. Keep manpower/equipment/suppression/request
  detail badged unavailable until a same-hour or one-variable controlled
  fixture proves a distinct vector/root layout.
- Collaboration is closed independently of government-in-exile state. Exact
  a729 producers, serializer, and UI consumer prove its owner/target/progress
  law; paused D04 exposes the sole live relation `NOR(24) -> GER(1) = 30000`,
  matching save value `0.3` with zero graph anomalies. Historical GIE probes
  below remain useful negative evidence: asset-scope `government_in_exile_tag`
  fields and country GIE legitimacy must not be substituted for collaboration.

## Compact schema

`collector_module.rs` is the lane-owned collector-side Rust module. It contains
the build gate, exact current-build offsets, `OccupationMemory` read trait,
state/country occupation extraction, fail-closed row badges, compact payload
encoding, and a baseline/delta tracker. Domain 52 is registered in the shared
collector and recorder path.

`domain52_compact.rs` defines the lane-local compact payload for domain id `52`.
`domain52_gate.rs` is a lane-local Rust decoder gate for either the raw domain
payload or the compact collector-style stream. The older `domain44_*` files are
kept only as compatibility sources for the previously generated proof code; new
registration should use the domain-52 named files.
Both standalone gates include the canonical dependency-free H4CR header parser
from `../../compact_contract/src/record_header.rs` through a Rust `#[path]`
module, so their direct-`rustc` build remains self-contained. From this lane
directory, build and invoke the current domain-52 gate with:

`live_probe.py` emits canonical one-based contiguous H4CR record sequences
(`1` PERF, `2` DATA, `3` FRAME_END), matching both standalone gates.

```powershell
rustc --edition 2021 domain52_gate.rs -o "$env:TEMP\domain52_gate.exe"
& "$env:TEMP\domain52_gate.exe" --stream "path\to\collector_stream.bin"
```

The compatibility gate uses the same contract and build shape:

```powershell
rustc --edition 2021 domain44_gate.rs -o "$env:TEMP\domain44_gate.exe"
& "$env:TEMP\domain44_gate.exe" --stream "path\to\collector_stream.bin"
```

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
`CCountryOccupationStatus + 0x78` validates as a `CDivisionTemplate*`. The
production Rust collector also emits the exact engine-derived per-state
`army_manpower_need`, `strength_ratio`, and `garrison_required` fields described
in the current closure section below.

The decoder gate fails closed if a row marks garrison signals unavailable while
carrying nonzero garrison scalar values.

The sequential `COL1` trailer carries the complete pointer-free collaboration
set after `GAR1`: owner tag-db id, target tag-db id, and signed i64
fixed-x100000 progress. A complete zero-row set is authoritative only after at
least one country root was observed with zero anomalies. Any malformed root,
relation, reciprocal join, duplicate, or out-of-range scalar marks the entire
extension unavailable. The recorder resolves numeric ids through the captured
country identity map and writes canonical
`countries.<OWNER>.collaboration.collaboration.<TARGET>.value`; unavailable
state becomes `collaboration: null`, not an invented empty set. Country-level
GIE fields remain a separate scope and are not inferred from `COL1`.

## Production registration

Domain 52 is scheduled in the shared collector and decoded by the recorder
host. `CompactOccupationResistanceComplianceTracker` owns its state baseline
and produces the sequential occupation rows, `GAR1`, and `COL1` payload. The
central validation owner must still require ordered compact records,
`.battleplan` packing, zero skipped hours, and no source/partial/worker
failures before promotion; lane subagents do not run independent builds.

## Latest proof

The collaboration/GIE blocker statements in the historical run summaries
below describe pre-D04 probes and are superseded for collaboration by
`extractor/re_evidence/E15_COLLABORATION/E15_EXACT_CLOSEOUT_20260721.md`.
They remain valid only as negative evidence against conflating GIE asset fields
or legitimacy with collaboration progress.

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots`
adds the residual-resolution pass against a fresh autosave snapshot:

- `parse_occupation.py`: save date `1943.5.1.1`, `439` countries with data,
  `31` active occupiers, `163` occupation pairs, and `443` per-state laws.
- `remaining_fields_probe.py`: resistance speed `0x280` validated `92/92`
  delta rows and compliance speed `0x2B0` validated `277/277` delta rows.
  The object graph validated `113` country occupation-data rows and `165`
  state-garrison rows; `CStateGarrisonData+0x48` again had `0`
  controller-country and `0` occupied-country matches. Collaboration/GIE
  verdict is `fixture_has_country_gie_payload_unjoined_to_live_collaboration_status`;
  targeted GIE live graph verdict is
  `rejected_targeted_scalar_hits_no_gie_contract_current_fixture` with `10/10`
  target collaboration-status roots, `1` `CCountryCollaborationData` hit,
  `16` scalar/tag hits, `0` contract-match scopes, and `6` rejected scopes.
- `garrison_scalar_probe.py`: verdict
  `scanned_garrison_candidates_rejected_current_fixture`; `368` save garrison
  rows, `305` live rows, `296` key matches, and `295` same-law matches. Inline
  scalar candidates, direct/root scalar candidates, manpower-pool value,
  equipment-need values, and reinforcement request count all had `0`
  threshold scopes. No compact schema change was made because no new field
  validated.

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v21_stable_autosave_residuals`
adds the stable autosave residual pass:

- `live_probe.py`: current live `hoi4.exe` build
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  `1081` state rows, `381` occupied rows, `345` garrison-template rows,
  `1042` law-token rows, `0` invalid state rows, and a domain-52 compact stream
  verified by `domain52_gate.rs`.
- Shared DLL-only stream
  `extractor/native/runs/occupation_resistance_compliance_live_shared_20260701T165935`
  was replay-packed from an unchanged compact stream under
  `...v21_stable_autosave_residuals/shared_stream_pack`: `81` domain-data
  frames, `80` game hours advanced, `81` unique game-hour samples,
  `0` skipped hours, no source/partial/worker/bridge failure flags, and
  `.battleplan` verification succeeded.
- `remaining_fields_probe.py`: resistance speed `0x280` validated `140/140`
  delta rows and compliance speed `0x2B0` validated `271/271` delta rows.
  The object graph again validates `CStateGarrisonData+0x50` as state join and
  `+0x58` as occupation-data join; `+0x48` again had `0` controller-country and
  `0` occupied-country matches. Collaboration/GIE verdict is
  `fixture_has_country_gie_payload_unjoined_to_live_collaboration_status`;
  targeted live graph verdict is
  `candidate_scalar_hits_require_controlled_fixture` across
  `BEL/ENG/FRA/HOL/LUX/NOR/PHI/POL/YUG` with `9/9` valid
  collaboration-status roots, `1` `CCountryCollaborationData` hit, and `13`
  ambiguous scalar/tag hits. This v21 candidate status is superseded by the
  v23 rejection proof above.
- `garrison_scalar_probe.py`: current stable fixture verdict is
  `fixture_insufficient`; pinned primitive inline scalar candidates in
  `CStateGarrisonData+0x0..0x17F` remain rejected or below threshold, so
  the then-named garrison placeholder slots stayed unavailable. This historical
  result was first superseded by the v23 detail rejection proof and, for the
  three production scalars, by the wave-3/current closure below.

`extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784`
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

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v17_replay_verify`
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

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v19_semantic_gaps`
adds semantic-gap evidence:

- `garrison_scalar_probe.py`: rejected pinned `CStateGarrisonData` candidates
  listed above; remaining garrison scalar search is fixture-blocked by only
  `18` same-law comparable rows in the current live/save pair.
- `remaining_fields_probe.py`: validated the per-state occupation data/garrison
  joins still hold, rejected `CStateGarrisonData + 0x48` as country identity
  again, and classified current collaboration/GIE save data as fixture-blocked
  because collaboration payloads are empty and `government_in_exile_tag` rows
  are leader scope.

## 2026-07-04 garrison scalar production wiring (wave-3 crack)

The three per-state garrison scalars are now emitted in production domain-52
rows (`ROW_FLAG_GARRISON_SCALARS = 1 << 15`, row bytes 32/36/40):

- `garrison_manpower_need` = `CStateGarrisonData+0x88` i32 plain manpower count
  (18/18 discriminating rows, 280/280 all);
- `garrison_strength_ratio_raw` = `+0xF0` native i64 fixed-x100000,
  checked-exact into the domain-52 i32 wire slot (288/288);
- `garrison_required_raw` = `+0xF8` native i64 fixed-x100000,
  checked-exact into the domain-52 i32 wire slot (282/282).

Chain: `CCountryOccupationStatus+0x48` -> occupation-data container ->
`CCountryOccupationData` (vtable RVA `0x296DD00`, `+0x08` status back-ref,
`+0x10` occupied grouping tag, `+0xD8` garrison container) ->
`CStateGarrisonData` (vtable RVA `0x296DCB0`, `+0x50` state join, `+0x58`
occupation-data back-ref). Containers are walked with the bounded 0x300-byte
qword pointer scan the validation probe used; every candidate must pass exact
vtable + back-ref + validated-state-pointer joins from a single blob read.
The earlier v19/v23 rejections of `0x88/0xF0/0xF8` are explicitly OVERTURNED by
the wave-3 D04 proof
(`extractor/documents/archive/WAVE3_CRACKS_870EBF_D04.md`, item 3:
clean diagonal, 303 rows, 0 stale).

Static serializer/deserializer RE on 2026-07-19 then confirmed the native
storage widths and exact save-schema keys. The reader now authenticates the
complete i64 at `+0xF0/+0xF8` and fails closed if either value cannot be
represented exactly by wire v1. See `GARRISON_RESIDUAL_ANALYSIS.md` and
`GARRISON_SCALAR_STATIC_COMPLETION_20260719.json`. Earlier references in this
README to `need/equilibrium/progress` being unavailable describe historical
Python-probe runs and are superseded for the production Rust path by this
section; `garrison_equilibrium` and `garrison_progress` are not engine schema
keys.

Join semantics (live-derived): the attach requires only controller equality
between the walk root and the state row (a 303/303 live invariant). The
`+0x10` occupied tag is the occupation GROUPING country (original/core owner,
e.g. colonial garrisons grouped under a never-independent core tag) and is NOT
required to equal the state owner; garrison rows also legitimately exist for
states whose owner equals their controller. Conflicting duplicate garrison
rows for one state are dropped (both) and counted in the new header
`garrison_anomaly_rows` (byte 48); attached rows are counted in
`garrison_scalar_rows` (byte 44). States without a validated garrison join
keep `ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE` when occupied and stay absent
otherwise (absent != 0).

Paused D04 diagnostic capture (`runs/wire_garrison_native_v3_20260704`,
gitignored): 1081 state rows, 305 garrison-scalar rows, 1 anomaly row (failed
closed), no failure flags, stream decodable, `pack_replay verify` OK.

RESIDUALS (documented, unpaused-fixture-gated):
- One-frame ABA window: the garrison walk runs before the per-state row reads;
  a controller flip A->B->A landing entirely between the two reads could
  attach one-frame-stale scalars. Controller flips are game-day-scale events,
  the window is sub-second, and the lane's own state-row reads carry the same
  single-read guarantee; adversarial review recorded it as the only refutable
  path (no generation counter exists to close it).
- Pre-existing lane-wide property (not introduced by this change): a state row
  whose read fails transiently is skipped (`invalid_state_rows`) without a
  per-row unknown tombstone, so the fold retains the last-known row until the
  next successful read re-emits changed values.
