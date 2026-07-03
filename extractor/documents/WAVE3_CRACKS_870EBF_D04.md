# Wave-3 reverse-engineering cracks — build 870ebf, paused D04 save (1944.2.11.18)

Six hard items cracked/advanced by a Fable reverse subagent, offline-verified by the
orchestrator against the same-tick save oracles (match counts confirmed real, clean
diagonals, no fabrication). Codex adversarially reviewed each. These are VALIDATED
OFFSETS ready to become lane integration packets — NOT yet wired into production
(like the naval crack before its port). Full run evidence: `runs/wave3_*_870ebf/`
(gitignored). Live re-verification + lane wiring is the remaining step per item.

Country roots used throughout: `CGameState=u64[module+0x3316E90]`; country table
`CGameState+0x310` count `+0x31C`; `CCountry` vtable `module+0x27D0E90`; tag-db-id
echo `CCountry+0x8`; tag string from `u64[country_database]+tag_id*0x20`.

## 1. d47 CAdvisor / CUnitLeaderTrait — CRACKED (promotable)
- `.?AVCAdvisor@@` vtable RVA **0x2990E60** (the historical "blocked" RVA never moved — still valid; 10,576 live instances, ~1005 dead-ptr objects carry no token and are correctly excluded).
- `.?AVCUnitLeaderTrait@@` vtable RVA **0x292F160** (CORRECTED — historical `0x291F3C0` was stale/wrong; 194 live instances).
- `.?AVCCharacter@@` vtable RVA `0x2967E30` (control, validated; 13,493 live).
- Offsets confirmed live-vs-save: CAdvisor `+0x10`->CCharacter, `+0x30` idea_token, `+0x80` slot; CUnitLeaderTrait `+0x10` token; CCharacter `+0xA8`->CUnitLeader, trait vector data `+0xDC8` / size `+0xDD4`.
- Oracle match: advisor idea-token sets 4874/4876 (2 diffs = UTF-8/latin-1 mojibake of the SAME two Chilean names, byte-verified identical); slot 9547/9547 (0 mismatch); per-leader trait multiset **1193/1193 EXACT** keyed by unit_leader_id.
- Wiring note: filter to token-bearing (appointed) CAdvisor objects; normalize name encoding.

## 2. Division -> battleplan-order linkage — CRACKED (primary order) + residual
- **COrderInstance instance_id = `+0x244` u32** (430/430 vs save (army_id,slot)->instance_id; next-best 22/430).
- **`CDivision+0xC8` -> the division's current/primary assigned COrderInstance**: 2035 divisions point at an enumerated army order; 2031/2035 have their `+0x244` id in that division's oracle scheduled-order set (4 = order advanced vs snapshot, 65 null). A further 1067 divisions' `+0xC8` points at an army-group COrderInstance (backref RTTI `.?AVCArmyGroup@@`), correctly outside the per-division army-order oracle.
- RESIDUAL (bounded negative): a division can be scheduled_member of multiple orders (3776 pairs / 2038 divisions); `+0xC8` stores only the primary. Order-body `[0..0x2000]` scan for the scheduled_member vector found 0 exact-set matches over ~80 orders — the member list is not an adjacent-count vector in the order body. Full multi-order membership remains open.

## 3. Garrison inline scalars — CRACKED (promotable)
303 CStateGarrisonData rows re-validated (0 stale). Clean diagonal (1.0 ratio, 0 cross):
- **`+0x88` i32 = army_manpower_need** (18/18 discriminating rows where it != manpower_pool_total; 280/280 all).
- **`+0xF0` i32 fixed-x100000 = strength_ratio** (288/288).
- **`+0xF8` i32 fixed-x100000 = garrison_required** (282/282).
`CStateGarrisonData+0x50/+0x58` state join + `CCountryOccupationStatus+0x78` template stay validated. Hardening: a second save with more divergent manpower rows.

## 4. Market sellable-equipment stockpile — CANDIDATE (container proven; owner chain open)
- Owner container class **`.?AVCEquipmentVariantPool@@`** vtable RVA **0x27330A0**; data vector at **pool+0x20**; entry stride 16 = `{CEquipmentVariant* @+0x0, i32 fixed-x100000 amount @+0x8, pad @+0xC}` (CEquipmentVariant vtable RVA 0x293AFB8, type 70 @+0x08, id @+0x0C).
- Validated: AFG 13/13 ids (12/13 amounts), BRA 14/14; whole-pool enumeration reproduces 22/70 countries' stockpiles exactly (id+amount) at data offset +0x20.
- OPEN: a stable `CCountry`->pool offset chain that generalizes (the BRA BFS path is a non-generalizing artifact) and 511/511 reproduction. `CCountry+0x1230` REJECTED (different vector, empty for AFG which has 13 entries) — earlier rejection stands.

## 5. Country delayed events — CRACKED (event id + owner + scope) + residual
- Record class **`.?AVCDelayedEvent@@`** vtable RVA **0x27D0FB8** (22 live). 17/17 primary oracle records joined via scope tag_db_id `+0x18` + scope.random seed pair `+0x1C/+0x20`.
- **Event id: `CDelayedEvent+0x08` -> CEvent (`.?AVCEvent@@`); `CEvent+0x0C` = combined id (namespace*100000+index): 17/17 EXACT** (GER 42900001, ENG 28200333, USA 36000046, ETH 6200001...). This explains the earlier "id not a raw u32 in CCountry" blocker — the id lives in the CEvent definition one hop away.
- Originator/owner tag_db_id: `CDelayedEvent+0x18` (17/17; second copy `+0xC0`).
- RESIDUAL: the months/days/hours countdown / absolute due-date is NOT a plain scalar in `CDelayedEvent[0..0x200]` (no consistent base across months==0 records) — it sits in CEvent or a date sub-struct (one more hop) or needs the console `ssfv_delayed_probe` fixture for exact due semantics. Owner + id + scope decoded now.

## 6. Air per-wing cumulative scalar state — CRACKED (promotable)
- CAirWing body vtable RVA 0x29641B0, type 69 @+0x08, wing_id `+0x0C` (globally unique). 640 live wings, all matched to same-tick save oracle by wing_id.
- **air_accidents (cumulative per-wing accidental-loss counter) = `CAirWing+0x74` u32**: **640/640 exact** (385 nonzero match; the 255 wings without the field have +0x74==0 zero-control passed); next-best offset 52/385.
- **experience = `CAirWing+0x208` i32 fixed-x100000**: 640/640 exact; next-best 168/640.
- count `+0x6C`, manpower `+0x70` confirmed.
- `missions_done` == 0 for all 640 this save -> fixture-ambiguous (correctly blocked). `last_killer`/`last_loss_cause` populate only during live combat -> needs an unpaused air-war fixture (user), documented not overclaimed.

## Ledger impact (apply at integration, after live re-verification + wiring)
- CRACKED/promotable roots: d47 advisor+trait, garrison scalars, air air_accidents+experience -> move toward ROOT validated / SHARED_INTEGRATION_PENDING once wired.
- Division order linkage: primary-order CRACKED; full multi-order membership stays open (bounded negative recorded).
- Delayed events: id+owner+scope CRACKED; countdown/due-date residual (one hop or ssfv fixture) — stays FIXTURE_BLOCKED/partial for the due field.
- Market stockpile: CANDIDATE (container+format proven; owner chain + full reproduction open).
- Air last_killer/last_cause: FIXTURE_BLOCKED (unpaused air-war combat fixture).
