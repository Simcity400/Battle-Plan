# Wire format

> **Read `documents/DECISIONS.md` first**, then `DOMAIN_CAPTURE_PATTERN.md`. This document is
> subordinate to both. The pattern says *what* a domain must do; this says *how the bytes look*.

Two rules from DECISIONS govern every choice below, in this order:

1. **The replay must look and feel like the game.** The viewer redraws the game's own scene from
   the player's own content. What the capture owes it is every input that drawing needs.
2. **Fidelity is never traded for bytes; subject to that, size is minimised aggressively.**
   3–8 GB per campaign is a ceiling, not a budget.

Two implementers reading this must produce **identical bytes** from identical state. Where that is
not yet true the gap is listed in §13 as OPEN, because byte-identity is what the dedup floor
depends on.

**This is not yet the byte-final fan-out contract, but the record grammar is settled.** The
largest §13 item — row-major versus column-major — was closed by decision on 2026-07-26: the
row-major grammar is retained, on the evidence recorded in §13 (not on a direct columnar-delta
comparison, which nothing exists to run). What §13 still holds open (symbol assignment, golden
cross-implementation vectors, per-domain cadence) can still move bytes — a cadence change even
re-hashes the schema, since `cadence_log2` is part of its canonical serialization (§11) — but
it does not change how the fan-out's deliverables are made: field ids, kinds and four-state
decisions survive those items outright, and boundary mappers survive structurally — with one
recorded exception: mappers that emit `ContentId` today cast raw source ids and will gain a
manifest-resolver input when §5's assignment rule lands, a planned cross-domain retrofit.
That is why `DOMAIN_SCHEMA_RECIPE.md` scopes the fan-out to schemas, mappings and
measurements rather than shipped bytes.

Source citations are paths from the repository root.

## 0. Conventions

Little-endian.

- `uN` is a fixed N-bit integer.
- `varint` is LEB128, unsigned, **shortest form required** — a representation with a redundant
  trailing zero group is a corrupt frame. Maximum ten bytes; an eleventh continuation byte is
  corrupt, and so is a tenth byte above `0x01`, which would carry the value past `u64::MAX`.
- `zigzag(n)` is `(n << 1) ^ (n >> 63)` encoded as a varint, so small negatives cost one byte.
  Used only where a value is genuinely signed.
- Every length prefix is a `varint`.
- **Bitmaps** are byte arrays; field `i` is bit `i & 7` of byte `i >> 3`, LSB-first. **Unused
  trailing bits are zero.** A bitmap of zero bits occupies zero bytes.
- **Padding bits are zero**, everywhere they occur.
- Raw IEEE values are **`f64`, 8 bytes**, as their exact bit pattern. A domain reading a 32-bit
  float widens it, which preserves the value. **A NaN is normalised to `0x7FF8000000000000`** — the
  payload and sign of a NaN are not game state, and leaving the pattern unpinned would let two
  writers emit different bytes for the same value.

## 1. Time, blocks and frames

### 1.1 Ticks and cadence

**There is one time coordinate for the whole file: the tick.** The container declares
`ticks_per_hour_log2` once; a tick is `1 / 2^ticks_per_hour_log2` of a game hour, and every
timestamp in this format — block bases, frame deltas, gap ranges, symbol availability — is an
absolute tick index from campaign hour 0.

One global unit is what makes timestamps comparable. Symbol availability, seek positions and gaps
all span domains, so a per-domain unit would make "is this symbol available yet" and "which gap
came first" unanswerable.

**`ticks_per_hour_log2 ≤ 63`**, because a tick index is a `u64` and `2^ticks_per_hour_log2` must be
representable in one. A container declaring more has no grid.

**A domain's schema separately declares `cadence_log2`**: it samples `2^cadence_log2` times per
game hour, `0` — once per hour — by default and by far the common case. `cadence_log2` must not
exceed `ticks_per_hour_log2`. A domain samples on ticks that are multiples of
`2^(ticks_per_hour_log2 − cadence_log2)`.

A domain's **sample stride** is `2^(ticks_per_hour_log2 − cadence_log2)` ticks — the spacing
between the ticks it may occupy. Every frame of that domain lands on a multiple of its stride.

Finer cadence is declared, never assumed. DECISIONS requires the rendered replay to match the game,
so a domain whose appearance is visibly wrong at hourly resolution raises its cadence and carries
the rendered evidence that hourly was not enough. Cadence costs only the domain that declares it.

`ticks_per_hour_log2` is not free, though: `tag` packs `tick_delta` above three `kind` bits, so a
one-byte tag holds `tick_delta ≤ 15` and an hourly domain's one-hour delta stays one byte only
while `ticks_per_hour_log2 ≤ 3`. Above that every hourly frame in the file costs a second tag byte.
That is the trade the value is chosen against (§13).

### 1.2 Blocks

The container compresses **domain-major blocks** spanning many hours. A block carries frames for
exactly one `(domain, branch)`. The header carries what every frame in it shares:

| Block header | Type |
|---|---|
| `domain` | `varint` |
| `branch` | `varint` |
| `schema_hash` | `u64` |
| `first_tick` | `varint` |

`cadence_log2` comes from the schema, so it is covered by `schema_hash` and is not repeated here.
Hoisting `domain` and `branch` out of every frame saves 2–3 bytes on each of ~7 million frames.

**`domain` and `branch` are bounded at `2^32 − 1`**, and a block declaring either above that is
corrupt. The varint is unbounded on the wire, so without the bound one reader truncates an
over-range identity onto a valid one — a block claiming domain `2^32` read as domain `0` — while
another rejects it, and the two disagree about which blocks exist.

**A block carries at least one frame.** `first_tick` is defined as the tick of the block's first
frame, so a header with no frames names a tick that nothing occupies.

### 1.3 Frames

| Field | Type | Meaning |
|---|---|---|
| `len` | `varint` | byte length of everything after `len` — `tag` + `body` |
| `tag` | `varint` | `(tick_delta << 3) \| kind` |
| `body` | `len` − sizeof(`tag`) bytes | per `kind` |

`kind` is `tag & 7`; `tick_delta` is `tag >> 3`.

| `kind` | Name | Part | Body |
|---:|---|---|---|
| `0` | `BASELINE` | state | complete, **self-contained** canonical state |
| `1` | `DELTA` | state | changes since the reconstructed prior state |
| `2` | `UNCHANGED` | state | `varint` run length in **sample ticks**, `≥ 1` |
| `3` | `EVENTS` | event | append-only batch |
| `4`, `5` | reserved | **state** | skipped by `len`; covers exactly one tick |
| `6`, `7` | reserved | **event** | skipped by `len`; covers exactly one tick |

**Reserved kinds are assigned to a part in advance** so that a reader which cannot decode one still
knows which chain its `tick_delta` advances. Without that assignment, skipping an unknown frame
would leave every later timestamp in the block ambiguous, which is the opposite of what `len` is
for.

### 1.4 The two time chains

**A block carries two independent frame sequences — the state part and the event part — each with
its own time chain.** `kind` says which part a frame belongs to (including reserved kinds), so no
discriminator is needed.

- `tick_delta` is an unsigned first difference against **the previous frame of the same part**,
  measured from that frame's **last covered tick**. Last covered tick differs from tick only for
  `UNCHANGED`, which spans a run.
- **`first_tick` is the tick of the block's first frame**, and each part's first frame measures
  `tick_delta` from it. The opening frame therefore carries `tick_delta = 0`; the other part's
  first frame carries its own offset from `first_tick`, which may be any value. A hybrid block whose state opens at tick 0 and whose first
  event batch lands at tick 500 encodes the latter as `tick_delta = 500`.
- **Frames are written in nondecreasing order of their own tick; at equal ticks the state frame
  precedes the `EVENTS` frame.** That fixes exactly one byte order.

**Each `(domain, branch, tick)` carries at most one state observation and at most one event batch,
across the whole file — not merely within a block.** A tick is covered by exactly one state frame
or one `UNCHANGED` run, never both and never twice. **The blocks of one `(domain, branch)` cover
disjoint tick ranges**, where a block's range is the hull from `first_tick` to the last tick
covered by either part. Without this, two blocks could each claim the same tick and a reader would
have no rule for which wins.

Independent chains are what make hybrid domains (§10) framable: a tick carrying events while the
state part is quiet needs the event frame not to disturb the state part's run. Nothing inside a
block goes backwards — a block is one `(domain, branch)` with its own `first_tick`, so a reload
branch starts a new block rather than moving time backwards, and a cadence change starts a new
block because cadence is part of the schema.

**`len` exists so a corrupt or unrecognised frame is skippable** without abandoning the rest of the
block. A reader that meets a reserved `kind`, or a body it cannot parse, skips `len` bytes and
continues; both chains survive because `tag` is always readable and its part is known. The block's
own domain is decided by its header, so an unknown *schema* is refused at block level and never
costs another domain anything.

Gaps are not a frame kind. They live in the container's gap stream (§8), so a gap spanning every
domain is written once rather than eighty times.

### 1.5 `UNCHANGED` covers witnessed ticks only

**The run is counted in the domain's own sample ticks, and both endpoints are sample ticks.**
`UNCHANGED` with run length `N` at tick `t` covers the `N` consecutive sample ticks
`t, t + S, …, t + (N−1)·S`, where `S` is the domain's sample stride (§1.1). Its **last covered tick
is `t + (N−1)·S`**, which is what the next frame's `tick_delta` measures from. Counting in ticks
instead would leave both endpoints free to sit anywhere between samples, so the same history would
have many encodings.

**A sample tick the domain did not successfully sample is never covered by `UNCHANGED`.** Coverage
lost to shedding, backpressure, a torn sweep or a failed walk is a gap (§8). Ticks between samples
are not coverage failures — the domain was never going to look at them. Silence must mean "we
looked and nothing changed", never "we did not look".

**A run covers exactly the maximal contiguous span of the domain's sample ticks that were sampled
and unchanged.** It ends at the first sample tick that was gapped, changed, or carried a
`BASELINE`. `EVENTS` frames never terminate a run — they belong to the other part.

**A gap affecting the state part forces the next state observation to be a `BASELINE`.** State may
have moved and returned while nobody was looking, so neither a delta nor an `UNCHANGED` can be
justified across unobserved time. A gap whose `part` is `1` — event-only — does not: it says
nothing about state, and forcing a baseline there would be an expensive lie about what was lost.

This makes thinned sampling genuinely expensive — a domain missing every other sample emits a
baseline every other sample — which is why shedding drops whole domains rather than thinning one
(pattern §8). A domain that lowers its rate permanently changes `cadence_log2`, which is a schema
change and a new block, not a gap.

**A `DELTA` or `UNCHANGED` frame is invalid before a `BASELINE` in its block.** Hard error to the
writer and CI; to the reader, a typed gap for that domain until its next baseline. Every state
discontinuity — a state-affecting gap, restart, rollover, branch, schema or cadence change, or the
start of a block — forces a `BASELINE` of the state part (§10).

**And a `BASELINE` appears only where one of those forces it.** A witnessed tick whose state
changed is a `DELTA`; one whose state did not is covered by an `UNCHANGED` run. Without this rule
the format has no byte-identity at all: given one history — state at tick 0, unchanged at tick 1 —
a writer emitting `BASELINE` + `UNCHANGED` produces 19 bytes and one re-emitting the identical
state as a second `BASELINE` produces 21, and both decode to the same thing. §7.1 pins the choice
between `DELTA` and `UNCHANGED`; this pins the choice against `BASELINE`, which is the same
question one level up. A reader still accepts a mid-block baseline, because a gap-forced one is
legal and gaps are recorded outside the block — so this is a writer rule that a reader cannot
check, which is exactly why it has to be stated rather than left to be inferred.

A domain static for a game year therefore costs a `BASELINE` plus an `UNCHANGED` frame of `len` (1)
+ `tag` (1) + run length (2–3) = **4–5 bytes**, rather than 8,760 frames. That figure is **per
block**: every block opens with its own baseline, so a year spanning several blocks costs one such
pair per block. It also assumes a one-byte tag — `ticks_per_hour_log2 ≤ 3`, §1.1.

## 2. Canonical form

`UNCHANGED` and the dedup floor rest on byte-identity, so canonical form must be fully determined.
Row order depends on the collection's declared shape (§7.3):

| Shape | Canonical row order |
|---|---|
| **keyed set** | sorted **numerically** ascending by the full key tuple, key position order |
| **ordered keyed list** | source order |
| **ordered occurrence list** | source order |

**Numerically**, because a signed (kind `1`) component sorts differently by zigzag code than by
value, and only numeric order makes §7.4's non-negative first-component difference true.

**Source order is the data** for both ordered shapes. Sorting them destroys state the game itself
maintains, which the governing rule forbids outright.

Within a row:

- **Tombstone lists are sorted** by the full key tuple.
- **Rounding is half-to-even** for `fixed(k)` and `ratio(d)`, per the arithmetic pinned in §6.

**A field is never itself a collection.** A set or list owned per row is modelled as its own
top-level collection, keyed by the parent's key. There is no nested-collection encoding and no kind
code for one, so a schema that needs a per-row set declares two collections.

Any nondeterminism here is a size bug, not a cosmetic one.

## 3. Records

**Key fields never appear in a record body.** In a keyed collection a row is its key (§7.2)
followed by a record of the **non-key fields only**; every bitmap and change mask below indexes
non-key fields. An occurrence list has no key, so its record covers all of its fields. Encoding a
key once removes both the duplication and any question of what a disagreement between two copies
would mean.

A record is four bitmaps followed by values:

```
  presence bitmap    ceil(O/8) bytes, O = optional non-key fields in the schema; one bit per
                     optional field, schema order. Omitted entirely when O = 0.
  null bitmap        one bit per PRESENT NULLABLE field, ascending schema order
  unknown bitmap     one bit per PRESENT UNKNOWN-CAPABLE field, ascending schema order
  escape bitmap      one bit per PRESENT QUANTIZED field, ascending schema order
  [ bit-packed region ]   enums, booleans, and ratio(d) values whose escape bit is CLEAR, that
                          carry a value, schema order, LSB-first, zero-padded to a byte boundary
  [ value bytes ]         remaining fields that carry a value, schema order — including any
                          escaped ratio(d), which carries raw f64 here rather than bit-packed
```

**The presence bitmap is always `ceil(O/8)` bytes**, fixed by the schema alone. Sizing it by how
many optional fields are present would be circular — the reader needs the bitmap to know that.

**The other three bitmaps exist iff the schema declares at least one non-key field with that
capability, and each is `ceil(q/8)` bytes.** `q` is the number of that capability's fields **present
in this row**, in schema order: **every required qualifying field — which has no presence bit and is
always present — plus every optional qualifying field the presence bitmap marks present.** Omitting
required fields from `q` would leave a required nullable, unknown-capable or quantized field with
two plausible encodings. A row in which no qualifying field is present spends zero bytes on that
bitmap. This is what makes four states cost nothing on records that need none of them.

Fields the schema marks **required** carry no presence bit — a bit spent on a field that can never
be missing costs a byte per row on wide records for nothing. d56 has nine non-key fields of which
at least `fleet_id` and `owner_tag_db_id` are structurally required.

**Absent, null, zero and unknown are four different things.** This is a fidelity rule:

| State | Encoding | Value bytes |
|---|---|---|
| **absent** — not applicable, or unsupported by this schema | presence bit `0` | none |
| **null** — applicable, the relation genuinely points at nothing | presence `1`, null `1` | **none** |
| **zero** — applicable, value is 0 | presence `1`, null `0`, unknown `0`, value `0` | yes |
| **unknown** — applicable, not captured | presence `1`, unknown `1` | **none** |

Exactly one state holds per field. **A field may set at most one of its null and unknown bits, and
its escape bit only when it carries a value** — presence `1`, null `0`, unknown `0`. Any other
combination is corrupt: the reader rejects the frame and gaps that domain until its next baseline.
Letting one silently win would let a writer bug pass as data.

**Known-null is not not-applicable.** d56 distinguishes a task force with no repair target (null,
and true) from a field this schema does not support (absent). Collapsing them loses the difference
between "we know there is nothing" and "we do not carry this".

Sentinels (`-1`, `u32::MAX`) are **forbidden** as encodings of absence, null or unknown: they
collide with real data, silently.

## 4. Scalars

| Code | Kind | Encoding |
|---:|---|---|
| `0` | unsigned | `varint` |
| `1` | signed | `zigzag` |
| `2` | monotone counter | **absolute form: `varint`.** In `BASELINE` and wherever else an absolute value is called for, absolute. In `DELTA`: `zigzag` difference against the reconstructed prior value |
| `3` | enum | bit-packed to `ceil(log2(n))` bits, cardinality `n ≥ 2` declared in the schema. **A code `≥ n` is corrupt**: the reader rejects the frame and gaps that domain until its next baseline, and a value the writer cannot map into `0..n-1` is a failed sample and a typed gap, never a coerced code |
| `4` | boolean | one bit |
| `5` | content id | `varint` symbol id, §5 |
| `6` | continuous | §6 |

The codes are normative: they appear in the schema serialization (§11).

**The delta base is the reconstructed prior value** — what the field holds after replaying every
frame from the last `BASELINE` up to the immediately preceding covered tick. Not "the previous
frame", which may be an `UNCHANGED` run or a `DELTA` that did not carry this field.

**The monotone difference is computed modulo `2^64`.** Both endpoints are `u64`, so their
mathematical difference need not fit an `i64` — a counter going `0 → 2^64 − 1` differs by more than
`i64` can hold. The difference is therefore taken as wrapping subtraction in `u64`, reinterpreted
as `i64`, then zigzagged; the reader adds it back the same way. This recovers every endpoint
exactly, and every pair of valid `u64` endpoints has exactly one encoding. Without the rule stated,
an implementation using checked arithmetic errors where this one wraps, and the two disagree on
bytes for state both consider valid — the difference is a compression device, not a claim that
counters only rise.

**A field with no concrete prior value is emitted absolute.** If the reconstructed prior state has
it absent, null or unknown, there is nothing to difference against. The reader sees the same
reconstructed state, so it knows to expect the absolute form.

**Baselines are absolute — always.** A baseline whose counter is encoded against a previous frame
would, after a gap, be encoded against a value nobody has, making the baseline non-self-contained
and the counter unrecoverable.

**Monotone is an encoding hint, never a validity constraint.** d56's `sunk_convoys` is a task-force
aggregate of per-ship counters, and ships transfer between task forces, so it legally decreases. A
decoder that rejects a negative delta rejects valid data.

Whether a second difference beats a first for counters is **OPEN** (§13): a steady increment above
63 costs two bytes as a first difference and one as a second, so the answer is per-field and needs
corpus evidence.

## 5. Symbols

Country tags, equipment names, province and state ids are content-defined. The stream carries
**manifest ids as varints**, never text.

This format requires that id assignment be **deterministic for a given content set** — two
recorders on the same content must mint the same id for the same symbol, or no two files are
comparable and no golden vector is stable. **That assignment rule does not exist yet and is listed
OPEN in §13.** `PORTABLE_CONTENT_MANIFEST.md` defines content and VFS identity, which is a
different thing and does not supply it.

**Symbol ids are file-global.** Symbols first seen mid-campaign are appended to a single
**append-only overflow table** in the container, each entry recording the `(branch, tick)` at which
it became available — the global tick of §1.1, which is why that unit is file-wide rather than
per-domain. A per-branch id space would store the same string once per branch at identical
fidelity, which the size rule forbids.

A reader seeking to tick `T` on branch `B` needs the overflow table's entries whose first-seen tick
is `≤ T` on `B` or an ancestor branch. That is a container read, not a stream scan. **In-stream
interning is forbidden** — it must be fully read before any seek.

## 6. Continuous values, quantization, derived fields

Each continuous field declares one of:

- `exact` — the `f64` bit pattern, 8 bytes, in the value region. **The default.**
- `fixed(k)` — `zigzag(scale(v, 10^k))` in the value region, **`0 ≤ k ≤ 22`**.
- `ratio(d)` — a `d`-bit unsigned in the bit-packed region, `1 ≤ d ≤ 32`, holding
  `scale(v, 2^d − 1)`.

**`scale(v, m)` is pinned to IEEE binary64 arithmetic**: compute `v × m` as a binary64
multiplication with round-to-nearest-even, then round that result to an integer half-to-even. Both
steps are named because they disagree: `0.15` under `fixed(1)` yields `2` under this rule and `1`
under exact-real arithmetic. Every implementation does the binary64 multiplication.

**`dequantize(q, m)` is pinned the same way**: convert the integer `q` to binary64 with
round-to-nearest-even, then perform **one binary64 division** by the same multiplier `m` that
`scale` used. Division, not multiplication by a precomputed reciprocal — the two disagree, and
because the escape rule below is a computation over `dequantize`, that disagreement is not confined
to a last digit: under `fixed(11)` the two readings classify **6,236** stored integers near `2^53`
differently, so one implementer would store the integer where the other escapes to raw `f64`.
Different bytes from identical state is the one thing this document exists to prevent.

**`k` is bounded at 22 so the multiplier itself is unambiguous.** Every power of ten up to `10^22`
is exactly representable in binary64; past that, a writer computing `10^k` by repeated
multiplication and one converting the decimal literal land on adjacent binary64 values and round
the same input to different integers. Within the bound there is one multiplier and no way to
disagree. `2^d − 1` is exact for every `d ≤ 32` by construction.

### What may be quantized

A field is `exact` unless its quantization is justified in the domain document, and **two
independent tests can each veto it**:

- **The outcome test.** Not for a value whose exact magnitude explains a threshold crossing, where
  rounding changes the story the replay tells.
- **The appearance test.** Not for a value the viewer **interpolates or animates between samples** —
  position, progress along a path, orientation, scale, blend weight. DECISIONS requires the rendered
  replay to match the game; a rounded position does not change any outcome, it just makes a unit
  stutter where the game glides, and the outcome test alone would let that through.

A field that survives both tests may be quantized. One that fails either is `exact`.

**Being drawn is not by itself disqualifying.** A value the viewer reads once per frame and renders
at low visual precision — a bar length, a badge threshold — may be quantized, provided the domain
document records the precision and shows the rounding is invisible at maximum zoom. d20's
`move_progress` fails the test because motion between provinces is interpolated from it; its
`organisation` and `strength` do not, because they set bar lengths a few hundred pixels wide where
five decimal places are far past anything visible. The distinction is interpolation, not exposure.

**The escape is a bitmap bit, not a reserved code.** A quantized field whose escape bit (§3) is set
carries raw `f64` bits, 8 bytes, in the value region instead of its quantized form. Reserving a
code inside the value cannot work: `ratio(d)` spends all `2^d` codes on the range, and `fixed(k)`
is an unbounded zigzag varint with no spare code at all. A bitmap bit costs nothing when unused and
works identically for both kinds.

**The escape fires in exactly these cases, and no others.** They are tested **in order**, because
the fourth is a computation over a stored integer `q` that only exists once the third has passed:

- any non-finite value — NaN or ±infinity — or negative zero;
- for `ratio(d)`, any `v` outside `[0.0, 1.0]`;
- for `fixed(k)`, any `v` whose `scale(v, 10^k)` is not representable as an `i64`;
- **any `v` whose stored integer `q` does not satisfy `quantize(dequantize(q)) == q`**, where
  `quantize` is the stored-integer computation above — `scale(v, 10^k)` for `fixed(k)`,
  `scale(v, 2^d − 1)` for `ratio(d)` — and **the comparison is between integers**, not between
  the floats they came from.

That last condition is not an optimisation, it is what makes §7.1 work. §7.1 obliges a writer to
compare against the state the *reader* reconstructs. Where the round trip is not a fixed point, an
unchanged source value differs from its own reconstruction: the writer emits a delta and the reader
rejects it as a mask bit on a field that did not change, from legal input. The property fails once
the stored integer leaves the range where `f64` holds integers exactly — under `fixed(11)`,
`90071.99254740993` scales up to `2^53` and dequantizes to a float that scales back to `2^53 − 1`.
It is stated as a computation rather than as a bound on `k` or on `v`, because every operation in it
is pinned binary64 (§0) and a bound would be a claim about float round-trips that this document
would then have to prove.

**A reader enforces the fourth condition too.** A stored integer that is not its own
reconstruction's quantization is one no legal writer emits, and two such integers can share a
reconstruction — so accepting one gives a single value two encodings, exactly what the mirror rule
(an escape bit set on a value that satisfies no escape condition) already forbids.

Ordinary rounding loss is **not** an escape condition — quantization exists to round, and escaping
on it would defeat the declaration entirely. Every `f64` takes exactly one branch, with no declared
range to disagree about.

A field whose escape fires often is mis-declared and reverts to `exact`.

An unquantized float that jitters in its low bits is never byte-identical, so it defeats
`UNCHANGED` and inflates every delta. This is where fidelity and size actually trade, per field.

### Derived fields

A field that changes **deterministically with the clock** may be declared `derived` and stored in a
form that does not change. The stored value is

```
  stored = v + c × (frame_tick >> u)
```

for a declared integer `c` and a declared shift `u`, `0 ≤ u ≤ ticks_per_hour_log2`, computed in
`i64`; overflow is a hard writer error, not a wrapped value.

**The transform is applied at the capture and replay boundaries, not inside the codec: a record
holds stored values.** Capture converts a live value to its stored form before building a record;
replay converts back after decoding one, for whichever tick it is drawing. Everything between —
`UNCHANGED`, the change mask, the minimality rules, every comparison §7.1 calls for — then operates
on stored values and needs no tick at all.

The alternative, transforming inside the encoder, is what makes the mechanism fail to pay: every
comparison would need the ticks of *both* records, reconstructed state would have to carry a tick
per row, and the tick-invariance that lets a countdown collapse into one `UNCHANGED` run would stop
being a property of the stored value and become an emergent property of the comparison. It is a
stated contract rather than a caller's private business because a transformation applied outside
the format cannot enforce the overflow rule above, and cannot supply the inverse a reader needs.

**A derived field's stored value is always encoded as an absolute `zigzag`, whatever its kind and
whatever the frame.** Zigzag because the transform can carry an otherwise unsigned quantity
negative; absolute because kind `2`'s `DELTA` differencing rule (§4) does not apply to a derived
field — a stored value that is constant by construction has nothing to difference against, and
allowing both readings would make `zigzag(stored)` and `zigzag(stored − prior)` equally defensible.

**The shift exists because a field's clock is not always the tick.** A countdown measured in game
hours advances once per `2^ticks_per_hour_log2` ticks, so a per-tick coefficient cannot hold it
constant for any integer `c`: the value would drift by `2^ticks_per_hour_log2 − 1` every hour and
the domain would emit a delta on every sample forever — the exact cost the mechanism exists to
remove. `u` names the field's own unit; `u = ticks_per_hour_log2` makes the term the game hour.

The transform is **clock-linear only**. An arbitrary expression would need a grammar, an evaluator
and its own determinism proof, and the one saving that motivates it is d64: `remaining_hours`
decrements by exactly one every game hour
(`extractor/documents/domains/d64_delayed_events_live.md:27`), so every pending row would otherwise
emit a delta every hour forever. Declared with `c = 1` and `u = ticks_per_hour_log2`, the stored
value is `remaining_hours + frame_hour` — the due hour, a lossless bijection given the frame tick,
and what replay already computes. The row then stays constant until the event is created or
dispatched, collapsing the domain to `UNCHANGED` runs.

A domain author checks every field against this before accepting a per-sample delta, **and states
the field's clock unit when doing so**. A field that changes every sample by a constant is a red
flag, not a fact of life.

## 7. State frame bodies

A domain's schema declares an ordered list of collections.

`BASELINE` body — every collection, in schema order:

```
  row count           varint
  [ row ]             in canonical order (§2)
```

`DELTA` body:

```
  collection change bitmap   ceil(C/8) bytes, C = collections in the schema, schema order
  [ contribution ]           for each collection whose bit is set, in schema order
```

**A collection whose bit is clear contributes no bytes.** Without the bitmap, an ordered collection
would be fully re-serialized every time any *other* collection changed — d56's supremacy list
re-emitted whenever one task force moved. That is pure waste at identical fidelity.

**What a set collection contributes depends on its shape:**

- **keyed set** — the delta grammar below.
- **ordered keyed list, ordered occurrence list** — a **full replacement**: `varint` row count then
  every row, identical to that collection's `BASELINE` contribution. An ordered collection has no
  delta grammar, so "changed" means "replaced".

A mixed-shape domain is therefore encodable in one frame: d56's task-force map contributes a delta
while its supremacy list contributes a replacement, or nothing at all.

### 7.1 Minimality

A `DELTA` must be the *only* encoding of what changed, so every level of it is pinned:

- **A collection's bit is set if and only if its reconstructed content differs this tick.** A set
  bit whose keyed contribution carries zero changed rows and zero tombstones is invalid; so is a
  set bit whose ordered replacement is byte-identical to the reconstructed prior content.
- **A change mask bit is set if and only if that field's state or value differs.** An all-zero
  change mask is invalid, and so is a mask bit on a field whose value is re-emitted unchanged.
- **A `DELTA` whose change bitmap is all zero is invalid** — that state is `UNCHANGED`.

Each of these would otherwise be a second legal encoding of "nothing happened", which both splits
byte-identity and wastes bytes at equal fidelity.

**Throughout §7, "differs" means differs *as stored*** — as the bytes this schema would hold, not
as the source value. Under `fixed(2)` a field moving from `0.33` to `0.331` is a different value
and the same stored value, so no bit is set and no row is emitted. This is not an optimisation: a
writer comparing source values emits a row that reconstructs to the state the reader already has,
which the reader must then reject. Quantization and the escape rule are both part of the stored
form, so an escaping value and a non-escaping one always differ.

**The writer compares against the reconstructed prior state, not against its own last input.**
That is the state the reader holds, and minimality is defined relative to it.

A keyed set's key sets are likewise exact, or one update could be split across two rows:

- **Every key appears at most once** across the changed-row list, and at most once across the
  tombstone list.
- **The two lists are disjoint** — a key is updated or deleted this tick, never both.
- **The changed-row list is exactly the keys that are new this tick plus the surviving keys whose
  record content differs**, no more and no fewer. Omitting a changed or newly-inserted row loses
  data without breaking any of the rules above, so it is stated as its own requirement.
- **The tombstone list is exactly the keys present in the reconstructed prior state and absent
  now**, no more and no fewer. A tombstone for a key that was already absent is invalid, and so is
  omitting one for a key that has genuinely gone.

### 7.2 Keyed-set delta grammar

```
  changed row count   varint
  [ changed row ]
  tombstone count     varint
  [ key ]             sorted, delta-encoded per §7.4
```

**Every changed row begins with its key**, encoded per §7.4. The reader reads the key, looks it up
in the state it has already reconstructed, and only then knows which grammar follows:

- **key not present → insert.** The rest of the row is a full record per §3 — non-key fields only,
  since the key prefix already carried the key.
- **key present → update.** The rest of the row is:

```
  change mask         ceil(F/8) bytes, F = non-key fields, one bit each, schema order
  new presence bits   bitmap, one bit per CHANGED OPTIONAL field, ascending changed-field order
  new null bits       bitmap, one bit per CHANGED NULLABLE field, ascending order
  new unknown bits    bitmap, one bit per CHANGED UNKNOWN-CAPABLE field, ascending order
  new escape bits     bitmap, one bit per CHANGED QUANTIZED field, ascending order
  [ bit-packed region ]   changed fields that carry a value, classified per §3 — enums, booleans
                          and unescaped ratio(d) — zero-padded to a byte boundary
  [ value bytes ]         the remaining changed fields that carry a value, per §3
```

Each of the four bit groups is a §0 bitmap: byte-aligned, zero-padded, zero bytes when empty. A
changed field whose new state is absent, null or unknown carries **no value**. **A changed field
whose new presence bit is `0` has its null, unknown and escape bits `0`** — it is absent, and the
other three states describe present fields.

**A change mask alone cannot express a field becoming absent, null or unknown**, and that
transition is routine: d56's repair-target province pointer goes non-null → null the moment a
repair completes. One bitmap cannot both select changed fields and carry their new state. Without
the extra bits a writer must either drop the transition — leaving a stale repair target in every
reconstruction until the next baseline — or emit a whole baseline for one cleared pointer.

Keys are immutable: a row whose key changes is a tombstone plus an insert, never an update.

### 7.3 Collection shapes

**Not every collection is a keyed set, and treating one as such destroys data.** Each collection
declares one shape:

- **keyed set** — unique stable key, no meaningful order. Upsert and tombstone semantics. The
  default, and **the only shape with a delta grammar**.
- **ordered keyed list** — unique stable key **and** meaningful physical order. d11's deployment
  queue is this: `extractor/documents/domains/d11_production_live.md:51`–`:52` records that
  conveyor and line `(id,type)` pairs are lifecycle keys and that source order is authoritative
  with contiguous ordinals, and the codec enforces ordinal contiguity and key uniqueness at
  `extractor/native/lanes/country_economy_production_stockpile_live/src/deployment_queue.rs:373`–`:376`
  and `:419`–`:422`. **Full replacement only.**
- **ordered occurrence list** — meaningful physical order, duplicates legitimate, no unique key.
  d56 naval supremacy is this: `extractor/native/recorder_host/src/world_fold.rs:22123` classifies
  it as an order-preserving occurrence list, and `:22131`–`:22133` records that the game's array
  "can carry several distinct entries for the same `(region, controller)`; collapsing them would
  (`extractor/documents/domains/d64_delayed_events_live.md:27`). **Full replacement only.**

A delta against either ordered shape would **double-append** rather than upsert, which is why
production already full-replaces them.

**An ordered array whose order is a total function of its carried fields does not need these
shapes.** Where the position is carried, or derivable at the boundary as a dense ordinal, the
collection stays a keyed set: it keeps the order, keeps the delta grammar, and refuses nothing
the fold accepts. The position rides in the key where the rows have no identity of their own
(d11's construction queue) and as an ordinary field beside a fold-enforced key where they do
(d11's production lines and convoy variant runs, both `Shape::KeyedSet` in the ported schema,
`extractor/native/wire_format/src/d11.rs:504`–`:542` and `:628`–`:649`) —
`extractor/documents/DOMAIN_SCHEMA_RECIPE.md` step 5 gives the test. The
shapes above are needed exactly where that test fails: where no carried field recovers the
order (d11's `convoy_priority_reservations`), or where duplicates are legitimate, no unique key
exists and no position persists (d16's `timed_live`, d56, d64).

**A domain with legitimately duplicate identities is not a broken keyed set.** d64's duplicates
carry a `scope.random` seed that the oracle folds into its join key
(`extractor/native/lanes/scripted_state_flags_variables_live/scripted_state_compact.rs:523`–`:527`),
so they are not identical — but that seed is not proven stable or unique across a reload, so it
cannot be a wire key. As an occurrence list, d64 needs no key at all and both the duplicates and
the order survive.

Full replacement costs little here: combined with the `derived` due-tick transform (§6) d64's
replacement bytes are **constant between creation and dispatch**, so `UNCHANGED` still fires and
the domain stays near-silent. Replacement is paid only on ticks something happens.

A domain that has not declared its collection shapes is not reviewable: the same bytes mean
different things under each.

### 7.4 Key encoding

**A key member is restricted to kinds `0` (unsigned), `1` (signed) and `5` (content id)** — the
three that encode as a self-delimiting varint or zigzag on their own. Enums and booleans live in a
record's shared bit-packed region and have no bit container in a key prefix; a monotone counter's
encoding depends on frame context. A schema declaring any other kind as a key member is invalid
(§11).

A key is the schema's key fields in `key_position` order. A component is **absolute** when encoded
by its §4 kind encoding.

- **Keyed set — `BASELINE` rows, `DELTA` changed rows, tombstones.** All three lists are sorted by
  the full key tuple, so all three use the same scheme: **the first component of the first row is
  absolute; each subsequent row's first component is an unsigned `varint` difference from the
  previous row's; every later component is absolute.** Sorted order makes the first component
  non-decreasing, so the difference is never negative and plain `varint` beats `zigzag` by a bit.
  Later components jump around under sort and would not compress.
- **Ordered keyed list rows.** Source order, not key order, so a difference has no reason to be
  small or non-negative: **every component is absolute.**
- **Ordered occurrence lists** have no key.

**A delta may only be emitted from a walk proven complete** (pattern §3.2). An incomplete walk
makes missing keys look like deletions, and the tombstone list would encode real data loss as
intent. An unproven walk emits a gap and the prior state stands.

## 8. Gaps

Gaps are recorded, never implied by absence. Per DECISIONS a gap carries range, reason, affected
domains and witness. **Gaps live in a container-level gap stream, not inside domain blocks** — one
record whatever its breadth, decodable without decoding any domain block, so a reader that refuses
one domain still sees every gap.

| Field | Type |
|---|---|
| `tick_start` | `varint` — global ticks, §1.1 |
| `tick_end_delta` | `varint` from `tick_start`; `0` is a single tick |
| `branch` | `varint` |
| `reason` | `u8` |
| `part` | `u8` — `0` state, `1` event, `2` both |
| `domain count` | `varint` |
| `domain` × count | `varint`, sorted ascending, delta-encoded against the previous |
| `witness_kind` | `u8` |
| `witness` | per kind |

**`branch` and every `domain` are bounded at `2^32 - 1`.** Both are unbounded varints on the
wire; accepting a larger value would let one reader truncate it onto a different valid identity
while another refuses it. **`domain count` is at least one.** Unlike the retired `H4GP` record,
§8 has no empty-list-means-every-domain sentinel: an empty list is corrupt rather than a gap over
every domain. The domain list is strictly increasing, so every delta after the first is nonzero;
a zero delta repeats a domain and is corrupt.

**`reason` is one of `0..=8`, `part` is one of `0..=2`, and `witness_kind` is one of `0..=4`.**
An unassigned value is corrupt rather than a future cause that cannot participate in step 2's
total order. A witness payload must have exactly its kind's declared shape: a short payload and
trailing bytes are both corrupt.

`part` is what lets a gap in a hybrid domain's event stream leave its reconstructed state valid
(§10).

**One normalisation algorithm is pinned**, because a set of gapped domains has many otherwise-legal
groupings. "Maximal" alone does not choose: if A is gapped over ticks 1–3 and B over 2–4, then two
per-domain records and the three slices `{A}`, `{A,B}`, `{B}` are all maximal in their own terms.

Stored ranges are **inclusive** — `tick_end_delta = 0` is one tick — so normalisation first
converts each to the half-open interval `[tick_start, tick_start + tick_end_delta + 1)` and
converts back on emit. If either checked addition overflows `u64`, the record is corrupt; wrapping
would move a gap at the end of the timeline to its beginning.

Normalisation is **per `(domain, part-component)`**, and only then grouped. Slicing the whole
timeline at every domain's endpoints would fragment one long outage at every unrelated domain's
boundary, which is size waste at identical fidelity.

Partition the observations by `branch`, normalise each partition independently, then combine the
partitions into the one sorted output stream. A branch is a partition key, not a filter: records
from distinct branches never merge and none is discarded.

Within one `branch`:

1. Take every gapped observation as a half-open interval keyed by
   `(domain, part-component, reason, witness)`, where **`witness` means the `(witness_kind,
   payload)` pair** — grouping on the payload alone would merge records of different kinds.
   **`part = 2` is two observations**, one per component, so overlap is always compared
   component-wise and never by the literal byte.
2. **Resolve overlapping causes per tick.** Where one `(domain, part-component)` is covered at a
   tick by observations with different `reason` or `witness`, that tick's cause is the one with the
   **lowest `reason` code**, then lowest `witness_kind`, then the smallest witness payload compared
   **as its canonical encoded bytes, unsigned and bytewise, shorter-is-smaller on a prefix**. The
   comparison representation is pinned because it changes the answer: counter ids `255` and `256`
   order one way as numbers and the other as the varints `FF 01` and `80 02`.
   **A losing observation is masked only over the ticks the winner covers and keeps its cause
   everywhere else** — dropping it wholesale would erase gap coverage outside the overlap,
   leaving ticks with neither frames nor a gap record, which is the silent omission this section
   exists to prevent.
3. **Take maximal spans.** Each `(domain, part-component)` now yields maximal contiguous spans over
   which its cause is constant.
4. **Group spans sharing exactly `(tick_start, tick_end, reason, witness)`** into one record's
   domain list. Nothing else merges: two domains gapped over different ranges stay separate records
   even under one cause.
5. **Split each group by part signature.** A domain in the group **via both components under this
   group's own `(reason, witness)` and range** goes in a `part = 2` record, state-only in
   `part = 0`, event-only in `part = 1`. A domain state-gapped by one cause and event-gapped by a
   different one is in two groups and is never `part = 2` in either. Up to three records per
   group; empty ones are not emitted. A record carries one `part` byte for its whole domain list,
   so a torn sweep hitting pure-state and hybrid domains together is two records, not one.

Deterministic and terminating: step 2 is a total order on causes, steps 3–5 only maximality and
equality. For A over ticks 1–3 and B over 2–4, both state-only under one cause, it yields two
records — `{A}` over 1–3 and `{B}` over 2–4 — not three slices.

**No two records overlap** in time for the same `(domain, part-component, branch)`.

**Records are sorted by `(tick_start, branch, reason, part, first domain)`**, which is total: two
records sharing all five would have to overlap in a component, which step 2 and the ban forbid.

| `reason` | Meaning |
|---:|---|
| `0` | torn sweep |
| `1` | source unavailable |
| `2` | partial data |
| `3` | coalesced |
| `4` | skipped |
| `5` | bridge unavailable |
| `6` | worker panic |
| `7` | coverage shortfall |
| `8` | backpressure |

| `witness_kind` | Payload |
|---:|---|
| `0` | counter: `varint` counter id, `varint` observed value |
| `1` | status bits: `varint` bit width, then `ceil(width/8)` bytes |
| `2` | coverage: `varint` observed, `varint` expected |
| `3` | tick bracket: `varint` start tick, `varint` end tick |
| `4` | tick discontinuity: `varint` previous tick, `varint` next tick |

## 9. Event frames

```
  event count       varint, ≥ 1
  [ event record ]           count full §3 records, in occurrence order
  frontier                   one varint per schema frontier component, AFTER this batch
```

The body contains exactly `count` records and exactly the schema's frontier arity; a decoder
rejects a zero count, a short frontier or any trailing byte. Every event record is a **full** §3
record. Events have no key, tombstone, insert/update distinction, change mask or
minimality-against-prior: those are state operations over a reconstructed prior (§7), while an
event is an append-only occurrence. Two byte-identical records in one batch are two events and
collapsing them is data loss.

Every event field uses §3's **absolute full-record** encoding. There is no prior event record
against which a field can be differenced:

| Declaration | Event-record rule |
|---|---|
| `unsigned` | absolute `varint` |
| `signed` | absolute `zigzag` |
| `monotone` | absolute unsigned `varint`; §4's prior-relative `DELTA` form never applies |
| `enum(n)` | §3 bit-packed code, with `n` declared and the code in `0..n-1` |
| `boolean` | one §3 bit |
| `content id` | absolute manifest-id `varint` |
| `continuous` | §6 `exact`, `fixed(k)` or `ratio(d)`, including the escape rule |
| `optional`, `nullable`, `unknown-capable` | the §3 presence/null/unknown bitmaps, unchanged |
| key | forbidden: event identity comes only from position plus frontier |
| `derived` | forbidden on an event field |

The `derived` prohibition is semantic, not an implementation shortcut. §6 transforms one record
against `frame_tick`; an `EVENTS` frame has one observation tick but may contain occurrences from
different occurrence ticks. Choosing the observation tick, an event field's occurrence clock, or
some other clock would produce different stored values from the same batch. A producer instead
materialises the concrete event value under its ordinary kind and, where needed, carries the
occurrence clock as another ordinary field. Schema validation and the event-body codec both reject
a derived event field.

Events are written in **producer occurrence order** and never reordered. One `EVENTS` frame per
observation tick per `(domain, branch)` carries every event the producer observed for the interval
ending at that tick. The frame tick is the **observation/drain tick, not necessarily the event's
occurrence tick**. A domain whose source carries an occurrence clock declares it as an ordinary
event field. d64 does: DEL2 reads each event's `game_hour_raw` at row `+8`
(`extractor/native/recorder_host/src/world_fold.rs:9733`–`:9735`). The earlier d64 event
measurement is invalid and its counts are not evidence; the corrected run must retain this field
and account for incomplete observations through §8 before promoting any corpus figure. Empty
batches are invalid and a second batch at the same tick is invalid, so a given non-empty
observation has exactly one legal framing.

**The frontier is composite and schema-defined**, not a single scalar. The schema declares an
ordered list of frontier components, each with a `component_id`, and designates exactly one as the
**sequence**; the rest are **epoch** components. The frame carries one `varint` per component.

d64's continuity is `(hook_epoch, sequence)`:
`extractor/native/recorder_host/src/world_fold.rs:9707`–`:9710` reads `hook_epoch`,
`first_sequence` and `last_sequence`, and the epoch check at `:9655`–`:9669` refuses continuity
across an epoch change — the `Some(_) => false` arm at `:9668` — even when the sequence numbers look
contiguous. A single high-water mark cannot express that: after a reattachment the epoch changes
while sequences may restart or appear to continue, so a scalar mark would let d64 silently
duplicate or skip lifecycle events.

**Event identity is derived, never carried.** For record position `i` in a batch of `count`, the
identity sequence is

```
  frontier_after.sequence - count + i + 1
```

and the frontier sequence must be at least `count`. Thus one batch is a dense contiguous sequence
run ending at the after-frontier, in producer occurrence order. d64's shipping row carries a
sequence word at `+0`, but its fold requires that word to equal `first_sequence + index`
(`extractor/native/recorder_host/src/world_fold.rs:9733`, `:9743`–`:9747`), so carrying it again
would encode the same fact twice and admit contradictory bytes. Content is never identity:
duplicate records remain distinct because their positions derive distinct sequences.

This frontier is what a state frame cannot express. A `BASELINE` is self-contained and repairs any
amount of lost state history. An append-only history has no baseline: a lost event is permanently
lost, and whether the interval since the preceding batch was complete cannot be decided from the
new records alone. The producer's after-frontier turns that interval claim into a checkable
counter. Event frames are therefore not “state frames with another tag”: §7 repairs state by
resynchronisation, while §9 preserves occurrences and detects history loss.

Continuity rules:

- Frontier components join across schema definitions by `component_id`, never by tuple position.
  Reordering an unchanged set of ids is continuous. Adding, removing or duplicating a component,
  or changing which id is the designated sequence, produces `SchemaChanged` and an event-part gap.
- A batch whose **epoch components differ** from the previous batch's is a discontinuity: the
  reader emits a gap for that domain's event part rather than assuming the streams join.
- Within one epoch, the batch's frontier sequence must equal the previous frontier's sequence plus
  `event count`. Checked-add overflow produces `SequenceOverflow`; any other value produces a
  sequence mismatch. Both are discontinuities and event-part gaps.
- **The first batch of a `(domain, branch)`, and the first batch after a recorded event-part gap,
  establish the frontier rather than being checked against one.** Continuity across a gap is
  exactly what the gap says was lost; re-checking it would turn one recorded gap into a permanent
  refusal of everything after it.

**Completeness is a producer precondition.** An `EVENTS` frame may be emitted only for an
observation interval whose producer-specific witness proves that every event in the interval was
admitted. An unproven interval emits an event-part gap and no event batch; a subset must not be
presented as “every event observed at this tick”. d64's DEL2 `COMPLETE` bit is exactly this witness:
it requires a non-zero hook epoch, a registered dispatch observer, no sticky completeness poison
and zero reentrant, unreadable, correlation, overflow and lock-busy drops
(`extractor/native/collector/src/attach_log.rs:61057`–`:61065`); its fold refuses publication when
the bit is clear (`extractor/native/recorder_host/src/world_fold.rs:9655`–`:9672`).

d64 assigns `sequence` when it constructs the observed event, before trying to retain it in the
queue (`extractor/native/lanes/scripted_state_flags_variables_live/src/a729_delayed_event_lifecycle.rs:476`–`:503`).
Consequently a queue overflow or lock-busy loss normally leaves a sequence jump for the next
frontier to expose. Specifically, the retain transforms construct their events before calling
`retain_all` (`:416`–`:473`), whose unreadable, overflow and lock-busy refusals are at
`:495`–`:506`. That arithmetic is not sufficient on its own: the hourly and
definition-cancellation hooks have unreadable, reentrant and correlation returns before their
`retain_*` calls can reach `event_from` (`:335`–`:373`, `:376`–`:413`), so those losses burn no
sequence and are invisible to the frontier. A final post-assignment drop also has no later batch
whose frontier can reveal the jump. The poison operation makes either class sticky
(`:719`–`:721`). The completeness witness is therefore normative, not belt-and-braces.

An ordinary proven-complete observation with zero events emits no `EVENTS` frame and no gap;
silence is represented by no event frame, never by a zero-count body. An incomplete observation,
including one that retained zero events, emits an event-part gap and no batch.

An epoch-only observation does **not** make an empty `EVENTS` frame legal. If the producer observes
an epoch change while draining zero events, it records an event-part gap at that observation tick;
if it cannot report that producer-side fact, the reader detects the epoch change at the next
non-empty batch. A frontier-only empty frame would be a second encoding of silence and would make
`event count ≥ 1` false. d64 takes the immediate producer-side path even on a zero-count trailer
(`extractor/native/recorder_host/src/world_fold.rs:9655`–`:9669`).

The frontier is recorded **after** the batch, so a restart resumes from it without re-reading or
skipping. No baselines, no tombstones. A gap here means "events between A and B were not
captured", carried with its own reason so the viewer says that rather than "state unknown".

The recovery rule above deliberately differs from d64's current production fold. The fold sets
`delayed_event_lifecycle_history_failed` on the first incomplete or discontinuous observation and
never clears it, so every later hour remains null. The flag is declared and initialized false at
`extractor/native/recorder_host/src/world_fold.rs:1702`, `:2403`; its parse-failure and settlement
assignments only set it true (`:9524`, `:9655`–`:9682`). §9 instead records one gap and
lets the next proven-complete batch establish a new frontier: events before the gap remain
incomplete, but permanently discarding all later known events would not repair them. Ports that
measure the production fold must name this divergence rather than quietly substituting either
policy.

## 10. Hybrid domains

A domain may declare **both** parts — d64 is mutable pending state *plus* an append-only lifecycle
trailer. Its block carries state frames and `EVENTS` frames under one `domain`, on the two
independent time chains of §1.4, and each part follows its own rules.

Migration may stop one part without stopping the other. A stopped part declares no collection in
the migrated schema and emits no frames; its collection ids remain reserved and the stop is
reported in the migration ledger rather than fabricated as state or charged to the implemented
part. The other part remains independently framable. d64 is the concrete case: its pending table
is an ordered occurrence list. The reference port currently reserves state collection id 1 and
declares only lifecycle event collection id 2. This is a migration state, not a claim that the
source domain ceased to be hybrid. The ordered-state codec is available, but d64's pending-state
port remains unimplemented and unevidenced; it is added only after its mapper, schema, fold-parity
tests and real-corpus measurement pass their own gates.

Every rule is scoped:

- `BASELINE`, `DELTA`, `UNCHANGED`, the delta-before-baseline rule and the forced-baseline-after-
  discontinuity rule apply to the **state part only**. Event streams have no baselines; their
  continuity mechanism is the frontier (§9).
- **`UNCHANGED` and the dedup floor scope over the state part only.** An event batch never makes
  the state part look changed, and a quiet state part never suppresses events. An `UNCHANGED` run
  spans ticks that carry events, because events are on the other chain.
- A gap record's `part` field selects which part it affects, so a gap in the event stream leaves
  the reconstructed state valid.

## 11. Schema, versioning, mods

A schema describes a domain: its cadence, its collections, their shapes and fields, and its event
part if any.

**`(domain, collection_id, field_id)` is the machine identity of a captured field**, and every
component is assigned append-only and never reused for a different thing:

- `domain` is unique across the registry and permanent once assigned.
- `collection_id` is unique within its domain and permanent. **The event part carries one too**, so
  an event field has the same three-part identity a state field does and the render-coverage ledger
  can join on it.
- `field_id` is unique within its collection — the event part included — and permanent.

Without this a reader holding an unknown schema definition can decode bytes into values but cannot
say what any of them mean, and the render-coverage ledger (pattern §11) has nothing to join on but
names — which would silently pair one domain's `status` with another's.

### Canonical serialization

Hashed and stored exactly as below. Every member is a `varint` except `derived_c`, which is
`zigzag` because a coefficient may be negative.

```
  format_tag          varint  = 1
  cadence_log2        varint  log2 of the number of samples per game hour
  collection count    varint
  per collection:
    collection_id     varint
    shape             varint  0 keyed set, 1 ordered keyed list, 2 ordered occurrence list
    field list                per the field grammar below
  event part          varint  0 absent, 1 present
    if present:
      collection_id     varint  the event part's own identity, distinct from every state collection
      field list              per the field grammar below — the event record's own fields
      frontier count    varint
      component_id      varint  × frontier count
      sequence index    varint  which component is the sequence

  field list:
    field count       varint
    per field:
      field_id        varint
      kind_code       varint  §4 table
      flags           varint  bit0 optional, bit1 nullable, bit2 unknown-capable,
                              bit3 key member, bit4 derived
      key_position    varint  iff bit3 — 0-based position in the key tuple
      enum_card       varint  iff kind_code = 3
      quant_code      varint  iff kind_code = 6 — 0 exact, 1 fixed, 2 ratio
      quant_param     varint  iff quant_code ≠ 0 — k for fixed, d for ratio
      derived_c       zigzag  iff bit4 — the coefficient
      derived_u       varint  iff bit4 — the shift naming the field's clock unit
```

The event part carries its own field list because §9 event records are §3 records: without it the
hash would not determine how an event decodes, and two domains with different event records could
share a hash. Frontier `component_id`s are serialized for the same reason — two schemas whose epoch
components mean different things must hash differently.

Presence bits are assigned to optional non-key fields in schema order, so there is no separate
presence index to encode: a field is optional iff `flags` bit0 is set, and required fields have no
bit. Key order is `key_position`, which §2 and §7.4 sort by.

**A schema is invalid — a hard writer and CI error — if:**

- `cadence_log2` exceeds the container's `ticks_per_hour_log2`;
- a state collection or event part has no fields;
- an event field list marks a key member (event records have no key prefix) or a `derived` field
  (an event batch has no unambiguous single occurrence clock);
- the frontier repeats a `component_id`, or `sequence_index` is outside the frontier;
- a shape-2 collection marks a key member, or a shape-0 or shape-1 collection declares none;
- `collection_id` repeats within the domain — counting the event part's — or `field_id` repeats
  within any one collection or within the event field list;
- `key_position` values in a collection are not `0..n-1` exactly once;
- a key member is optional, nullable, unknown-capable, derived, or of a kind other than `0`, `1`
  or `5` (§7.4);
- a `derived` field is not of kind `0`, `1` or `2`, or its `derived_u` exceeds
  `ticks_per_hour_log2`;
- a continuous field omits an explicit `exact`, `fixed(k)` or `ratio(d)` declaration, or its
  parameter is outside §6's legal range; or a non-continuous field carries a quantizer.

**`schema_hash` is the first 8 bytes of the BLAKE3 digest of those bytes.** Collisions are handled
by detection, not by hope: the container stores hash → definition, a reader verifies that a stored
definition rehashes to its key, and **a writer that would store a second, different definition
under an existing hash fails hard.**

### Storage and change

**Schema definitions are stored in the container, append-only, keyed by hash.** Recording only the
hash would strand physically-present, losslessly-encoded data behind a version mismatch, turning a
versioning inconvenience into unreadable data. A reader that meets an unknown hash *and* cannot
find its definition refuses that domain and reports a gap; with the definition present it decodes.

A schema change — game build, mod set, cadence, field added or retired — starts a new block under
the new hash. **The state part forces a `BASELINE` in that block. The event part does not**: it has
no baselines, and its continuity is carried by the frontier, which crosses a schema change unbroken
so long as the same component ids remain, the designated sequence id is unchanged, and their values
join under §9. Tuple order may change because `component_id`, not position, is the identity.

Each frame decodes against the definition its block names, so a retired field costs nothing: the
rule that matters is that **`field_id` is never reused for a different field**.

## 12. What this format deliberately does not do

No field names. No self-describing types. No sentinels. No per-frame compression — that is the
container's job over multi-hour blocks, and a frame compressed alone forfeits the cross-hour
redundancy that dominates this workload.

## 13. Open

These are why this document is not yet the fan-out contract. The reference domain (pattern §10)
closes them on a named corpus and harness before ~80 encoders are written.

- **Wave-A measurement status.** The §7.3, §8 and §9 codecs and their literal golden vectors are
  implemented, but codec proof is not corpus proof. d11 and d56 ordered-port figures await
  regeneration; §8 rounds 1–4 are invalid and the next publishable run is round 5; and the d64
  lifecycle-event measurement is invalid pending a corrected run that charges every incomplete
  observation to an event-part gap under §8. Historical ratios below describe the older stated
  arms only and must not be relabelled as measurements of these new ports.

- **Column-major blocks — CLOSED BY DECISION 2026-07-26: the row-major grammar is retained.**
  The synthetic month said columnar wins on baselines: **columnar compresses to 79.8% of
  row-major** — 5.08 MB against 6.37 MB over 720 hourly d20 baselines at 3,051 divisions, zstd
  level 3, raw 90.9%; 75.7% at 6-hour block spans, 79.1% at 48-hour — sliced on the encoder's
  recorded hour boundaries, not an average. Reproduce with
  `cargo run -p wire_format --bin bench_layout` then `python extractor/native/scripts/bench_layout.py`;
  raw output in `extractor/native/runs/d20_reference/12_layout_compressed.txt`. But real
  captures overturned the model that produced that margin: real *changed-row* churn is
  21–72%/hour (the real-dynamics anchors below) — a quantity the synthetic model never
  controlled, since its few-percent figures were movement/combat *occupancy* while its stored
  rows churned near-totally (every division's organisation and strength walked each hour). The
  synthetic margin therefore has no data point at the real regime, and since the surface is
  non-monotone in churn, the margin at real rates is simply unmeasured. What is measured:
  row-major beat columnar on the one real measurement that compared them (a single baseline,
  `runs/d20_real_save/`), and most real hours are deltas — for which no columnar grammar
  exists, so no direct columnar-delta comparison can be run.
  On that basis §3 and §7's row-major record grammar is the committed one — an engineering
  decision informed by these measurements, not itself a measurement — and a columnar delta
  grammar will not be specified unless new evidence reopens the question.

  **The margin is model-dependent, and the model is an assumption, not a measurement.** How much
  the world churns per hour is the assumption that moves the number. The committed model holds
  d20's stated shape — a few percent of divisions moving or fighting at any hour, on bounded random
  walks — and the benchmark *measures* its own occupancy rather than asserting it, printing the
  realised rates beside the sizes.

  **The ratio is not monotone in churn**, which is the trap. A full-churn control — every carried
  value redrawn independently each hour, runnable as
  `cargo run -p wire_format --bin bench_layout -- --white-noise` — reports **85.3%**, which is
  *above* the committed model, not below it: destroying cross-hour redundancy costs columnar as
  much as row-major and pushes both toward parity. Columnar's best case is neither extreme but the
  middle, where noisy columns sit beside frozen ones. So the surface is measured at exactly two
  points — 79.8% at d20's *modelled* rate, 85.3% at full churn — and between them it is
  unmeasured and, being non-monotone, cannot be interpolated. The modelled rate is an assumption
  about d20, not a measurement of it — and its few-percent figures are movement/combat
  *occupancy*, not changed-row churn; real captures (the anchors below) measured changed-row
  churn at 21–72%/hour, a quantity and regime this surface simply has no data point for.

  What this synthetic run does *not* settle by itself: it models baselines only, and most real
  hours are deltas — the real-dynamics measurements in the anchors below are that evidence, and
  the closure decision above rests on them.

  **One real anchor exists.** The D04 deep-war save (3,168 divisions, 1944.2.11.18, build a729)
  encoded as a single BASELINE against the shipping 120-byte compact rows carrying the same
  information, both at zstd 3: **the new format is 5.3× smaller raw (22.8 vs 120.0 B/div) and
  71.9% of shipping compressed** — conservative, because the save extract left the shipping rows'
  supply/experience fields as well-compressing zeros. On that single baseline row-major beats
  columnar (8.88 vs 9.48 B/div compressed): columnar's win needs cross-hour runs inside a block,
  which one snapshot does not have — consistent with, not contradicting, the 720-hour result.
  Reproduce with `python extractor/native/scripts/flatten_divisions.py` then
  `cargo run -p wire_format --bin bench_real_d20`; raw output and provenance in
  `extractor/native/runs/d20_real_save/`.

  **Real dynamics are now measured.** A live d20-only capture (2026-07-26, Ireland-1936 world,
  3,126 contiguous game-hours, ~1,353 divisions/hour) replayed into hourly states and re-encoded
  as 48-hour blocks of BASELINE + stored-minimal DELTAs, against the shipping d20 records
  verbatim, both at zstd 3: **the wire format is 9.9% of shipping raw, 43.6% whole-stream
  compressed, 54.2% at the container's 48-hour block shape.** Every hour was decode-verified —
  the reader replays its own reconstructed prior and must land stored-equal to the fold; all
  3,126 hours passed. **Measured real churn is 287 changed rows/hour ≈ 21%/hour at fixed-2
  precision** — a changed-row rate the synthetic surface never measured (its few-percent
  figures were movement/combat occupancy, not row churn). (The surface is
  non-monotone in churn, so this invalidates the modelled margin without locating the ratio
  at real rates.) Reproduce with
  `cargo run -p wire_format --bin bench_real_dynamics` then
  `python extractor/native/scripts/bench_real_dynamics.py`; provenance and full table in
  `extractor/native/runs/d20_real_dynamics/`.

  **The deep-war end of the surface is measured too.** The same instrument on the D04 1944
  world (1,022 game-hours, ~3,322 divisions/hour): churn **2,393 changed rows/hour ≈ 72%/hour**
  with zero empty hours — and the ratio holds: **12.4% raw, 54.0% whole-stream, 52.0% at
  48-hour blocks**, all 1,023 hours decode-verified. Real changed-row churn therefore spans
  21–72%/hour, a quantity and regime the synthetic surface never measured (its few-percent
  figures were movement/combat occupancy), and the wire format sits at ~52–54% of shipping across the
  whole measured range — the win is not a low-churn artifact. Provenance in
  `extractor/native/runs/d20_deepwar/`. Still open here: the columnar arm has no delta grammar
  to measure, and at these churn rates the case for specifying one is weak.

  **First cross-domain anchor (d63, 2026-07-26).** The fan-out pilot measured a small
  country-keyed table (d63 `country_core_scalars_live`, ~439 rows/hour, D04 deep-war world,
  903 hours, every hour decode-verified): churn 39 changed rows/hour ≈ 9%/hour — below the
  d20 band — and the wire format at **6.9% of shipping raw, 89.5% whole-stream, 83.2% at
  48-hour blocks**. The compressed ratio narrows on small, low-churn tables: 48-hour
  baselines re-photograph rarely-changing rows, and zstd already crushes the shipping arm's
  redundancy. Still a win, and the raw margin (14.6×) is what the container's block shape
  trades away. One datum, not a law; per-domain measurement is the point of the fan-out.
  Provenance in `extractor/native/runs/batch1/pilot_d63/`.

  **Wave 1 (2026-07-27): six more domains, one pattern and two measured spec gaps.** d10
  fuel, d22 map_control, d23 map_detail, d24 weather, d26 division_equipment, d62
  hub_motorization — each 903/903 hours decode-verified on the same D04 capture. Three
  result layers, which must not be conflated. *Delta paths:* the wire grammar won every one
  measured — 0.96–1,427 B/hour averaged over all non-baseline hours, against shipping
  per-hour floors of ~64–9,640 B (d24's patch hours: 41% raw, 89% compressed).
  *Whole-stream zstd-3:* d62 36.4%, d23 35.8%, d22 41.9%, d26 49.7% — but d10 101.2% and
  d24 111.6% lose even here (small state and dense higher-churn state respectively).
  *48-hour blocks:* all six worsen relative to whole-stream and five of six lose outright,
  from d10's 117.3% up to d22's 709% (d62 still wins at 77.9%), each loss decomposing to
  the 48-hour re-baseline against shipping lanes that baseline once per campaign — **block/re-baseline cadence is a per-domain container
  question, not a grammar defect**, and it dominates exactly where state size exceeds
  hourly churn by orders of magnitude. The two gaps: (1) power-of-two fixed-point sources
  have no lossless `Fixed(k)` under the current codec — the identity
  `r/32768 = r·5¹⁵/10¹⁵` holds mathematically, but binary64 scaling rounds to an adjacent
  integer that passes the stability check and decodes to the neighbouring float — so d10's
  `/32768` fields ship as `Exact` f64 at +10.5% zstd vs the shipping i64 words, and closing
  the gap needs a codec-level exact path (integer-domain `Fixed` or a declared power-of-two
  quantizer), while decimal sources have a cheap exact `Fixed(k)` (d24); (2) §8 gap normalisation
  is now implemented — d26's `sparse_partial` marks a *domain-hour* unknown, which §8's
  `(domain, part)` scope can express. The earlier §8 benchmark rounds are invalid; no gap-size
  figure is promoted until the corrected measurement is regenerated.
  Separately, the wave-1 survey stopped d12/d14/d15/d18 on the absence of a string kind; the
  formal stop reports since then locate the real mechanism in §5's symbol-id assignment rule,
  under the symbol bullet below. Provenance in `extractor/native/runs/batch1/`.

  **Waves 2–3 and the XL tier (2026-07-27): fourteen further domains.** d83, d29, d17, d35,
  d34, d13, d31, d28, d36, d32, d39+d40, d16 and d11 — each hour reader-replayed against the
  fold, on the same 903-hour D04 capture or the 146-hour batch-2 capture. Four anchors move
  the picture wave 1 drew. *Size is not where the grammar fails:* **d11 production**, the
  largest domain — ten trailer segments folded into seven keyed collections — measures
  **7.8% of shipping raw, 45.7% whole-stream and 58.4% at 48-hour blocks** over 903/903
  verified hours. *Two more domains win at BOTH shapes*, joining wave 1's d62:
  **d31** air_region_activity at **20.4% raw / 61.2% whole-stream / 66.7% at blocks**
  (~109 buckets/hour) and **d16** at **3.4% whole-stream / 23.2% at blocks** against its
  shipping stream as-is — a stream of which the wire arm claims the 52.9% country-row slice,
  while 37.1% is per-record framing and 10.1% is the stop-reported `timed_live` suffix the
  wire arm makes no claim over. Small state makes re-baselining cheap, which is the other
  face of wave 1's block-cadence finding. *Static catalogs collapse at the block shape, and it is
  re-baseline policy every time:* d83's static table costs **42 compressed bytes**
  whole-stream (0.3%) against 2.1% at blocks; the d39/d40 pair measures 15.2% and 26.5%
  whole-stream with the same block collapse; d13 inverts at whole-stream (**117%**) because
  zstd deduplicates 903 near-identical shipping payloads, while its delta path costs 0.66%
  raw / 70.6% compressed. *Delta paths keep winning where they were measured* — d28 9.7×
  raw, d36 10.5×, d32 7.3×, d34 9.6× on its sparse path. Provenance in
  `extractor/native/runs/batch2/` and `extractor/native/runs/batch1/`.

  **Wave 5 (2026-07-27): twelve more domains, three degenerate captures, and the first
  dense-array counter-evidence.** d48, d49, d50, d52, d56, d61, d65, d67, d70, d92, d94 and
  d95, on the 514-hour `deepwar_a`, 750-hour `deepwar_b`, 1,033-hour `deepwar_b2` and
  896-hour `deepwar_c` captures, every measured hour reader-replayed against the fold.
  *Delta paths* stay the grammar's result: **d48** construction spends 5.99% of shipping raw
  over the hours both arms carry a delta (16.7×) while its whole-stream zstd-3 is a wash at
  99.9% and its blocks lose at 304.7%; **d61** air losses average 95.5 B per delta hour
  against a 5,264 B/hour full-replace record (55×); **d67** dynamic modifiers carry every
  change in 1,011 non-baseline hours in **202 raw bytes** against 24,053,712 B of shipping
  records for the same hours; **d65** special projects change five rows in 514 hours at
  7–11 B per emitting hour against a flat 488 B; **d70** sunk-convoy history carries every
  change in 734 non-baseline hours in 1,246 B against 8,171,584 B; **d95** wins 7.9×.
  *Whole-stream zstd-3 / 48-h blocks:* d56 18.8% / 38.7% (29.6% / 59.7% against the arm with
  its stop-reported `NSUP` trailer cut), d70 18.4% / 58.1%, d67 20.2% / 82.4%, d95 16.9% /
  28.2%, d61 48.5% / 56.1%, d65 4.7% / 24.7% — the block deficit decomposing to re-baseline
  cadence every time. **d52** occupation is the honest-pair instance: 20.29% raw at the
  shipping cadence but 89.8% whole-stream compressed — its delta path wins 5.38× raw and only
  **1.11× compressed**, because a 64-byte row that changes one word still ships 63 mostly
  unchanged bytes and zstd eats exactly that (474 kB → 57.5 kB, 8.2×); the grammar's
  advantage is real but largely pre-paid by the compressor. **d94 pov_province_visibility is
  the first dense-array counter-evidence — offered as evidence, not a reopening.** Its delta
  path wins 4.5× compressed (4.35 vs 19.78 B/hour over the same 1,032 hours; 1,211× raw),
  but one keyed-set baseline loses **23×** to the dense positional array plus zstd (36,115 B
  compressed against 1,586 B for one shipping snapshot; 4.15 vs 1.00 B/row raw): the source
  array's index is implicit where the keyed set spells every key out. Break-even is
  ~2,320 hours per baseline against a 1,033-hour campaign, so no cadence within that capture
  makes the wire arm smaller after compression. The closure above rests on 21–72%/hour
  changed-row churn; d94 is 0.013%/hour over a dense, contiguous `ContentId` key space with
  the cost entirely in the baseline's key list — whether that is new evidence in this
  bullet's sense is the spec owner's call
  (`runs/batch3/d94_pov_province_visibility/measurement.md`). **d56 confirms §7.3's by-name
  stop with the first measured ordinal failure**: the supremacy list's one insertion
  (64 → 65 rows) shifted the 40 entries behind it, which an ordinal key would publish as
  forty updates to the *same* entries — false continuity; the ordinal-keyed reading was
  built, measured (49,657 B raw against 28,288 B for the ported table alone) and rejected.
  **d95 is the first wire encoding of unmeasured collection membership**: a clear per-surface
  flag maps to Unknown on the frame's own `capture` record — a per-POV singleton shipped
  every hour — and the codec refuses a payload whose unflagged surface carries rows
  (`compact_contract/src/spotted_units.rs:153`–`:156`), so "unmeasured, with rows" is not a
  representable input; the d17 war-graph blocker does not apply because the state has a
  carrier that cannot contradict it. The state is measured, not hypothetical — the
  task-force surface is unflagged on the first 53 of 1,033 hours — but 0 rows appear on any
  hour, so d95's ratios measure a header-only table. **Three captures are degenerate, and
  their ratios are not grammar results.** d49/d50: the naval province→region promotion
  resolves nothing on `deepwar_a` (0 of 146 lifecycle rows carry a region), so d49 folds no
  live row in 514 hours and its 0.03% raw / 0.6% compressed compare ~29 KB of framing and
  tombstones against ten bytes of empty baselines; d50's fold-faithful arm keeps one hour
  (512 of 514 records are orphans of the empty d49 accumulator), and its 14.9%-raw figure
  comes from a labelled parent-identity-only size-accounting arm, not the recorder's
  semantics. **d92 preferred-tactic was the third — an unsatisfiable-by-construction gate,
  now fixed**: d92 publishes the complete 439-tag roster while D18 publishes a deliberately
  filtered 356-tag cache, a strict superset every hour of `deepwar_c`, so the fold's
  roster-EQUALITY gate folded 0 of 896 hours (production's own `stream_fold` says so
  verbatim); equality could never pass, and the gate now demands **containment** — every D18
  tag covered by a d92 row, which is what the projection actually reads
  (`world_fold.rs:10345`–`:10359`). Post-fix, on real rows: 896/896 hours measured, 25,122 B
  wire against 6,415,489 B shipping — **0.4% raw**
  (`runs/batch3/d92_country_preferred_tactic/post_fix/`). Provenance for the wave:
  `extractor/native/runs/batch3/`.

  **Wave 6 (2026-07-28): four more domains, an ordinal finally priced, and a stop that owns its
  domain.** d46, d51, d54 and d96, on the 514-hour `deepwar_a`, 750-hour `deepwar_b` and 896-hour
  `deepwar_c` captures, every measured hour reader-replayed against the fold.
  **d54 army_battleplans_depth is the batch's genuine ordinal-device decision, and the first to
  measure what the ordinal costs instead of arguing it.** §7.3 names its ordered-shape instances
  by name and does not name d54, so the evidence decided: **12 of 616 path-carrying rows repeat a
  province inside one polyline**, which a province-id key would collapse on input the producer
  demonstrably emits, and **0 of 77 observed path changes is a pure reorder**, so d11's stop
  condition never fires. The premium is recorded beside the decision — 186 of 401 positions
  differ under the ordinal key against the 148 id-set changes an (impossible) id key would have
  emitted, about **26% more churn** — and the divergence from d27, whose sibling child table
  carries its ordinal as a value field, is grammar rather than taste: d27's fold proves per-row
  target-state-id uniqueness and d54's decoders prove nothing of the kind. Its headline
  **5.40% raw** measures recorder withholding, not grammar: **66 of its 72 baselines are §1.5
  gap-forced**, because the fold withholds 241 of 514 hours.
  **d46 air_variant_archetypes carries the most lopsided stop measured.** The `ACE1` ace roster
  stops on two independent grounds — runtime-generated ace names no content manifest resolves,
  and a retain-on-unavailable membership needing §8 — and it is **99.5% of the domain's shipping
  bytes** (10,138,319 of 10,187,457 B), against 0.1% for the archetype rows the wire arm replaces
  and 0.4% framing. Every ratio is therefore given twice: 4.4% against the whole shipping arm and
  22.0% against a labelled cut arm, whole-stream at zstd-3. It also splits a rule stated so far as
  one: a stopped trailer is validated at **two** positions — the fold's, behind its failure gate,
  and the status-independent payload authority's, in front of everything — so a malformed trailer
  refuses the stream even on a failure-status hour, through the authority and not the fold.
  **d51 scripted_state settles what makes a §5 assignment band claimable — the key ORDER.**
  Because §7.4 delta-chains only the first key component, a symbol at position ≥ 1 permutes rows
  only within a run of equal leading components, and the band is then exactly computable:
  **10,830…12,406 B** over a 4,354-key vocabulary against a 12,353 B sorted-index stand-in,
  16.23%…18.59% of the 66,730 B one-baseline wire arm. A symbol-led key would have made it a
  permutation optimisation over the delta chain instead, with no per-row formula. d51 also bounds
  wave 1's "decimal sources have a cheap exact `Fixed(k)`" above: that case rests on an **i32**
  source, and d51's fold reads an **i64** (`world_fold.rs:19000`), so `Fixed(5)` would stop
  reproducing the raw integer above ~2^51 on input the fold accepts — the word travels raw as
  `Signed`, and
  the domain has no `Continuous` field at all.
  **d96 tactic_weight is the first carried presence bool ruled to be a four-state rather than a
  field.** PTW1 rows carry `accumulator_entry_present` beside `accumulator_raw` and the codec
  makes a clear bit imply a zero word (`compact_contract/src/tactic_weight.rs:112`–`:113`,
  `:328`); the collector reads a country's whole modifier accumulator, failing the row on any
  unreadable entry, and only then binary-searches it — so a clear bit is §3's *null*,
  "applicable, the relation genuinely points at nothing", on direct producer evidence rather than
  the Null/Unknown tie-break, and the bit is recoverable from the four-state with nothing carried
  twice. 896/896 hours measured: **0.4% raw, 5.0% whole-stream zstd-3, 17.7% at 48-hour blocks**,
  with all 19 baselines at a 48-hour block start and **zero forced by a §1.5 state gap** — the
  clean counter-case to d54's arm, where the same rule forces 66 of 72. Provenance for the wave:
  `extractor/native/runs/batch3/`.
- **`ticks_per_hour_log2`.** The mechanism is here; the value is set by the finest cadence any
  domain justifies, and is owned by the container format.
- **Which domains declare a finer cadence, and how fine.** Each candidate domain carries its own
  rendered-parity evidence — one reference domain proves the pattern and the harness, not another
  domain's answer.
- **Counter delta width.** Whether a second difference beats a first for monotone counters (§4) is
  per-field and needs corpus evidence.
- **`f32` storage.** Source 32-bit floats are widened to `f64` (§0). Whether a declared 4-byte form
  pays after compression is unmeasured.
- **Position-delta encoding for ordered keyed lists.** Full replacement is specified and safe; a
  position-op grammar is worth defining only if measurement shows replacement costs too much.
- **Block span.** The committed benchmark shows 48-hour blocks beating 24-hour; the trade is
  against a seek budget with no ratified number.
- **Per-domain zstd dictionaries** trained on baselines — 10–40% is an unverified target.
- **Aggregate budget numbers.** The pattern's four budgets exist as a mechanism with no ratified
  values.
- **Tick origin and branch id assignment.** Both are consumed here and owned by the container
  format, which specifies them.
- **Symbol id assignment (§5).** The rule that makes base and overflow ids deterministic for a
  given content set, and that orders symbols first seen at the same tick, is unwritten. Golden
  vectors are not stable until it exists; it belongs with the content manifest, which does not
  currently carry it.

  **It is the highest-value unwritten rule for the fan-out.** The four formal stop reports in
  `extractor/native/runs/batch1/stop_reports/` (d12 `focus_live`, d14 `politics_live`, d15
  `politics_residuals`, d18 `country_identity`) establish that nearly every string in those
  domains is a content-defined token — focus and focus-tree ids, ideology and law-group names,
  idea/advisor/spirit tokens, country TAGs, cosmetic tags — which §5 already says the stream
  carries as varint manifest ids; and because `ContentId` is a legal key kind (§7.4), the same
  rule fixes their string-**keyed** collections with no new shape. It resolves every field
  kind in all four: d18 outright, d12 apart from its completed-focus occurrence list, d15's
  country row (already representable today) and its advisor-detail fields, and every
  content-token field of d14. What it leaves are residuals of two other kinds. *Shape:*
  d14's `country_idea` and d15's `advisor_detail` both have unproven key uniqueness — the
  folds reject no duplicate and the only corpus has zero rows of either — so each may yet be
  an occurrence list; d14's `party_popularity` and `country_law` are settled, as is d15's
  country row. *Representation:* `politics_country` would still be missing `leader_name`. **A string kind is not the mechanism these domains need.** The one genuine
  string-kind candidate in all four is `leader_name`, a rendered display name on 174 rows that
  carry no leader token at all: it needs a portable character-id-to-name authority (the row
  does carry a numeric `char_id`) or a string kind. Another of the reports' strings needs
  nothing new at all — d15's `last_election` is an `i32` datehour the collector renders to text
  and which stays numeric.

  **d18 is better read as the container's symbol-table feed than as a domain block.** It *is*
  the country
  TAG-to-symbol binding, and its access pattern is a container artefact's: the roster is
  perfectly static (0 changed rows over 902 transitions) and its CIM1 extension changes twice
  in 903 hours — mint once, never revise. Written as a domain block it would instead be a
  block every reader of every other domain must decode first, which is the objection §8
  already makes about gaps.

- **d21 land combats is ported but corpus-blocked.** The reference port preserves the shipping
  fold's structural/status ordering and its distinction between measured-empty and unknown
  participant sides. The only scheduled d21 corpus contains 32-byte, zero-row `0x300` failure
  records, so it provides no authoritative size or dynamics measurement. A ratio remains
  withheld until the rebuilt collector produces the requested mixed-manager capture; failure
  hours must be charged as unknown rather than decoded as empty snapshots.

- **Ordered collection shapes have measured demand and an implemented codec (§7.3).**
  `OrderedOccurrenceList` has **four measured instances** — d16's `timed_live`, whose fold
  pushes identical timed rows onto a country's array in source order without upsert, dedup or
  sort; d56 naval supremacy; d64 delayed events; and d12's completed-focus set, where
  duplicates are measured in the corpus. One further ordered array survives the ordinal device
  and stops: d11's `convoy_priority_reservations`, where 2,633 consecutive samples are pure
  reorders of an identical `{priority, count}` multiset, so no carried field recovers the
  order — an `OrderedKeyedList` if `priority`'s observed per-country uniqueness holds in
  general, an `OrderedOccurrenceList` if not, since the fold checks neither. Both shapes use
  full replacement. d11 convoy reservations and d56 naval supremacy now have reference ports;
  their prior size figures remain withheld pending regenerated measurements. d16, d12 and d64
  pending state remain separate domain-port work rather than codec stops; d64's lifecycle
  event part is independently ported under §9.

- **Gaps below the whole domain: three measured instances; d26 is unblocked, and two still need
  a scope §8 does not have.** d26's `sparse_partial` marks a *domain-hour* unknown, which the
  implemented §8 `(domain, part)` record expresses (wave 1, above). The other two
  need a narrower scope as well: d17's war graph needs "this
  collection's membership was not measured this hour", the state of **903 of 903** captured
  hours, and gapping it under §8 as written would gap d17's countries collection too, which
  measured fine; and d18's CIM1 extension carries three independent per-family availability
  words inside one record, with the cores family unavailable across the entire capture while
  claims and major status are not. A keyed set can mark a *row* unknown and §8 can gap a whole
  domain; there is nothing in between. The symbol rule does not touch either of those two —
  d17's war graph and d18's CIM1 families need this mechanism whatever §5 lands.

- **Frame-level scalars have no home.** `elapsed_wall_ms`, the frame's own hour, source
  sequence numbers, authority hashes and receipt words are facts about a frame, not about any
  row, and §3's record grammar has nowhere to put them. Ported domains do one of three things
  with them, and none of the three is a wire representation: parse and expose them at the
  boundary where they never reach a record (d63, d34 — the header word is read, tested and
  dropped); enforce them as invariants and keep them off the wire (d83's four authority hashes
  are constants on every acceptable frame); or refuse the frame when the fact cannot be
  checked (d13 refuses A29-bearing streams whose digest clauses it cannot evaluate). d83 makes
  it acute: every scalar in its header is either an invariant or unrepresentable, so a capture
  in which one of them legitimately varied would have nowhere to record it.
- **Golden vectors.** Byte-exact round-trip fixtures written with the reference domain are what
  turn "two implementers produce identical bytes" from a claim into a gate. The set covers every
  state in §3, the escape branches in §6, insert-versus-update and the minimality rules in §7.1,
  the two time chains in §1.4, both ordered replacement shapes in §7.3, and the event body plus
  epoch/sequence/schema-change/overflow/post-gap frontier outcomes in §9. Domain-specific corpus
  evidence remains separate from these codec vectors.

## 14. What this format retires

This is the second record format in the tree. The first one keeps working until the last encoder
is migrated, and then it is deleted — a format that outlives its replacement is a second way to
write the same data, which is the one thing §2 exists to prevent.

Superseded, and deleted when the last domain stops calling them:

| what | where |
|---|---|
| H4CR record framing constants | `compact_contract/src/lib.rs`, `record_header.rs` |
| record layout and validation | `compact_contract/src/record_semantics.rs`, `stream_validator.rs` |
| gap records | `compact_contract/src/gap_record.rs`, superseded by §8 |

Explicitly **not** superseded, and not to be deleted alongside them. `compact_stream` is a
compression container over an opaque byte stream and is orthogonal to how a record is laid out.
The domain descriptors, build profile, control transport, campaign identity and early-render
modules in `compact_contract` describe what is captured and how the collector is driven, not the
bytes it produces.

The retirement condition is mechanical, so it can be checked rather than remembered: when no
crate outside `compact_contract` imports the four files above, they go in the same commit that
migrates the last caller. Migrating a domain and leaving its old encoder in place is not a
finished migration.
