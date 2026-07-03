# Naval Combat Chain Reversal 870ebf

Provenance: `extractor/native/runs/naval_combat_fixture_20260703_biscay_v2`
captured live on 2026-07-03 from the Bay of Biscay fixture
(`hoi4.exe` sha256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`).
The authoritative summary in that directory is `CHAIN_FINDINGS.md`; raw byte
evidence comes from `raw_body_dump.json`, `side_dump.hex`, and `side2_dump.hex`.

## Adopted Layout

- `CNavalCombat` vtable RVA: `0x29C6F18`.
- `CNavalCombatant` vtable RVA: `0x29CD298`.
- `CFEXMember` vtable RVA: `0x2A0A228`.
- `CShip` vtable RVA: `0x2941748`.
- `CFleet` vtable RVA: `0x294BF28`.
- `CTaskForce` vtable RVA: `0x294C480`.
- `CNavalCombatSide` has no own RTTI hit on this build; the old side/registry
  split is not retained for the 870ebf lane.

Body `0x1833E4C8420` byte evidence:

- `body+0x00`: `18 6f b6 57 f7 7f 00 00` = module base
  `0x7FF7551A0000` + `0x29C6F18`.
- `body+0x18`: `69 12 00 00 20 93 03 00` = handle type `0x1269`,
  combat id `0x39320`.
- `body+0x28`: `e0 65 c7 3f 83 01 00 00` = attacker
  `0x1833FC765E0`, whose vtable is module+`0x29CD298`.
- `body+0x30`: `a0 6d c7 3f 83 01 00 00` = defender
  `0x1833FC76DA0`, whose vtable is module+`0x29CD298`.
- `body+0x38`: `b0 91 dc 5a 82 01 00 00` = front-like pointer
  `0x1825ADC91B0`. `front+0xA4` is only a location candidate.

Side byte evidence:

- `side_dump.hex +0x00`: `98 d2 b6 57 f7 7f 00 00` =
  module+`0x29CD298`.
- `side_dump.hex +0x18`: `20 84 4c 3e 83 01 00 00` =
  parent `CNavalCombat` body `0x1833E4C8420`.
- `side_dump.hex +0x2A0`:
  `90 be 08 ab 82 01 00 00 10 bf 08 ab 82 01 00 00 10 bf 08 ab 82 01 00 00 07 00 00 00 00 00 00 00 08 00 00 00 00 00 00 00`
  gives begin `0x182AB08BE90`, end `0x182AB08BF10`, cap-end
  `0x182AB08BF10`, allocation length `0x80`, header count/cap `7/8`.
- `side2_dump.hex +0x2A0`:
  `00 ac 08 ab 82 01 00 00 80 ac 08 ab 82 01 00 00 80 ac 08 ab 82 01 00 00 07 00 00 00 00 00 00 00 08 00 00 00 00 00 00 00`
  gives begin `0x182AB08AC00`, end `0x182AB08AC80`, cap-end
  `0x182AB08AC80`, allocation length `0x80`, header count/cap `7/8`.

The lane adopts `side+0x2A0/+0x2A8/+0x2B0` as the group allocation triple and
walks each `0x80` group allocation with the established member vector fields
`group+0x08` and `group+0x14`. The next live fixture must confirm this walk
against real CFEXMember rows before the shared collector is updated.

`front+0xA4` is not validated. The probe reports it only as
`location_candidate_front_0xA4` with `location_candidate_front_0xA4_validated`
set to `false`. Rust compact lifecycle rows must leave `location_id` and
`region_id` absent and set `STATUS_LOCATION_CANDIDATE_UNVALIDATED`.

## Rejected Candidates

Rejected side candidates: 7.

- `side+0x08`: scalar/non-pointer metadata, not a group vector.
- `side+0x20`: country/side data, not CFEXMember groups.
- `side+0x40`: duplicated side-local pointer pattern, not member-bearing.
- `side+0x68`: country tag vector; counts do not describe combat members.
- `side+0xC8`: stale pre-870ebf group vector; the Biscay walk produced no
  validated CFEXMember rows.
- `side+0xF8`: scalar/counter region; not a pointer triple.
- `side+0x290`: nearby scalar/float positioning data, not the group allocation.

Rejected body side mirrors: `body+0x98/+0xA0`, `body+0x4E8/+0x4F0`, and
`body+0x558/+0x560` duplicate or secondary side slots in the raw body dump.
They are not adopted as the canonical active attacker/defender chain because
`body+0x28/+0x30` are the first pair with direct CNavalCombatant vtable evidence
and parent backrefs.

## Collector Follow-Up

`extractor/native/collector/src/lib.rs` still carries the old naval chain:
old RVAs `0x29B6660/0x29BC908/0x29B6588/0x29F9608` and the old offsets. On
870ebf that fails closed as `fixture_absent` through the body vtable mismatch.
Updating the shared collector is a separate serial integration step by the
orchestrator after live validation. That integration must port this lane's exact
870ebf constants and gates, including the unvalidated-location behavior and the
required CShip join failure quarantine.
