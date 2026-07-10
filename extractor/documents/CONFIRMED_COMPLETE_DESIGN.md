# Confirmed-complete semantics — design of record (2026-07-10)

Origin: the hour-latch live soak booked `capture_failed == attempts` on every frame of a
production run and the recorder gate failed every production ShortSpeed5 run. First
misdiagnosis (FRAME_END mask, ec9cf0b15) was refuted and reverted. Second misdiagnosis
("save lacks bridge prerequisites; wire `--division-body-bridge` into presets or add
expected-absence") was **refuted by the Codex design analysis of 2026-07-10** (thread
`019f4b8f-c5c5-7a91-894b-69e3e572d7b1`, reply preserved in the orchestrator session log;
key citations re-verified by the orchestrator against `compact_contract/src/lib.rs` and the
latchsoak2 manifest before adoption).

## The corrected diagnosis (verified)

Status flags: `BRIDGE_UNAVAILABLE = 1<<7 (128)`, `SOURCE_UNAVAILABLE = 1<<8 (256)`,
`PARTIAL_DATA = 1<<9 (512)` (`compact_contract/src/lib.rs`). Tonight's "failing" domains:

| domain | status_or | decode | manifest reality (latchsoak2) |
|---|---|---|---|
| d43 market_trade_live | 256 | SOURCE only | **188,389 contract rows captured** — data flows |
| d69 tech_sharing_groups_live | 512 | PARTIAL only | **27,715 groups + 93,452 members** — data flows |
| d47 characters_leaders_cabinet_live | 768 | SOURCE\|PARTIAL | adapter processes 39.5M character rows, **emits 0 data rows**, `domain47_frame_parse_errors = 1560` — **a real reader/adapter defect, live data loss** |

`bridge_unavailable_seen: false` in the same manifest. **No missing-bridge story survives.**
Also: most compact domains have native rooted discovery and never needed CLI bridges;
`recorder_host` only carries division + special-projects bridges at all (the rest are Python
dev-harness artifacts: per-process heap-pointer snapshots, unfit for production reuse).

## Adopted design — option (c), staged

1. **Fix the real defects first (no semantics change buys anything before this):**
   - **d47**: find why the adapter's built rows never emit (1,560/3,471 frames parse-error;
     `data_rows_total = 0` while `domain47_adapter_character_rows = 39.5M`). This is the
     top-priority extractor defect as of 2026-07-10.
   - **d43/d69**: identify which sub-source sets SOURCE/PARTIAL every frame despite rows
     flowing (fail-closed sub-source or real gap? attribute before excusing).
   - **d17 war-relations segment**: without `war_relation_bridge`, `at_war`/casualties are
     silently absent and NOT flagged — silent-coverage debt; needs an anchored native reader
     or explicit segment accounting.
2. **Typed source disposition (compact_contract, versioned):** per scheduled segment per hour
   exactly one of `Captured | MeasuredEmpty | ExpectedAbsent | Failed`, with a declaration id
   for ExpectedAbsent. NEVER encoded by clearing/masking the existing failure bits.
   `MeasuredEmpty` = complete enumeration proved zero objects; `ExpectedAbsent` = positively
   proven session-static prerequisite absence, declared BEFORE capture. Missing CLI flags,
   empty caches, timeouts, failed reads NEVER qualify.
3. **Automatic prerequisite plan:** bounded anchored discovery runs before the first
   hour-latch observation and produces declarations (process identity + save epoch pinned;
   reload/identity change invalidates). Turnkey in production presets; manual plan files are
   debug-only. If data appears for a declared-absent segment: capture it AND fail the gate
   (declaration mismatch) — real data wins.
4. **Hour-latch completion becomes tri-state** `Full | WithExpectedAbsence | Failed`; the
   zero-silent-gap invariant `observed == attempts + coalesced + in_flight` is untouched;
   manifest partitions `frames_complete_full + frames_complete_with_expected_absence +
   capture_failed == capture_attempts`.
5. **Recorder gate:** validated expected absence = accounted visible condition, not a
   problem; anything unproven stays a failure with recovery-replay behavior unchanged.

## Forbidden (the silent-loss traps, verbatim from the adversarial analysis)

- No expected-absence classification for d43/d47/d69 (they carry real data / a real bug).
- Never infer absence from a missing flag, empty bridge list, zero rows, timeout, or
  incomplete scan; never clear SOURCE/BRIDGE/PARTIAL bits; never weaken FRAME_END/aggregate
  completeness (the reverted-defect class); never excuse a whole domain for one absent
  segment; no startup pointer snapshot as completeness proof for dynamic entities; no bridge
  reuse across process/save epochs; no delta-baseline advance or tombstones from expected
  absence; no expected-absence enablement after a mid-run failure; no user-provided
  production absence lists.
