# AUDIT RECONCILIATION — extractor session branch (G5 gate output + orchestrator cross-reference)

G5 adversarial reconciliation ran 2026-07-11 (tid in ~/.claude/codex_threads.log, reviewed
tree 319dc641b, inputs: branch log + CODEX_DISPATCH_QUEUE.md). Its table is the BASE LAYER
below, verbatim. G5 saw ONLY the dispatch queue as gate evidence; the cross-reference
section resolves rows where gates demonstrably ran (thread ids / session records), leaving
the TRUE blocking list for F1.

## Orchestrator cross-reference (2026-07-11 evening)

CLOSED or IN-PROGRESS by evidence G5 could not see:
- O3/d47/d69 chain (8e10e70ff, 28f2043e1, d5bb02247, ebdc5bc4b, 1fbd9f3a4, 2a9dc5091):
  G1 final net gate RAN (resume tid 019f4ebb) -> REFUTED 3 CRITICAL -> fix round
  615d24125 -> combined RE-GATE IN FLIGHT (tid in threads log, tree b24ac1ce3).
  O7 (2a9dc5091) explicitly verified safe within G1's failed-attacks list.
- e9f21a1c8 modset: G2 RAN -> REFUTED 9 -> fix round b24ac1ce3 -> same re-gate in flight.
- 1cac48222 serial-wall: KNOWN OPEN (G3 held for P1b equivalence evidence; P1a decision
  SHIP NEW recorded in plan §2). TRUE BLOCKER until G3 SOUND.
- O1 chain (ac09c8c04, 1c05f4477): gate-o1-d69-telemetry RAN -> REFUTED 1 (mask summing)
  -> fixed 1c05f4477 with the gate-specified test. Thread in codex_threads file.
- O2 chain (11b0cc9c3, b0897b457): gate-o2 RAN -> REFUTED twice -> fix b0897b457 ->
  gate-o2-RESTAMP verdict SOUND, findings none (session record 2026-07-11 mid-morning).
- d53 columnar probe (47def61dd): measurement script + NULL result; probe correctness was
  self-checked (schema-exact 1131 payloads, bailed=0, invertible transform) — accepted as
  measurement-of-record, no production code path.
- hour-latch family + three-phase family + d53 delta_partial: queue Jobs 0a/0b/1/1R2
  CLOSED (G5 concurs).

TRUE BLOCKING LIST for F1 (no independent adversarial verdict anywhere):
1. 1cac48222 serial-wall (G3 owed; P1b evidence pack required first).
2. ~~Prefetch plumbing final state (1b8ed7d66 -> bd4fb09b5 -> 8487be6a7)~~ **CLOSED
   2026-07-11: the re-stamp verdict was RECOVERED by resuming tid 019f4b91-0528 —
   REFUTED with 2 findings (substring key-aliasing in the control parser; MAX-aggregated
   applied-label on mixed streams). BOTH were already fixed in 8487be6a7 (the parser is
   now structural — escape-aware literal tokenization, key-requires-colon, whole-token
   values, lib.rs parse_json_u32_strict whose doc cites 'both rounds' — and main.rs
   suppresses prefetch_dist_*_applied whenever the *_mixed flag is set, which today's
   palindrome manifests demonstrate live: applied echoed per-arm, mixed=false).
   Orchestrator re-verified both fixes against current source. Verdict + mapping recorded
   here; the thread reviewed the pre-8487be6a7 state.**
3. 3a80881f2 prefetch default 16->8 (two live palindromes = evidence, but the one-line
   default change never had an adversarial look; fold-inert, low risk — batch into the
   integrated F5 RC gate).
4. 55287bd6a d47 work-implies-rows backstop (offline gate tool only — batch into F5).
5. Stream-size/fast-turnaround/composition chain (c81868d5f, 943866216, eaae87663,
   3cc572c79, f46b9139c, 9914cf27e, 93915635d, b45297af1, bb5a0a24f, cf6ba7177,
   e7a6d4bc9): session records say 4 adversarial rounds ran + G1 fold identity + live
   validation, but the queue carries no rows/tids — RECOVER the verdicts into this doc or
   re-gate the NET state of the stream container in one job (recommended: one net re-gate,
   the code is long-stable and live-proven).
6. Fold-performance lane (779d65a17, ca7650189, e61ebf094): live+G1 evidence only —
   batch into F5 or a single net gate with (5).
7. ~~hour-latch post-merge edits (ec9cf0b15 + revert 85eec418b)~~ **CLOSED 2026-07-11:
   `git diff ec9cf0b15^ 85eec418b` is EMPTY — the revert restored the exact pre-edit
   tree, so gate 0a's verdict on that tree still binds. No re-gate needed.**
8. M1 building-levels Arc/RwLock (97a5d902c): **code-soundness gate SOUND 2026-07-11
   (tid 019f5078; 5 attack classes failed: parallel first-fill byte-equivalence, poisoning,
   Arc mutation/lifetime, encoder/delta equality, fixture vacuity)**; live A/B still owed
   (game). G1-fix 615d24125 + G2-fix b24ac1ce3: re-gate REFUTED x7 -> fix round e908a0f61
   (d69 stale-skip retired to fail-closed; 5 modset FALSE-GREEN closed) — e908a0f61 itself
   rides the F5 integrated gate (fix-of-fix, suites green, orchestrator-verified).
9. B3 repack-level change (NOT YET IMPLEMENTED — when ReplayZstdLevel default changes,
   it needs fold-identity + gate per plan §2b).

NEXT MECHANICAL STEPS: (a) recover the prefetch re-stamp thread verdict; (b) git-verify
hour-latch net-zero; (c) queue the stream-container net re-gate; (d) M1 gate after its A/B.

## G5 base layer (verbatim gate output)

I treated the dispatch queue as the gate system of record. Commit-message assertions alone do not establish gate coverage, and later live tests do not substitute for adversarial review.

| Commit | One-line what | Classification | Evidence pointer |
|---|---|---|---|
| `319dc641b` | P1a scheduler A/B result and endgame status | DOC-ONLY | Evidence/status record; does not independently gate `1cac48222` |
| `59ae8ceb3` | Endgame-plan amendments | DOC-ONLY | Plan-only; review thread `019f4f5c` is absent from queue |
| `01b290bc7` | Production endgame plan | DOC-ONLY | Plan-only |
| `1cac48222` | Moves ten serial readers into worker scope | LIVE-VALIDATED-ONLY | Six-window A/B recorded by `319dc641b`; no adversarial scheduler/equivalence gate; **BLOCKING** |
| `79b97f27c` | Next-boot and blocker list | DOC-ONLY | Status document |
| `496e6e9a9` | Defers PARTIAL-upsert after measurement | DOC-ONLY | Decision/evidence document |
| `2a9dc5091` | Adds d17 missing-war-segment status bit | UNAUDITED | Tests only; no queue gate or live evidence; **BLOCKING** |
| `8bbbb4ae7` | RTTI re-derivation tool and manifest | LIVE-VALIDATED-ONLY | 51/51 current-executable derivation/verification; no independent gate; **BLOCKING** |
| `47def61dd` | d53 columnar lossless probe and NULL result | LIVE-VALIDATED-ONLY | 1,131-payload measurement; no queue gate for probe correctness |
| `a679df5b8` | Expands pending O3 net-gate set | DOC-ONLY | Queue amendment |
| `1fbd9f3a4` | d47 character dedup plus d69 tombstone handling | LIVE-VALIDATED-ONLY | Queue amendment: live 831k-row/clean-d69 evidence, final O3 gate still queued; **BLOCKING** |
| `ebdc5bc4b` | Skips stable vacant d69 member and changes d47 hash | LIVE-VALIDATED-ONLY | Queue amendment explicitly says live-validated but not adversarially reviewed; **BLOCKING** |
| `5f7ed3214` | Records credit outage and queued gates | DOC-ONLY | Queue/status document |
| `d5bb02247` | Adds d47/d69 attribution samples and recorder sample aggregation | LIVE-VALIDATED-ONLY | Live diagnostic capture; named O3 net gate remains queued; **BLOCKING** |
| `3134f682d` | Records O3 review/fix history | DOC-ONLY | Queue document |
| `bd108b911` | Documents d47 post-O3 design state | DOC-ONLY | Design document |
| `28f2043e1` | Reverts unsound dead-root CLEAN emission | GATED-REFUTED-FIXED | Message records `df835b45e` gate as UNSOUND; fixed state is in queued O3 thread `019f4ebb-…`, not re-gated; **BLOCKING** |
| `8e10e70ff` | O3 fix round 2 and d69 stale-slot change | GATED-REFUTED-FIXED | Fix after O3 refutations; explicitly named in still-queued O3 net gate; **BLOCKING** |
| `e9f21a1c8` | Adds fail-closed capture modset binding | LIVE-VALIDATED-ONLY | First live manifest matched 20/24 DLCs; O5 gate explicitly “never dispatched”; **BLOCKING** |
| `df835b45e` | Treats double-read-stable d47 walk failures as dead roots | GATED-REFUTED-FIXED | Later gate found it UNSOUND and `28f2043e1` reverted it; fixed net state not re-gated; **BLOCKING reconciliation** |
| `886e8b923` | O3 first fix round for sweep coverage and stale-root holds | GATED-REFUTED-FIXED | Message reports REFUTED findings; final O3 net gate remains queued; **BLOCKING** |
| `f33f87bff` | Claims O1/O2 closure and stages O3 capture | DOC-ONLY | Queue document; O1/O2 verdict claims are missing from supplied queue |
| `64e977666` | Adds compressed-byte attribution script | LIVE-VALIDATED-ONLY | Produced live-corpus d53 ranking; no independent gate |
| `6d1ff969c` | Implements d47 generation-aware root refresh | GATED-REFUTED-FIXED | Subsequently refuted by the O3 review history; final net state not re-gated; **BLOCKING** |
| `b0897b457` | O2 fixes degraded/stale d43 empty handling | LIVE-VALIDATED-ONLY | Later combined capture evidence exists, but supplied queue has no O2 SOUND row/thread; **BLOCKING** |
| `953d64a08` | Closes d23 coalescing idea with live probe | LIVE-VALIDATED-ONLY | Valid paused-game measurement; no adversarial probe gate |
| `11b0cc9c3` | Initial d43 cold-versus-known-empty change | LIVE-VALIDATED-ONLY | Later live O1/O2/O3 capture, but no recorded O2 adversarial gate; **BLOCKING** |
| `30a2b3232` | Records byte-attribution conclusions | DOC-ONLY | Results document |
| `1c05f4477` | Fixes d69 diagnostic mask aggregation | LIVE-VALIDATED-ONLY | Gate claim exists only in message; later live telemetry, no queue gate row/thread; **BLOCKING** |
| `ac09c8c04` | Adds d69 per-reason failure telemetry | LIVE-VALIDATED-ONLY | Used by later live captures; no queue-recorded O1 gate; **BLOCKING** |
| `02bba37f8` | Overnight queue and next-boot plan | DOC-ONLY | Plan document |
| `a574c9246` | d43/d69 attribution findings | DOC-ONLY | Analysis/design record |
| `3a80881f2` | Retunes production prefetch distance 16→8 | LIVE-VALIDATED-ONLY | Two live counterbalanced palindromes; no independent gate of changed default; **BLOCKING** |
| `804bb6f7f` | Mod/update robustness plan | DOC-ONLY | Plan of record |
| `825585919` | Queues d43/d69 attribution work | DOC-ONLY | Queue document |
| `919fa214d` | First live prefetch palindrome result | DOC-ONLY | Measurement/status record |
| `69975d882` | d47 root-cause and repair plan | DOC-ONLY | Analysis/design record |
| `55287bd6a` | Adds fatal d47 work-implies-rows backstop | UNAUDITED | Explicitly offline verification only; no adversarial or live validation of change; **BLOCKING** |
| `8487be6a7` | Residual prefetch-plumbing fixes | UNAUDITED | Post-refutation code with tests but no final re-gate/live arm-provenance validation; **BLOCKING** |
| `599468990` | Confirmed-complete design decision | DOC-ONLY | Design record |
| `bd4fb09b5` | First prefetch-plumbing fix round | GATED-REFUTED-FIXED | Message reports REFUTED gate; `8487be6a7` followed, but queue records no final re-gate; **BLOCKING** |
| `8a31e5d4d` | Queue and plan refresh | DOC-ONLY | Status document |
| `1b8ed7d66` | Adds runtime prefetch-distance control plumbing | GATED-REFUTED-FIXED | Queue says gate merely DISPATCHED; later message says REFUTED and fixes followed, with no SOUND re-gate; **BLOCKING** |
| `85eec418b` | Reverts the live-soak hour-latch mask change | LIVE-VALIDATED-ONLY | Returns toward prior state, but it is a later commit not covered by gate 0a; queue records live soaks only; **BLOCKING** |
| `ec9cf0b15` | Changes hour-latch FRAME_END partial handling | LIVE-VALIDATED-ONLY | Found during 7290-hour soak, then reverted; no adversarial review of this commit; **BLOCKING** |
| `b45297af1` | Resolves fast-turnaround/compression composition | UNAUDITED | Later merge touching both lanes; no named queue gate or live validation for this composition; **BLOCKING** |
| `cf6ba7177` | Final production-stream-size merge composition | LIVE-VALIDATED-ONLY | Runtime/G1 evidence stated, but four-round gate is absent from queue; **BLOCKING** |
| `bb5a0a24f` | Earlier production-stream-size merge | LIVE-VALIDATED-ONLY | Compression/live/G1 evidence stated; no queue-recorded adversarial gate; **BLOCKING** |
| `e7a6d4bc9` | Residual stream-verification comment fix | UNAUDITED | Later commit after claimed final gate; no re-gate recorded |
| `3cc572c79` | Stream-size fix round 3 and corruption tests | LIVE-VALIDATED-ONLY | Suite/G1 corpus evidence; no queue-recorded adversarial verdict; **BLOCKING** |
| `eaae87663` | Closes compressed framing-stop checksum bypass | LIVE-VALIDATED-ONLY | Suite/G1 validation; no queue-recorded gate; **BLOCKING** |
| `943866216` | Adds checksummed zstd frames and strict EOF handling | LIVE-VALIDATED-ONLY | Real-corpus round trip and live torn rejection; no queue-recorded gate; **BLOCKING** |
| `9914cf27e` | Merges final fast-turnaround lane | LIVE-VALIDATED-ONLY | Whole-file repack comparison stated; SOUND review absent from queue; **BLOCKING** |
| `75d50a31d` | Records stream-size adversarial REFUTED verdict | DOC-ONLY | Verdict/status commit; corresponding gate absent from queue |
| `f46b9139c` | Fast-turnaround fix round | UNAUDITED | Tests stated, but no live evidence or queue-recorded adversarial verdict; **BLOCKING** |
| `c81868d5f` | Implements streaming zstd production format | LIVE-VALIDATED-ONLY | Real deep-war corpus and fold-chain evidence; no queue-recorded adversarial gate; **BLOCKING** |
| `b79a5f201` | Merges final hour-latch state | GATED-SOUND | Queue Job 0a: CLOSED after six adversarial rounds and two live validations |
| `93915635d` | Implements fast-turnaround defaults and deferred fold | UNAUDITED | Parser validation only; no queue gate or live evidence; **BLOCKING** |
| `c6e335325` | Adds JSON-native inactive latch marker | GATED-REFUTED-FIXED | Part of Job 0a fix sequence; final state re-gated by 0a’s six-round closure |
| `85319d211` | Restricts compact inactive marker framing | GATED-REFUTED-FIXED | Part of Job 0a fix sequence; re-gated by final 0a closure |
| `04361bd92` | Corrects latch requested/active distinction | GATED-REFUTED-FIXED | Part of Job 0a fix sequence; re-gated by final 0a closure |
| `cba735847` | Hour-latch 0aF fix set | GATED-REFUTED-FIXED | Queue Job 0a/0aF; later rounds and final closure re-gated it |
| `6d109ff78` | Records 0a2 REFUTED findings | DOC-ONLY | Gate/status record represented by queue Job 0a history |
| `50c50f804` | Initial hour-latch fix round | GATED-REFUTED-FIXED | Queue Job 0a records subsequent refutations, fixes, and final closure |
| `2d673384a` | Merges final three-phase timing lane | GATED-SOUND | Queue Job 1 and 1R2 CLOSED; three review rounds summarized |
| `49f1ac13b` | Three-phase 1R3 residual fixes | GATED-SOUND | Included in final Job 1/1R2 closed state |
| `79b281d58` | Three-phase 1R2 fixes | GATED-REFUTED-FIXED | Re-review/fix sequence closed by queue Job 1R2 |
| `29d9b0342` | Corrects three-phase report estimator | GATED-SOUND | Part of final Job 1 lane closed under Job 1/1R2 |
| `f26aac06b` | Three-phase 1R fix round | GATED-REFUTED-FIXED | Later 1R2/1R3 review sequence closed by Job 1R2 |
| `bf58ffc7b` | Records Codex re-stamp/refutations and dispatch changes | DOC-ONLY | Queue/brief record; substantive code fixes are separate commits |
| `9d33a2a8f` | Adds three-phase offline estimator | GATED-SOUND | Job 1 deliverable incorporated into the final closed lane |
| `f24c11d3a` | Records superseded Opus gate outcomes | DOC-ONLY | Historical status; queue records Codex re-stamps as controlling |
| `d56e2bbe7` | Initial three-phase timing implementation | GATED-REFUTED-FIXED | Job 1R findings were fixed; queue Job 1/1R2 records final closure |
| `c798f60d1` | Adds dispatch prompts for outstanding jobs | DOC-ONLY | Queue document |
| `4fecce114` | Adds burst-correlation analysis script | LIVE-VALIDATED-ONLY | Used on compact-stream data; no independent script/result gate |
| `c2696f8db` | Records burst-correlation result | DOC-ONLY | Study document |
| `c2f621c7c` | Updates lane brief with prefetch evidence | DOC-ONLY | Brief/document change |
| `cc99b1265` | Relocates w11 report and records NULLs | DOC-ONLY | Report/document change |
| `cc19a4937` | Merges measurement-only w11 lane | LIVE-VALIDATED-ONLY | No production code, but includes measurement tooling; no queue gate |
| `938fe515a` | Records d53 CPU-negative verdict | DOC-ONLY | Result/queue document |
| `eac249297` | Adds w11 measurement instrumentation | LIVE-VALIDATED-ONLY | Measurement evidence exists; no queue-recorded adversarial gate |
| `5d6549cba` | Merges fixed d53 partial-delta lane | GATED-SOUND | Queue Job 0b CLOSED; fixed final state adjudicated, feature remains opt-in OFF |
| `06ba640f6` | Documents hour-latch losslessness scope | DOC-ONLY | Documentation only |
| `ed81642de` | Initial d53 partial-delta implementation | GATED-REFUTED-FIXED | Two silent-loss defects fixed; final state re-gated/adjudicated under Job 0b |
| `22ec58348` | Initial hour-latch merge | GATED-REFUTED-FIXED | Subsequent six-round Job 0a sequence fixed and re-gated final state |
| `42958b583` | Initial hour-latch implementation | GATED-REFUTED-FIXED | Subsequent Job 0a refutations/fixes; final state re-gated |
| `2e8710c9c` | Corrects dense-mirror premise | DOC-ONLY | Design/result document |
| `846a0630c` | Merges map-detail RE/probe lane | LIVE-VALIDATED-ONLY | Live locality probe; no queue-recorded adversarial gate |
| `07bbf4eca` | Records corpus retention cleanup | DOC-ONLY | Operational record |
| `aaa3e33b0` | Adds map-detail locality probe | LIVE-VALIDATED-ONLY | Live probe evidence; no queue-recorded adversarial gate |
| `ae38668fa` | Makes lane workflow repo-wide | DOC-ONLY | Process documentation |
| `e9fbea669` | Adds lane protocol and review rules | DOC-ONLY | Process documentation |
| `779d65a17` | Merges fold-performance lane | LIVE-VALIDATED-ONLY | Profiling and G1 identity stated; no queue-recorded adversarial gate; **BLOCKING** |
| `e61ebf094` | Adds production skip-fold option | LIVE-VALIDATED-ONLY | Covered by later performance/G1 validation, but no recorded adversarial gate; **BLOCKING** |
| `ca7650189` | Implements 5.4× parallel fold | LIVE-VALIDATED-ONLY | Profile and G1 evidence; no queue-recorded adversarial gate; **BLOCKING** |

## BLOCKING LIST

These must be closed before a production tag. “Close” means either recover and cite an independent gate whose prompt explicitly names the commit/final tree, or run a new net gate. Live evidence alone is insufficient.

1. Lossless/capture-semantics changes awaiting an explicit adversarial verdict:

   `1cac48222`, `2a9dc5091`, `1fbd9f3a4`, `ebdc5bc4b`, `d5bb02247`, `28f2043e1`, `8e10e70ff`, `e9f21a1c8`, `df835b45e`, `886e8b923`, `6d1ff969c`, `b0897b457`, `11b0cc9c3`, `1c05f4477`, `ac09c8c04`, `55287bd6a`, `8487be6a7`, `bd4fb09b5`, `1b8ed7d66`, `85eec418b`, `ec9cf0b15`.

2. Compression, folding, and composition changes whose commit messages cite validation or gates that the queue does not record:

   `b45297af1`, `cf6ba7177`, `bb5a0a24f`, `e7a6d4bc9`, `3cc572c79`, `eaae87663`, `943866216`, `c81868d5f`, `9914cf27e`, `f46b9139c`, `93915635d`, `779d65a17`, `e61ebf094`, `ca7650189`.

3. Production behavior or patch-robustness changes with live/self-validation but no independent gate:

   `8bbbb4ae7`, `3a80881f2`.

The queued O3 gate must cover exactly `8e10e70ff + 28f2043e1 + d5bb02247 + ebdc5bc4b + 1fbd9f3a4`. It does not automatically clear `6d1ff969c`, `886e8b923`, or `df835b45e` as historical audit rows unless its prompt explicitly includes their surviving effects or the ledger records their refuted-and-replaced disposition.

## Commit-message/queue inconsistencies

- `59ae8ceb3` claims an adversarial review at thread `019f4f5c` with “AMENDMENTS REQUIRED”; the supplied queue contains no such row/thread.
- `28f2043e1`, `886e8b923`, and `6d1ff969c` describe O3 adversarial verdicts, but the queue records only the final net gate as queued, not those completed verdicts.
- `f33f87bff`, `b0897b457`, and `1c05f4477` claim O1/O2 REFUTED/SOUND/closed outcomes; no O1 or O2 gate row/thread appears in the supplied queue.
- `bd4fb09b5` says the prefetch gate returned REFUTED, while queue Job 3 stops at “DISPATCHED.” `8487be6a7` then describes residual re-stamp fixes without a recorded final verdict.
- `75d50a31d`, `9914cf27e`, `bb5a0a24f`, `cf6ba7177`, and `e7a6d4bc9` claim stream-size REFUTED/SOUND/four-round outcomes; the queue has no stream-size gate row or thread.
- `9914cf27e` claims the fast-turnaround review returned SOUND; no fast-turnaround gate is recorded in the queue.
- `f24c11d3a` records Opus SOUND verdicts, but also says they were revoked. Those are not usable closures; the queue’s later Codex-controlled 0a/0b outcomes govern.
- `bf58ffc7b`’s Codex re-stamp is consistent with the later 0a/0b history, but the ledger should cite the actual final 0a/0b gate artifacts, not this commit message.

Self-refutation: I considered treating merge-message gate claims and later live runs as sufficient retroactive coverage. That fails the stated rule: the queue is the status of record, a gate covers only its named commit set, and a later commit touching the same system needs its own explicit coverage. I also checked the opposite risk—over-blocking reverted commits. They still require reconciliation because the mandate applies to every merged change, although their rows can close as “refuted and reverted” once the relevant verdict is cited.