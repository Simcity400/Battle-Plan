# Crack-queue results — no-user reverses (2026-07-04, exe 870ebf, D04 1944.2.11.18 paused)

Results of the four-item no-user crack queue (`CRACK_QUEUE_NO_USER_2026-07-04.md`),
run by the discovery agent and INDEPENDENTLY re-verified by the orchestrator with
fresh probe code against the same-tick melt oracle
(`runs/wave5_independent_verify/crackqueue_verify_report.json`). Agent evidence:
`runs/crackqueue_c{1,2,3,4}_*/EVIDENCE.md` (gitignored).

**Score: 2 of 4 CRACKED without the user (C1 market owner, C3 diplo queue — both
wire-ready). C2 (speed) + C4 (intel graph) DEFERRED with precise negatives.**

## C1 — Market manager per-country owner chain — CRACKED (independently verified)

Pointer-only, CCountry-rooted, NO oracle needed at runtime:
```
entry   = u64(CCountry + 0xFB8)     # per-country market participation entry
wrapper = u64(entry   + 0x00)       # economy wrapper (= pool - 0x38)
pool    = wrapper + 0x38            # CEquipmentVariantPool (vtable module+0x27330A0)
# decode as already validated: array begin @pool+0x20, packed u64 @pool+0x28
#   {lo32 cap, hi32 live count}, entries stride 16 {u64 variant_ptr, i64 amt/1e5}
```
Fail-closed invariants: `u64(entry+0x40)==CCountry` (back-ref), `u64(pool)==pool
vtable`, `u32(pool-0x18)==owner tag_db_id`.

**Orchestrator independent verification (own code, fresh walk):** 437/437 countries
resolve a pool via +0xFB8; back-ref + owner-db invariants hold 437/437; the derived
pools reproduce `full_pool_to_tag.json` **70/70 EXACT, 0 mismatch**. The "pools have
zero incoming refs" myth is resolved — only the *wrapper* base (pool-0x38) is
pointed-to. This UNBLOCKS the market-stockpile wiring that WAVE5 §1 held.

DISPOSITION: PROMOTE the chain for production per-country market attribution.
Carry-forward caveat (both Codex layers): the structure proves "canonical
country-owned equipment-variant pool"; proving it is specifically the MARKET
stockpile (vs some other equipment pool) at cross-tick would want a mutation delta
test, which the paused/no-control save forbids. NOT required for the DONE criterion
— the market identity is already fixed by the 70/70 EXACT content match of the
derived pool to the serialized `market_stockpile` (globally-unique fingerprints).
**Wiring MUST keep a fail-closed content cross-check** (decoded pool must match the
serialized market_stockpile shape; abort on drift).

## C3 — Incoming diplomatic-action queue — CRACKED (independently verified)

**ROOT CORRECTION: the real container is `CCountry+0xFF8`, NOT `CCountry+0x4E0`.**
The earlier +0x4E0 finding (in WAVE5 "Partials" + the crack-queue brief) was a
neighbor-object coincidence — its "id 150442 at +0x3CC" was a co-located heap
object. Orchestrator independent check: walking +0x4E0 as a queue produces **0**
action-vtable hits across all 437 countries; +0xFF8 reproduces all 7 actions.

```
queue  = u64(CCountry + 0xFF8)       # CIncomingDiplomaticAction queue (PdxArray)
data   = u64(queue + 0x08)           # -> CIncomingDiplomaticAction*[]
size   = u64(queue + 0x10) & 0xFFFFFFFF     # cap = high32; queue+0x20.lo = owner db_id
action = u64(data + i*8)             # vtable rva 0x2A209A0 (.?AVCIncomingDiplomaticAction@@)
  handle @+0x08 {type u32 (=4713 market_access_rights), id u32}
  body   = u64(action + 0x18):  actor@+0x14, orig_actor@+0x18, recipient@+0x1C,
           orig_recipient@+0x20, date@+0x30, last_command_date@+0x48  (the game hours)
```
the game date decode: `days,hour=divmod(hours,24); year,doy=divmod(days,365)`, subtract
month lengths `[31,28,31,...]` (NO leap), display `(year-5000).(m+1).(d+1).(h+1)`.
Confirmed against the current date at `CGameState(module+0x3316E90)+0x468` → 1944.2.11.18.

**Orchestrator independent verification (own code + own melt oracle parse):** the
melt has exactly 5 populated recipients / 7 actions; the +0xFF8 walk reproduces all
**5/5 recipients and 7/7 actions EXACT** (id, actor, date), 0 false positives across
437 countries: CZE{150442 GER 1944.2.3.1, 150608 ITA 1944.2.4.1}, AUS{150443 GER,
150609 ITA}, LIT{113475 MON 1943.3.21.1}, EST{113476 MON}, LAT{113477 MON}.

DISPOSITION: PROMOTE the +0xFF8 queue walk for production. Wire target: D53 sidecar
rows {recipient, action_id, actor, date} (fail-closed: type must ==4713, action
vtable-gated, owner-db guard). Open (not required, flagged for wiring): the `envoy`
field (melt −1) is not pinned to a fixed body offset.

## C2 — Game-speed root — DEFERRED (no paused oracle; cannot mutate)

Pause subsystem fully mapped (is_paused @CInGameIdler+0x6C1; sibling flags
+0x6C2..+0x6C5; toggle code @0x7FF755F66954; command-dispatcher setter
@0x7FF755F5E27F). Speed is genuinely decoupled from pause and cannot be validated on
a paused save (the stored speed value has no melt oracle — it's a UI/runtime setting,
not serialized). Two finish paths, neither guessing:
- (A) disassemble the tick-sink → scheduler backward-slice (higher effort, no user).
- (B) ~10s user fixture B0: tap speed-down once → diff `idler[0..0x1000]` +
  `CGameState[0..0x2000]`; the changed small-int field is the speed root.
Round-2 sharpened this: **`idler+0x66C` is RETIRED** — it's a constant `mov
dword[rdi+0x66C],0x64` (=100) written by idler init and read generically by 12
unrelated fns (a fixed frame budget, NOT speed). The 0.3s gate is a UI debounce, not
the sim scheduler. Mechanism FOUND via RTTI: `CSetGameSpeedCommand` (vt RVA
0x2960?A0-region), `CIncreaseGameSpeedCommand`, `CDecreaseGameSpeedCommand`, and the
define `NGameGAME_SPEED_SECONDS` (helper) — the speed-specific DoExecute is behind the
network-command framework, and no paused oracle exists for the value. Two closes: (A)
deeper no-user disasm — index-slice `GAME_SPEED_SECONDS` by the bounded 0..5 operand,
or trace the command DoExecute's stored field; (B) 10s fixture B0 — tap speed once,
diff **CGameState[0..0x2000]** (now the TOP watch-list, a small u32/u8 clamping 0..5)
+ idler[0x600..0xA50]. Codex: DEFER correct. Remains the cheapest user fixture (B0).

## C4 — Intel network graph — CRACKED (round-2, orchestrator-verified)

The data model was corrected: not `map<owner CCountry, coverage>` but
`CStrategicOperative → vector<CCountryIntelNetwork*>`, 66 networks summed across
operatives. Proven chain (live-validated on paused D04):
```
CStrategicOperative (vt RVA 0x298B448; 440 live, 38 own networks)
  +0x10 ptr = CCountryIntelNetwork*[] data ; +0x18 u32 = size
  CCountryIntelNetwork (vt RVA 0x298AB48; EXACTLY 66 live)
    +0x08 i32 = target CCountryTag db_id (ENG=2, SOV=3, AST=33, GRE=22, …)
    +0x18 ptr -> graph G:  G+0x40 u32 = vertices count ;
                           G+0x38 ptr -> array of 16-byte vertex structs (state_id = elem+0x00 i32)
```
**Orchestrator independent verification (own heap scan + melt):** RTTI confirmed
`.?AVCCountryIntelNetwork@@`; **exactly 66 live instances**; **ENG(tag 2) vertices=14
with state set == the melt 14-set EXACTLY**; **SOV(tag 3) vertices=43**; reachability
(union of all operative vectors) == the 66 objects, 0 missing / 0 extra. Full compare
60/66 exact on (tag + sorted state set); the 6 residual are live-vs-save dynamic-intel
recompute divergence (incl. one tag → live "D04", a released nation, not a decode bug).
Codex: CRACK CONFIRMED. Wire target: a spy-network/intel-map domain (per (owner
operative, target, vertex state set)).

## Net status
- C1 market owner, C3 diplo queue, **C4 intel graph: CRACKED + orchestrator-verified**
  (wire-ready — C1 market attribution, C3 D53 diplo sidecar, C4 spy-network domain).
- C2 speed: DEFERRED. Mechanism found; value has no paused oracle. Close via deeper
  no-user disasm (GAME_SPEED_SECONDS index-slice / command DoExecute), OR the 10s B0
  fixture (cheapest). This is the only crack-queue item still open.
- (Market-specific cross-tick mutation proof for C1 needs game control — not required
  for wiring; the wiring content cross-check stands in for it.)
