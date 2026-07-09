# Lane workflow — how delegated work is run in this repo

**Audience: every agent working in this repository — orchestrator or delegate, Claude or Codex,
any layer (extraction, replay, RE, tooling).** This file is the *method*. It is deliberately
free of any one layer's file paths, offsets, and domain facts, so it can be applied to work that
does not exist yet.

Each layer keeps its own **lane brief** holding that layer's hard requirements, established
facts, proven NULLs, and gates. A lane reads this file **and** its layer brief before starting.

| Layer | Lane brief |
|---|---|
| Extraction | [`../extractor/documents/CODEX_LANE_BRIEF.md`](../extractor/documents/CODEX_LANE_BRIEF.md) |
| Replay | owned by the replay agent — see that layer's documents |

Related canon: repo rules `documents/AGENTS.md` · Codex mechanics
`C:\Users\Example\.claude\CODEX_PROTOCOL.md` · Claude handoffs `documents/CLAUDE.md`

---

## 0. Roles

- **Orchestrator** — plans, decomposes into lanes, reviews every diff, runs every gate, owns the
  outcome. Never rubber-stamps a delegate's report.
- **Delegate (lane)** — does one well-scoped job in an isolated worktree, proves its own claims,
  and reports evidence.

A delegate's *reply is not the diff*. It can be wrong, it can over- or under-claim, and it can
write files it never mentions. The orchestrator's verification is what makes delegation safe.

---

## 1. Measure before you optimize

Never optimize on intuition, and never accept a bottleneck you were *told* about — profile it.

This has repeatedly overturned confident assumptions. The most valuable results in this repo
have come from lanes discovering the brief was wrong: a fold's real cost was in building
snapshots, not the serialization everyone blamed; a suspected stale-tracker bug was actually a
bypassed code path; an entire reverse-engineering hunt for a change signal ended in proving no
such signal exists — which redirected the design onto something safer.

So: **a lane is authorized, and expected, to refute its own brief.** A refuted premise, reported
with evidence, is a first-class deliverable — often more valuable than the code it was sent to
write. Say so plainly rather than building the thing you were asked for on a false premise.

## 2. Prove, don't assert

Every claim carries its evidence:

- **Numbers, not adjectives.** "1.35x, 3.07 → 4.14 MB/s on a 512 MiB sample, same rig" — not
  "significantly faster."
- **Cite `file:line`** for code claims, addresses for reverse-engineering claims.
- **Say what is measured vs projected**, and **synthetic vs live**. Label which results still
  need validation against reality, and how.
- **Report NULL results honestly and revert them.** A lever that didn't help, measured and
  documented, stops the next person from re-trying it. This repo keeps an explicit
  proven-NULL list per layer for exactly that reason.
- **State what you did NOT cover.** Silent truncation (top-N, sampling, skipped cases) reads as
  full coverage when it isn't.

## 3. Correctness gates come before performance

Where a layer defines a correctness invariant (byte-identity, a hash chain, an equivalence to a
reference implementation), that invariant is **not negotiable for a speedup**. Any change that
alters output is a defect unless the change in output *is* the documented intent.

Two patterns worth reusing anywhere:

- **Identity gate** — the optimized path must produce byte-identical output to the reference
  path over the real corpus, not just a toy fixture. Run it yourself.
- **Shadow mode** — to validate a new implementation against an old one under real conditions,
  run **both** and compare per unit of work, failing closed on any mismatch and counting
  mismatches in telemetry. Keep the old path compiled and selectable as the referee for
  re-validation after upstream changes. This is how a risky rewrite earns its default-on status:
  the new path stays off until a live shadow run passes clean.

**When a guarantee cannot be proven, STOP and report the blocking evidence — do not ship the
code.** Default to "refuted / unsafe" under uncertainty. A false "this is safe" on an invariant
is worse than no change at all, because it surfaces much later and much more expensively.

## 4. The implementer never reviews its own diff

A model reviewing its own work shares the assumptions that produced the bug, and it decides which
findings against itself to accept. Both halves of that are disqualifying.

- A lane **may** run a self-refutation pass as a sanity check. It must then report **every**
  finding verbatim — including ones it disagrees with — and say what it did about each. It may
  not silently dismiss one.
- The **orchestrator** runs an independent adversarial gate on the final diff: a fresh,
  read-only reviewer prompted to *refute* the change, given **named prime suspects** rather than
  a generic "review this." Name the specific ways this specific change could be wrong.
- Prefer a reviewer that is not the implementer's own model. When one model both implements and
  reviews, model-diversity in review is lost; the orchestrator is then the only remaining
  independent check and must act like it.

## 5. Escalation cap — and its exception

**One delegated fix round per adversarial review.** If the re-review still finds defects, the
orchestrator implements the remainder directly, self-verified, and moves on. Convergence speed
beats a fifth round.

**Exception — critical diffs.** Where a change touches an invariant the project cannot afford to
break (data integrity, correctness of a canonical artifact, anything whose failure is *silent*),
the **final state** gets one more independent adversarial gate, regardless of who authored the
last fix. Otherwise the orchestrator's own closing fixes ship with zero review — which is
precisely where a silent bug hides.

## 6. Lane isolation

- **One lane per worktree, one job per worktree.** Keep lanes on disjoint files/crates where
  possible; if a lane must touch a hot shared file, confine edits to its own functions and say so.
- **Never push.** Commit on the lane branch; the orchestrator merges after verification.
- **Verify freshness** against the intended base commit before starting; a stale worktree silently
  reverts other lanes' work.
- **Lanes do not edit a shared plan-of-record document.** Concurrent appends to one file is the
  most reliable way to manufacture merge conflicts. Each lane writes its **own report file**
  (`<layer>/documents/lane_reports/<lane>.md`); the orchestrator folds findings into the plan.
- **Large corpora are read in place, never copied.** Delete benchmark intermediates. Verify free
  disk before large writes — and after cleanup, because some deletions fail silently.

A lane report states: mission; what changed (`file:line`); findings **including refuted
hypotheses**; the correctness argument and the gate that proves it; gate evidence with exact
numbers and test counts; NULLs; what still needs live validation.

## 7. Orchestrator verification battery (non-negotiable)

Before merging **any** delegated work:

1. **Run every gate yourself.** A reported pass is not a pass.
2. **Scope review** — `git status --porcelain`: no out-of-scope files, no stray edits to rule
   documents, new source files actually staged (untracked sources have silently broken commits).
3. **Read the key diff hunks**, especially unsafe code, live-memory access, and anything touching
   a correctness invariant.
4. **Mechanically spot-check at least one claim** — run the script, grep the value, recompute the
   hash.
5. **Run the independent adversarial gate** on the final diff (§4).
6. **Compose-check.** Lanes are verified in isolation but merged together: after merging a wave,
   re-run the full suite *and* the identity gate on the merged tree. Two individually-correct
   changes can still compose into a wrong one.

## 8. Delegates cannot touch the outside world

A sandboxed delegate cannot read another process's memory, drive a live application, or run a
GPU capture. For that class of work: **the delegate writes the script/tooling; the orchestrator
executes it outside the sandbox** and feeds results back into the same thread.

Consequence for planning: prove offline everything that can be proven offline, *before* spending
a live session. Live sessions are scarce, hands-on, and worth spending only on questions that
genuinely require reality to answer.

## 9. Context does not travel — carry it

A fresh delegate starts with **zero memory of the project**. Its prompt must carry, or point at,
the full picture: goal, constraints, the hard requirements, the acceptance gates, and the
relevant established facts.

Point at the canonical files (this one and the layer brief) rather than re-pasting their contents
per prompt. Hand-rebuilt context drifts, and a dropped constraint is how a hard requirement
quietly stops being enforced. If a delegate's instructions and a canonical file disagree, the
canonical file wins — and the delegate should say so rather than guess.
