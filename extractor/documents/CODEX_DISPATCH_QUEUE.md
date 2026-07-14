# CODEX DISPATCH QUEUE — status of record (live head; updated 2026-07-14)

Orchestrator-owned running log of the Codex delegation/gate pipeline. Method:
`documents/LANE_WORKFLOW.md`. Context brief every lane must read:
`extractor/documents/CODEX_LANE_BRIEF.md`. Protocol: `C:\Users\Example\.claude\CODEX_PROTOCOL.md`.

**Completion status + residuals live in `MASTER_EXTRACTION_CONTRACT.md`, not here.** This file is
the chronological dispatch/verdict trail only. Records older than the current session were moved to
`archive/CODEX_DISPATCH_QUEUE_HISTORY_20260710-12.md` on 2026-07-14 (evidence only).

Mode A invocation (verbatim; per global rules — never drop the model/effort pins; RESUMABILITY
LAW: NEVER `--ephemeral`, always capture + persist the thread id):

```bash
cd "/c/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/mod/battle-plan"
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
printf '%s' "$PROMPT" | codex exec --json -s read-only -c approval_policy=never \
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=xhigh -o "$OUT" - > "$AJ"
TID=$(python -c "import json,sys;print(json.loads(open(sys.argv[1]).readline()).get('thread_id',''))" "$AJ")
# persist $TID durably (e.g. ~/.claude/codex_threads_hoi4.txt); verify exit 0 AND non-empty
# $OUT; retry once — EXCEPT usage-limit/out-of-credits (queue + resume later, never retry into)
```

The `cd` matters: the read-only sandbox roots at the invocation cwd, and gate jobs need
file access to the repo.

---

```

---

## 2026-07-14 harvest: autoattach fix3 + replay-loader (both PASS impl gates) -> gates launched
- autoattach fix3 DONE (tid 019f5d51-33e2-7b51-9e6c-98046870ec5f): R3-F1 CRITICAL closed - the
  RetainedTargetLease/job poison is now UNCONDITIONAL on every injection entry incl. ordinary
  --pid/--dll (main.rs ~1395: lease no longer gated on proof_emitter.is_some()), before pipe
  exposure/hook install. R3-F2 HIGH closed - FirstCompactRecordGate::new(expected scheduled
  domain ids); Recording proof = ordered PERF->DATA for a SCHEDULED domain + plausible clock +
  clean status + valid counters + nonzero data + PRODUCTION WorldFold decode + material world
  diff excl. provenance (proof.rs ~242). Scope clean: exactly main.rs + auto_attach/proof.rs.
  ORCHESTRATOR VERIFIED: recorder_host 302 lib + 205 bin, 0 failed (lane's 2 RmStartSession
  modset.rs flakes did not reproduce). Residual: truly simultaneous uncooperative external
  injection = irreducible, fails closed. Lane could not self-review (app-server access denied) -
  irrelevant, independent re-gate is ours: regate-autoattach-r4 LAUNCHED (prompt pins both R3
  claims + regression check of rounds 2-3 closures).
- replay-loader resume DONE (tid 019f5d52-6d72-7c31-9a6e-07ca96706d21): implementation complete
  per W12B_DLC_META r9 law set - strict container open (exact immutable buffer, recursive dup-key
  + non-string-role + ASCII-lookalike rejection in new strict_json.rs), dir_store module
  (windows.rs relative NtCreateFile + reparse discriminator + identity-bound reopen + volume
  identity; linux.rs openat + stx_mnt_id + fail-closed openat2 fallback; other_posix fail-closed
  stubs), r9-F4 zero descendant-pathname opens + static prohibition gate, source.rs three-way
  routing/reconciliation/bind_or_match (+690), world_timeline e2e (+802). +17 snapshot / +22
  replay_core tests, 0 removed. ORCHESTRATOR VERIFIED: replay_core 66/0; snapshot green except
  EXACTLY the documented Dev-Mode symlink capability gate (Win32 1314). Scope clean (snapshot +
  replay_core + locks only). ENV-GATED residuals (pre-ship, not pre-merge): Windows Developer
  Mode for the symlink test (USER decision - system setting, orchestrator will not change it);
  real dehydrated OneDrive fixture (BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE); Linux host run.
  regate-replayloader LAUNCHED (first full adversarial gate: TOCTOU sweep, Windows namespace
  attack incl. reparse-class discrimination, strict_json two-decoders-two-truths, source.rs
  fail-open hunt, signal-reaches-consumer check on world.rs canonical keys, test-fidelity attack
  on the simulated stand-ins).
- Still in flight: foldwave2 fix4, facs-wire fix3, stitching r3 (event streams advancing).

## 2026-07-14 SCOPE RULING: Windows-only (owner confirmed)
Product ships WINDOWS-ONLY. "Linux host run" is DROPPED from the replay-loader residual list
(it was reflexively emitted by the lane's portable per-platform dir_store module). dir_store/
linux.rs is cfg(target_os=linux) so it never enters the Windows build - zero cost, but UNTESTED +
UNSUPPORTED = dead code eligible for a cleanup-pass removal (NOT merge-blocking, NOT verified).
other_posix.rs fail-closed unimplemented!() stubs stay. Remaining replay-loader residuals are
Windows-relevant only: Developer-Mode symlink capability test + real dehydrated OneDrive fixture.
CLEANUP-LATER (low priority, post-merge): strip dir_store/linux.rs + any Linux-only tests to
shrink the surface, OR reduce linux.rs to a fail-closed stub matching other_posix.

## 2026-07-14 autoattach r4 gate = REJECT (2 real findings survived fix3) -> fix4 launched
R3-F1 (universal injection lease) CONFIRMED CLOSED - repo-wide exactly 1 LoadLibraryW / 1
SetWindowsHookExW / 1 attach_hook; ordinary CLI + auto-attach converge through the unconditional
lease at main.rs:1403 before pipe+hook, same retained authority; existing-job-membership aborts
fail-closed. No regressions in rounds 2-3 closures (journal hash-chain, retained authorities, DLL
hash through LoadLibraryW, ChildWorkerHandle Drop all intact). BUT:
- F1 CRITICAL: ConnectNamedPipe (main.rs ~6935) accepts first client with NO PID binding to the
  retained target. The 128-bit pipe token lives in same-user-readable collector_control.json =
  capability hardening, not process/session binding. A same-user process can connect first once
  the DLL is resident and REPLAY a prior-session H4CR stream; worker self-generates the challenge
  heartbeat from only the accepted seqno (~7040); stale stream passes fold/pack = SILENT
  wrong-session replay data. => data-integrity hole, not just security.
- F2 HIGH: recording proof (proof.rs ~336) never binds DATA back to PERF. DomainPerfEvidence keeps
  only header+elapsed; DATA checked only for domain/hour/seq/elapsed/"some nonzero byte", NOT
  status or counters. Bypasses: (a) scheduled d44 clean PERF + DATA carrying SOURCE_UNAVAILABLE ->
  WorldFold handles Ok, emits supply w/ null fields, non-provenance diff passes on ZERO data;
  (b) scheduled d21 clean PERF + EMPTY combat snapshot (changed_rows=1) -> combats:[] accepted as
  material. Empty/unavailable wrongly = "material world data".
Gate could not run cargo (read-only sandbox blocked /tmp/aa_r4 create) but enumerated prebuilt
304 lib/209 bin + ran 15 targeted (proof/job/dll/d21-fold) 0-fail; verdict stands on static
analysis + targeted. FIX4 launched (workspace-write, builds on uncommitted fix1-3 tree, tid in
threads/w12g-autoattach-fix.tid): F1 = GetNamedPipeClientProcessId == retained target PID +
target-originated session nonce, fail-closed; F2 = match DATA status+counters to PERF for the same
scheduled domain + reject empty/unavailable/unknown as material (require actually-present
rows/entities from the real WorldFold output). Prompt mandates RED->GREEN tests for all 4 bypasses
+ self-refutation. This is the ONE fix round for review r4; if it doesn't fully close, orchestrator
fixes directly. NOTE: fix3 CODE is still uncommitted in the worktree (only the RECORD was queued) -
nothing merged yet; fix4 continues on top, re-gate r5 after.

## 2026-07-14 replay-loader gate = REJECT -> fix2 launched; facs-wire fix3 verified -> r3 gate
REPLAY-LOADER gate REJECT (2 Windows-relevant real findings + 1 gate-completeness; F4 Linux
IGNORED per Windows-only ruling):
- F1 CRITICAL: cross-object routing. Descendant handles dropped between logical reads (games ->
  metadata -> index() -> frames); only ROOT handle persists. Rename game-dir object A->B mid-
  sequence: each handle-relative open individually valid but A's DLC/language binding injected into
  B's snapshots, consumed by world.rs. No symlink needed. Same race timeline.json->manifest.
- F2 HIGH: snapshots still decoded with permissive serde_json::from_slice while manifest/timeline/
  recorder use strict_json. Duplicate session/active_dlcs keys in flattened Snapshot.rest go
  last-wins -> ambiguous recording silently reconciles to wrong DLC binding.
- F3 MEDIUM: r9-F4 static "no descendant pathname reads" gate too narrow (scans 2 files, misses
  File::open/OpenOptions/container.rs/dir_store; no current escape but wouldn't catch a new one).
- F4 MEDIUM (Linux mount self-skip): IGNORED - Windows-only, that code is being removed.
Attack axes: Windows namespace primitives PASS (name-surrogate mask correct, junctions/symlinks
rejected, placeholder reopen path correct, ADS/DOS-device/case/exact-alias rejected); scope/writer-
regression PASS. FIX2 launched (tid threads/i1-replayloader-e2e.tid): F1 = retain selected-game dir
identity (handle or volserial+128-bit fileID) + reuse/revalidate across ENTIRE routing, fail-closed
on mismatch, no double-independent index read; F2 = decode snapshots through strict_json at every
nesting level for BOTH stores, duplicate=REJECT; F3 = broaden gate to all loader file-access modules
+ full open-pattern set, fail-closed on unclassifiable. Mandates RED->GREEN for object-swap, parent-
swap, duplicate-key (both stores), planted-open-gate. ONE fix round for this review.
FACS-WIRE fix3 DONE + ORCHESTRATOR-VERIFIED GREEN (tid 019...; lane's own self-review was
sandbox-blocked = irrelevant, independent gate is ours): R2-F1 (outer-oracle single-read) closed -
witness captures country-table alloc/count/accepted, re-resolves AFTER faction reads, re-reads
outer root, mismatch suppresses both; R2-F2 (unavailable-d53 not serialized -> MeasuredEmpty) closed
- failed/malformed/absent-after-present serialize faction_roster_status=unavailable, modern status
beats legacy; R2-F3 (malformed legacy fail-open) closed - atomic whole-roster fail-closed; R2-F4
test uses production encode->decode->fold->typed + overlays valid legacy to prove Unavailable wins.
VERIFIED: diplomacy_intel_operatives_live 121/0, recorder_host 209 lib/199 bin(2 ign), replay_core
48/0; scope 5 files within the 8-file fence. regate-facs-wire-r3 LAUNCHED.
IN FLIGHT: autoattach fix4, replay-loader fix2, facs-wire r3 gate, foldwave2 fix4, stitching r3.

## 2026-07-14 facs-wire r3 gate = REJECT (witness still leaks) -> fix4 launched
R2-F2 (unavailable never downgraded) CONFIRMED CLOSED. But the POSITIVE COMPLETENESS WITNESS took
a 2nd round of holes (c1stageb-style piecemeal-check smell):
- F1 CRITICAL: country oracle SKIPS ambiguous/duplicate-tag/unreadable slots (extractor.rs:1621) +
  validates members by tag-SET not accepted-POINTER (:1487). Slots A(X)+B(X): both passes accept A
  skip B; member->B passes on tag X => positive witness on an ambiguous table.
- F2 CRITICAL: final validation binds only OUTER game-state root (:1413), NOT the inner country
  table; table can move while root identical after oracle_after(:1408) => stale roster witnessed.
- F3 CRITICAL/wiring: signal-reaches-consumer, LAST hop. UI binding preserves enum (binding.rs:483)
  + map helper rejects Unavailable (mapmode_factions.rs:136) BUT country_faction_memberships called
  ONLY from #[cfg(test)] (:217); production just re-exports => map enforcement proven only in tests.
- F4 HIGH: raw duplicate legacy JSON keys collapse last-wins in serde_json::Map (snapshot/src/
  lib.rs:65,291) BEFORE the atomic parser (world.rs:3367). => NOT facs-wire's: SAME root cause +
  SAME FILE as replay-loader gate F2, already being fixed in the replay-loader/snapshot lane via
  strict_json snapshot decode. Fix ONCE there, do NOT double-patch. (memory: raw-bytes-before-Map
  law added to signal-must-reach-consumer.)
FIX4 launched (tid threads/i1-facs-wire.tid): F1 = account for ALL slots incl rejected, DUPLICATE
TAGS poison the witness, validate members by accepted POINTER not tag-set; F2 = fingerprint WHOLE
table once + re-fingerprint as a unit bound THROUGH the final approval alongside outer-root reread;
FAIL-CLOSED FALLBACK explicit - the positive witness is an OPTIMIZATION, if any ambiguity remains
SUPPRESS it (roster = not-proven-complete, always safe, loses no data); F3 = resolve HONESTLY -
investigate whether a production faction-mapmode consumer exists; if yes wire+integration-test, if
no (map render UI not built yet) document "consumer not yet built, signal preserved, helper ready,
test owed" + strongest real typed->UI e2e test - do NOT fake a consumer. Prompt EXCLUDES
snapshot/src/lib.rs. ONE fix round for review r3; if r4 still rejects the witness => orchestrator
either fixes directly OR ships with the witness SUPPRESSED (the safe degenerate). escalation noted.
IN FLIGHT: autoattach fix4, replay-loader fix2, facs-wire fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 autoattach fix4 verified GREEN -> r5 gate launched
Both r4 findings closed in the diff: F1 = GetNamedPipeClientProcessId == retained target PID
BEFORE stream writer / any byte read (main.rs:6959,7042), same RetainedProcessAuthority FILETIME/
liveness-revalidated before+after the PID query (PID-reuse), accepted endpoint must contain THIS
attempt's token (main.rs:6922), API-fail/PID-mismatch/malformed-token/stale-endpoint abort before
data; F2 = DomainPerfEvidence retains status + 4 counters (proof.rs:233), DATA must match PERF
elapsed+status+domain counter semantics or gate keeps WAITING (proof.rs:483), provenance/nulls/
empty-arrays/empty-objects stripped before material comparison (proof.rs:589,622). Lane self-review
sandbox-blocked (irrelevant). ORCHESTRATOR-VERIFIED: recorder_host 306 lib(2 ign)/207 bin(4 ign),
0 failed, RmStartSession flakes did not even reproduce; scope 2 files clean. Residual: in-process
compromise can originate its own endpoint (in-process threat, not another process as pipe client) -
irreducible + fail-closed. regate-autoattach-r5 LAUNCHED (attacks PID-reuse window, token
guessability/staleness, semantically-empty-but-present material, cross-domain/hour counter reuse,
recursive empty stripping, + regression of universal lease/journal/authorities/DLL-hash).
IN FLIGHT: autoattach r5 gate, replay-loader fix2, facs-wire fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 replay-loader fix2 verified GREEN -> r2 gate launched
All 3 findings closed in the diff: F1 cross-object routing = retained BoundDir + volume-serial +
128-bit fileID + first index.json buffer (lib.rs:180, dir_store/mod.rs:59); games() establishes
retained objects, metadata/cached-index/snapshots REUSE them (lib.rs:309); every later read
identity-revalidates the same-name dir (mismatch REJECTS) but reads bytes relative to the RETAINED
handle (windows.rs:66); index.json read once never reopened (lib.rs:337); errors propagate
Store::load_file -> ReplaySource::frame. F2 duplicate snapshot keys = one strict recursive decode
builds duplicate-free Value then derives typed view from that tree (lib.rs:44), used by BOTH dir
snapshots (lib.rs:422) + container index/representative/lazy loads (container.rs:417); dup session
+ nested dup active_dlcs reject in both backends. F3 = static gate scans 8 loader/read modules,
inventories fs/aliases/File/OpenOptions/.open/.join/CreateFileW/NtCreateFile, 14 classified/0
unclassified, planted opens detected (source.rs:1014/1099/1127). NOTE: F2 fix here ALSO closes
facs-wire r3 F4 (same file/root cause - the dedup). Lane self-review network-blocked (irrelevant).
ORCHESTRATOR-VERIFIED: replay_core 69/0(1 ign); snapshot 70/0 with ONLY the accepted Dev-Mode
symlink test excluded; scope clean (snapshot + replay_core + new dir_store/ + strict_json.rs).
regate-replayloader-r2 LAUNCHED (attacks fileID-reuse race on the revalidation reopen, retained-
handle-held-open requirement, any remaining serde_json::from_slice on snapshot bytes, deep/array-
element dup keys, gate fail-closed-on-unclassified, signal-through-frame-to-world.rs).
IN FLIGHT: autoattach r5 gate, replay-loader r2 gate, facs-wire fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 autoattach r5 gate = REVISE (R4-F1 CLOSED, R4-F2 material-proof leaks) -> fix5
R4-F1 (pipe client PID-binding to retained target) CONFIRMED CLOSED: same Arc<RetainedProcess-
Authority> for lease+hook+module-proof+pipe-auth; auth at main.rs:7042 BEFORE writer(7072) + first
ReadFile(7092); API-fail/PID-mismatch close handle accept no data; token CSPRNG-per-attempt (not a
secret - the PID check IS the auth; racing external proc can only DoS, cannot submit replay bytes);
PID-reuse closed by liveness/FILETIME check AFTER the PID query. No regressions (lease/journal/
authorities/DLL-hash/Drop all intact). R4-F2 (material recording proof) NOT closed - 3 findings,
ONE root cause = GENERIC interpretation of domain-specific DATA defeated per-domain (SAME recurring
shape as facs-wire witness + c1stageb deadlines: generic check over domain-specific structure):
- F1 HIGH d68: generic parser misreads presence(2)=row_count, DELTA_DATA status(1024)=changed_rows,
  playerID=status_flags => DELTA_DATA record passes w/ apparent status 0; fold reads status @off12
  and doesn't reject DELTA, folds dlcs=0, scalar-zero preserved => delta proves Recording.
- F2 HIGH d44: 5 MANDATORY computed BADGE rows (nested source/confidence/names/codes) survive
  empty-pruning even w/ ZERO real province/division/csupply rows => zero real data proves Recording.
- F3 MED d10: DATA record_flags not validated (PERF whitelisted @381, DATA not) => invalid DATA
  flag passes.
FIX5 launched (tid threads/w12g-autoattach-fix.tid): STRUCTURAL INVERSION = default-DENY. A domain
proves Recording ONLY via an explicit domain-correct real-material predicate; un-predicated domains
do NOT prove (keep waiting, don't accept, don't reject-as-invalid) => sound by construction, false-
negative safe. Plus correct predicates for d68 (read real status @off12, reject DELTA/failure,
require real dlcs), d44 (require real entity rows, exclude badges), d10 (validate DATA record_flags
like PERF). Plus residual re-word: in-process-compromise + simultaneous-injection = OUT-OF-THREAT-
MODEL (needs arch trust boundary / OS code-integrity), NOT "fail-closed". Mandates RED->GREEN for
d68/d44/d10/default-deny/genuine-real-record-still-proves + self-refute "name a domain that still
proves w/o real rows". ESCALATION: if r6 still finds a domain leak, ship default-deny AS-IS (sound;
un-predicated domains = "can't yet serve as recording proof" residual, not a defect). ONE fix round.
IN FLIGHT: autoattach fix5, replay-loader r2 gate, facs-wire fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 replay-loader r2 gate = REJECT (3 deeper Windows findings) -> fix3
R1-F2 (loader snapshot dup keys) CLOSED for snapshot read paths; R1-F1 valid-ID reuse + A->B
routing CLOSED. Error chain Store::load_file->ReplaySource::frame->WorldTimeline->world.rs correct,
canonical keys consumed by real build_session_metadata. NEW findings (genuine adversarial
deepening, not a recurring leak):
- F1 HIGH: Windows 128-bit file-ID SENTINELS accepted as valid identity (windows.rs:145,248,
  mod.rs:244). MS: all-ZERO = file IDs unsupported, all-ONES = uniqueness unestablishable; both
  must be IGNORED. Attack: placeholder reopen returns same sentinel for a swapped impostor ->
  validate_reopen_identity accepts it. FIX: zero/all-ones ID => identity UNAVAILABLE, fail closed.
- F2 HIGH: STREAMING verifier verify_replay_container_streaming (stream_verify.rs:121) still
  permissive serde_json::from_slice; recorder finalization (main.rs:4678) promotes on its result.
  Dup-role manifest (dup inserted after digest) decodes last-wins -> digest+hashes pass -> false-
  safe promotion. SAME dup-key root cause but a THIRD consumer (finalization) the strict signal
  didn't reach. FIX: route manifest through strict streaming parser IN stream_verify.rs so the
  recorder caller inherits it w/o editing recorder_host.
- F3 MED: F4 static gate open-world/bypassable (source.rs:1071,1127) - 8 hand-listed files (new
  module escapes) + discards everything after first #[cfg(test)] (production code after a test block
  escapes). FIX: closed-world module discovery (walk snapshot/src/** + replay_core/src/** prod
  files, fail if discovery incomplete) + scan past test blocks + fail-closed on unclassified.
FIX3 launched (tid threads/i1-replayloader-e2e.tid): sentinel-reject + strict-streaming-verify +
closed-world-gate; RED->GREEN for zero/all-ones reopen reject, dup-manifest-through-real-streaming-
verify reject, planted-open-in-new-module + post-test-block open both caught. Scope snapshot/* +
replay_core/*, do NOT edit recorder_host logic (caller inherits F2 fix). ONE fix round for r2.
IN FLIGHT: autoattach fix5, replay-loader fix3, facs-wire fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 facs-wire fix4 verified GREEN -> r4 gate launched (SOUND witness, not suppressed)
Achieved a SOUND positive witness (NOT the suppress fallback): F1 = ordered fingerprint over EVERY
counted slot incl vacant/unreadable/rejected/accepted/duplicate-tag (extractor.rs:444); positive
completeness REQUIRES full bulk-alloc capture + stable header + ZERO rejected/ambiguous + ZERO
duplicate tags (:475); duplicate tags POISON colliding slots, none enters accepted pointer set
(:1749); members must resolve to EXACT accepted CCountry* + matching live tag (:1538); clean null
slots stay measured-vacant NOT unavailable. F2 = outer root reread FIRST then COMPLETE inner-table
oracle recaptured as final nested obs, must match EXACTLY (:1450); fingerprint=base+alloc+count+
ordered identities+accepted+ambiguity (:1645); header 2nd read, short/changed fail-closed (:1791).
F3 = CASE (b) - NO production factions-map consumer exists in-tree (all country_faction_memberships
calls under #[cfg(test)], production only re-exports); documented owed residual (mapmode_factions.
rs:41), helper still rejects Unavailable (:148), strengthened REAL recording->typed->game-UI binding
test across all 3 states incl empty-under-Unavailable (diplomacy_binding.rs:171). F4 (dup JSON keys)
correctly NOT touched (fixed in replay-loader/snapshot lane). ORCHESTRATOR-VERIFIED: diplomacy_
intel_operatives_live 125/0, recorder_host 209 lib/199 bin(2 ign), replay_core 48/0, game_ui all
green, map_viewer 755+/0(1 ign); scope 7 files + Cargo.lock within the 8-file FACS fence.
regate-facs-wire-r4 LAUNCHED (attacks any residual witness-emit under ambiguity, inner-table-move w/
identical outer root, member tag-borrow, case-(b) truth via repo-wide production-caller grep, +
R2-F2 regression after the 343-line world.rs churn).
IN FLIGHT: autoattach fix5, replay-loader fix3, facs-wire r4 gate, foldwave2 fix4, stitching r3.

## 2026-07-14 autoattach fix5 verified GREEN -> r6 gate launched (default-deny recording proof)
Structural inversion landed: only d10/d44/d68 are proof-capable, EVERY other scheduled domain keeps
WAITING (proof.rs:493); DATA record_flags==0 whitelist for all 3 (:524); d10 status@off16 + real
fuel layout/rows/scale/identities + folded fuel material (:546); d44 status@off16 + counts real row
KINDS, EXCLUDES badges AND tombstones, requires nonempty folded supply.by_province (:594); d68 real
status@off12 + rejects delta/failure + production PERF shape + nonzero folded session field (:679).
Residual re-worded OUT-OF-THREAT-MODEL (in-proc compromise + simultaneous injection), not fail-
closed (proof.rs:245, main.rs:6959). Lane self-refute caught+closed a d44 badge-relabel + tombstone-
only edge. ORCHESTRATOR-VERIFIED: recorder_host 314 lib(2 ign)/207 bin(4 ign), 0 failed, RmStart-
Session flakes did not reproduce; scope 2 files clean. regate-autoattach-r6 LAUNCHED (attacks
default-deny fallthrough, each of d10/d44/d68 proving from empty/delta/failure/badge/tombstone/
diagnostic, record_flags semantic-change-yet-0, cross-domain/hour PERF reuse, + regression of
lease/pipe-auth/journal/authorities/DLL-hash/Drop after the 1454-line churn).
IN FLIGHT: autoattach r6 gate, replay-loader fix3, facs-wire r4 gate, foldwave2 fix4, stitching r3.

## 2026-07-14 facs-wire r4 gate = REJECT (witness SOUND, one last UI-seam leak) -> fix5
GOOD PROGRESS: R3-F1 (sound witness: fingerprint-all-slots + dup-tag-poison + pointer-validation)
CLOSED; R3-F2 (inner-table bound through final approval) CLOSED; R2-F2 (unavailable never->measured-
empty) regression CLOSED; case-(b) confirmed (0 production callers of country_faction_memberships,
8 test-only). ONE finding left:
- F1 CRITICAL: game_ui binding builds the `factions` LIST solely from countries.*.politics.faction
  (binding.rs:414) and emits it REGARDLESS of FactionRosterStatus (:483). Snapshot w/ modern status
  = unavailable + retained country politics.faction => returns Unavailable + factions_available=false
  but NONEMPTY factions list. Consumer reading the LIST (not the FLAG) gets stale positive data.
  signal-reaches-consumer at the SIBLING-FIELD level: tri-state reached the flag, not the list. Test
  (diplomacy_binding.rs:234) can't catch it - fixture has "countries":{} (:207) so nothing to leak.
FIX5 launched (tid threads/i1-facs-wire.tid): gate the `factions` LIST on status so flag+list can't
disagree - Unavailable/absent => EMPTY list, only MeasuredNonEmpty => nonempty; strengthen test w/
a fixture having BOTH unavailable status AND populated countries.*.politics.faction, drive REAL
loader->WorldTimeline->production binding, assert empty factions + RED-on-HEAD/GREEN-after, plus a
positive MeasuredNonEmpty-still-populates case. Scope game_ui/src/binding.rs + tests/diplomacy_
binding.rs (in fence). ONE fix round for r4; witness itself is DONE.
IN FLIGHT: autoattach r6 gate, replay-loader fix3, facs-wire fix5, foldwave2 fix4, stitching r3.

## 2026-07-14 autoattach r6 gate = REVISE (structure CONFIRMED, d10 predicate weaker than siblings) -> fix6
CONVERGENCE not divergence: DEFAULT-DENY HOLDS structurally (only d10/d44/d68 proof-capable, all
else _=>false); d44 PASSES all attacks; d68 PASSES all attacks; record_flags==0 HOLDS; PERF<->DATA
binding INTACT; NO regressions (lease/pipe-auth main.rs:7051/journal/authorities/DLL-hash main.rs:
5956/Drop all confirmed intact). ONE finding:
- F1 CRITICAL d10: proves w/ ZERO real material. DATA tag_db_id=2049 (ABOVE the production 2048
  ceiling) + all-zero fuel (current/max/gain/cost=0) passes because d10 checks only nonzero/unique
  ID + nonnegative current/max, treats object EXISTENCE as material. Production caps IDs at 2048
  (build_profile.rs:45) + fuel validator rejects above (lib.rs:729) but proof mirrors neither.
FIX6 launched (tid threads/w12g-autoattach-fix.tid): bring d10 to d44/d68 rigor - (1) apply
production identity ceiling (reject tag_db_id>2048), (2) require GENUINELY nonzero fuel material
(all-zero fuel object != material), (3) validate gain/cost per production, (4) join ID to current-
hour identity if cheap. Keep existing d10 checks (status@off16/scale/nonzero-row/flags==0). RED->
GREEN for the EXACT 2049+all-zero payload (reject) + positive in-range+real-fuel (still proves).
Scope proof.rs d10 only. ONE fix round for r6; d10 is the last seam, other two domains + structure
DONE.
IN FLIGHT: autoattach fix6, replay-loader fix3, facs-wire fix5, foldwave2 fix4, stitching r3.

## 2026-07-14 replay-loader fix3 verified GREEN (stale out-file caught!) -> r3 gate launched
OPS NOTE: the lane's -o out-file was STALE (echoed the fix2 summary verbatim). The CODE had
fully landed - caught by verifying the worktree directly instead of trusting the report
(verify-codex-work law paying off). Confirmed present by direct read:
- F1 sentinel IDs: ObjectId::from_windows_file_id (dir_store/mod.rs:194) REJECTS all-zero AND
  all-ones at construction; windows_file_id_is_unavailable (:213) + validate_reopen_identity
  (:270) reject sentinel on inspected OR reopened side; test
  windows_unavailable_file_id_sentinels_cannot_bind_a_reopen (:415). Fail-closed at BOTH layers.
- F2 streaming verify: stream_verify.rs:122 parses the manifest via crate::strict_json::from_slice
  BEFORE trusting digest/entry hashes; dup manifest entry rejected (:175); recorder finalization
  caller (main.rs:4678) inherits w/o recorder_host edits.
- F3 closed-world gate: fn walk + fs::read_dir discovery (source.rs:1097-1160) over the source
  dirs; classified_root_or_handle_site/classified_non_loader_site; fails on unclassified.
ORCHESTRATOR-VERIFIED: snapshot 72/0 (only accepted Dev-Mode symlink excluded), replay_core 70/0,
recorder_host BUILDS clean (RH_BUILD_EXIT=0). regate-replayloader-r3 LAUNCHED (prompt warns the
reviewer the out-file was stale, review the ACTUAL diff; attacks: every ID path routed through
from_windows_file_id incl FileIdExtdDirectoryInfo enumeration, strict manifest = the REAL bytes
finalization validates, walk truly closed-world + scans past #[cfg(test)], + regressions).
IN FLIGHT: autoattach fix6, replay-loader r3 gate, facs-wire fix5, foldwave2 fix4, stitching r3.

## 2026-07-14 autoattach fix6 verified GREEN -> r7 gate launched (d10 last seam)
d10 predicate brought to d44/d68 rigor (proof.rs:550-595,750): every ID 1..=2048 production ceiling
(:584); production fuel-present row flag + emission bounds current/max 0..=1e9, gain/cost +-1e9 at
scale 32768 (:587-591); >=1 genuinely NONZERO current/max/gain/cost across d10 rows (:595); folded
proof needs 4 numeric fuel fields + >=1 nonzero (:750). Preserved status@16/scale/nonzero-row/flags/
baseline/PERF-binding. Req (4) current-hour identity join NOT added (no current-hour identity set in
proof context) - accepted, (1)(2)(3) hold. Lane RED 1/3->GREEN 6/0 incl the exact 2049+all-zero
leak. ORCHESTRATOR-VERIFIED: recorder_host 320 lib(2 ign)/207 bin(4 ign), 0 failed; scope 2 files
(main.rs in cumulative diff, fix6 touched only proof.rs). regate-autoattach-r7 LAUNCHED (attacks
out-of-range/all-zero/wrong-scale d10, ceiling on folded output, nonzero across real rows only, +
full regression of d44/d68/default-deny/record_flags/PERF-binding/pipe-auth/lease).
IN FLIGHT: autoattach r7 gate, replay-loader r3 gate, facs-wire fix5 verify->r5, foldwave2 fix4,
stitching r3.

## 2026-07-14 facs-wire fix5 verified GREEN -> r5 gate launched (last UI seam)
F1 closed: binding.rs:481 gates the `factions` LIST on exact MeasuredNonEmpty - all other statuses
emit Vec::new(). Behavior: Unavailable/absent = available=false + EMPTY; MeasuredEmpty = available=
true + EMPTY; MeasuredNonEmpty = available=true + genuine rows. Test fixtures (diplomacy_binding.rs
:170+) now have POPULATED GER/ITA country faction data across all 4 status cases; RED 1/1fail (leaked
Axis GER,ITA) -> GREEN 2/2; positive case green before+after (no false-negative). Lane self-review
sandbox-blocked (irrelevant). ORCHESTRATOR-VERIFIED: game_ui 139/0 (incl new tests), replay_core
48/0, map_viewer 755+/0(1 ign), diplomacy_intel_operatives_live 125/0, recorder_host 207 lib/199
bin(2 ign) modulo RmStartSession flakes. Scope 9 files within FACS fence, snapshot/src/lib.rs
untouched. regate-facs-wire-r5 LAUNCHED (attacks any alternate factions-list constructor/mutation
under non-MeasuredNonEmpty, test fidelity, + full regression of sound witness R3-F1/F2 + R2-F2 +
map-helper + stream-compat). Expected FINAL round if SHIP.
IN FLIGHT: autoattach r7 gate, replay-loader r3 gate, facs-wire r5 gate, foldwave2 fix4, stitching r3.

## 2026-07-14 replay-loader r3 gate = REJECT (2 findings) -> fix4
R2-F1 (file-ID sentinels) CONFIRMED CLOSED (both single-handle FileIdInfo + dir-enum route through
from_windows_file_id; sentinels can't become ObjectId; inspected handle live across reopen). Strict
dup-key DECODE confirmed fixed. Regression sweep clean (BoundDir A->B, recursive dup rejection all
artifacts, Windows namespace primitives, error propagation, canonical keys -> build_session_metadata
world.rs:2228). Two remain:
- F1 HIGH: verify->promote TOCTOU. verify_replay_container_streaming opens+CLOSES a handle;
  finalize_verified_replay (main.rs:4676/4692/5183) promotes via pathname MoveFileExW retaining
  NEITHER handle NOR file-ID. Post-verify a write-access process swaps an impostor (dup-role
  manifest) at the same pathname -> recorder promotes the UNVERIFIED object. => R2-F2 not closed
  END-TO-END (decode fixed, promotion unbound). Same verify-one/use-another family as stitching r2.
- F2 MED: F4 gate closed over MODULES but open over SITES. Walk covers all .rs but site recognition
  = substring allowlist; a new module w/ windows-sys PathBuf::push + CreateFile2 = zero classified
  sites, so classified==60 + empty-violations BOTH pass -> open invisible.
FIX4 launched (tid threads/i1-replayloader-e2e.tid): F1 = bind promote to verify - hold the verified
handle open w/ deny-write/delete share THROUGH promotion + rename-by-handle (SetFileInformationBy-
Handle FileRenameInfo), OR re-query target file-ID identity == verified handle immediately before
MoveFileExW, fail-closed on mismatch (AUTHORIZES a precise recorder_host/main.rs finalize edit -
F2 chain isn't closed without it). F2 = default-deny over sites - expand primitive detection
(CreateFile2/CreateFileW/NtCreateFile/PathBuf-push-open/windows-sys) + replace classified==N with a
FAIL-CLOSED invariant (a file-access-capable module w/ an unclassified open = FAIL). RED->GREEN for
post-verify-swap-not-promoted + CreateFile2-module-caught. ONE fix round for r3. ESCALATION: if r4
still finds a promotion-binding gap, orchestrator fixes F1 directly (bounded Win32 handle-lifetime).
IN FLIGHT: autoattach r7 gate, facs-wire r5 gate, replay-loader fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 *** autoattach r7 = SHIP *** (FIRST hard lane closed; 6 adversarial rounds r2->r7)
VERDICT SHIP, zero findings. R6-F1 CLOSED (d10: ID 1..=2048 ceiling @proof.rs:583, scale==32768,
row-flag==1, current/max 0..=1e9*32768, gain/cost +-1e9*32768, material bit set ONLY after full
validation from the 4 fuel magnitudes, all-zero rejected, folded check through production WorldFold
@720 requires 4 numeric fields + >=1 nonzero @750, genuine in-range nonzero still proves). ALL
regressions PASS: d44, d68, default-deny (_=>false both dispatches), record_flags==0, PERF<->DATA
binding, pipe PID/session auth, retained lease, journal/authorities, DLL mapped-image hash,
ChildWorkerHandle Drop. Residuals = only the acknowledged OUT-OF-THREAT-MODEL (in-proc compromise/
simultaneous injection) + user-gated live multi-domain capture. Gate couldn't run cargo (read-only
fence) but I INDEPENDENTLY VERIFIED fix6 green (320 lib/207 bin, 0 failed) before launching = full
standard met (my verify + independent adversarial SHIP). PRESERVED: committed fix1-6 on the lane
branch @15a817f3e (was 1590-line uncommitted diff). => autoattach is MERGE-READY, joins the 2nd
merge wave (with boundary-auth).
MERGE-READY now: boundary-auth (guards), autoattach (SHIP @15a817f3e).
IN FLIGHT: facs-wire r5 gate, replay-loader fix4, foldwave2 fix4, stitching r3.

## 2026-07-14 facs-wire r5 gate = REVISE (witness SOUND; sibling-fields still disagree OTHER direction) -> fix6
CONFIRMED CLOSED no-regression: sound witness R3-F1/F2 (extractor 125/125), R2-F2, map helper
(Unavailable->None), stream compat, case-(b). fix5 status-gating intact. ONE finding:
- F1 CRITICAL: the `factions` LIST + faction_roster_status are built from TWO sources -> disagree
  under MeasuredNonEmpty. Status from d53 faction_members (world.rs:444); list from faction_rows
  reading countries.*.politics.faction.name (binding.rs:414/419) - a SPARSE projection the fold
  leaves incomplete (world_fold.rs:3281,3303-3311: writes only leader/is_leader; d53 has only
  name_hash so politics.faction.name ABSENT). Real roster => MeasuredNonEmpty + factions_available
  =true + factions=[] = a real nonempty roster shows as ZERO factions = SILENT LOSS. fix5 (gate
  list on status) can't fix this - under MeasuredNonEmpty the list SOURCE is still empty.
FIX6 launched (tid threads/i1-facs-wire.tid): SINGLE SOURCE OF TRUTH - derive the `factions` LIST
from the SAME d53 roster the STATUS comes from (faction_members), NOT politics.faction.name. Co-
derived => (MeasuredNonEmpty + empty) AND (Unavailable + nonempty) both structurally impossible.
Authorizes threading the decoded roster members onto DiplomacyView via world.rs if not exposed.
Test must use a REAL fold where d53 has members but politics.faction.name ABSENT (removes the
hand-injected "name":"Axis" crutch at diplomacy_binding.rs:198), RED-on-HEAD/GREEN-after. This is
the DEFINITIVE fix (closes both directions), not another per-direction patch - same "fix the root
not the symptom" move as autoattach default-deny. ONE fix round for r5.
IN FLIGHT: facs-wire fix6, replay-loader fix4, foldwave2 fix4, stitching r3. MERGE-READY: boundary-
auth, autoattach (SHIP @15a817f3e).

## 2026-07-14 replay-loader fix4 verified GREEN -> r4 gate launched (expected FINAL)
Both r3 findings closed (out-file fresh this time; verified worktree directly regardless):
- F1 verify->promote TOCTOU (mechanism a): recorder opens temp FILE_SHARE_READ only (deny external
  write/delete/rename), holds THAT handle through strict verify + capture-hash compare (main.rs:4720),
  promotes via SetFileInformationByHandle(FileRenameInfo) on the SAME handle, NO source-path
  re-resolution (main.rs:5236,5266); other 2 finalization paths also hold the guard; test
  post_verify_pathname_swap_never_promotes_impostor (main.rs:28847) RED (impostor promoted) ->
  GREEN. recorder_host/main.rs authorized into scope for this.
- F2 F4 gate default-deny over SITES: expanded primitives (std::fs/File/OpenOptions/CreateFile/
  CreateFile2/A/W/Nt+ZwCreateFile/Nt+ZwOpenFile/OpenFileById/ReOpenFile, multi-line + whitespace-
  split + identifier-boundary, Path/PathBuf join+push); file-access-capable module w/ ZERO recognized
  sites FAILS (source.rs:1644); classified==60 REMOVED; RED CreateFile2 violations:[] -> GREEN all
  variants caught.
ORCHESTRATOR-VERIFIED: snapshot 72/0 (only accepted Dev-Mode symlink skipped), recorder_host 200
lib/200 bin(2 ign) 0-failed incl swap test, replay_core 71/0 incl F4 gate. Scope: snapshot/* +
replay_core/source.rs + recorder_host/main.rs (F1). regate-replayloader-r4 LAUNCHED (attacks deny-
share race window, rename collision/ReplaceIfExists, other finalization paths, unrecognized opens
CreateFileTransacted/mmap/tokio/alias, file-access-capable detection holes, + full regression).
IN FLIGHT: replay-loader r4 gate, facs-wire fix6, foldwave2 fix4, stitching r3. MERGE-READY:
boundary-auth, autoattach (SHIP @15a817f3e).

## 2026-07-14 facs-wire fix6 verified GREEN -> r6 gate launched (single-source, expected FINAL)
Structural fix: binding.rs:418 - ONE exhaustive match on view.faction_members produces BOTH status
AND rows: measured-empty map => MeasuredEmpty + [] (:420); nonempty => MeasuredNonEmpty + one row per
roster entry (:443); Unobtained/Absent => Unavailable + [] (:445). binding.rs:489 builds
factions_available + status + list from that single result - BOTH disagreement directions
structurally impossible. Country politics can only DECORATE an existing member's leader (cannot
create/suppress/reassign). Test (diplomacy_binding.rs:265,416) uses REAL d53 encoder + production
stream_fold -> open_recording -> WorldTimeline -> diplomacy_panel_from_frame; proves roster {1,3,22}
with ALL politics names ABSENT renders nonempty rows via authoritative roster identity "1"; RED
(MeasuredNonEmpty + []) -> GREEN. Residual (honest): no certified name_hash->display-name resolver
so binding shows numeric faction identity - COSMETIC (membership content preserved), not loss.
ORCHESTRATOR-VERIFIED: game_ui 139/0 (incl new tests), replay_core 48/0, plus prior map_viewer 779+/
d53 125/recorder 207+199. regate-facs-wire-r6 LAUNCHED (attacks any residual politics.faction.name
read that adds/removes a row, alternate constructor, numeric-residual = cosmetic-only check, test
fidelity, + full regression witness/R2-F2/map-helper/MeasuredEmpty-vs-Unavailable). Expected FINAL.
IN FLIGHT: facs-wire r6 gate, replay-loader r4 gate, foldwave2 fix4, stitching r3. MERGE-READY:
boundary-auth, autoattach (SHIP @15a817f3e).

## 2026-07-14 replay-loader r4 gate = REJECT (4 findings, deepest lane) -> fix5 + F4 scope ruling
Post-guard swap + rename-by-handle CONFIRMED closed; no regressions (sentinels, strict decode,
BoundDir, namespace, error prop, canonical keys all intact). But adversarial deepening found 4 more
(genuinely NEW/subtler Win32, not a recurring leak):
- F1 HIGH: permissive-share CREATION window + mapped-view attack. Temp created w/o explicit share
  (main.rs:5192) => Rust defaults share write/delete/rename; writer dropped + reopened by pathname
  (4716, 4335-6, 4527-8). Attacker in the pre-guard window maps the file WRITABLE, closes the map
  handle (view SURVIVES per MS), mutates bytes AFTER hashing; deny-share guard doesn't revoke the
  view => rename-by-handle promotes mutated bytes. FIX: writer itself deny-share (FILE_SHARE_READ
  only) from ORIGINAL creation, deny-share handle held CONTINUOUS creation->verify->rename, no
  reopen-by-pathname gap (all 3 finalization paths); atomic identity-bound handoff if unavoidable.
- F2 HIGH: capture hash not bound to ONE canonical entry. verify_replay_container_streaming ignores
  game_id (stream_verify.rs:231), overwrites report for EVERY stream-role entry last-wins (:249).
  Decoy entry w/ same role holding genuine bytes ordered LAST => report ends w/ decoy hash, compare
  passes, canonical stream is malicious. FIX: require EXACTLY ONE role entry AT canonical path
  (bind game_id + payload/<game>/collector_raw/collector_stream.bin); >1 / non-canonical / zero =
  FAIL.
- F3 MED: recorder finalization read_session_manifest_bytes_validated (main.rs:4847) permissive
  serde_json => dup-nested-modset last-wins passes, but LOADER rejects (source.rs:362) => finalize
  and load DISAGREE. FIX: strict_json-decode the recorder manifest in finalization.
- F4 MED: ORCHESTRATOR RULING = OUT-OF-SCOPE OVER-REACH. The F4 static gate is a DEV-TIME LINT for
  honest introduction of a pathname open in OUR OWN code, NOT a runtime security boundary; a text
  scanner can't defeat renamed FFI (#[link_name])/macros/generated code (not Turing-complete, not
  in threat model). Required only: cheap extern/link_name file-API detection + an explicit
  accepted-limitation note. Do NOT chase completeness.
FIX5 launched (tid threads/i1-replayloader-e2e.tid): F1 deny-share continuity + F2 canonical-entry
binding + F3 strict recorder manifest + F4 note. RED->GREEN for deny-share/decoy-role/dup-modset/
link_name. 4th REJECT but findings are distinct adversarial-deepening (mapped views, decoy entries),
NOT a recurring leak - each narrower than the last. THREAT-MODEL NOTE: F1/F2 attacks need a hostile
SAME-USER process racing the recorder on the user's own machine; F1/F2/F3 fixes are correct
regardless (verification must bind to promoted bytes = core lane purpose). ESCALATION: if r5
surfaces YET another same-user-race class, raise the threat-model scope question w/ the user (is
same-user-attacker hardening of replay finalization in scope?) rather than grind - that's a scope
decision, not technical.
IN FLIGHT: replay-loader fix5, facs-wire r6 gate, foldwave2 fix4, stitching r3. MERGE-READY:
boundary-auth, autoattach (SHIP @15a817f3e).

## 2026-07-14 facs-wire r6 gate = REVISE (R5-F1 CLOSED structurally; only test-infra) -> orchestrator-fixed test path -> r7
R5-F1 (single-source list+status) CONFIRMED CLOSED STRUCTURALLY: binding.rs:417 one exhaustive
faction_members match, disagreement impossible both directions, politics only decorates leader, no
alternate constructor/serde/mutation, typed parse rejects empty/dup/cross-faction/invalid-leader/
contradictory before a measured map, numeric identity cosmetic (members {1,3,22} preserved), test
uses real d53 encoder+stream_fold+open_recording+WorldTimeline+binding + would fail pre-fix. ALL
regressions intact (witness, R2-F2, map helper, stream compat, MeasuredEmpty-vs-Unavailable). ONLY
finding = MEDIUM TEST-INFRA: world_timeline.rs:667 hardcoded extractor/native/target/tmp for nested
cargo TEMP/TMP -> fails under read-only source fence even w/ external CARGO_TARGET_DIR, so the gate
couldn't run the current UI suite (=> couldn't SHIP despite the code being sound - I'd already
verified game_ui 139/replay_core 48 green on my env). ORCHESTRATOR-FIXED DIRECTLY (trivial test
hygiene, not a Codex round): world_timeline.rs:667 now root.join("extractor_tmp") (writable, matches
game_ui harness) + comment. VERIFIED: the nested-cargo production_encode_decode_fold_typed test now
PASSES (1/0) under external target. regate-facs-wire-r7 LAUNCHED (confirm R5-F1 + test-path = temp-
only change + full regression + suite now runs green). Expected FINAL/SHIP.
IN FLIGHT: facs-wire r7 gate, replay-loader fix5, foldwave2 fix4, stitching r3. MERGE-READY:
boundary-auth, autoattach (SHIP @15a817f3e).

## 2026-07-14 *** facs-wire r7 = SHIP *** (SECOND hard lane closed; 5 adversarial rounds r3->r7)
VERDICT SHIP, zero findings. R5-F1 CLOSED (single-source co-derivation binding.rs:417/489,
disagreement impossible both directions, politics only decorates leader, no alternate constructor/
serde/mutation). Test-path fix confirmed temp-only. ALL regressions PASS: witness R3-F1/F2 (complete
stable oracle + every-member validation + double-read + outer-oracle recapture), R2-F2 (unavailable
never->measured-empty world.rs:3169), fail-closed/zero-loss, map helper (Unavailable->None,
MeasuredEmpty preserved), stream compat v1/v2, MeasuredEmpty-vs-Unavailable distinguishable through
fold->typed->binding->map. Residuals = user/consumer-gated only: live FACS v2 injected capture, owed
live-map integration test (case-b, no production consumer yet), cosmetic name_hash display-name
resolver. Gate couldn't run cargo (read-only fence) but I INDEPENDENTLY VERIFIED green (d53 125/0,
game_ui 139/0, replay_core 48/0, map_viewer 755+/0, recorder 207+199, nested-cargo prod test 1/0) =
full standard met. PRESERVED: committed fix3-6 + test-path on the lane branch @1da8812b8.
=> facs-wire MERGE-READY, joins the 2nd merge wave.
MERGE-READY now: boundary-auth (guards), autoattach (SHIP @15a817f3e), facs-wire (SHIP @1da8812b8).
IN FLIGHT: replay-loader fix5, foldwave2 fix4, stitching r3.

## 2026-07-14 replay-loader fix5 verified GREEN -> r5 gate launched
All 4 r4 findings addressed (verified worktree directly incl F3 which grep flagged - confirmed
read_session_manifest_bytes_validated main.rs:4848 + pre-pack finalize 4864 BOTH use
snapshot::parse_json_strict): F1 deny-share continuity = temp create_new(true)+share_mode(FILE_
SHARE_READ only) at main.rs:5204-5, original handle held through pack/sync/verify/handle-promotion
in all 3 finalizers (4322/4509/4697), create_new = first appearance so no earlier writable mapping;
test 28937 concurrent write/delete fails. F2 canonical-entry = exactly one collector_compact_stream
role at payload/<game>/collector_raw/collector_stream.bin, zero/dup/non-canonical FAIL before hash,
sole hash from canonical path not iteration order (stream_verify.rs:196-213); test 1068 decoy
rejected. F3 strict recorder manifest via parse_json_strict; test 18837 dup-nested-modset rejected.
F4 extern/link_name detection (source.rs:1158) + accepted-limitation note (1647) per orchestrator
ruling; test 1810. ORCHESTRATOR-VERIFIED: snapshot 72/0 (Dev-Mode symlink skipped), recorder_host
200 lib/202 bin(2 ign) 0-fail incl all new tests (RmStartSession flakes didn't reproduce),
replay_core 72/0. Scope: snapshot/stream_verify + replay_core/source + recorder_host/main
(finalization). regate-replayloader-r5 LAUNCHED (attacks any creation->promotion substitution/
mutation window incl CreateFileMapping-needs-write, canonical-path normalization tricks, remaining
serde_json on embedded manifest, + full regression). F4 = accept documented lint limitation.
IN FLIGHT: replay-loader r5 gate, foldwave2 fix4, stitching r3. MERGE-READY: boundary-auth,
autoattach (SHIP @15a817f3e), facs-wire (SHIP @1da8812b8).

## 2026-07-14 replay-loader r5 gate = REVISE (converging: R4-F2/F3/F4 CLOSED) -> fix6
R4-F2 (canonical-entry hash), R4-F3 (strict manifest decode), R4-F4 (lint) all CLOSED. No
regressions. The STREAMING finalizer is ALREADY CORRECT (verifies tmp_file directly through the
retained handle, main.rs:4729) = reference pattern. 2 HIGH left, same "verify/decision must bind to
the promoted object" class in the TWO OTHER finalizers:
- F1 HIGH: in-memory (4344) + file-backed (4536) finalizers retain the handle but VERIFY BY PATHNAME
  reopen. Writable reparse-ANCESTOR (junction J) rebind after packing => verifier inspects a decoy
  under a different dir while finalize promotes the original. FILE_SHARE_READ blocks leaf swaps +
  open-descendant blocks ancestor RENAME, but a reparse-point ancestor is unpinned. Streaming path
  immune (verifies handle directly).
- F2 HIGH: pack-time strict manifest bytes not bound to the decision manifest. Decisions from object
  A (game_id/config/capture/dest/static-artifacts) but embeds a REREAD disk manifest B (4271/4457/
  4635); nothing proves B==A => a concurrent valid-B swap yields a SEALED MIXED-SESSION container.
FIX6 launched (tid threads/i1-replayloader-e2e.tid): F1 = make the 2 non-streaming finalizers verify
THROUGH the retained creation handle (match streaming @4729) OR pin/reject ancestor reparse points,
fail-closed on identity mismatch; F2 = read the manifest ONCE, use that one object for BOTH decisions
AND embedding (or identity-bind the reread to A). RED->GREEN for reparse-ancestor-rebind + concurrent-
manifest-swap. CONVERGENT not divergent - the correct pattern already exists in the streaming path;
this applies it to the 2 lagging finalizers. NOT a threat-model escalation (same class, known fix).
IN FLIGHT: replay-loader fix6, foldwave2 fix4, stitching r3, + 2 DOC-AUDIT lanes (docaudit-refs,
docaudit-design). MERGE-READY: boundary-auth, autoattach (SHIP @15a817f3e), facs-wire (SHIP @1da8812b8).
