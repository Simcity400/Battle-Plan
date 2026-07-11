# Overnight queue + next-boot test plan (2026-07-10 → 11, user asleep, full autonomy)

User directive: queue ALL remaining optimization work; do maximum offline; have every live
test staged for the next game boot. Game is up but will self-pause at some point.

## Running unattended right now

1. **default-8 validation capture** — 600 s production window (`runs/prod_default8_20260710`)
   validating the retuned `PREFETCH_DIST_RELATION_DEFAULT = 8` build in production
   (expect d53 read_time ≈ 51–53k µs and `prefetch_dist_relation_applied: 8` from the
   default, no override).
2. **pause-watcher** (detached PowerShell, `runs/pause_watcher_20260710.log`) — polls every
   10 min; when the game self-pauses and no capture is running, runs the d23 locality probe
   (pause-only experiment; self-discards on hour movement). Stops after one VALID probe or
   game exit.
3. **byte attribution study** (`scripts/byte_attribution.py` on the latest deep-war window)
   — post-compression per-domain byte shares + container-level bracket (levels 3/13/19) +
   domain-grouped-vs-interleaved comparison. OUTPUT DECIDES the byte-lever priority below.

## Offline implementation queue (no game needed; each = implement → tests → Codex Mode-A gate)

Ordered by value; sandbox for Mode C is broken so orchestrator implements, Codex gates.

| # | Item | Size | Notes |
|---|---|---|---|
| O1 | ✅ MERGED (1c05f4477 + fix round): d69 per-reason telemetry; gate found MEDIUM mask-sum defect, fixed (OR-aggregation) | S | First live frame now attributes the d69 failure |
| O2 | ✅ MERGED (11b0cc9c3 + fix round b0897b457, re-stamp gate SOUND/no findings): d43 cold-vs-known-empty; my first fix had 3 silent-loss findings (degraded/stale-epoch Some(empty) emitted clean) — fixed via market_trade_warm_complete_empty() + reap-restart snapshot reset (protects ALL sweep consumers) | S | LESSON: snapshot completeness = !degraded + current-epoch, always |
| O3 | ⚙️ REFRESH SHIPPED + 2 gate fix rounds (6d1ff969c → 886e8b923 → 8e10e70ff; final net gate pending — CLI startup flake, retrying): refresh works LIVE (o123: 163 rebuilds/944 frames, 22k roots) but d47 rows STILL 0 — churn × whole-frame fail-closed. Dead-vs-torn CLEAN emission tried (df835b45e), gate-REFUTED as UNSOUND, REVERTED (28f2043e1). **Row restorer of record = PARTIAL-upsert fold contract** (design in CONFIRMED_COMPLETE_DESIGN.md '## d47 status after O3'; d48 precedent). Sweep-completeness class bugs fixed on the way (skipped_ranges/coverage_complete gating d47 AND d43; enumeration gaps incl. pre-existing adjacent-region first-page skip). Remaining unknowns: ~3.6 parse errors/frame (strings via Debug capture d69stale_d47diag), prewarm/walk validator mismatch | L | d69 stale-slot fix rode along (8e10e70ff): stable non-group slot = beyond active extent, clean emit; expect 3 rows CLEAN + stale mask 504 in the Debug capture |
| O4 | **RTTI auto-re-derivation tool** (U1): scan hoi4.exe RTTI → regenerate all `*_VTABLE_RVA` in build_profile; validate by reproducing the CURRENT profile byte-for-byte from the current exe (self-proving offline) | M | Patch-day preparedness; F:\SteamLibrary…\the game\hoi4.exe |
| O5 | **Modset-in-manifest** (M2): read enabled mods + DLCs (dlc_load.json, .mod descriptors, checksums) → control file + manifest | M | Unblocks modset-bound evidence; M1 catalog resolution follows |
| O6 | **Byte levers** — implement per attribution-study ranking: likely container long-matching first (config-only), then domain-grouped ordering, then columnar+varint d53 record layout (largest, lossless, fold-identity gated) | M–XL | DO NOT start columnar before the study output confirms d53's compressed share |
| O7 | d17 war-relations segment accounting or anchored native reader | M | zero-silent-gap debt |
| O8 | SourceDisposition + prerequisite plan (design §2–5) | XL | After O1–O3 land |

## Next-boot live test list (each ≤15 min; run in this order)

STATUS 2026-07-11 morning: game up; items 1–3 combined into ONE 600s capture
(`o123_validate_20260711`, ShortSpeed5, fresh O1+O2+O3 build) — IN FLIGHT. The overnight
d69/d43 capture never launched (no run dir; game had stopped advancing).

1. **d69 attribution capture** (after O1 merges): one short window → per-reason counters name
   the failing slot → unlocks the d69 topology fix (restores a whole domain to replays).
2. **d47 validation** (after O3): deep-war window; expect `data_rows_total > 0` every hour,
   work-implies-rows gate green; check sweep-generation telemetry.
3. **d43 validation** (after O2): first frame clean (no SOURCE at elapsed-0), gates green.
4. **Candidates prefetch palindrome**: `BATTLE_PLAN_PREFETCH_DIST_CANDIDATES` ∈ [32,16,0,0,16,32]
   via `-PrefetchDistCandidates` (plumbing already live) — same method as the relation sweep.
5. **Pace-exogeneity experiment**: fixed save+speed, domain subsets (~40/~80/~151 ms sweep
   cost), watch the hour-interval distribution — decides whether frame-time cuts speed up
   the game itself.
6. **Hour-latch ON default decision**: latch proven cost-free; one green validation run with
   `--hour-latch` under the new build → recommend flipping the preset default (user call).
7. If pause-watcher missed it: d23 locality probe on the paused game before unpausing.
8. (Different rig, whenever available): 6-core latch portability run.

## Standing constraints (unchanged)

≤15 min turnaround; large files only for debug; zero-silent-gap accounting; never mask
failure bits; one Codex fix round per review then orchestrator fixes; resumability law on
every Codex call; `replay_interface/` untouched; `documents/CLAUDE.md` +
`documents/archive/CODEX_PROTOCOL_legacy.md` carry another session's edits — never stage.

## Byte attribution study RESULTS (2026-07-11 00:0x, w6_d16 window: 9.65GB decoded / 59MB stored = 163x)

- **d53 = 86.6% of compressed bytes** (94.1% raw; per-domain zstd ratio 31.9x vs 4.5-9.6x for
  d20/d44/d24). The columnar+varint d53 record layout is THE byte lever; nothing else is worth
  more than ~5% combined.
- **METHODOLOGY CORRECTION (own study, caught on read-back):** the whole-stream brackets used
  zstd DEFAULT window; production streams use window_log 27 (128MiB) — stored 59MB vs my
  level-19-default-window 278MB means **window/long-range matching dominates level and is
  ALREADY SHIPPED**. Therefore: (a) container-level tuning is near-exhausted; (b) the
  "domain-grouped beats interleaved by 21%" comparison is a default-window artifact — assume
  it EVAPORATES at window 27 unless re-measured with production parameters.
- **O6 next step (the only big one):** offline columnar/varint re-encode of d53 payloads,
  measured against the 59MB baseline AT PRODUCTION SETTINGS (level 3, window_log 27),
  fold-identity gated. Do not bother with domain grouping or container levels first.

## O6 CLOSED 2026-07-11: columnar d53 = MEASURED NULL (negative). Byte program COMPLETE.

`scripts/d53_columnar_probe.py` on w6_d16 (9.65GB decoded, 1131 d53 DATA payloads = 9.09GB,
schema-exact, 0 bails), IDENTICAL compressor both sides (zstd level 3, window_log 27, LDM):
- baseline re-encode 53.68MB vs COLUMNAR re-encode 55.21MB = **-2.9% (columnar is WORSE)**.
- Mechanism: window-27 long-range matching already models cross-frame row-major repetition
  (near-identical hourly world = long identical byte runs); byte-plane transposition fragments
  those matches per column. Varint/delta layouts fragment alignment strictly more => dominated
  without needing a separate run. The transform was invertible by construction, so this is an
  apples-to-apples lossless comparison.
- Byte-lever end state: window-27 LDM SHIPPED (dominant), container levels near-exhausted,
  domain-grouping = default-window artifact, d53 delta-emit CPU-negative + byte-redundant,
  columnar NEGATIVE. **No remaining lossless layout lever worth building.** Future size wins
  can only come from semantic scope decisions (out of bounds under zero-data-loss).

## O4 SHIPPED 2026-07-11 (SELF-PROVEN): RTTI auto-re-derivation tool

`scripts/rtti_rederive.py` + `scripts/rtti_vtable_map.json` (the durable manifest,
auto-derived FROM the current exe: constant -> mangled RTTI class + COL offset, which
disambiguates multiple-inheritance vtables like CCountryOccupationStatus @0/@24).
- `--derive-map`: 51/51 build_profile `*_VTABLE_RVA` constants resolved to RTTI identities
  (bonus corrections of record: DOMAIN54_ARMY is COrdersGroup; DOMAIN57_CLOCK is CInGameIdler).
- `--verify` (the self-proof): re-derives all 51 from class names alone against the CURRENT
  exe — **51/51 byte-for-byte exact, 0 unresolved**.
- `--rederive <new hoi4.exe>` (patch day): emits the updated constants block, flags CHANGED
  values, FAIL-CLOSED lists unresolved/ambiguous classes rather than guessing.
SCOPE: covers every vtable RVA. NOT covered (stays validator-oracled per the U-series plan):
field offsets, function-sentinel offsets (daily-update etc.), static data RVAs (gs pointer,
version banner). Those still need the post-patch shadow/scan re-validation pass (d53
offsets-vs-scan referee is MANDATORY after any exe update — already standing law).
