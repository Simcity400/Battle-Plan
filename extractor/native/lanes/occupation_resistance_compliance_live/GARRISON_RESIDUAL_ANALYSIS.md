# Garrison Residual Analysis

Lane: `occupation_resistance_compliance_live`

Scope: reverse-engineering discovery for the open "Garrisons" master-contract
row. This document does not promote any new production root. It records the
candidate containers reachable from the validated Domain 52 roots and the
current rejection history that the next live run must respect.

## Contract Status

Domain 52 already has production-validated occupation/resistance/compliance
state rows. The master contract still keeps `Garrisons` open because garrison
law/template/manpower/equipment/suppression details are not fully captured.

The validated Domain 52 payload can carry:

- state owner/controller and occupied-state identity;
- resistance/compliance raw values and speed values;
- controller default occupation-law token;
- per-state occupation-law token where same-offset law/garrison joins validate;
- controller garrison template id when `CCountryOccupationStatus+0x78` validates.

It intentionally keeps garrison scalar/detail fields badged unavailable.

## Candidate Containers From Validated Roots

These are reachable from already validated occupation roots and are safe to
probe in the next live session:

- `hoi4.exe+0x3316E90 -> CGameState*`.
- `CGameState+0x310/+0x31C -> CCountry**` and country count.
- `CCountry+0xFD0 -> CCountryOccupationStatus*`.
- `CCountryOccupationStatus+0x78 -> CDivisionTemplate*` garrison template root.
- `CCountryOccupationStatus+0x118 -> COccupationLaw*` default law root.
- `CCountryOccupationStatus+0x48` occupation-data container.
- `CCountryOccupationData+0x08 -> CCountryOccupationStatus*` parent.
- `CCountryOccupationData+0x10 -> occupied/original tag_db_id`.
- `CCountryOccupationData+0x20` state-list container.
- `CCountryOccupationData+0xB0` occupation-law container.
- `CCountryOccupationData+0xD8` garrison-list container.
- `CStateGarrisonData+0x50 -> CState*`.
- `CStateGarrisonData+0x58 -> CCountryOccupationData*`.
- Heap pointers found inside `CStateGarrisonData+0x0..0x17F`, sampled as direct
  child roots only when they validate as readable heap objects.

The same-offset pair between `CCountryOccupationData+0xB0` law entries and
`CCountryOccupationData+0xD8` garrison entries remains the strongest current
per-state law join candidate.

## Save Oracle Surface

The existing `extractor/save_oracle/parse_occupation.py` exposes the occupier
governance layer: default law, priority, garrison template id, occupied country
groups, resistance/compliance, and per-state occupation laws.

The new `extractor/memory_reader/occupation/build_garrison_oracle.py` extends
the save-visible garrison oracle for probe use. It records:

- `state_garrison_data.<state>.strength_ratio`;
- `state_garrison_data.<state>.garrison_required`;
- `state_garrison_data.<state>.army_manpower_need`;
- `manpower_pool` rows with tag/value;
- `equipment_need` token/value rows;
- `garrison_reinforcement_requests` count/status/progress/date.

These save fields are oracle facts only. They can reject or classify live
candidates, but they cannot promote an offset without same-build live evidence.

## Rejection History

Rejected for current build `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
unless a new fixture provides contrary proof:

- `CCountryOccupationStatus+0x80` as garrison template root. The fresh
  autosave/live proof had `99` mismatches. `+0x78` matched `102/102`.
- `CStateGarrisonData+0x48` as controller-country or occupied-country identity.
  The v23 object-graph proof had `0` controller-country matches and `0`
  occupied-country matches while validating garrison rows by `+0x50/+0x58`.
- Pinned inline scalar candidates:
  - `strength_ratio`: `i32/u32@0x10`, `i32/u32@0x78`, `i32/u32@0x7C`,
    `fixed100000_i32/i64@0xF0`;
  - `garrison_required`: `fixed100000_i32/i64@0xF8`;
  - `army_manpower_need`: `i32/u32@0x88`.
- Direct/root scalar candidates from heap pointers in
  `CStateGarrisonData+0x0..0x17F`, sampled for `0x300` bytes per root.
- Detail candidates for manpower-pool value, equipment-need values, and
  reinforcement request count in the same inline/direct-root scan scope.

The v23 proof basis was `368` save garrison rows, `305` live garrison rows,
`296` key matches, and `295` same-law matches. It found `0` threshold scopes for
the inline scalar, direct/root scalar, manpower-pool value, equipment-need
value, or reinforcement-count candidates.

The current fixture also had `340` manpower-pool rows, all single-tag and all
matching the already emitted controller tag. That proves the current save does
not distinguish an independent manpower-vector root.

## Candidates Still Worth Probing

The next live run should not retry rejected offsets as if they were fresh.
Useful work is:

- revalidate the reachable object graph on the current 1943 IRE session;
- compare the save-visible garrison fields against the exact same live
  `CStateGarrisonData` rows;
- inspect whether a new fixture with more extensive occupations produces a
  unique candidate outside the rejected scan scope;
- use the occupied-territories UI as fallback evidence for per-state manpower,
  equipment, and suppression values;
- run a short unpause window to see whether candidate values move with
  suppression/drain dynamics.

## Promotion Bar

No offset should be promoted unless all of these are true:

- live `hoi4.exe` SHA matches the expected `870ebf` hash;
- the candidate is reached from validated Domain 52 roots;
- the row joins by controller tag, occupied tag, state id, and no contradictory
  law mismatch;
- exactly one candidate scope crosses the configured threshold for the target
  field;
- the candidate is not one of the already rejected scopes unless the new proof
  explicitly overturns the rejection;
- for dynamic fields, a before/after or UI transcription fixture supports the
  same semantics.
