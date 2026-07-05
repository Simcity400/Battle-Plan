# air_loss_stats_live (domain 61) — integration packet (C1 wiring)

Status: **WIRED-PENDING-VERIFY (data layer landed; production scheduling fixture-gated).**
Domain 61 registered `Reserved` / `schedulable=false` (mirrors domain 58
country_economy_counters). DO NOT COMMIT — left in the working tree for orchestrator review.

## What this packet is
Per-country CUMULATIVE air-combat loss statistics = the melt `history{}` **statistic id 23**,
an 89-field i64 (×100000) aggregate record with a 5-slot yearly ring. This is DISTINCT from the
existing domain 42 (`air_combat_losses_live`), which emits per-WING equipment deltas. No lane
previously emitted the per-country cumulative air-loss statistic (grep of lanes/collector/
compact_contract for CLoopHistory / statistic-23 / registry+0x690 = none). So this **ADDS** a
packet; it does not replace a heap scan.

## Proven root chain (build 870ebf; runs/cracklist_c1_air_stats/EVIDENCE.md, orchestrator-verified)
```
CGameState  = u64[module_base + 0x3316E90]
registry    = u64[CGameState + 0x690]
mgr_array   = u64[registry + 0x30]
country_mgr = u64[mgr_array + country_idx*8]
stat_base   = u64[country_mgr + 0x158]           // 89-field aggregate-history array
loophist    = u64[stat_base + 23*8]              // CLoopHistory* (vt rva 0x29BBE58)
record      = u64[ u64[loophist+0x10] + 0x30 ]
field[i]    = i64[ u64[record] + i*8 ] / 100000
```
Structural guards (fail closed): `u64[loophist] == module+0x29BBE58` AND
`u32[loophist+0x34] == 89`.

## The country_idx binding (Codex-flagged integration risk) — VERIFIED
`country_idx` is the stats registry's internal slot. The extractor indexes CCountry by internal
slot via the country table `country_table = u64[CGameState+0x310]` (indexed by `tag_db_id`,
count at `+0x31C`, CCountry vtable rva 0x27D0E90, `tag_db_id` at `+0x08`). The binding
`country_idx == tag_db_id` is PROVEN (runs/cracklist_c1_air_stats/c1_14_idx_binding.py +
c1_15_binding_semantic.txt):
1. Both arrays are length 440 (== country_count).
2. `country_table[idx]` is self-consistent: `tag_db_id == idx` for 437/440.
3. **Semantic witness:** resolving the stat-bearing `mgr_array[idx]` managers THROUGH the
   extractor's own `country_table[idx]` yields exactly the 1939.12 belligerent set — GER
   (richest, 10 stats), ENG/FRA/POL (WW2, sharing stat keys 7/8/19/38/39), SOV+FIN (Winter
   War, both key 13), ITA (idx 16, the only one carrying air key 23), SPR (civil-war residue),
   Commonwealth NZL/CAN/RAJ. If `mgr_array` were indexed differently the tags would be random,
   not war-consistent. → binding holds.

Runtime guard in `derive_country_row` (src/lib.rs): a row is emitted ONLY when
`country_table[idx]` is a valid CCountry with `tag_db_id == idx`. If not, the row FAILS CLOSED
(no emission — absent, never 0).

## Fail-closed discipline (both directions)
`derive_country_row` returns `None` (row absent) on ANY of: torn read, binding not verified,
CLoopHistory vtable mismatch, `field_count != 89`, absent air stat (country never fought in the
air → absent, NOT zero), or an out-of-range channel value (> 10M planes raw). Six value-pinned
channels only are labeled (f3 our_fighters_lost, f10 our_bombers_lost, f44 enemy_fighters_lost,
f51 enemy_bombers_lost, f58 enemy_cas_lost, f80 enemy_bombers_disrupted); the remaining ~18 of
the 24 channels are an open multi-writer trace and are intentionally NOT emitted (never promote
an unproven label).

## Why Reserved / not scheduled (the honest gate)
The root chain + binding are proven, and the 89/89 byte-exact VALUE match stands from the prior
D04 1944 fixture. But on the CURRENT live fixture (IRE 1939.12) early-game air losses are ~0 and
only ITA even carries key 23, so the per-frame air-loss VALUES cannot be re-value-validated here.
Per fail-closed discipline (never promote unproven data into production), domain 61 stays
`schedulable=false` with `STATUS_VALUE_VALIDATION_FIXTURE_REQUIRED` set on any payload until a
populated fixture (D04 1944, or a live air-combat frame) validates the reads. This mirrors the
domain 58 `PROOF_STATUS_FIXTURE_REQUIRED` precedent.

## Remaining wiring to schedule into production (orchestrator, once fixture-validated)
1. Collector read: iterate `country_idx` 0..country_count; call the equivalent of
   `derive_country_row` (port to the collector's live `Process`/read closures — the pure
   `Mem`-trait logic in src/lib.rs is the reference and is unit-tested with a mock image).
2. Wire the decoder into `recorder_host` `CompactDomainRegistry` (a `CompactDataDecoder` that
   calls `decode_payload` into the frame gate summary) and add 61 to a scheduled profile.
3. Flip the descriptor `schedulable=true` and status to `ValidatedSliceOpenGaps` once the D04
   value match is reproduced live; update `domain_registry.json` + the compact_contract
   `expected` id array (already contains 61) accordingly.
4. Extend `PINNED_CHANNELS` from 6 to the full **24 display channels — now PROVEN** (Codex-gated,
   `runs/cracklist_c1_air_stats/EVIDENCE_FINISH.md`). The air-view details panel
   (`strategicairview.cpp`, builder 0x18738e0..0x1874160) reads per-channel values through a
   static `u32[24]` channel→field table at **RVA 0x29f8940** =
   `{0,1,2,3,10,17,24,39,36,37,38,40, 41,42,43,44,51,58,65,80,77,78,79,81}`, labeled by
   localisation keys `AIRVIEW_DETAILS_CHANNEL_1..24`. Full map (our / enemy, mirror = our+41):
   buildings_bombed f0/f41, troops_bombed f1/f42, ships_bombed_or_sunk f2/f43, fighters_lost
   f3/f44, bombers_lost f10/f51, cas_lost f17/f58, transports_lost f24/f65, bombers_disrupted
   f39/f80, trains_lost f36/f77, trucks_lost f37/f78, railway_guns_lost f38/f79, missiles_lost
   f40/f81. All 24 display fields are OUTSIDE the alias-prone f82..87 range, so reading them is
   clean. Display-scale quirk: fields 0 and 41 divide by 1000, all others by 100000 (store raw;
   apply scale at display). The current crate conservatively emits the 6 originally value-pinned
   channels; expanding to 24 is a mechanical `channels: [i64; 24]` change (ROW_LEN 16+24*8=208).

## Files
- `src/lib.rs` — schema + encode/decode + gate summary + fail-closed `derive_country_row`
  (Mem-trait, unit-tested: 13 tests incl. binding-mismatch / wrong-vtable / field-count /
  absent-stat / out-of-range / torn-read fail-closed cases + encode/decode roundtrip).
- `runs/cracklist_c1_air_stats/c1_14_idx_binding.py`, `c1_15_binding_semantic.txt` — binding proof.
