# Wave-5 cracks — market / projects / operations (2026-07-04, exe 870ebf, D04 1944.2.11.18 paused)

The final three not-user-blocked deep-reverse items, cracked by the discovery
agent and INDEPENDENTLY re-verified by the orchestrator with fresh probe code
(`runs/wave5_independent_verify/verify_report.json`), then adversarially gated
(agent's own Codex reviews in `runs/attemptcrack_*/codex_review_v2.txt` + the
orchestrator's own Codex gate `/tmp/cdx_wave5gate_latest.txt`, both Mode A
xhigh, exit 0, non-empty). Agent evidence: `runs/attemptcrack_market_close/`,
`runs/attemptcrack_projects/`, `runs/attemptcrack_operations_intel/`
(EVIDENCE_v2.md each; runs/ is gitignored — key numbers reproduced here).

## 1. Market stockpile — layout VALIDATED 70/70; production wiring HELD on owner chain

Container (all vs module base):
- `CEquipmentVariantPool` vtable `0x27330A0`; entry array begin @pool`+0x20`;
  packed u64 @pool`+0x28` = {lo32 capacity, hi32 live count}.
- Entries stride 16 = `{u64 variant_ptr, i64 amount_fx100000}` — **amount is
  i64, NOT i32** (i32 truncated every stockpile > ~21474 units; USA id 2348 =
  5,017,500,000 raw = 50175.0 units proves it).
- Valid entry: variant_ptr → vtable `0x293AFB8` (`CEquipmentVariant`), equip id
  u32 @variant`+0x0C`. Non-type-70 slots beyond the live range are garbage
  (doubles/stray pointers) — verified directly.

Independent verification (orchestrator, fresh code): all 70 oracle countries
validated against the same-tick melt market oracle. 55 exact multiset matches;
15 subset matches where live carries EXTRA type-70 entries the save omits —
every extra is amount==0 or sub-unit (<100000 raw). All 70 oracle amounts are
exact integer-unit multiples of 100000 ⇒ the save serializes whole units and
drops round-to-zero entries. Containment is UNIQUE across all 70 pools for
every country. Pools can hold DUPLICATE equip ids (CAN id 7311 ×4).

Count rule (sharpened past the agent's `max(lo,hi)`): 69/70 pools have
`hi32 ≤ lo32` with the first hi32 entries all valid; the one exception (CAN,
lo=19 hi=15, 19 type-70-passing slots) has tail entries beyond hi32 that are
exactly the sub-unit id-7311 duplicates the save omits ⇒ tail-beyond-hi32 is
stale/sub-serialization data. **Production rule: read hi32 entries only.**

DISPOSITION (per Codex gate): the pool/entry LAYOUT is promoted; per-country
attribution is NOT wireable yet — content-based matching needs a same-tick
oracle, which production doesn't have. Wiring is HELD on the remaining
disassembly item: the market manager's per-country pool index (next step:
disassemble the 23 code xrefs to pool vtable `0x27330A0`). Fail-closed
invariants when wired: variant-vtable filter, hi32≤lo32≤4096 sanity, multiset
(not set) validation, abort on ambiguity.

## 2. Special projects lifecycle — CRACKED + independently verified 21364/21364

Chain (RTTI-rediscovered): `CCountry+0xFA8` → `CProgramStatus` (vt `0x295AC88`)
→ `+0x18` `CProjectPool` (vt `0x29B0238`) → `+0x10` inline `CProject` array,
stride `0x1C0`, walk until element vtable ≠ `0x296C270`; per project: local id
u32 @`+0x0C`, `start` u8 @`+0x1BC`, `completed` u8 @`+0x1BD`.

Independent verification (orchestrator, fresh walk): 436 countries with rows /
21364 projects; per-record diagonal vs melt **21364/21364, zero mismatches**.
Oracle faithfulness proven directly against the melt text: the adjacent
`start=(yes|no) completed=(yes|no)` pair occurs exactly 21511 times (= oracle
rows) with 75 start=yes / 68 completed=yes (= oracle totals).

The 147-row live-vs-oracle gap is exactly **D01/D02/D03** (3×49): dynamic
observer countries excluded by the country-roots resolver — a systemic bound
shared by every country-rooted domain, not a projects defect. Rows for them
are absent (fail-closed), never zero-filled. `phase_progress` confirmed
serialize-time computed (no live offset) — replay derives it if needed.

DISPOSITION (per Codex gate): promote id/start/completed + the owner-join
chain for 870ebf, hash-gated, with production invariants: walk-until-vtable-
mismatch capped at the global project-definition count, require the expected
local-id set, abort on mismatch; read the two flags as u8 bytes with boolean-
domain validation (no independent-C++-bool claim without mutation evidence).
Wiring target: extend domain 41 `projects_live` with per-project
started/completed + the (tag, local_id) owner join.

## 3. Running intel operations — target CRACKED 8/8; owner offset REJECTED

`COperationInstance` vtable `0x29A76C0` (distinct from `COperation` definition
`0x2928C98`). Orchestrator's own heap scan found exactly 8 instances (the
save's full running-op population), all with handle u64 @`+0x08` =
{id=hi32, type=lo32=4713} and def ptr @`+0x48` → COperation (8/8 vtable-ok).

- **target/associated country @`+0x50` → CCountry: 8/8 exact** vs melt
  operative-join (129014→SOV, 128904→DEN, 137624→ITA, 138201→POR, 144513→BRA,
  140988→ENG, 149591→CHL, 149158→PER); all 8 ids present in melt.
- **owner @`+0x10`: REJECTED.** The discovery agent's claim (u32 tag_db_id,
  GER for 5, 0 for 3) did NOT reproduce: independent read shows low byte = 1
  on all 8 with varying/polluted high bytes (1,1,1,1,1, 0x3C01, 0x593B0001,
  0x50FF0001) — not a clean u32, field width/semantics unpinned. Not promoted,
  not even as a typed observation.

DISPOSITION (per Codex gate): promote `+0x50` as "observed associated-country
pointer" with PROVISIONAL target semantics — all 8 cases coincide with the
operative's location, so target-vs-location is undistinguished until a
contrast fixture exists (an op whose operative location differs from its
target). Usable operationally as target_country with that badge; not for
analytics claims. Wiring target: domain 53 sidecar rows
{instance_id, def_ref, associated_country} badged provisional.

## Partials banked — ALL SUPERSEDED by the crack queue (see `CRACK_QUEUE_RESULTS_2026-07-04.md`)

- **Diplomatic-action queue: CRACKED.** ⚠️ The `CCountry+0x4E0` root recorded here
  was WRONG (a neighbor-object coincidence; independently confirmed to yield 0
  action hits). The real container is **`CCountry+0xFF8`** — CRACKED 5/5 recipients
  / 7/7 actions exact (id/actor/date). Full chain in the results doc §C3.
- **Market manager owner chain: CRACKED.** `CCountry+0xFB8 → wrapper → pool`
  reproduces the 70/70 pool→tag map by pointer (independently verified). Unblocks
  §1 market wiring. Results doc §C1.
- **Intel network graph / intel-map %**: DEFERRED but no longer "not found" — the
  container type family (`CCountryIntelNetwork` etc.) is identified; one bounded
  serializer-string xref pass remains. Results doc §C4.

## Verification provenance

- Independent probe (fresh code, shared primitives only): projects walk +
  diagonal, melt pair-count faithfulness check, market re-decode +
  re-attribution from scratch, operations heap scan + target resolution.
- Scope check: working tree clean of source edits (discovery was
  runs/-only). No cargo changes ⇒ no test run needed for this batch.
- Both Codex layers agree with the final dispositions; the orchestrator gate
  additionally forced: multiset validation, hi32 count rule, D01–D03 gap
  explanation, owner@+0x10 full rejection, provisional-target badge.
