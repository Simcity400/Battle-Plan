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

## d47 root cause (Codex root-cause investigation 2026-07-10, orchestrator-verified)

Thread `019f4ba7-7522-7e62-be95-6e75ef0da5dc`. A complete streaming H4CR scan of the
latchsoak2 stream partitions the 3,471 d47 records exactly (reconciles to the manifest's
1,560 parse messages = 104×15 and 6,458 rejected roots = 250×1 + 3,104×2):

| first suppression path | records | share |
|---|---:|---:|
| cold/rootless SOURCE\|PARTIAL | 13 | 0.37% |
| self-decoder failure (15 msgs/record) | 104 | 3.00% |
| **adapter rejection: 1–2 dead cached characters** | **3,354** | **96.63%** |

Three composing defects (all verified against source):
1. **Stale root cache never refreshes (96.6%, dominant).** `prewarm_...roots`
   (collector/src/lib.rs:17867) returns early forever once `has_stable_roots()`; it never
   re-checks the shared heap-sweep generation the roots came from (:17925). One cached
   character dying → `has_rejections()` (adapter.rs:126, `rejected_roots != 0`) → the
   scheduler discards the whole populated frame for a rowless PARTIAL (scheduled.rs:96-102).
   Two deaths this run (hours 60824244, 60824352) then failed every subsequent frame. New
   characters are also never added after the first prewarm.
2. **d47 consumes a CHANGED-ONLY politics delta as if it were a full snapshot** (latent).
   `domain47_country_politics_roots` (collector/src/lib.rs:18507) iterates only emitted
   politics delta rows; empty→BRIDGE_UNAVAILABLE, but a nonempty SUBSET is accepted as
   complete (adapter.rs:301) while the fold clears d47 country fields before applying
   (world_fold.rs:7675) — a full replacement. A changed-subset hour could silently replace
   the whole active-ruler set. (Not the cause of THIS run's zeros, but real.)
3. **read_time_us=0 is a measurement lie** (finding 4): `empty_native_scheduled_read`
   hardcodes `read_time_us: 0` (scheduled.rs:136), and the wrapper rebuilds status from the
   OUTER status not `frame.status_flags` (collector/src/lib.rs:18371), so `bridge_unavailable_seen`
   can read false while the country vector was actually empty (90/104 decoder-fail frames had
   0 country roots). `domain47_frame_parse_errors` counts MESSAGES not frames (104 frames × 15).

History: d47 HAS emitted in production (ShortSpeed5 run: 2,360,421 rows, 0 rejected roots);
12/118 retained manifests show nonzero emission. The all-or-nothing scheduler dates to
`8dd85e295`; no later regression bit. So this is a pre-existing latent fragility, tripped
whenever a cached character dies.

## Repair plan (STAGED as Codex Mode-C job — full-replacement critical path, needs the sandbox)

Do NOT emit surviving rows on a rejected frame (would convert an unreadable root into an
unproven deletion — the fold is full-replacement). The repair is to RECOVER:
1. **Generation-aware root cache + rejection recovery.** Add `sweep_generation: Option<u64>`
   to `CompactDomain47PrewarmCache` (collector/src/lib.rs:4248), set on adoption. Rebuild
   roots when the shared sweep generation advances (drops dead characters, adds new ones —
   cheap reassembly from the already-published snapshot, not a cold scan). On any adapter
   rejection, invalidate the cached roots so the next prewarm revalidates against the latest
   snapshot. Keep the failed hour rowless+PARTIAL (lossless). OPEN QUESTION the implementer
   must resolve live: does the shared sweep generation actually advance mid-session? If it is
   effectively one-shot, generation-awareness is inert and the rejection-triggered
   invalidation is the load-bearing half — verify before claiming the fix.
2. **Full country topology, not politics delta** (defect 2): pass the complete validated
   country-identity roots into d47 (collector/src/lib.rs:15353) and replace the delta walk
   at :18507 with a full-root builder. Verify appointed-advisor derivation is truly disabled
   (adapter.rs:256/276) so active-idea deltas are unneeded.
3. **Preserve failure evidence** (defect 3): on payload replacement (collector/src/lib.rs:18379)
   copy the original `read_time_us`, OR in `frame.status_flags`, and emit normalized
   parse-error-kind counts/hashes so ONE live frame identifies the exact 15-message family
   (do not guess the adapter/decoder relaxation before that evidence exists).
4. **DONE (this session, 55287bd6a):** gate-fatal work-implies-rows backstop.

Regression tests to require: `domain47_stale_character_fails_one_frame_then_rebuilds_and_recovers`,
`domain47_new_sweep_generation_adds_new_character_roots`,
`domain47_uses_full_country_roots_when_politics_delta_is_empty`,
`domain47_failure_substitution_preserves_read_time_status_and_error_kind`.

## d43/d69 attribution (Codex 2026-07-10 late, resumed thread 019f4bd7…; citations orchestrator-verified)

**d43 market_trade_live — NOT chronic (premise refuted).** SOURCE_UNAVAILABLE on exactly
1/3,471 frames: the elapsed-0 cold frame (heap sweep deliberately waits 250 ms before its first
pass; lib.rs:52529-52544), where purchase-contract discovery was genuinely unavailable while the
MTRF resource-flow trailer already carried 55 rows. Because status is a domain-wide bit and the
session OR paints everything, one cold frame flagged the whole run. LATENT BUG found: the reader
conflates cold `None` with warm known-complete-empty `Some([])` — both hit the unconditional
`contract_ptrs.is_empty()` SOURCE flag (lib.rs:28264-28270). Fixes (small): (1) only `None` sets
SOURCE; (2) production preflight: warm the d43 needle before the first latch frame (timeout =
fail closed, never clear the bit); (3) per-segment disposition so MTRF folds while contracts
stay explicitly failed on a cold frame.

**d69 tech_sharing_groups_live — chronic AND not tolerable; a REAL coverage hole TODAY.**
PARTIAL on 3,471/3,471 frames (8 groups on 3,418, 7 on 53); the fold rejects every partial d69
frame and no clean baseline ever folds ⇒ **tech-sharing contributes NOTHING to any replay right
now** despite 27.7k groups captured. Cause unattributable from the stream: seven distinct
failure reasons collapse into ONE boolean (`torn_or_skipped_nonnull_slot`, lib.rs:36493-36584)
with zero reason telemetry. Strong inference (not proven): the hardcoded NINE-slot topology
(build_profile.rs:403-419, provenance = a single nine-group fixture) has a stale tail — a
retained capture shows exactly 8 groups/frame with status 0 (live_validation_20260709_220224
stage0), consistent with a dynamic active extent. Fixes: (1) per-slot/per-reason counters FIRST
(one live frame then attributes it); (2) RE the manager's authoritative active extent and
iterate only that; (3) keep PARTIAL for failures inside the proven extent; (4) live shadow +
same-tick save oracle through group create/destroy lifecycles. DO NOT add d69 to the
tolerances — the emitted set may genuinely be short and the fold would full-replace with it.

**Tolerance machinery behaves as designed** (neither domain listed; tolerances grant only
PARTIAL and only when no other bit is set). Also refuted: these flags alone cause recovery
replays — `skipped_game_hours_detected` independently fails the artifact gate.

## Forbidden (the silent-loss traps, verbatim from the adversarial analysis)

- No expected-absence classification for d43/d47/d69 (they carry real data / a real bug).
- Never infer absence from a missing flag, empty bridge list, zero rows, timeout, or
  incomplete scan; never clear SOURCE/BRIDGE/PARTIAL bits; never weaken FRAME_END/aggregate
  completeness (the reverted-defect class); never excuse a whole domain for one absent
  segment; no startup pointer snapshot as completeness proof for dynamic entities; no bridge
  reuse across process/save epochs; no delta-baseline advance or tombstones from expected
  absence; no expected-absence enablement after a mid-run failure; no user-provided
  production absence lists.

## d47 status after O3 (2026-07-11) — refresh SHIPPED, rows still blocked; PARTIAL-upsert is the row restorer

Shipped + gate-hardened (commits 6d1ff969c, 886e8b923, 8e10e70ff): generation-aware root
refresh, rejection-triggered invalidation, fail-closed refresh holds with bounded-staleness
escalation, sweep-coverage accounting (`SweepSnapshot.skipped_ranges` / `coverage_complete()`)
gating d47 adoption AND d43 rebuilds/known-empty, enumeration-gap fixes in the sweep pass
itself (adjacent-region first-page skip, chunk-failure page-skip, aligned short-read resume).
Live validation (o123_validate, 944 frames): refresh works (163 rebuilds, ~22k stable roots,
new characters picked up) — but `data_rows_total` is still 0.

**Why rows stay blocked (measured + gate-adjudicated):**
1. Whole-frame fail-closed under churn: ~1 failing root/frame (wartime character deaths)
   × any-failure `has_rejections()` = every frame rowless PARTIAL. The dead-vs-torn CLEAN
   emission attempt (df835b45e) was REFUTED as UNSOUND (gate verdict accepted, reverted
   28f2043e1): a stable read failure does not prove the ROOT is dead — nested objects
   (skill source, trait vector, referenced trait/character) can stably fail while the parent
   lives, and a CLEAN full-replacement frame omitting a live root silently deletes it.
   Positive absence = omission from a COMPLETE sweep + prewarm rebuild ONLY.
2. ~3.6 frame parse errors/frame of UNKNOWN text (manifest counts messages, not strings).
   NEXT DIAGNOSTIC: one short Debug-profile capture (sidecar on) to read the actual
   messages — they may be an independent universal blocker (payload self-decode).
3. Validator mismatch (gate HIGH, unfixed): prewarm validates SHALLOWER than the walk —
   unit leaders: prewarm checks the vtable qword only, walk checks type id/ids/skill child/
   trait vector/trait objects; country-leader roles: prewarm follows the character ptr but
   never revalidates the CCountryLeader root vtable. A root that stably fails only the
   deeper walk is re-adopted forever → permanent rejection loop. Fix belongs with the row
   restorer (either deepen prewarm to walk-equivalent validation, or the upsert design
   makes per-root failures non-frame-fatal).

**Row restorer of record — d47 PARTIAL-upsert fold contract (d48 precedent: partial
projection, upserts-only, never tombstone on partial):**
- Adapter emits the rows it DID read on a frame with failures, PARTIAL flag kept (never
  masked). Torn/stable failures keep their per-root attribution (dead_roots/rejected_roots).
- world_fold treats a PARTIAL d47 frame as UPSERT-ONLY: present rows update; absent rows
  are RETAINED (no deletion without proof). Deletions happen only on CLEAN frames (zero
  failures — the complete-observation proof) or via positive absence from rebuild-diffing.
- Verification gates: work-implies-rows extended per-frame-family; fold-identity gated
  (old streams must fold byte-identically — PARTIAL d47 frames in old streams are rowless,
  so upsert-of-nothing is a no-op = compatible by construction; prove it).
- Replay contract impact: REPLAY_DATA_REQUIREMENTS.md must record the d47 partial-upsert
  semantics change in the same commit that lands it.
- Mass-failure defense in depth: a CLEAN frame whose row families shrink implausibly vs
  the adopted root count should fail closed (bound TBD with live dead_roots/stale rates
  from the new diagnostics).

### PARTIAL-upsert re-priced 2026-07-11 (post char-dedup): DEFERRED

The contract above was priced when EVERY d47 frame was discarded (~1 failing root/frame
read as churn). The duplicate-character dedup (1fbd9f3a4) revealed most 'churn' was
duplicate-object noise: the decisive2 capture measured **dead_roots = 1 across 509 hours**
— i.e. ~one rowless PARTIAL frame per session, prior state retained across it. Building a
lossless-critical full-replacement fold change (clone-prior + keyed merge, new gates,
fold-identity proof) to recover ~1 frame per session fails the measure-first bar.
DEFER until live captures show a materially higher PARTIAL rate (the dead_roots and
refresh_held_roots units make that visible from any manifest). Design retained above.

---

## G4 DESIGN AMENDMENTS (2026-07-11, adversarial review tid 019f4f8d — VERDICT REFUTED, 9 findings; these amendments are now part of the design of record and BIND C1)

The O8 core (typed dispositions, never mask failure bits, forbidden list) survives; the
review broke the surrounding machinery. Amended requirements:

1. **Closed declaration-kind registry (deny-by-default).** `ExpectedAbsent` can only be
   constructed by the registry. Every entry names: exact segment, authoritative evidence
   field, evidence reader, immutability argument, invalidation event, proof/test id.
   Negative pointer searches and bounded discovery NEVER declare (they prove only "this
   scan didn't find it"). **Until at least one entry passes this bar, stage 3's tolerance
   set is EMPTY** — O8 ships with accounting + registry machinery and zero tolerated pairs.
   Every scheduled segment is explicitly `Allowed(kind)` or `Forbidden` against the
   compact_contract roster; adding a segment fails tests until the registry changes.

2. **Transactional epochs.** Declarations and frames carry: session nonce, process-start
   identity, save-epoch generation, modset hash, exe hash, schema version. Capture stops
   when reload MAY have begun (not when the new epoch is recognized); frames overlapping
   identity/reload uncertainty are Failed; old epoch flushes and seals before new
   declarations; recorder verifies per-hour disposition epoch tokens match. Sweep-thread
   death/restart never yields ExpectedAbsent — dependent segments stay Failed until a new
   complete sweep generation publishes.

3. **Mismatch detector is mandatory.** A declaration either (a) revalidates its
   authoritative immutable configuration token every frame, or (b) runs an independent
   bounded-skew positive-presence sentinel while active. Sentinel failure = Failed (never
   continued absence); a presence hit atomically revokes the declaration, captures via the
   ordinary reader, and FAILS THE RUN. Detection skew is part of the production bound.

4. **Raw aggregate status bits never grant tolerance.** Domain-wide OR'd bits stay an
   OR-summary. Tolerance keys ONLY on per-attempt failure-evidence units:
   (session/epoch, hour, domain, segment, subsource, cause code, attempt id, status bits,
   declaration id, result, evidence digest). The only excusable cause is "scheduler did
   not attempt this source because immutable prerequisite declaration D applies". All
   unattributed SOURCE events remain fatal. (This makes per-subsource attribution wiring a
   PREREQUISITE stage of C1, before any recorder-gate change.)

5. **Captured = contract completion, not rows>0.** An incomplete full-replacement
   observation is Failed even when its read rows ride the stream (diagnostic/upsert-only
   under an independently reviewed fold contract; never advances a complete baseline).
   MeasuredEmpty requires a serialized completeness witness (frame, source generation,
   enumeration bounds, epoch). Fold acceptance is disposition-aware. The per-segment
   partition invariant joins the hour-accounting invariant.

6. **Enforcement shape:** ExpectedAbsent constructors private to the registry; contract
   tests exercise d43/d47/d69 through every entry point; property tests over reload/
   restart/epoch-uncertainty/sweep-death/late-availability/torn/combined-bits sequences;
   mutation tests (drop attribution, change a cause code, add a segment) must fail the
   gate; production cannot load user/manual absence lists.

7. **Wire safety = major version bump.** Dispositions ride a new top-level stream major
   version that old decoders reject before reading records; unknown discriminants reject;
   declaration records precede references (or repeat per recoverable chunk); declaration
   ids are session/epoch-scoped with content hashes; the packer rejects mixed majors; new
   decoders read old streams in legacy mode but never synthesize dispositions
   (LegacyUnverified never satisfies the production gate). Test battery: old-reader/
   new-stream, new-reader/old-stream, truncated declaration table, unknown disposition,
   restart concatenation, mixed-fold rejection.

8. **C0 is discovery, not classification.** The census finds incidence; classification of
   each nontrivial segment additionally requires: code-level source-to-bit dataflow audit,
   snapshot-vs-delta + fold semantics, completeness evidence, prerequisite lifecycle
   analysis, per-cause live telemetry, reload/restart transition probes, save/native
   oracle where feasible, mod/DLC coverage per claimed scope. C0 exit = "every incidence
   has an evidence dossier".

9. **Provenance is a separate axis from completion.** Dispositions stay exactly
   {Captured, MeasuredEmpty, ExpectedAbsent, Failed}; provenance = Native |
   DerivedFrom(segment, proof_id). **d17: NATIVE CAPTURE IS THE DEFAULT** — the
   DerivedFrom(d53) equivalence bar (total deterministic mapping incl. war lifecycle,
   casualties semantics, tombstones, hour coherence, oracle comparisons across mods) is
   unproven and possibly unreachable; if d53 fails an hour, a derived d17 fails that hour
   too. Removing d17 from the roster instead requires a versioned product decision.

C1 STAGE ORDER (amended): stage A per-subsource failure-evidence units (collector) →
stage B declaration registry + epoch tokens (empty tolerance set) → stage C recorder
accounting consuming evidence units (still zero tolerated pairs = behavior identical,
evidence richer) → stage D first registry entry IF any segment ever passes the bar →
stage E fold/decoder major-version passthrough. GATES GREEN is achieved by FIXING or
correctly typing sources, not by tolerance: with the tolerance set empty, a clean vanilla
run goes green only when the C0-listed unattributed failures are individually closed.
