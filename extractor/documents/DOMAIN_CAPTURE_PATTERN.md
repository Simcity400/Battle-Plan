# Domain capture pattern

> **Read `documents/DECISIONS.md` first.** This document is subordinate to it. Where DECISIONS
> sets a product rule — gaps are first class, pause is forbidden, reach over depth, the raw stream
> is canonical and folding is lazy — that rule wins and this pattern bends to it.

The rebuilt collector will carry roughly eighty domains. Today five emit deltas and the rest
re-emit their whole state every hour, so the shape a new domain copies is whichever one its author
happened to read. This is the shape to copy.

Every rule is written so a reviewer can check a new domain against it without knowing that domain.

## The question a domain must answer first

**What does the viewer need from you in order to redraw this?**

DECISIONS makes playback the game's own scene, redrawn from the player's own content — not a
summary of what happened. So a domain's coverage is set by what the renderer consumes, not by what
an analyst would find interesting. Position, progress, orientation, ownership, the draw state d77
carries: if the viewer reads it to put something on screen, this domain owes it.

Two rules below exist only because of this, and they are the two most likely to be skipped:

- **The appearance test on quantization** (§4). A value that decides where something is drawn is
  `exact`, even though rounding it changes no outcome.
- **The declared cadence** (§1). One sample per game hour is the default and is right for state the
  engine itself only changes hourly. A domain that looks wrong at that resolution declares a finer
  one and says why.

A domain that captures everything an analyst wants and still renders wrong has failed. §11 makes
that checkable: the replay layer's demands are extracted into a ledger and CI fails on any field it
reads that no domain produces.

## 0. The arithmetic this document is sized against

the game runs **8,760 game-hours per game-year**. A campaign is not "thousands of hours":

| Campaign | Game-hours |
|---|---:|
| 1936–1939 | ~26,000 |
| 1936–1942 | ~53,000 |
| 1936–1945 | ~88,000 |

At 79 scheduled domains, a decade is **~7 million domain-samples**. Every per-domain-hour cost
multiplies by that. **These are working figures, not an upper bound**: campaigns run past 1945,
mods change the date range, and DECISIONS retains both branches of every reload — so recorded
branch-hours can exceed calendar hours arbitrarily. Size for growth beyond the table, not to it.

**~7 million domain-samples is the number every budget below is derived from**, and it is why
per-domain intuitions are worthless: 1 KiB per domain-hour is ~7 GB uncompressed.

**The size rule is not a target.** `DECISIONS.md` sets it: fidelity is
never traded for bytes, and subject to that, size is minimised aggressively. **3–8 GB per campaign
is a ceiling, not a budget to spend** — coming in far under it is better, and being within it is
not a reason to stop reducing. So the aggregate byte cap in §1 is a *failure* threshold; the
design goal is the smallest file that costs no fidelity.

Two consequences run through everything here. **Emission must default to nothing when nothing
changed**, and **compression must see across hours**.

## 1. The four budgets

A domain is conformant because it is inside these, measured — never because it looks right.

| Budget | Bounds | Enforced as |
|---|---|---|
| **µs per sweep** | CPU stolen from the game | per domain **and** a total sweep deadline |
| **bytes per game hour** | file growth | per domain **and** a total, projected to 88,000 hours |
| **ms to seek** | viewer scrubbing | worst case over branch, baseline cadence **and the set of domains a view binds** |
| **bytes of RAM** | footprint **in the game process and in the host** | per domain **and** a total, both sides |

**Aggregate caps are the enforceable ones.** Seventy-nine individually reasonable domains can sum
to an unusable hitch: 100 µs each is a 7.9 ms stall every game hour, 1 ms each is 79 ms. A
per-domain number cannot reject that; only the total deadline can.

**Cadence multiplies three of the four.** A domain declares how many samples it takes per game hour
— `cadence_log2` in its schema, one sample per hour by default (`WIRE_FORMAT.md` §1.1). **Doubling
the sample rate** doubles that domain's sweep cost, its bytes per hour and its share of the
aggregate caps, so a finer cadence is declared against those budgets and carries the rendered
evidence that hourly was not enough. It is not free and it is not a preference.

Budgets live in a **CI-generated ledger**, not hand-maintained tables that drift and not scattered
across domain documents where no one can sum them.

## 2. Acquisition — the read path

**Read, do not hook.** Executable instrumentation is permitted only where a value is *provably*
unsampleable, per DECISIONS. The 2026-07-26 measurement — 79 domains of hooks crashed the game 6/6,
one domain 0/6 — establishes that **instrumentation volume destabilises the game**. It does not
establish that any individual hook is unsafe, and the floor is two bootstrap hooks, not zero. Cite
it for what it proves.

**Share acquisition across domains.** `CGameState`, the country table, tag and ownership maps, and
page/region validation are resolved **once per sweep** into a shared immutable context, not once
per domain. A heavy frame is already "dominated by ~1e6 `VirtualQuery` syscalls + allocs"
(`extractor/native/collector/src/attach_log.rs:86336`–`:86337`) — that is a whole-frame total
across domains, with d53 alone near 660k, so treat it as evidence that guard-probe volume
dominates, **not** as a figure to multiply by eighty. The region-interval cache at `:86490`–`:86494`
exists because per-page probing thrashed at ~946 µs per read.

**Batch reads and reuse scratch buffers.** Read adjacent required fields in one guarded span and
decode locally rather than one guarded probe per field. The nearest measurement in this codebase —
a ~3.4× speedup at `extractor/native/collector/src/attach_log.rs:87246` — is for *scratch-buffer reuse on bulk
reads*, not adjacent-field batching; it supports the buffer rule directly and the batching rule
only by analogy. Batching needs its own measurement before it is claimed as a number.

**Revalidate identity, not just type.** A cached address may be reused by the allocator for a
*different instance of the same class* — the vtable still matches. Type validation accepts the
wrong object and silently attributes rows to the wrong division. A cached root is rejoined to its
authenticated owner edge and its stable key and generation re-read and matched, or it is
re-resolved.

**Bound every read**: clamp to region end, require `size <= capacity`, double-read any header a
concurrent reallocation could tear.

**Bracket every sweep by the engine tick.** DECISIONS makes torn sweeps typed gaps. A sweep that
spans a tick boundary is emitted as a gap, never as a coherent world. Bounded reads do not detect
in-place mutation; only the bracket does. Sweep scheduling across eighty domains is therefore a
correctness decision, not just a performance one — the longer the sweep, the higher the gap rate.

**Bounded pools, not "no allocation".** Cardinality is not statically bounded across arbitrary mod
sets, so a fixed buffer either wastes memory or overflows and gaps a valid campaign. Use reusable
pools with defined growth **outside** the tick-bracketed interval and defined exhaustion behaviour
(a typed gap, never a truncated row).

## 3. Emission — default to silence

Three mechanisms, in the order a domain should reach for them.

**This whole section scopes to a domain's state part.** An event stream is append-only: it has no
prior state to compare against, no baseline to fall back to, and silence in it means "no events
happened", which is already its smallest encoding. A hybrid domain applies §3 to its state part and
§7's event rules to its event part, independently — the two never suppress each other.

**3.1 Emit-side byte-identity dedup — mandatory floor for state, always safe.**
After walking and canonically encoding the domain, compare against the **previous canonical full
encoding** — not against the last bytes emitted. For a differ domain (3.2) the last emitted bytes
are a field-mask delta and would never match a full encoding, so comparing against them silently
disables dedup. Every domain therefore retains its last full encoding regardless of what it
emitted. A hash may serve as a *prefilter* only, followed by a byte comparison — hash equality is
a proxy for identity, and this document rejects proxies everywhere else for the same reason.

Identical means emit an `unchanged` token, not the payload. This **cannot lose data**: the
comparison is against actual canonical output, not a proxy, and deletion is expressed by the bytes
differing. Near-static domains — tech, focus trees, equipment catalogues — are unchanged for most
hours of a campaign, so this is the single largest byte win available and needs no per-domain proof.

**Discontinuity beats dedup — 3.4 wins.** After a state-affecting gap, restart, rollover, branch,
schema or cadence change, or the start of a block, the retained comparison state is invalid and
dedup and the differ are both suppressed until a baseline has been emitted. Without this rule a writer can emit `unchanged` immediately after a gap, which the reader
must discard under 3.4's own delta-before-baseline rule, leaving that domain gapped for the rest
of the campaign. That is a permanent-loss bug hiding inside the mandatory floor.

**3.2 Generic keyed differ — the default for row-shaped domains.**
Compare stable-keyed rows against the previous successful sample and emit field-mask changes plus
key-set tombstones, in shared code rather than per domain. For domains where 1–10% of rows change
hourly this removes most row bytes.

**Completeness is a precondition, and skipping it destroys data.** A differ turns a missing key
into a tombstone. An incomplete walk — torn sweep, partial read, failed identity revalidation,
duplicate key — therefore looks exactly like mass deletion, and the differ would encode it as
one. The sample must be *mechanically proven complete* before any tombstone is computed; anything
less gaps the hour atomically and retains the prior state, which is what d56 already requires
("must not silently clear task forces"). Emitting a partial diff is worse than emitting nothing,
because it is indistinguishable from a real change.

Also requires: a stable key contract, duplicate-key rejection, and the reset rules in 3.4.

**3.3 Engine dirty signals — an optimisation, never a requirement.**
Skipping the *walk* requires a signal **proved** to change whenever any emitted field changes.
Proved means a named evidence artifact: RE evidence that the setter writes the signal, or a
differential fixture that mutates the field in-game and asserts the signal fires. A wrapper or
container fingerprint is not a dirty signal when rows mutate in place. Rotating full walks bound
how late a correction arrives; they do not prevent losing transient state.

Without a proved signal a domain still **walks**, but 3.1 and 3.2 mean it usually emits almost
nothing. Sampling cost and emission cost are separate problems and must not be conflated.

**3.4 Baselines and resets.**
- **Adaptive checkpoints**, not a fixed N: emit a baseline when any bound below is reached. **Every
  trigger is a deterministic function of the data**, never of measured wall-clock decode time: a
  machine-dependent trigger makes two recorders emit different bytes for identical state, breaking
  the dedup floor. A fixed cadence wastes baselines on quiet domains and under-bounds bursty ones.
- **Each counter is defined exactly, or identical bounds still split runs differently.** All three
  accumulate since the last emitted baseline:

  | Counter | Counts |
  |---|---|
  | **delta samples** | sample ticks carrying a `DELTA` frame. **A `BASELINE` tick does not advance it** — the baseline resets every counter, so the counter is exactly `0` immediately after one. **`UNCHANGED`-covered ticks do not advance it either.** |
  | **delta bytes** | total encoded `DELTA` frame bytes, **including `len` and `tag`**, not body alone |
  | **patch operations** | changed rows + tombstones + ordered-replacement rows, summed across those `DELTA` frames |

- **`UNCHANGED` is excluded from the temporal counter deliberately.** A maximal run is already a
  single bounded replay step, so counting its ticks would periodically force a full baseline through
  byte-identical state — repeating forever, since the counter resets each time. The bounds exist to
  cap the work of reconstructing a tick, and an unchanged run costs one step regardless of length.
- **Comparison and reset are pinned**: a bound fires when the accumulated value **reaches or
  exceeds** it (`≥`), the prospective frame is **not** counted toward the bound that decides it, and
  **every counter resets to zero on each emitted baseline**. The bound *values* are part of the
  domain's declared contract, recorded in its document and the CI ledger.
- **A baseline is emitted if and only if a trigger fires.** The triggers are: a checkpoint bound,
  a state discontinuity (below), **the start of any block** — including campaign start, a schema or
  cadence change, and a rollover — since `DELTA` and `UNCHANGED` are invalid before a baseline in
  their own block (`WIRE_FORMAT.md` §1.5). There is no discretionary baseline, because a writer
  that may emit one whenever it likes gives identical state two encodings.
- **A pending checkpoint outranks dedup.** A bound reached while encoding one sample fires on the
  next; if that next sample is byte-identical, the baseline is emitted and `UNCHANGED` is not. The
  bound caps the delta chain a reader must replay to reconstruct any tick. Deferring the baseline
  until the domain next changes would hold that chain at its full bound for arbitrarily long — a
  domain that goes quiet right after a long burst pays the worst-case seek across the whole quiet
  stretch, for no saving.
- **Forced baseline after any discontinuity** — a state-affecting gap, a malformed frame, a restart,
  a rollover, a reload branch. Without this, state that changed during a gap is never reconstructed.
  This is the single most likely permanent-loss defect in a delta design. An **event-only** gap is
  not a state discontinuity and forces nothing (`WIRE_FORMAT.md` §1.5).
- **Delta-before-baseline**: a hard error to the writer and to CI; to the *reader*, a typed gap for
  that domain until its next baseline. Failing a whole campaign for one bad segment contradicts
  "always produce a recording".

## 4. Encoding

**Field-presence and change bitmaps, varint and zigzag integers, bit-packed enums, differenced
monotone counters, row-major records** (the layout question was closed by decision 2026-07-26 —
see `WIRE_FORMAT.md` §13). Fixed-width is not the
alternative to self-describing JSON; it merely removes field names while leaving 8-byte integers
holding values of 3. Target 25–75% fewer pre-compression bytes than a fixed-width record.

**Absent, null, zero and unknown stay distinguishable** after encoding — via presence, null and
unknown bitmaps, not sentinel values that collide with real data. Null is a fourth state, not a
flavour of absent: "the relation genuinely points at nothing" and "this schema does not carry the
field" are different facts, and d56's repair target is both at different times.

**Quantize continuous values to a declared precision, where the value's use permits it.**
Quantization is a per-field decision recorded with its justification, never a blanket rule, and
**two independent tests can each veto it** (`WIRE_FORMAT.md` §6):

- **Outcome** — not for a value whose exact magnitude explains a threshold crossing, where rounding
  changes the story the replay tells.
- **Appearance** — not for a value the viewer **interpolates or animates between samples**:
  position, progress along a path, orientation, scale, blend weight. DECISIONS requires the
  rendered replay to look and feel like the game. A rounded position changes no outcome at all, so
  the outcome test passes it; it just makes a unit stutter where the game glides. A value merely
  *drawn* — a bar length, a badge threshold — may still be quantized, with the precision and an
  invisible-at-max-zoom argument recorded in the domain document. It is not a cosmetic saving: an unquantized float
that jitters in its low bits is never byte-identical, so it **defeats the mandatory dedup floor and inflates every
field-mask delta** for float-carrying domains — economy, supply, combat. Each such field declares
its precision in the wire format, and the declared precision is part of the domain's contract.

**Symbol space belongs to the content, not the stream.** Country tags, equipment names and province
ids are defined by game and mod content; carry integer ids, never text. **The rule that makes those
ids deterministic for a given content set does not exist yet** and is listed OPEN in
`WIRE_FORMAT.md` §13 — the portable content manifest defines content and VFS identity, which is a
different thing. Mod symbols first seen mid-campaign go to an append-only overflow table. File-wide
interning inside the stream is incompatible with independent decoding unless dictionary
availability, ordering and recovery are all specified — prefer not to need it.

**The wire format is `extractor/documents/WIRE_FORMAT.md`.** Section 4 states requirements; that
document specifies the bytes. Per-domain record layouts are reviewed against it, and it carries the
mechanisms these requirements need — presence, null and unknown bitmaps for the four states, per-field
quantization, and a clock-linear `derived` declaration for fields that change deterministically with the
clock.

## 5. Compression and seek

**Do not reset the compressor every hour.** Reproduce with
`extractor/native/scripts/bench_chunked_compression.py` — 3,000 hours of full-state frames with two
rows changing per hour, zstd level 3:

| Framing | Compressed | vs per-hour |
|---|---:|---:|
| independent frame per hour | 5.65 MB | — |
| 6-hour blocks | 0.98 MB | 17.3% |
| 24-hour blocks | 0.28 MB | **4.9%** |

**24-hour blocks cut compressed bytes by 95%.** Per-hour framing forfeits exactly the cross-hour
redundancy that dominates this workload. This is a committed benchmark rather than a citation
because the margin is large enough to decide the design.

**Read the benchmark for what it is.** It models full-state JSON — the shape this document intends
to eliminate — so it establishes that cross-hour redundancy is large and worth keeping, and nothing
more. It does not predict savings on the bitmap/keyed-delta wire, where per-hour output is already
far smaller and the ratio will differ. Note also that 48-hour blocks beat 24 in the same run
(0.16 MB vs 0.28 MB): the block span is **not settled here** and cannot be until there is a ratified
seek budget to trade against. Re-measure on real captures in the target format before fixing it.

**Indexed chunks, domain-major.** Compress in blocks spanning many hours, indexed, so seek costs
one block decode rather than a scan, and so the viewer decodes only the domains the current view
binds. **The span is not fixed here** — the benchmark above has 48 hours beating 24, and the trade
is against a seek budget with no ratified number. Chunk span is chosen from that budget, measured,
not assumed.

**Consider per-domain or prefix dictionaries** for small delta blocks whose byte patterns
independent compression cannot learn. Treat 10–40% as an unverified target until measured.

**The seek key is `(branch, tick)`, never a bare time.** Reloads keep both branches, so the same
game time exists in multiple segments and a bare index can select the wrong one. The tick is the
file-global unit of `WIRE_FORMAT.md` §1.1 — an hour index cannot address a playback position inside
an hour once any domain samples faster than that.

**Seek cost is baseline + bounded deltas + one block decode per bound domain.** With
domain-major chunks a view binding N domains pays N block decodes, not one — the budget ranges
over the bound set, and the seek budget is stated that way. This repository already contains a
binary hour index and a sparse checkpoint/delta cache with a certified bounded seek — reuse it
rather than reinventing.

## 6. Writing

**Append-only, with paged indexes and batched durability.** Rewriting a growing index every hour
is quadratic: at only 3,000 hours a 100–200 byte index record costs 450–900 MB of cumulative
rewrite traffic, before timeline rewrites or a whole-file finalisation copy. At 88,000 hours it is
untenable, and DECISIONS requires that sealing a session must not rewrite the campaign file.

**Minimal in-DLL producer, asynchronous host pipeline.** The injected DLL samples into a
preallocated ring buffer — memcpy-speed, and the only part that must sit inside the tick bracket.
Diffing, encoding, hashing, compression and file writing happen in the recorder host. This removes
compression and filesystem stalls from game time entirely.

**Backpressure must produce typed gaps, and the ring must be able to say so when it is full.** A
gap enqueued through the same full ring cannot be written, so silent loss is the default failure
unless prevented: reserve a control lane, or carry sequence numbers so the host detects a
discontinuity the producer could not report. A dropped-data path that cannot report dropping data
is not fail-closed. Every such discontinuity forces a baseline per 3.4.

## 7. Two domain shapes

Baseline / delta / tombstone models **state sets**. It is the wrong shape for **event streams** —
fired popups, sunk-ship history, terminal combat records — where a "baseline" would re-emit
unbounded history. Event domains are append-only with a resumable high-water mark; their gap
semantics are "events between tick A and B were not captured", not "state unknown".

**Classify by mutability, not by the word "event" in the name.** d64 delayed events has a *state*
the instance on dispatch, so those rows mutate and are consumed. Getting this backwards makes an
append-only stream of rows that change every hour — the worst of both shapes.

**Hybrids are permitted and must be declared as such.** A domain carrying both live state and a
historical record declares both, and each part follows its own rules; it does not pick one and
force the other through the wrong model. **d64 is a hybrid**, not a pure state set: its mutable
pending rows are a state part and its lifecycle trailer is an event part. `WIRE_FORMAT.md` §10
scopes every baseline and dedup rule to the state part for exactly this reason.

**A state part declares the shape of each of its collections** — keyed set, ordered keyed list, or
ordered occurrence list (`WIRE_FORMAT.md` §7.3). This is not a formatting detail: the same bytes mean
different things under each, only a keyed set's contribution to a `DELTA` uses the delta grammar —
ordered collections are replaced whole — and a domain that guesses either double-appends its rows
or sorts away order the game treats as state.

## 8. Degradation on slow hardware

DECISIONS requires typed gaps rather than failure when a machine cannot keep up. So the pattern
needs a **shedding order**: which domains degrade first, at what measured threshold, and how that
becomes a typed gap. Absent a policy, an overloaded machine either stalls the game or drops data
silently. Shedding is a product decision recorded per domain, not an emergent behaviour.

**Shed whole domains; do not thin one domain's sampling.** Every state-affecting gap forces the next state
observation to be a `BASELINE` (`WIRE_FORMAT.md` §1.5), because state may have moved and returned
unobserved. So a domain sampled every other tick emits a baseline every other tick — the most
expensive frame there is, at the moment the machine is least able to afford it. Dropping a domain
entirely costs one gap and one baseline on recovery. A domain that should permanently sample less
often lowers its `cadence_log2`, which is a schema change, not a gap.

## 9. Per-domain conformance checklist

Answered in the domain's document; the ledger entries are CI-generated.

**Both shapes:**

- [ ] Shape declared: state set, event stream, or hybrid carrying both.
- [ ] **Render inputs listed** — every field the viewer reads to draw this domain, reconciled
      against the render-coverage ledger (§11). A domain that renders wrong has failed regardless
      of its other numbers.
- [ ] **Cadence declared**, with its cost against the four budgets. A finer-than-hourly cadence
      carries the visual evidence that hourly was not enough.
- [ ] Hooks: each provably unsampleable, registered, counted against the CI cap.
- [ ] Acquisition uses the shared sweep context; no per-domain re-resolution of shared roots.
- [ ] Reads batched; cached roots revalidated by **owner edge + stable key + generation**.
- [ ] Sweep tick-bracketed; torn sweeps emit typed gaps.
- [ ] Buffers are bounded pools; growth outside the bracket; exhaustion emits a gap.
- [ ] Encoding conforms to `WIRE_FORMAT.md`; absent/null/zero/unknown distinguishable; every
      continuous field declares its quantization precision.
- [ ] Shedding position and threshold recorded.
- [ ] Four budgets measured on the standard corpus, inside per-domain **and** aggregate caps.

**State-set domains additionally:**

- [ ] Every collection's shape declared — keyed set, ordered keyed list, or ordered occurrence
      list — with the evidence that decided it. Only keyed sets have a delta grammar; ordered
      shapes are full replacement.
- [ ] Emit-side byte-identity dedup against the previous canonical full encoding (mandatory).
- [ ] Keyed differ in place, or a recorded reason it does not apply.
- [ ] Any dirty-signal gating cites its evidence artifact path.
- [ ] Forced baseline after every state-affecting gap, restart, rollover and branch; dedup and differ suppressed
      until that baseline is emitted.
- [ ] Adaptive checkpoint bounds recorded.

**Event-stream domains additionally** — baselines and tombstones do not apply, so these replace
the four items above rather than adding to them:

- [ ] Append-only with a resumable frontier; no re-emission of prior history.
- [ ] Frontier components declared — which one is the sequence, which are epoch components
      (`WIRE_FORMAT.md` §9). A single scalar mark is insufficient wherever an epoch exists: d64's
      is `(hook_epoch, sequence)`, and
      `extractor/native/recorder_host/src/world_fold.rs:9500`–`:9510` refuses continuity across an
      epoch change even when sequences look contiguous.
- [ ] Gap semantics are "events between tick A and B were not captured", not "state unknown".
- [ ] Frontier re-established after every restart, rollover and branch, and the recovery path
      proved not to duplicate or skip across the discontinuity.
- [ ] Retention bound recorded — an unbounded history has no baseline to fall back on.

## 10. Benchmark corpus and the reference domain

**One corpus, or the numbers are incomparable.** A synthetic campaign of at least 50,000 hours
with a fixed change-rate profile, and one harness. **The change-rate profile is calibrated
against a recorded real campaign** — an uncalibrated synthetic lets every domain pass CI on
fiction, which is worse than no gate because it reads as evidence. Every domain's four numbers come from that
harness or they do not count.

**A domain is judged against its own recorded budget and the aggregate caps — never against
another domain's absolute numbers.** Cardinalities differ by orders of magnitude; d20 will always
cost more than d41. The reference domain establishes the *pattern* and the harness, not a
universal threshold.

**The record layout is settled: row-major, closed by decision 2026-07-26** (`WIRE_FORMAT.md`
§13). The synthetic baseline measurement here —
`extractor/native/runs/d20_reference/12_layout_compressed.txt`, **column-major at 79.8% of
row-major**, 5.08 MB against 6.37 MB over 720 hourly baselines at 3,051 divisions — was
computed on a churn model that real captures later invalidated (measured changed-row churn is
21–72%/hour, a regime the synthetic surface never measured), the one real columnar-vs-row
comparison favored row-major, and no columnar delta grammar exists while most real hours are
deltas. The layout-before-fan-out ordering this section argued for happened; the answer it
produced is row-major.

**Report the model with the number, and make the benchmark measure its own model.** An
full-churn control — every carried value redrawn each hour — says 85.3%, and ships as a flag on
the same binary so the figure can be re-run rather than taken on trust. Those are only two measured
synthetic points, not a directional churn law: the non-monotone ratio cannot be interpolated, and
real changed-row churn is a quantity and regime this surface never measured. The answer is not monotone in the assumption it turns on, so quote the surface, not a bound. A model stated in a
comment is not a model that was run. This one has been wrong four separate times — uncorrelated
noise, a symmetric toggle reaching 50% by month end, a move destination that contradicted where the
unit arrived, and a stop process that cancelled 98.8% of the journeys it advertised — and each
correction moved the headline. The benchmark now prints its realised occupancy beside its sizes,
and the destination is one shared function rather than two expressions, so the stated model and
the executed one can be compared without re-deriving either.

**It establishes how cadence gets decided, not what every domain's cadence is.** The reference
domain must drive something the viewer draws in motion, so its capture can be replayed and watched
at hourly cadence and at a finer one. That comparison produces the *method* — a rendered parity
test — and the harness to run it. **Each domain that claims a finer cadence then carries its own
rendered evidence**: one domain's answer does not transfer to an unrelated render path, and the
question is always what it looks like, which no amount of reasoning about tick rates settles.

Build the pattern and the harness before the fan-out. Eighty domains built first and optimised
second is eighty rewrites plus a container that can no longer be changed.

## 11. Render coverage — the capture must satisfy the replay layer

DECISIONS makes playback the game's own scene redrawn. The replay layer already exists and already
states what it needs. **Its demands are enumerable, so coverage is a checkable property rather than
a judgement.**

**The ledger is rooted at the shipped product, not at a list of crates.** Its scope is the
transitive call graph of `replay_interface/native/replay_app/src/main.rs:644` — the shipped `fn
main`. A hand-maintained crate list goes stale silently and understates coverage: the application
itself consumes replay state directly — `replay_interface/native/replay_app/src/app.rs:750` reads
`production_render_status()` and `:910` reads `d48_buildings_status` straight off the `WorldFrame` —
so a scanner pointed only at `replay_core` and the viewers would pass while missing product reads.

**The join is typed, not by name.** Each demand resolves to a `(domain, collection_id, field_id)`
producer, the machine identity pinned in `WIRE_FORMAT.md` §11. Matching on field name would
silently pair a renderer's `status` with an unrelated domain's `status` and report coverage that
does not exist.

**The ledger is CI-generated and CI-enforced.** It extracts every field the replay layer reads,
resolves each to its producer, and **fails the build on any read with no producer**. A missing
field then surfaces as a named build error before the fan-out, not as something absent from the
screen after eighty domains exist.

This works without anyone understanding the replay layer's internal structure, which is the point:
the ledger needs only what that layer *reads*, and reading is mechanically extractable. The
rebuild changes acquisition and the wire format, but not what the renderer needs to draw a frame —
so the demand list is stable across the rebuild and can be enforced now.

**What the ledger cannot catch** is a field the replay layer *should* read and does not, because it
was never captured to begin with. No static check finds an absence nobody encoded. That gap closes
only by rendering a real capture and comparing it against the game, which is why the reference
domain (§10) must drive something visible and why every domain's checklist asks for its render
inputs by name.

Coverage failures are ordinary work, not exceptions: expect the first ledger run to name many
misses, and treat that list as the fan-out's real scope.
