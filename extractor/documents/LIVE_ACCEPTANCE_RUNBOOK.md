# Live acceptance runbook — the remaining owner-run product gap

**Purpose.** Every offline/code acceptance gate the extractor owes is merged. What is left can only
be produced by a live the game process on the owner's machine(s). This runbook schedules **every
remaining owner-run live acceptance gate** into the **minimum honest number of the game launches** —
and, where a gate genuinely cannot be served by any live session today, says so plainly in the
OWED-PRODUCER register instead of hiding it inside a session.

**Authority.** This document is a schedule of owner-run sequences. It invents no acceptance
criteria: every gate cites its clause in `MASTER_EXTRACTION_CONTRACT.md`, the ladder script, a lane
packet, or a design doc. Where a doc is silent, the point is flagged **OPEN QUESTION**, never
fabricated.

**How to read this.** A **sequence** is a batch of related the game **launches** sharing one setup
(same save, same machine, same tooling warm). Launches inside a sequence are short; do the
sequences in order (1 → 5). The acceptance ladder is fail-closed and later rungs re-verify earlier
rungs' bound artifacts, so **do not delete any run directory between launches or sequences**.

---

## Standing realities before you start

### 1. ONE INJECTED CAPTURE PER the game LAUNCH (hard law)

Every injection entry — including a plain `--pid/--dll` CLI invocation — commits the exact target
identity and establishes a **cross-entry single-injection lease with kernel-retained history that
poisons the target after either worker exits** (`extractor/native/recorder_host/src/main.rs:1088-1094`,
`RetainedTargetLease`). A second injected capture attempt into the same the game process is refused
**even after the first worker exits**. Each acceptance-ladder rung creates a **fresh capture
attempt** in a fresh run dir (`Invoke-AcceptanceLadder.ps1:765-776`). Therefore:

> **One the game launch = at most ONE injected capture attempt.** Each ladder rung, and each other
> injected capture, gets its own game launch. **Read-only probes** (the D57 speed probe, and the
> future CID1 identity probe) use `OpenProcess(PROCESS_VM_READ)` only — they do **not** inject and
> may share a launch with one injected capture.

The launch counts in this runbook are computed under this law. Do not try to "save a launch" by
running two captures against one game process — the second attempt fails closed by design.

### 2. ACTIVATION FREEZE (owner law, in force)

No default-enable constant is flipped speculatively. **An activation flip happens only after the
owner has seen the corresponding gate pass** on this build. Until then the reader/path stays
`DEFAULT_ENABLED = false` / Legacy. See `MASTER_EXTRACTION_CONTRACT.md` §Current state (P1–P5
"Activation remains false, the active path remains Legacy") and `I1_D57_SPEED_EVIDENCE.md` §7
("Only after the paired live gate pass should another lane change
`DOMAIN57_GAME_SPEED_READER_DEFAULT_ENABLED`").

### 3. Fixed environment facts (do not hardcode anything else)

- HOI4 install (this machine): `<local-path> of Iron IV\hoi4.exe`.
- Accepted build SHA-256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
  (every probe/launcher rejects any other build).
- Windows-only. Everything else (user-dir, mods, output paths) is probed at runtime — no rig paths
  beyond the install above and this repo.
- **the game pause = exactly one cold frame.** Warm, advancing data needs the game unpaused and running
  (memory: the game capture pause semantics).
- **Never inject a second DLL over a shared hooked path.** The tooling stages a unique per-attempt
  DLL copy; combined with the single-injection lease above, this is why one launch carries one
  capture (memory: 2nd-injection crash; `AUTO_ATTACH_DESIGN.md` §5 "Unique DLL path").
- **Frame-budget criterion (authoritative):** the validator fails any capture whose
  `frame_time_us.max >= 100000` — i.e. the **maximum** frame time must be strictly under
  100,000 µs (100 ms) (`ProductionCapture.Validation.ps1:222-227`, problem key
  `frame_time_over_100ms`; `MASTER_EXTRACTION_CONTRACT.md` §Final user-gated acceptance proof).
  The ladder rung *description* says "p95 < 100ms" but the in-process validator enforces **max**;
  the validator is the authority.

### The workhorse save: "D04 1944.2.11.18"

Most sequences load the late-active-war paused oracle save the offline gates already use — player
tag `D04`, date `1944.2.11.18` (`EXTRACTION_ATLAS.md:12,143-149`). A 1944 game at war naturally
exercises naval missions/combat, subjects, and weather drift, so the soak launch harvests several
fixtures passively.

> **OPEN QUESTION (save availability).** The docs cite the D04 melt but name no loadable `.hoi4`
> save path. Confirm the owner still has the exact D04 1944.2.11.18 save to *load into a running
> game*; if only the melt survives, a fresh comparable late-war paused save must be produced first
> (and CID1 L0's exact seed/UUID equality in `MASTER_EXTRACTION_CONTRACT.md` P3 is only meaningful
> against *that* save's melt values).

---

## OWED-PRODUCER REGISTER (gates no live session can serve today)

These gates **cannot be closed by any owner play session right now** because a producer, reader,
or harness is missing on the tooling side. They are listed here once — sessions below reference
this register instead of pretending a soak closes them. Each row is orchestrator work; when its
producer lands, the gate slots into the sequence named in the last column.

| Gate | Missing producer / tool (why live play cannot close it today) | Source citation | Re-enters at |
|---|---|---|---|
| F6 rung 1 (synthetic-fault) | **SUPERSEDED 2026-07-20 — producer and independent proof regrade are wired.** The Safe launcher owns a fresh nonce-bound synthetic-fault mode and the ladder refuses an unbound or non-verifying proof. Only the owner-gated live launch remains. | `Invoke-AcceptanceLadder.ps1` `Get-RungBoundProducerOwed`; `MASTER_EXTRACTION_CONTRACT.md` F6 | Seq 1, launch 1.2 (live acceptance only) |
| F6 rung 2 (paused-short-proof) | **SUPERSEDED 2026-07-20 — producer, fold, extractor, and independent prover are wired.** `Start-Hoi4ProductionCaptureSafe.ps1` launches the nonce-bound paused mode, extracts only index-authorized replay snapshots, and requires `verify_f6_pause_proof.py` to join a known D57 pause with a same-snapshot non-D57 `compact_status_failure`. The resolved attempt marker retains the hash-bound proof receipt. Only the live launch remains. | `Start-Hoi4ProductionCaptureSafe.ps1`; `extract_f6_replay_snapshots.py`; `verify_f6_pause_proof.py` | Seq 1, launch 1.3 (live acceptance only) |
| F6 graded rungs 3–6 | **SUPERSEDED 2026-07-20 — the evidence-pin set, Safe-launcher handoff, distinct-attempt checks, and in-process regrade are wired.** Rung 6 now forces a request of at least 21,600 seconds, requires both wall and monotonic elapsed telemetry to cover the complete window, and publishes a manifest/plan-bound soak receipt. Only the ordered live observations remain. | same; `Start-Hoi4ProductionCaptureSafe.ps1` param block | Seq 1 launches 1.4–1.6, Seq 2 (live acceptance only) |
| F6 rung 5 constraint | **SUPERSEDED 2026-07-20 — live affinity application/readback/restore is wired.** The ladder applies the six-core mask to the pinned game process before capture, binds PID and creation FILETIME to the capture plan, and revalidates the retained proof. | `Invoke-AcceptanceLadder.ps1` `Set-ProcessAffinityFailClosed` / `Test-LadderAffinityProof` | Seq 1, launch 1.6 (live acceptance only) |
| A1 hub motorization LEVEL (d62) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** d62 schema v2 carries exact inner actor/level attribution; the narrower node-membership-only citation is stale | `RE_REMAINING.md` A1; `production_promotion/contract.json` A1 packet | future soak (live grading only) |
| A2 focus-paused bit (d45) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** exact `CNationalFocusProgress+0xB0` pause state is wired and verified; the fail-closed-badge citation is stale | `RE_REMAINING.md` A2; `production_promotion/contract.json` A2/A12 packet | future soak (live grading only) |
| A12 decision targets/cooldowns (d45) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** exact target union and both cooldown vectors are wired and verified; the excluded-slice citation is stale | `RE_REMAINING.md` A12; `production_promotion/contract.json` A2/A12 packet | future soak (live grading only) |
| A4 air-combat lifecycle (d42) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** exact stored country/region/date/mission history is wired; persistent wing pairing and reinforcement-source enum are proven absent and fail closed; the `ROOT_DISCOVERY_OPEN` citation is stale | `RE_REMAINING.md` A4; `production_promotion/contract.json` A4/A21 packet | future soak (live grading only) |
| A21 per-entity killer label | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** typed damage causes plus dedicated sunk-convoy history (d70) are wired | `RE_REMAINING.md` A21; `production_promotion/contract.json` A4/A21 packet | future soak (live grading only) |
| A14 construction repair (d48) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** exact line `+0x78` repair counter, building `+0x48` partial health, reason enum 0..5, and typed location pass the joint matrix; the health-rows-only citation is stale | `RE_REMAINING.md` A14; `production_promotion/contract.json` A14 packet | future soak (live grading only) |
| A22 air-loss 24-channel (d61) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** schema v2 carries all 24 field/label/divisor mappings; the six-channel citation is stale | `RE_REMAINING.md` A22; `production_promotion/contract.json` A22 packet | future soak (live grading only) |
| A15 MP chat/hotjoin/tag-switch (d57) | **SUPERSEDED 2026-07-19 — producer landed, promotion-ready:** the recorder fold now projects the four promoted A15 kinds (`Hotjoin`/`TagSwitch`/`Chat`/`SystemMessage`) under `_r2_session_multiplayer_metadata_live` (replay-boundary patch COMPLETED / PRODUCTION ADMITTED), and the `--expect-text` chat-oracle probe exists (`probe_870ebf_session_event_roots.py`, `PROBE_RUNSHEET.md`); the reject/ignore and "neither built" citations are stale. The live MP oracle run itself remains Seq 5's job | `RE_REMAINING.md` A15; `production_promotion/contract.json` A15 packet; `domains/d57_session_multiplayer_metadata_live.md` | Seq 5 (live grading only) |
| CID1 all arms (L0, S1–S4, D1–D2, M1–M3) | **STATIC PRODUCT ROUTE DONE; LIVE EVIDENCE OWED.** The owner-runnable read-only identity probe is merged. The collector sequence-0/CFA1 barrier, CRP1/CRA1/new-CID1/second-CFA1 rollover, recorder durable logical-stream store, ordered package-set publication, and replay selection are composed. The four independent duplex/rollover activation latches remain false, so an owner-authorized evidence build/run is still required for S/D/M and rollover lifecycle receipts; L0 needs only the probe + melt values. | `CAMPAIGN_IDENTITY_DESIGN.md` §Validation plan; `CAMPAIGN_ROLLOVER_TRANSACTION_CORRECTIONS_20260720.md`; probe at `extractor/native/scripts/*campaign_identity*` | Seq 1 (L0), 3 (S/D), 5 (M) |
| D57 criterion 4 (emission comparison) | A harness comparing the collector's **explicit opt-in** d57 emission against the raw words on the same transitions ("no coercion and no accepted out-of-range value") — no such comparison tool is named in-tree | `I1_D57_SPEED_EVIDENCE.md:322-334` (criterion 4) | Seq 1, launch 1.1 |
| P4 stitching live driver | Production recorder has **no call site** for pre-fold segmentation / per-segment folds / durable append tickets / atomic stitched publication | `PRODUCT_SESSION_SEMANTICS.md` §4 | Seq 3 |
| P1 packaging/graceful-quit | `collector_record --auto-attach` and the resident supervisor **exist** (see Seq 4); what is missing is the packaging/tray UI with graceful quit — the supervisor currently "runs until the process is terminated" (design §9, outside that lane) | `extractor/native/recorder_host/src/main.rs:832-922`; `AUTO_ATTACH_DESIGN.md` §9 | Seq 4 |
| P2 duplex activation | Recorder and collector H4CT/CID1 clients are composed, but `AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED`, `DUPLEX_CONTROL_CLIENT_ENABLED`, and both rollover live-evidence latches remain false; permit construction/capability negotiation therefore fail closed | `extractor/native/recorder_host/src/auto_attach/duplex_worker.rs`; `extractor/native/collector/src/duplex_client.rs`; `PRODUCT_SESSION_SEMANTICS.md` §5 | Seq 4 |
| Idle-cost measurement harness | A self-timing harness that runs each 30-minute no-game window and records snapshot count / CPU / wakeups / private bytes / WMI reconnects itself — so the owner starts it and walks away instead of hand-timing the windows (six of them: three per machine) | `AUTO_ATTACH_DESIGN.md` §10 Idle-cost gate (bounds defined; harness unnamed) | Seq 4, 5 |

---

## Summary — sequence / launch plan

| # | Sequence | the game launches | Gates served (once owed producers land) | Machine | Est. owner time |
|---|---|---|---|---|---|
| 1 | Speed evidence + short-proof rungs | **6** (1.1 D57+L0; 1.2–1.6 rungs 1–5) | D57/A3 raw transcript + opt-in emission comparison; CID1 **L0**; F6 rungs 1–5; S9 static bundle (rides rung 3's capture) | Main PC | ~2–2.5 h attended (graded launches 1.4–1.6 each repeat the D57 transcript for their own PID) |
| 2 | Six-hour soak + in-capture fixture harvest | **1** (+ post-run script, no launch) | F6 rung 6; combined speed-5 proof; FullGame size projection (separate script); passive A8, A9, A20 (+A5/A6/A7 and optional A16 regression opportunistic) | Main PC | ~30 min active (incl. this launch's D57 transcript) + ~6 h unattended |
| 3 | Campaign-identity save/reload/relaunch matrix | **2** (3.1 in-process arms; 3.2 relaunch arm) | CID1 **S1, S2, S4, D1, D2** (launch 3.1), **S3** (launch 3.2); P4 stitching seam evidence | Main PC | ~60 min attended |
| 4 | Auto-attach live matrix + idle-cost | **~12 short launches** + **3 no-game idle windows** (30 min each) | **P1** §10 live-Windows matrix, main-PC arms (incl. non-default Steam drive, Known-Folder/OneDrive Documents, access-denied/different-session) + main-PC idle-cost (3 windows); **P2** duplex liveness (blocked on activation) | Main PC | ~2.5–3 h spread + 3×30 min unattended |
| 5 | **BLOCKED on 2nd machine + 2nd player** | **15 launches across 2 machines** (MP 3: M1 host+client, M2 reuses that session, M3 one rehost launch; + the 12-launch weaker-PC matrix) + **3 no-game idle windows (weaker PC)** | **A15** (producer landed 2026-07-19 — see register; live oracle run here); CID1 **M1, M2, M3** (recorder on BOTH machines); auto-attach **complete weaker-PC matrix** (~12 launches — `AUTO_ATTACH_DESIGN.md` §10 requires the full matrix on both machines) + weaker-PC idle-cost (3 windows, no game running, **not** during MP) | Main + 2nd PC | ~half a day, two operators |

Total: **~36 the game launches** plus **6 no-game idle windows (3 per machine)**. That is the honest
count under the one-injection-per-launch law and the both-machines matrix requirement; there is no
way to compress injected captures below one per launch.

> Ladder order: rungs are fail-closed, ordered synthetic-fault → paused → speed5 → five-minute →
> weak-pc → soak. Launches 1.2–1.6 do rungs 1–5; Sequence 2 does rung 6. Rung 6 re-verifies rungs
> 1–5 against their **bound run directories**, so keep every run dir on disk
> (`Invoke-AcceptanceLadder.ps1` `Test-PredecessorsPassed` / `Test-RungBoundVerdict`).

---

## Sequence 1 — Speed evidence + short-proof rungs (main PC, D04 1944 save, 6 launches)

**Serves:** D57/A3 (raw transcript + opt-in emission comparison) · CID1 L0 · F6 rungs 1–5 · S9.
**Est. owner time:** ~2–2.5 hours attended (launches 1.4–1.6 each add a ~10–15 min per-launch D57
transcript — see the note before the launch table).

### Preconditions (whole sequence)

- Main dev PC. Run the root-owned centralized `Release` for the extractor or
  all workspaces first. It writes the five release binaries under
  `.cargo-target/release` plus the hash-bound
  `hoi4-production-prebuilt-artifacts.json` receipt. The Safe launcher never
  invokes Cargo and refuses missing, stale, source-mismatched, or byte-mismatched
  prebuilt artifacts; `-SkipBuild` is retained only for acceptance-ladder CLI
  compatibility.
- The D04 1944.2.11.18 save is available to load (see OPEN QUESTION above).
- Production profile: `vanilla_latest_2026_06_30.json` (`Start-Hoi4ProductionCaptureSafe.ps1`
  `$profilePath`). Zero active mods, 24 active DLCs is the expected S9 bundle resolution
  (`MASTER_EXTRACTION_CONTRACT.md` S9).
- **Static-disasm artifact (required by launch 1.1 and the graded-rung pins):** the pinned artifact
  (SHA-256 `789cd1d7d6f743d5699c1bb286100076b935a4cdcec41054886cfd4a3aaa6ebc`) is **UNTRACKED /
  gitignored** (`.gitignore:39` ignores `extractor/native/**/runs/`). It exists on the capture rig
  from the 2026-07-16 attempt at
  `extractor/native/runs/d04_speed_evidence_20260716/speed_root_static_disasm.json`, but is **not in
  a fresh clone**, and the verifier only accepts a **byte-identical** file. Locate it (or restore it
  from the rig) before starting; a regenerated file with a different hash fails the gate.

### Launch 1.1 — D57 speed/pause evidence + CID1 L0 (one opt-in emission capture)

**Prior-attempt status (honest):** the 2026-07-16 attempts produced **no sealed verified
transcript** — the first attempt failed the 5-second resume rule, the second was never completed.
The full D57 capture is therefore **owed in its entirety**. The raw `0..4` speed-mapping
observations from those attempts did match the static artifact, but they are **unsealed and
informational only** — they close nothing.

Steps:

1. Launch the game, load the D04 save, leave it **paused**.
2. **CID1 L0 (paused read, read-only — no injection).** Run the identity probe. L0's full
   criterion (`MASTER_EXTRACTION_CONTRACT.md` P3 "L0 paused D04"; `CAMPAIGN_IDENTITY_DESIGN.md`
   §Validation plan) is more than two pointer reads:
   - **running-image/root admission**: exact executable hash + validated `CGameState` root, with
     same-hour brackets around each observation;
   - **one coherent guarded read of `+0x720..+0x747`** (the seed i32 + the MSVC-string object as
     one consistent snapshot, malformed string state failing closed);
   - **exact oracle equality against the D04 melt**: seed `1026046527` and UUID
     `a4b17fed-ed12-4361-9064-fb6576638110`;
   - **two complete identical reads bracketing the first proposed capture boundary** (a 2/2 match
     only admits an exact-build candidate).
   *(OWED: the identity probe does not exist — see register. Skip until it lands; the rest of this
   launch is still valid.)*
3. **Start the launch's ONE injected capture: an explicit opt-in d57 emission run** (d57 scheduled
   by explicit opt-in — **not** a default flip; the reader stays `DEFAULT_ENABLED=false`). This is
   what lets criterion 4 compare the collector's emission against the raw words on the same
   transitions. *(OWED: the emission-vs-raw comparison harness — see register. Without it the
   opt-in capture still records; the comparison is graded when the harness lands.)*
4. **D57 raw transcript (read-only probe, runs alongside).** Exact invocations, all from the repo
   root, PowerShell. **Assign `$run` explicitly first.** Everything below was verified against
   master (subcommands, action tokens, nonce format, verifier flags, exe SHA):

   ```powershell
   # 0. Fresh run directory for this attempt (explicit assignment — nothing implicit).
   $run = "extractor/native/runs/d57_speed_evidence_$(Get-Date -Format yyyyMMdd_HHmmss)"
   New-Item -ItemType Directory -Path $run | Out-Null

   # 1. Mint a fresh evidence nonce (64 hex) and initialize the transcript once.
   #    --static-disasm must point at the pinned artifact described in Preconditions
   #    (byte-identical; SHA-256 789cd1d7d6f743d5699c1bb286100076b935a4cdcec41054886cfd4a3aaa6ebc).
   $nonce = python -c "import secrets; print(secrets.token_hex(32))"
   python extractor/native/scripts/collect_speed_root_evidence_v2.py init `
     --pid <hoi4.exe PID> --session-id d57_speed_<date> --evidence-nonce $nonce `
     --nonce-ledger-root $run/nonce_ledger --static-disasm <pinned static artifact path> `
     --out $run/transcript.json --action-token AUTHORIZE_READ_ONLY_SPEED_ROOT_EVIDENCE_INIT

   # 2. Speeds 1..5, in order, any pace: set the visible speed in game, then acknowledge.
   #    (Run once per N = 1,2,3,4,5.)
   python extractor/native/scripts/collect_speed_root_evidence_v2.py speed `
     --transcript $run/transcript.json --ui-speed <N> `
     --action-token SET_VISIBLE_UI_SPEED_<N>_AND_ACK

   # 3. The whole pause cycle is ONE invocation. Set visible speed 3, leave the game
   #    running, start the command, and follow its printed prompts: PAUSE whenever
   #    convenient, hold until it says it has enough, UNPAUSE whenever convenient.
   #    No stopwatch anywhere — the tool keeps a rolling paused-read buffer and detects
   #    the unpause itself (an accidental early unpause just restarts the window).
   #    NOTE: pause-cycle takes NO --read-count / --read-delay-ms flags (its pacing is
   #    fixed internally); it accepts an optional --timeout-s (60..3600, default 900).
   python extractor/native/scripts/collect_speed_root_evidence_v2.py pause-cycle `
     --transcript $run/transcript.json `
     --action-token RUN_PAUSE_CYCLE_AT_VISIBLE_UI_SPEED_3_AND_ACK

   # 4. Seal (bytes only; no verdict), then run the INDEPENDENT verifier.
   python extractor/native/scripts/collect_speed_root_evidence_v2.py seal `
     --transcript $run/transcript.json --action-token SEAL_TRANSCRIPT_WITHOUT_VERDICT
   python extractor/native/scripts/verify_speed_root_evidence_v2.py `
     --transcript $run/transcript.json --nonce-ledger-root $run/nonce_ledger `
     --expected-transcript-sha256 <seal-time transcript_sha256 field> `
     --expected-session-id d57_speed_<date> --expected-evidence-nonce $nonce `
     --expected-exe-sha256 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad `
     --expected-probe-tool-sha256 <sha256 of collect_speed_root_evidence_v2.py> `
     --expected-static-disasm-sha256 789cd1d7d6f743d5699c1bb286100076b935a4cdcec41054886cfd4a3aaa6ebc `
     --out $run/speed_root_gate.v2.json
   ```

   PASS = the verifier prints a `speed_root_gate.v2` report with `"verdict": "passed"`. Any failed
   or aborted step burns the evidence nonce fail-closed — mint a fresh nonce and start a new
   `init`; nonces are free, evidence retries are not hidden.
5. Quit the game (this launch's injection lease is spent).

**VERIFY — A3 / D10 (all four dossier criteria; `I1_D57_SPEED_EVIDENCE.md` §7):**

1. Visible 1→2→3→4→5 maps exactly and stably to raw `0→1→2→3→4`; raw `5` never observed — from
   the sealed, verified transcript.
2. Pause toggling changes only `CInGameIdler+0x6C1`, leaves `CGameState+0x4BC` unchanged across
   both edges — the pause-cycle samples.
3. `f32[+0x4B8]` behaves as a bounded normalized accumulator while unpaused; does not advance
   through the pacing path while paused.
4. **The collector's opt-in emission equals the raw words, no coercion, no out-of-range value
   accepted** — the opt-in capture from step 3, graded by the (OWED) comparison harness.

**A3 PASS — and default-enable eligibility — requires all four**, including criterion 4. The
python transcript alone does **not** satisfy the dossier; criteria 1–3 without 4 leave A3 open.

### Launches 1.2–1.6 — F6 rungs 1–5, one launch each

For each: launch the game, load the D04 save, put the game in the rung's required state, run the one
rung, quit. The paused-short-proof producer is wired and fail-closed; its remaining dependency is
the real launch and retained live artifacts, not hook engineering or a hand-authored sidecar.

**Per-launch D57 transcript (all Safe-launcher rungs 1.2–1.6 — and the Sequence-2 soak).** The Safe
launcher's admission gate rejects a speed gate whose recorded `pid` differs from the current
`$Hoi4Pid` (`Start-Hoi4ProductionCaptureSafe.ps1` speed-root admission — "speed-root v2 gate does not match
out-of-band pins"), and under the one-injection law every graded rung is a **fresh the game process**.
So launch 1.1's sealed transcript can only admit launch 1.1's capture. **Each Safe-launcher rung must
first repeat the launch-1.1 transcript procedure against its own PID** (fresh nonce → `init` →
speeds 1..5 → `pause-cycle` → `seal` → verify) before starting its rung capture, then set the
rung's game state. Launch 1.1's transcript serves A3 acceptance; the per-launch transcripts serve
each capture's admission pins. Budget ~10–15 min per Safe-launcher rung.

| Launch | Rung | Game state during capture | Rung-specific proof / notes |
|---|---|---|---|
| 1.2 | `synthetic-fault` | any | The Safe launcher records a fresh non-production acceptance nonce and the independent `verify_f6_synthetic_fault_proof.py` regrade must prove injected, observed, per-domain degrade, intact framing, and exact plan/attempt binding. The remaining action is the live rung and retained receipt. |
| 1.3 | `paused-short-proof` | **paused** | **First: this launch's own sealed D57 transcript.** The launcher generates a fresh nonce, records `acceptance_test_observation` as non-production, extracts index-authorized folded snapshots from the retained replay, and requires `verify_f6_pause_proof.py` to prove D57 `pause.state=known/is_paused=true` plus a same-snapshot non-D57 `compact_status_failure` joined to the manifest's SOURCE_UNAVAILABLE domains. The proof rejects a hand-authored manifest `pause_proof`. **Remaining action: run this live rung and retain the resolved attempt receipt.** |
| 1.4 | `speed5-short-proof` | advancing, **speed 5** | **First: this launch's own sealed D57 transcript** (see note above). Validator `ok:true`; **frame criterion = `frame_time_us.max < 100000` µs** (see §Fixed facts); zero skipped + zero coalesced hours. S9 rides this capture: the Safe path resolves live mod/DLC/load order, rebuilds all seven artifacts, finalizes token inventory **`unknown=0`**, and the exact bundle must survive pre-injection AND pre-publication revalidation (`MASTER_EXTRACTION_CONTRACT.md` S9, S6). Ladder pins are forwarded structurally to the Safe launcher. |
| 1.5 | `five-minute-full-roster` | advancing, speed 5 | **First: this launch's own sealed D57 transcript.** Same validator gates over 5 min full roster through the bound Safe-launcher path. |
| 1.6 | `weak-pc-six-core` | advancing, speed 5 | **First: this launch's own sealed D57 transcript.** Same gates under a 6-core game-process constraint. Pins must state `AffinityTarget=game`; the ladder applies mask 63, verifies the applied mask, retains an affinity receipt, and restores the original mask in `finally`. Recorder targeting is rejected rather than guessed. |

Every graded verdict comes from the in-process authoritative validator
(`Get-ArtifactGateVerdict` → `Test-ProductionCaptureArtifacts`), schema
`battle-plan.production_capture_validation.v2`, `ok` strictly boolean `true` — never a sidecar.

### Activation implications (post-pass, frozen)

- A3 pass (all 4 criteria) → a later lane may flip `DOMAIN57_GAME_SPEED_READER_DEFAULT_ENABLED`
  (wire semantic ratified: raw `0..4` canonical, displayed `1..5` derived —
  `I1_D57_SPEED_EVIDENCE.md` §7). Not this sequence's job.
- S9 pass → S9 closes; feeds the Production Gate's "admitted and unchanged seven-artifact static
  bundle" clause.

---

## Sequence 2 — Six-hour soak + in-capture fixture harvest (main PC, 1 launch)

**Serves:** F6 rung 6 · combined speed-5 proof · FullGame size projection ·
passive A8/A9/A20 (+A5/A6/A7 and optional A16 regression opportunistic).
**Est. owner time:** ~30 min active (including this launch's own D57 transcript), then ~6 h
unattended.

**What this soak does NOT close (moved to the OWED-PRODUCER register — do not expect them in the
stream):** A1, A2, A12, A4, A21, A14, A22. *(CORRECTION 2026-07-19: this narrower-slice claim is
stale — all seven producers have since landed promotion-ready (`RE_REMAINING.md` PROMOTION READY
rows; `production_promotion/contract.json`), so a soak on the promoted build CAN witness them; see
the updated register rows. The original text is retained below for the record.)* Their domains today
emit narrower slices than the gates
require (d62 node membership only, d45 fail-closed pause/targets, d42 deltas only, d48 health rows
only, d61 six pinned channels only). Performing those in-game actions during this soak would
produce **nothing gradable**; they re-enter a future soak after their readers land.

### Preconditions

- Rungs 1–5 PASS with run dirs intact (fail-closed predecessor re-verification).
- Enough free disk for a six-hour full-roster stream (the Safe launcher checks the safe floor and
  fails loudly rather than deleting bytes).

### Operator steps

1. Launch the game, load the D04 save.
2. **This launch's own D57 transcript first** (the soak is a graded capture; its speed-gate `pid`
   must equal this launch's PID — `Start-Hoi4ProductionCaptureSafe.ps1:141`; see the Sequence-1
   per-launch-transcript note): fresh nonce → `init` → speeds 1..5 → `pause-cycle` → `seal` →
   verify.
3. Put the product-selected deployment-profile file in the rung pins as
   `DeploymentProfilePath`, then start the **six-hour-soak** rung (`FullGame` capture mode). The ladder's actual Safe-launcher
   route explicitly supplies at least `-DurationSeconds 21600` and `-MinGameHours 48`; values below
   either minimum are rejected. **Wait until the tooling confirms it is recording** before relying
   on the run for any optional in-capture observation.
4. **Optional A16 regression only.** If extra empirical coverage is desired, change one subject's
   autonomy after recording begins. This does not gate production. The post-run variance verifier
   grades authenticated `subject_autonomy_current_rows` state/score observations; a change in the
   legacy `CSubjectRelation+0x60` row alone is insufficient.
5. Optional (A9 assist): order a fleet into a contested sea zone so a naval engagement begins and
   ends within the window (see OPEN QUESTION on naval-combat guarantee).
6. Set **speed 5** and walk away for the full window. The 1944 war produces naval missions/combat
   and weather drift passively.
7. After the run seals: run the size projection — **a separate, explicit invocation, not automatic
   and not a ladder rung** (`Get-FullGameSizeProjection.ps1:23-31,405-408`):

   ```powershell
   powershell -File extractor/native/scripts/Get-FullGameSizeProjection.ps1 -RunDir <soak run dir>
   ```

   Output: `<soak run dir>\full_game_size_projection.json` (default projection horizon 105,192
   game hours = a 1936→1948 campaign; override with `-FullGameHours`).

### VERIFY

| Gate | Acceptance criterion (citation) | PASS |
|---|---|---|
| **F6 rung 6** six-hour-soak | Ladder rung `six-hour-soak`; Production Gate | In-process validator `ok:true` (max frame < 100,000 µs; zero skipped + zero coalesced hours; hourly met), bound to this rung's run_dir; predecessors 1–5 re-verified. `six_hour_soak_acceptance_receipt.json` must bind the selected deployment-profile leaf/hash and exact plan/manifest hashes, a >=21,600-second request, >=21,600,000,000 µs wall and monotonic observations, and positive retained resource/stream measurements. |
| **Combined speed-5 proof** | `MASTER_EXTRACTION_CONTRACT.md` §Done means; Production Gate | Soak PASS completes the combined proof rungs 3–5 began. |
| **FullGame size projection** | `MASTER_EXTRACTION_CONTRACT.md` F6 (final ladder item); `Get-FullGameSizeProjection.ps1` | The explicit script run above emits `full_game_size_projection.json` with a bounded projection. |
| **A16 optional regression** (d53) | `H_A16_LIVE_MATRIX_GATE_20260720.json`; `verify_acceptance_fixture_variance.py` schema v2 | Two consecutive clean complete AUST observations contain the same subject whose current state or signed score changes. The verifier independently requires exact u32/i64 widths, checked score/bounds ratio arithmetic, base keys, and distinct state identities; an unavailable observation breaks continuity. Legacy relation-only variance is rejected. This row is informational and does not gate promotion. |
| **A8** naval missions (d56) | `MASTER_EXTRACTION_CONTRACT.md` A8 | Passive: fleets on mission populate d56 mission enum/home base/repair/retreat/route. |
| **A9** naval combat lifecycle-end (d49–50) | `MASTER_EXTRACTION_CONTRACT.md` A9 | Requires a naval battle that **ends** in-window (see OPEN QUESTION). |
| **A20** weather drift (d24) | `MASTER_EXTRACTION_CONTRACT.md` A20 | Passive: 6 h of advancing weather drifts region phenomenon flags. |

Opportunistic if they occur in-window: **A5** (ace kill_type/wounded, d46), **A6** (convoy
disrupted-supply pressure, d44), **A7** (convoy combat loss route, d44). Not guaranteed; do not
block on them.

### Activation implications

- Soak pass → the Production Gate's combined-proof clause and `Done means` combined proof are
  satisfied. The harvested fixtures move their domains toward `CONCEPT_COMPLETE` **only for fields
  an asserting gate actually graded** (Completion Rule).

---

## Sequence 3 — Campaign-identity save/reload/relaunch matrix (main PC, 2 launches)

**Serves:** CID1 S1, S2, S4, D1, D2 (launch 3.1) · S3 (launch 3.2) · P4 stitching seam evidence.
**Est. owner time:** ~60 minutes attended.

**Every arm here remains owner-run evidence** (see register). The identity probe and complete
CID1/rollover product route are present, but the independent recorder, collector, and rollover
promotion latches remain false. The steps below are the acceptance plan for an explicitly
authorized evidence build; no normal product build may mint `Known` authority before it passes.

### What one arm actually requires (the pair standard — not just two pointer reads)

Per `CAMPAIGN_IDENTITY_DESIGN.md` §Validation plan (:387-405), **each arm is a capture pair**, and
each pair must retain:

- **both streams/manifests** (arm A and arm B), captured with CID1 emission active;
- **same-tick melted-save values** for both sides (save at the measured tick; melt via the rakaly
  pipeline; the live raw values must **exactly equal** the same-tick save values);
- build/modset/process provenance, role, and the **first/last stable identity observations**;
- **two stable reads bracketing admission** on each side (no world record admitted before the
  bracket closes);
- the **first record of each stream decodes as CID1 and is hash-bound to the manifest**;
- **no data precedes the header and no row crosses an identity change**;
- missing/torn/conflicting reads produce **counted failures**, never a guessed ID.

A reload inside one process is handled by the rollover state machine within the **single**
injected capture (logical streams roll at the acknowledged boundary) — which is what makes the
multi-arm launch 3.1 possible under the one-injection law.

**Acceptance-only opt-in — the plan must not be circular.** The ACTIVATION FREEZE
(`MASTER_EXTRACTION_CONTRACT.md` P3: "Do not add a production reader, build-profile fallback, CID1
activation, or live manifest authority until every matrix arm below passes") forbids using the
*production* CID1 path to produce this very matrix. Every CID1-emitting capture in Sequences 3
and 5 is therefore an **acceptance-only, explicitly opted-in instrumentation run** — not a
production-reader activation, not a build-profile fallback, and its manifests mint no production
`Known` authority. The production flip happens only *after* the owner sees the whole matrix pass.

### Launch 3.1 — in-process arms (S1, S2, S4, D1, D2)

1. Launch HOI4. Start the launch's one **acceptance-only opt-in** CID1-emitting evidence capture.
   Load the D04 save.
2. **S1** — save, then reload that exact save (same process). Expected: pair/digest **equal**;
   lower/equal hour may only begin a new segment; no uncertain frame assigned to the old identity.
3. **S2** — reload an **older** save from the same lineage (save-scum to a lower hour). Expected:
   pair **equal** despite rollback; new segment, same campaign.
4. **S4** — rename/copy the measured save (file-explorer action while the game is at menu), load
   the copy. Expected: pair **equal**; path/filename irrelevant.
5. **D1** — create two brand-new campaigns with identical build/modset/bookmark/countries, saving
   each. Expected: raw pairs and digests **differ**; any whole-pair alias **blocks activation**.
6. **D2** — load measured campaign A, then campaign B, in this same process. Expected: identity
   changes **before B's first world row**; no row crosses the boundary.
7. For every arm: take the same-tick save for the melt comparison before moving on.

### Launch 3.2 — the relaunch arm (S3)

1. Close the game **entirely** (this is the arm's defining action). Relaunch, start a fresh
   **acceptance-only opt-in** CID1-emitting capture (new injection, new process), load the same
   save.
2. **S3** expected: pair **equal** across process and d68-session change.

### VERIFY

Each arm passes only when the full pair standard above holds (both streams/manifests, exact
live-vs-melt equality, brackets, hash-bound CID1 first record, no boundary crossing) **and** the
arm's equal/differ rule holds. **Failure matrix (all arms):** unreadable/malformed/torn = explicit
`unavailable`; one-component change = `identity_component_conflict`; a stable whole-pair change
stops admission pending an acknowledged rollover; digest equality with unequal raw components fails
closed (`MASTER_EXTRACTION_CONTRACT.md` P3).

**Stitching (P4).** S1 (reload) and S3 (relaunch) are exactly the seams the stitching planner must
segment: rollback creates a **new segment**, superseded spans are retained as rollback segments
(zero-data-loss), the later-played branch is the main timeline (`PRODUCT_SESSION_SEMANTICS.md` §4).
**OWED:** the production stitch call site (register) — until it lands, P4 gets seam *evidence* from
these launches, not a produced stitched container.

### Activation implications

- The full matrix (L0 + S/D here + M in Sequence 5) gates **CID1/IdentityV1 activation** and,
  transitively, **stitch activation** — both frozen until every arm passes
  (`MASTER_EXTRACTION_CONTRACT.md` P3, P4).

---

## Sequence 4 — Auto-attach live matrix + idle-cost (main PC, ~12 short launches + 3 no-game windows)

**Serves:** P1 §10 live-Windows matrix (main-PC arms) + main-PC idle-cost · P2 duplex liveness
(blocked).
**Est. owner time:** ~2.5–3 h spread over the arms + 3×30 min unattended no-game windows.
**Note:** `AUTO_ATTACH_DESIGN.md` §10 requires this complete matrix — and all three idle-cost
windows — on **both** the normal dev PC and the weaker target PC. This sequence is the main-PC
half; Sequence 5 repeats the whole thing on the weaker machine.

### Capability status (corrected)

`collector_record --auto-attach` and the resident dual-order supervisor **already exist**
(`extractor/native/recorder_host/src/main.rs:832-922`: CLI seam wiring the real Windows adapters over the
unit-tested `recorder_host::auto_attach` module; `PRODUCT_SESSION_SEMANTICS.md` §1 "foundation
built"). The true blockers are **narrower** (register): the packaging/tray UI with graceful quit —
today the supervisor "runs until the process is terminated" (design §9, outside that lane) — and
duplex activation, which is hard-disabled (`AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED = false`,
`extractor/native/recorder_host/src/auto_attach/duplex_worker.rs:35-50`). The matrix arms below that only need discovery / attach /
journal / finalization semantics are runnable against the existing supervisor; arms needing
graceful quit or duplex liveness wait on the register items.

### The matrix is inherently multi-launch

Relaunch, two-simultaneous-instances, and crash/restart arms **require** multiple game launches by
definition — no single-launch claim is honest here. Launch count per arm below.

### Operator arms (`AUTO_ATTACH_DESIGN.md` §10 "Live Windows matrix", :403-414 — complete main-PC list)

| Arm | Launches | What to do |
|---|---|---|
| App-first | 1 | Start the supervisor, then launch the game → attaches. |
| Game-first | 1 | the game already running, start the supervisor → attaches immediately. |
| Menu-first / loaded-campaign-first | folds into the two above | One attach at main menu, one with a campaign loaded. |
| Game relaunch while app remains | 1 | Quit and relaunch the game with the supervisor up → new identity, new attempt. |
| **Non-default Steam drive** | 1 | Launch the game from a Steam library on a different drive (the live process supplies its own path; nothing is hardcoded). Precondition: such an install exists. |
| **Known-Folder / OneDrive Documents** | 1 | Run with the Paradox user-dir under a redirected/OneDrive Documents location → discovery resolves it. |
| WMI unavailable | 1 | Simulate WMI provider loss → 2 s degraded scan discovers the game; recovery returns to 30 s reconciliation, no duplicate attach. |
| Two simultaneous games | 2 | Two the game instances (unique per-attempt DLL dirs) → both attach independently, distinct streams, never merged. |
| **Access denied / different session** | 1 | An the game process under a different user/session → fail-closed `WaitingForIdentity`, never injected. |
| Explicit app quit | (reuses an arm above) | Graceful stop-and-finalize. **OWED: graceful-quit UI (register).** |
| Game crash | 1 | Kill the game mid-capture → valid verified prefix finalized with abnormal termination. |
| Supervisor crash/restart while game remains | 2 | Kill and restart the supervisor while the game lives → journal replay reconciles the identity. **Reinjection into the surviving game process is FORBIDDEN and refused by design**: the `RetainedTargetLease` job association is kernel-retained for the **lifetime of the target process** — it survives supervisor/worker crashes and collector unload (`extractor/native/recorder_host/src/main.rs:6505-6512`), and a target already in the lease job is refused (`require_target_not_in_job`, `main.rs:6529`). Recovery therefore means the restarted supervisor closes the surviving process's identity to a terminal state and capture resumes only on a **new the game launch** (the arm's second launch). The older design text ("restart waits until the old unique module is proven unloaded before reinjection", `AUTO_ATTACH_DESIGN.md` §6) is superseded by the shipped lease code — the code is the authority. |

### Idle-cost (no game running — hard requirement)

`AUTO_ATTACH_DESIGN.md` §10 (:417-423): measure **30 minutes with no game** for **each** of
(a) app-without-watcher, (b) healthy WMI, (c) forced degraded scanning — recording snapshot count,
CPU time, wakeups/context switches, private bytes, WMI reconnects. Bounds: healthy WMI ≤ **61**
snapshots; degraded ≤ **900**; unbounded growth or busy loop fails. §10 requires CPU/memory deltas
reported on **both machines**, so the full requirement is **six windows: these three on the main
PC, and the same three on the weaker PC in Sequence 5**.

**OWED (register):** a self-timing measurement harness, so the owner starts each window and walks
away rather than hand-timing six 30-minute measurements. Until it lands, these windows are the one
place this runbook still contains clock-bound operator work — flagged, not hidden.

### VERIFY

| Gate | Acceptance criterion (`AUTO_ATTACH_DESIGN.md` §10) | PASS |
|---|---|---|
| Live-Windows matrix (main-PC arms) | Every arm: zero missing candidate closures, zero duplicate attachments, zero shared DLL/control paths, zero discarded bytes; verified replay or explicit failed artifact per attempt; timestamps retained for observation/injection/heartbeat/first-baseline/terminal | Journal + artifacts satisfy all five invariants per arm |
| Idle-cost | Bounds above | Counts within bound; CPU/memory deltas recorded (acceptance also needs the weaker-PC numbers — Sequence 5) |
| P2 duplex liveness | `PRODUCT_SESSION_SEMANTICS.md` §5 | **Blocked only on owner-authorized live evidence/promotion:** the paired client/server and CID1 lifecycle are composed, while all activation latches remain false. |

### Activation implications

- Matrix pass + **owner-approved weaker-PC evidence** (Sequence 5) → auto-attach product path may
  report `Recording`/`Known`; idle-cost numbers are not labelled acceptable until the weaker-PC
  deltas are owner-approved (`AUTO_ATTACH_DESIGN.md` §10). Duplex Stage B stays frozen until its
  register items land and pass.

---

## Sequence 5 — BLOCKED: multiplayer + cross-machine (2nd machine + 2nd player; 15 launches across 2 machines: 3 MP + 12 weaker-PC matrix)

**Serves:** A15 (producer landed 2026-07-19 — see register) · CID1 M1/M2/M3 · auto-attach **complete** weaker-PC matrix + weaker-PC
idle-cost (3 windows).
**Est. owner time:** ~half a day with two operators (MP arms ~90 min; the weaker-PC matrix is
another ~12 short launches; plus 3×30 min no-game windows on the weaker machine).
**Why separated:** these gates cannot be produced on one machine by one operator.

### Preconditions

- Main PC **and** a second machine (ideally a **weaker** one, serving the weaker-PC arms in the
  same sitting) with the same accepted the game build **and the full extractor build — REQUIRED, not
  optional**: the CID1 pair standard retains **both streams/manifests for every pair**
  (`CAMPAIGN_IDENTITY_DESIGN.md` §Validation plan, "Each capture pair retains both
  streams/manifests…"), so for M1/M2/M3 **both machines run the recorder** with an acceptance-only
  opt-in CID1 capture. Host-side reads alone do not meet the pair standard.
- Two players (or one player driving two clients across the two machines).

### Operator steps

1. **A15 — 2-player same-build MP fixture.** Host on the main PC; join from the second machine.
   Send a unique chat line (`D57_CHAT_PROBE_<timestamp>`), generate a join/leave system message,
   perform a hotjoin and a tag-switch.
   *(CORRECTION 2026-07-19: the A15 producer has landed promotion-ready — the fold projects all
   four promoted kinds and the `--expect-text` probe exists; see the updated register row. The
   decode-rejection claim below is stale; the oracle-run procedure itself still applies.)*
   **A15 is OWED (register): recording the current d57 slice does NOT pass it.** Production decode
   rejects/ignores `Chat`, `SystemMessage`, `Hotjoin`, `TagSwitch`
   (`extractor/native/lanes/session_multiplayer_metadata_live/INTEGRATION_PACKET.md:219-225`); acceptance requires the
   unique-chat-line oracle run with `--expect-text` **plus** a stable pollable message/root probe
   showing count/text deltas — text observation alone is insufficient
   (`SESSION_EVENT_DISPOSITION.md:107-117,133-140`). Do this fixture only when those tools exist,
   or accept that this MP sitting produces probe raw material, not an A15 pass.
2. **CID1 M1 — host + hotjoin client at the same game hour.** Full pair standard (Sequence 3),
   **acceptance-only opt-in CID1 captures running on BOTH machines**: complete pairs **equal**
   despite roster/role (`MASTER_EXTRACTION_CONTRACT.md` P3 M1).
3. **CID1 M2 — host + observer at the same game hour.** Both machines captured; pairs **equal**
   without a controlled country (P3 M2). Can reuse the M1 session with the client rejoining as
   observer.
4. **CID1 M3 — rehost the same save on the other machine/player** (new launch on machine 2, its
   own capture). Pairs **equal** across machine, persona, path, and host-country changes (P3 M3).
5. **Weaker-PC auto-attach matrix — the COMPLETE §10 matrix, not a subset.**
   `AUTO_ATTACH_DESIGN.md` §10 ("Run on the normal dev PC and at least one weaker target PC"
   followed by the full arm list) requires every Sequence-4 arm repeated on the weaker machine:
   the same ~12-launch table (app-first, game-first, relaunch, non-default Steam drive,
   Known-Folder/OneDrive, WMI-unavailable, two simultaneous games, access-denied, game crash,
   supervisor crash/restart-with-new-launch).
6. **Weaker-PC idle-cost — three separate 30-minute windows with NO game running on that machine**
   (app-without-watcher, healthy WMI, forced degraded — `AUTO_ATTACH_DESIGN.md` §10: idle-cost is
   defined as "30 minutes with no game"; it cannot run "while MP proceeds"). Schedule them before
   or after the MP sitting. Report CPU/memory deltas for owner approval.

### VERIFY

| Gate | Acceptance criterion | PASS |
|---|---|---|
| **A15** (d57) | `MASTER_EXTRACTION_CONTRACT.md` A15 + the lane's oracle/probe procedure (register) | Chat oracle observes the unique line via `--expect-text` AND a pollable root shows count/text + join/tag-switch deltas — register producers landed 2026-07-19, so this MP sitting can grade it |
| **M1/M2/M3** | `MASTER_EXTRACTION_CONTRACT.md` P3 M1–M3 + the full pair standard, **both machines' streams/manifests retained** | Complete identity pairs equal across host/hotjoin/observer/rehost, each arm meeting the Sequence-3 pair standard on both sides |
| Weaker-PC complete matrix + idle-cost | `AUTO_ATTACH_DESIGN.md` §10 (full matrix + all three windows on the weaker machine) | Every arm attaches; all three idle-cost windows within bound; owner approves the deltas |

**Both machines instrumented — a requirement, not a question.** The pair standard
(`CAMPAIGN_IDENTITY_DESIGN.md` §Validation plan: "Each capture pair retains both
streams/manifests, same-tick melted-save values, build/modset/process provenance, role…") applies
to every M-arm pair, so the second machine runs the recorder with its own acceptance-only opt-in
CID1 capture. Plan the second extractor setup accordingly.

### Activation implications

- M1–M3 complete the CID1 activation matrix (with L0 + S/D) → unfreezes **CID1/IdentityV1
  activation** (post-pass, orchestrator).
- **Stitch activation needs BOTH preconditions**: the complete P3 identity matrix **and** the
  production stitch call site, which is still OWED (register;
  `MASTER_EXTRACTION_CONTRACT.md` P4: "Stitch activation remains blocked until P3's complete
  identity matrix and boundary behavior pass" — and P4's own deliverable is wiring the planner
  into production). Completing M1–M3 alone does **not** unfreeze stitching.
- Weaker-PC owner approval unfreezes the auto-attach idle-cost acceptance.

---

## After all sequences — what becomes flippable / DONE

Once the owner has **seen each gate pass** (ACTIVATION FREEZE lifts per-gate, only on observed
pass), and **only for gates whose OWED producers landed and graded the evidence**:

**Activation constants that become flippable (post-pass, by a later orchestrator lane — never
speculatively):**

- `DOMAIN57_GAME_SPEED_READER_DEFAULT_ENABLED` → after A3 passes **all four** §7 criteria
  including the opt-in emission comparison (Sequence 1). Raw `0..4` canonical, displayed `1..5`
  derived (`I1_D57_SPEED_EVIDENCE.md` §7).
- **CID1 / IdentityV1 activation** (collector sequence-0/CFA1 barrier, `Known` manifest
  authority) → after the complete L0 + S1–S4 + D1–D2 + M1–M3 matrix (Sequences 1, 3, 5;
  `MASTER_EXTRACTION_CONTRACT.md` P3).
- **Reload/relaunch stitch activation** → after CID1 activation + the P4 live seam + the
  production stitch call site (Sequences 3, 5; `MASTER_EXTRACTION_CONTRACT.md` P4).
- **Auto-attach `Recording`/`Known` + duplex Stage B**
  (`AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED`) → after the §10 matrix + owner-approved weaker-PC
  evidence + live duplex liveness (Sequences 4, 5; `MASTER_EXTRACTION_CONTRACT.md` P1, P2).

**Contract markers that flip to DONE / CONCEPT_COMPLETE:**

- **F6** — after rungs 1–6 pass (Sequences 1–2) with their owed producers wired.
- **S9** — Sequence 1, launch 1.4.
- **A3, D10** — Sequence 1 (all four criteria).
- **A8, A9, A20** — Sequence 2, for the fields an asserting gate actually graded. A16/B9 are
  already production-closed; any Sequence-2 AUST observation is optional regression evidence.
- **A1, A2, A12, A4, A21, A14, A22, A15** — *(updated 2026-07-19)* all eight register producers
  have landed promotion-ready (`RE_REMAINING.md`; `production_promotion/contract.json`; A15
  replay-boundary patch COMPLETED / PRODUCTION ADMITTED), so these now close in the soak/MP
  sittings themselves; the earlier "not closeable by these sessions" reading is stale.
- **P1–P5** — progressively as the register items land and Sequences 3–5 pass.
- The Production Gate's combined **speed-5 proof** clause and `Done means` — after Sequence 2.

**Explicitly NOT gated here (out of scope, no session):** host-local presentation state — camera,
selections, hover, open screens, map-mode/layer toggles, outliner, tooltips, drag state — is
RATIFIED out of production capture scope (`documents/DESIGN.md` §Production Capture Scope;
`MASTER_EXTRACTION_CONTRACT.md` §Host-local presentation exclusion, X13).

**Adjacent save-oracle fixtures (NOT live-capture gates):** an **active peace-conference** save and
an **active naval-battle** save are still owed for the S8 / `history` (E12) save-oracle closure
(`COVERAGE_REPORT.md` §6; `MASTER_EXTRACTION_CONTRACT.md` S8, E12). Those are paused-and-save
captures for the offline oracle — kept on the radar, not scheduled here.

---

## OPEN QUESTIONS (doc-silent points — not fabricated)

1. **D04 save availability.** Docs cite the D04 1944.2.11.18 *melt* but no loadable `.hoi4` path.
   Does the owner have the save to load, or must a comparable late-war paused save be produced
   first? (CID1 L0's exact seed/UUID equality only holds against *that* save's melt values.)
2. **Naval-combat guarantee (A9/A8).** A9 needs a naval battle that *ends* in-window; a passive
   soak may not produce one. Is steering a fleet into combat an acceptable owner step, or does A9
   need a dedicated scripted naval fixture save?
3. **Emission-comparison harness shape (D57 criterion 4).** The dossier requires comparing the
   opt-in emission with the raw words but names no tool; is this a new verifier mode over the
   opt-in capture's stream + the sealed transcript, or a live side-by-side probe?
4. **Fixture variance assertion (unverified).** It has **not been verified** whether the existing
   fold/validation gates *assert* the required variance for the Sequence-2 harvest fixtures
   (A8 mission semantics, A9 lifecycle-end, A20 drift) over a soak
   stream, or merely record it. If no asserting consumer exists, capture alone would not close
   those gates (memory: signal-must-reach-consumer). This is a verification task against the
   fold/validation gates, not an established claim either way — resolve it before grading the
   Sequence-2 harvest.

Resolved since round 1 (kept for the record): the Safe-launcher pin question is **answered** —
the speed gate's `pid` must equal the current `$Hoi4Pid`
(`Start-Hoi4ProductionCaptureSafe.ps1:141`), hence the per-launch transcripts in Sequences 1–2;
and the M-arm instrumentation question is **answered** — both machines run the recorder
(`CAMPAIGN_IDENTITY_DESIGN.md` §Validation plan; see Sequence 5).
