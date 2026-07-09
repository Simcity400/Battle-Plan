# CODEX LANE BRIEF — canonical context for every delegated extractor lane

> **Every Codex job prompt must point here and instruct the job to read this file FIRST.**
> Codex starts each job with zero memory of this project. This file is the single source of
> truth for the laws, the facts, and the gates. Do not re-paste this content into prompts —
> point at the path and add only the lane-specific mission.
>
> Path: `extractor/documents/CODEX_LANE_BRIEF.md`
> Machine-wide Codex standard: `C:\Users\Example\.claude\CODEX_PROTOCOL.md`

---

## 1. What this system is

An in-process collector DLL runs inside a live `hoi4.exe`, samples game state every game-hour,
and streams H4CR compact records over a named pipe to a Rust recorder host
(`collector_record.exe`), which folds them into per-hour world snapshots and packs a
`.battleplan` zip container.

The end goal: hand the replay file to a reverse-engineered replay engine so a whole game can be
watched back in full detail, read-only. **The compact stream is the canonical artifact.**

Workspace: `extractor/native` (Rust). Key crates:
- `collector` — the in-process DLL: domain readers, SEH-guarded reads, emit gates, delta trackers
- `recorder_host` — the host: `world_fold.rs` (fold), `stream_fold.rs`, pack/zip, manifest
- `compact_contract` — the shared wire contract; owns H4CR framing and status-flag masks
- `replay_decoder` — pull-based per-hour cursor over a raw stream or container
- `fold_check` — the G1 lossless gate tool (per-hour canonical SHA-256 + chain hash)
- `lanes/*` — per-domain reader crates

---

## 2. THE HARD REQUIREMENTS (these override everything, including performance)

1. **ZERO DATA LOSS.** Hourly-or-finer data. No cadence thinning. No skipped game-hours. Any
   delta, skip, or change-driven optimization must be **provably lossless**, with an automated
   gate. If losslessness cannot be proven, **STOP and report the blocking evidence — do not
   ship the code.** A false "this is safe" here means a silent hole in a replay that surfaces
   months later. Default to REFUTED/unsafe when uncertain.
2. **MUST RUN ON WEAKER PCs.** Cut CPU work per capture frame. Do **not** lean on core count —
   target machines have fewer cores than the dev rig. (Post-capture/offline batch work such as
   the host fold is exempt: parallelism is welcome there.)
3. `replay_interface/` is **off-limits** — another agent owns it. Never edit it.

---

## 3. Losslessness vocabulary (get this exactly right)

- **Baseline** record: full row set, `COMPACT_STATUS_DELTA_DATA` clear. The fold treats any
  non-delta record as a **full replacement**.
- **Delta** record: `DELTA_DATA` set; only changed rows.
- **Tombstone**: high bit `1<<31` on the identity key, live fields zero = row removed.
- **Partial frame**: `COMPACT_STATUS_PARTIAL_DATA` — reads were accurate but incomplete (torn
  reads of a live, mutating world). **`PARTIAL_DATA` is a member of
  `COMPACT_STATUS_FAILURE_MASK`** — this single fact caused a real, shipped bug (see §5).
- **Trackers never advance on failure/partial frames.** A partial frame's rows must never
  pollute the delta baseline.
- **Positive-absence-only tombstones**: a row may be tombstoned only when its absence is
  *positively observed* (e.g. a slot pointer read cleanly as 0). A torn or failed read is
  **not** evidence of absence. Mistaking one for the other deletes live data.

---

## 4. The gates (run them yourself; a reported pass is not a pass)

- **G1 fold identity** — `cargo run --release -p fold_check -- hash <stream.bin>` produces a
  per-hour SHA-256 list plus a chain hash. Any change that alters folded output for an existing
  stream is a **defect** unless it is the explicit, documented intent.
- **Fold-identity-vs-full-emission** — for any delta/reduced-emission change: the reduced stream
  must fold **byte-identical** to the full-emission stream, over sequences that include
  unchanged frames, genuine changes, reverted values, tombstones, partial frames, hard-failure
  frames, re-baselines, and multi-hour spans.
- **Shadow mode** — the proven pattern for validating a new reader against an old one live: run
  BOTH in-process, compare per frame, fail-close the frame as partial on any mismatch, and count
  mismatches in the manifest. `d53_relation_read_mode = offsets|scan|shadow` is the reference
  implementation; its live shadow gate **passed with 0 mismatches**, which is how the
  exact-offset reader was proven. Copy this shape.
- `cargo build --release` and `cargo test` in `extractor/native` must be green. Report exact
  test counts.
- **Mandatory after any hoi4.exe patch:** re-run the d53 shadow gate. Offsets can move.

---

## 5. Established facts — do not re-derive, do not contradict without evidence

**Performance shape**
- The frame is **critical-path bound, not sum-bound**: ~693.6 ms of per-domain read-work
  compresses to a 151.5 ms frame at `reader_concurrency=12` (~4.58x). Removing work from a
  domain only helps the frame if that domain sits on the slow lane. Per-lane attribution now
  exists (manifest `lane_wall`, domain-250 probe) — use it instead of guessing.
- **77% of read-work reads provably-unchanged rows** just to prove they are unchanged. That is
  the irreducible cost without change-driven reads.

**PROVEN NULLs — do not retry these:**
- Thread concurrency (the workload is CPU-throughput-bound).
- Reducing read *count* when the removed reads were cheap MRU cache hits (count ≠ time).
- Batching cheap sub-reads.
- Only a **paired/live A/B** counts as evidence (~±13% single-capture noise).

**Phase 4 (change-driven capture)**
- **There is no cheap engine dirty signal.** Static RE proved it: the MP desync checksum is an
  and it cannot attribute rows. No per-object version counters exist (the opinion writer
  `extractor/documents/PHASE4_DIRTY_SIGNAL_SCOUT.md`.
- **The spine is the DENSE-MIRROR pattern**, and it is *complete by construction*: linearly scan
  a dense contiguous mirror array (cheap), then diff **host-side**. It reads everything, so it
  cannot miss a change — this is why it is lossless, and why it is strictly safer than a
  dirty-flag signal. Exemplar: `map_control` reads the same 20.6M-row province set as
  `map_detail` at **1.2 ms vs 49.2 ms (40x)**.

**Skips are not a frame-time problem**
- The game's local tick rate is **bursty**: min inter-hour interval **28.2 ms** (~35 game-h/s
  bursts) against a 167.6 ms average. Skips correlate weakly with frame time — one occurred at
  the run's *fastest* frame. Zero-skip would need ≤25 ms frames: unreachable. True zero-skip
  requires a cheap **hour-boundary latch** decoupled from the heavy sweep. Hook site:
  `(gs[0x468] - 43800000) % 24`, bracketed by `preHourlyUpdate`/`postHourlyUpdate`.

**Bytes**
- **d53 is 92.3% of all stream bytes.** Its delta path *never fires in deep war*: 400/400 frames
  are `PARTIAL_DATA`, the emit gate returns full on partial, and the fold hard-rejects
  `DELTA|PARTIAL` (`world_fold.rs:10530`). Real churn is ~0.0% for opinion. Any claim that d53
  churn is "genuine" is an artifact of this bug.
- Container is zstd (zip method 93) + zip64, measured **163x**. It absorbs constant redundancy —
  so a byte-saving lever worth <0.01% of the stream is usually **not worth the correctness
  surface**. Measure the share before implementing; a documented "not worth it, here are the
  numbers" is a valid, valued deliverable.

---

## 6. Lane discipline

- **One lane per git worktree, one Codex job per worktree.** Never two jobs in one worktree.
  `cd` into the worktree **before** invoking Codex — the sandbox roots at the invocation cwd.
- Keep lanes on **disjoint files/crates** where possible. `collector/src/lib.rs` and
  `recorder_host/src/main.rs` are hot, shared files: if your lane must touch them, confine edits
  to your own functions and say so in your report.
- **Do NOT edit `EXTRACTION_OPTIMIZATION_PLAN.md`.** It is the plan of record and concurrent
  lanes conflict on it. Instead write **`extractor/documents/lane_reports/<lane>.md`** — a new
  file, no conflict possible. The orchestrator folds it into the plan.
- **Never push.** Commit on your lane branch only.
- Corpora are read **in place, never copied** (they are 12–26 GB):
  - deep-war: `extractor/native/runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin`
  - morning baseline: `extractor/native/runs/production_speed5_ShortSpeed5_20260709_124210/collector_stream.bin`
  Delete any benchmark intermediates you create; verify free disk before large writes.
- Report **numbers, not adjectives.** Cite `file:line` for code claims and addresses for RE
  claims.

---

## 7. Adversarial review

Per `C:\Users\Example\.claude\CODEX_PROTOCOL.md`:

- **The implementer does not review its own diff.** The orchestrator runs an independent Mode A
  read-only refutation gate on the final diff, with named prime suspects. A job may run its own
  self-refutation one-shot as a sanity check, but that never substitutes for the orchestrator's
  independent gate, and a job never decides which findings against itself to dismiss.
- **Escalation cap:** ONE Codex fix round per adversarial review; then the orchestrator fixes
  the remainder directly. **Exception — lossless-critical diffs** (anything touching emission,
  delta/tombstone semantics, the fold, partial-frame handling, or a change-driven read path):
  the *final* state gets one more independent read-only gate regardless of who authored the last
  fix. The cap protects convergence speed; it must never leave a zero-data-loss change
  un-refuted.
- Verification of any delegated work is non-negotiable and belongs to the orchestrator: run every
  gate yourself, `git status --porcelain` scope review, read the key diff hunks (especially
  unsafe / live-memory code), and mechanically spot-check at least one claim.

## 8. What the Codex sandbox cannot do

It cannot read another process's memory, drive the live game, or run a GPU capture. For those:
**Codex writes the script/tooling, the orchestrator executes it outside the sandbox**, and the
results feed back into the same thread (Mode B resume). Live validation is therefore always
hands-on — plan lanes so that everything provable offline is proven offline first.
