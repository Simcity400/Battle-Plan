# Domain capture pattern

> **Read `documents/DECISIONS.md` first.** This document is subordinate to it. Where DECISIONS
> sets a product rule — gaps are first class, pause is forbidden, reach over depth, the raw stream
> is canonical and folding is lazy — that rule wins and this pattern bends to it.

The rebuilt collector will carry roughly eighty domains. Today five emit deltas and the rest
re-emit their whole state every hour, so the shape a new domain copies is whichever one its author
happened to read. This is the shape to copy.

Every rule is written so a reviewer can check a new domain against it without knowing that domain.

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
branch-hours can exceed calendar hours arbitrarily. Size for growth beyond the table, not to it. This is the number every budget below is derived from, and it is why
per-domain intuitions are worthless: 1 KiB per domain-hour is ~7 GB uncompressed.

**The size rule is ratified, and it is not a target.** `DECISIONS.md` now sets it: fidelity is
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
(`collector/src/attach_log.rs:86337`) — that is a whole-frame total across domains, with d53 alone
near 660k, so treat it as evidence that guard-probe volume dominates, **not** as a figure to
multiply by eighty. The region-interval cache at `:86490` exists because per-page probing thrashed.

**Batch reads and reuse scratch buffers.** Read adjacent required fields in one guarded span and
decode locally rather than one guarded probe per field. The nearest measurement in this codebase —
a ~3.4× speedup at `collector/src/attach_log.rs:87246` — is for *scratch-buffer reuse on bulk
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

**3.1 Emit-side byte-identity dedup — mandatory floor, always safe.**
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

**Discontinuity beats dedup — 3.4 wins.** After a gap, restart, rollover or branch, the retained
comparison state is invalid and dedup and the differ are both suppressed until a baseline has been
emitted. Without this rule a writer can emit `unchanged` immediately after a gap, which the reader
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
- **Adaptive checkpoints**, not a fixed N: emit a baseline when any of max hours, cumulative delta
  bytes, cumulative patch operations, or measured decode time crosses its bound. A fixed cadence
  wastes baselines on quiet domains and under-bounds bursty ones.
- **Forced baseline after any discontinuity** — a gap, a malformed frame, a restart, a rollover, a
  reload branch. Without this, state that changed during a gap is never reconstructed. This is the
  single most likely permanent-loss defect in a delta design.
- **Delta-before-baseline**: a hard error to the writer and to CI; to the *reader*, a typed gap for
  that domain until its next baseline. Failing a whole campaign for one bad segment contradicts
  "always produce a recording".

## 4. Encoding

**Field-presence and change bitmaps, varint and zigzag integers, bit-packed enums, delta-of-delta
for monotone counters, columnar blocks for large homogeneous tables.** Fixed-width is not the
alternative to self-describing JSON; it merely removes field names while leaving 8-byte integers
holding values of 3. Target 25–75% fewer pre-compression bytes than a fixed-width record.

**Absent, zero and unknown stay distinguishable** after encoding — via presence bitmaps, not
sentinel values that collide with real data.

**Quantize continuous values to a declared precision, where the value's use permits it.**
DECISIONS sets comprehension-grade fidelity rather than pixel-exact reproduction, which licenses
quantization in general — but not for a value whose exact magnitude explains a threshold crossing,
where rounding changes the story the replay tells. Quantization is a per-field decision recorded
with its justification, never a blanket rule. (An earlier draft cited `repair_progress_milli` as
precedent; d56 records that field as unsupported and absent, so it is not one.) It is not a
cosmetic saving: an unquantized float that jitters in its
last bits is never byte-identical, so it **defeats the mandatory dedup floor and inflates every
field-mask delta** for float-carrying domains — economy, supply, combat. Each such field declares
its precision in the wire format, and the declared precision is part of the domain's contract.

**Symbol space belongs to the content manifest**, not the stream. Country tags, equipment names
and province ids are defined by game and mod content; assign ids from the manifest and carry
integers. Mod symbols first seen mid-campaign go to an append-only overflow table. File-wide
interning inside the stream is incompatible with independent decoding unless dictionary
availability, ordering and recovery are all specified — prefer not to need it.

**The wire format is `extractor/documents/WIRE_FORMAT.md`.** Section 4 states requirements; that
document specifies the bytes. Per-domain record layouts are reviewed against it, and it carries the
mechanisms these requirements need — presence and unknown bitmaps for the trichotomy, per-field
quantization, and a `derived(expr)` declaration for fields that change deterministically with the
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
redundancy that dominates this workload. An earlier draft claimed that penalty was "a few percent";
it was wrong by more than an order of magnitude, which is why this is a committed benchmark rather
than a citation.

**Read the benchmark for what it is.** It models full-state JSON — the shape this document intends
to eliminate — so it establishes that cross-hour redundancy is large and worth keeping, and nothing
more. It does not predict savings on the bitmap/keyed-delta wire, where per-hour output is already
far smaller and the ratio will differ. Note also that 48-hour blocks beat 24 in the same run
(0.16 MB vs 0.28 MB): the block span is **not settled here** and cannot be until there is a ratified
seek budget to trade against. Re-measure on real captures in the target format before fixing it.

**Indexed chunks, domain-major.** Compress in blocks spanning roughly 6–24 hours, indexed, so seek
costs one block decode rather than a scan, and so the viewer decodes only the domains the current
view binds. Chunk span is chosen from the seek budget, measured, not assumed.

**Consider per-domain or prefix dictionaries** for small delta blocks whose byte patterns
independent compression cannot learn. Treat 10–40% as an unverified target until measured.

**The seek key is `(branch, hour)`, never `hour` alone.** Reloads keep both branches, so the same
game hour exists in multiple segments and a bare hour index can select the wrong one.

**Seek cost is baseline + bounded deltas + one block decode per bound domain.** With
domain-major chunks a view binding N domains pays N block decodes, not one — the budget ranges
over the bound set. Say that plainly; do not claim
"one entry, never a scan" while also specifying delta chains. This repository already contains a
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
semantics are "events between hour A and B were not captured", not "state unknown".

**Classify by mutability, not by the word "event" in the name.** d64 delayed events is a state
the instance on dispatch, so its rows mutate and are consumed. Getting this backwards makes an
append-only stream of rows that change every hour — the worst of both shapes.

**Hybrids are permitted and must be declared as such.** A domain carrying both live state and a
historical record declares both, and each part follows its own rules; it does not pick one and
force the other through the wrong model.

## 8. Degradation on slow hardware

DECISIONS requires typed gaps rather than failure when a machine cannot keep up. So the pattern
needs a **shedding order**: which domains degrade first, at what measured threshold, and how that
becomes a typed gap. Absent a policy, an overloaded machine either stalls the game or drops data
silently. Shedding is a product decision recorded per domain, not an emergent behaviour.

## 9. Per-domain conformance checklist

Answered in the domain's document; the ledger entries are CI-generated.

**Both shapes:**

- [ ] Shape declared: state set, or event stream.
- [ ] Hooks: each provably unsampleable, registered, counted against the CI cap.
- [ ] Acquisition uses the shared sweep context; no per-domain re-resolution of shared roots.
- [ ] Reads batched; cached roots revalidated by **owner edge + stable key + generation**.
- [ ] Sweep tick-bracketed; torn sweeps emit typed gaps.
- [ ] Buffers are bounded pools; growth outside the bracket; exhaustion emits a gap.
- [ ] Encoding conforms to `WIRE_FORMAT.md`; absent/zero/unknown distinguishable; every continuous
      field declares its quantization precision.
- [ ] Shedding position and threshold recorded.
- [ ] Four budgets measured on the standard corpus, inside per-domain **and** aggregate caps.

**State-set domains additionally:**

- [ ] Emit-side byte-identity dedup against the previous canonical full encoding (mandatory).
- [ ] Keyed differ in place, or a recorded reason it does not apply.
- [ ] Any dirty-signal gating cites its evidence artifact path.
- [ ] Forced baseline after every gap, restart, rollover and branch; dedup and differ suppressed
      until that baseline is emitted.
- [ ] Adaptive checkpoint bounds recorded.

**Event-stream domains additionally** — baselines and tombstones do not apply, so these replace
the four items above rather than adding to them:

- [ ] Append-only with a resumable high-water mark; no re-emission of prior history.
- [ ] Gap semantics are "events between hour A and B were not captured", not "state unknown".
- [ ] High-water mark re-established after every restart, rollover and branch, and the recovery
      path proved not to duplicate or skip across the discontinuity.
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

**The reference domain also settles the record layout.** Row-major and column-major encoders are
measured against each other on this corpus before the fan-out. The wire format documents
column-major as likely to win for large homogeneous tables; ~80 encoders must not be built to the
loser, and after they exist the layout cannot be changed without rewriting all of them.

Build the pattern and the harness before the fan-out. Eighty domains built first and optimised
second is eighty rewrites plus a container that can no longer be changed.
