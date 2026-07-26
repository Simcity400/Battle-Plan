# Wire format

> **Read `documents/DECISIONS.md` first**, then `DOMAIN_CAPTURE_PATTERN.md`. This document is
> subordinate to both. The pattern says *what* a domain must do; this says *how the bytes look*.

Governing rule, from DECISIONS: **fidelity is never traded for bytes; subject to that, size is
minimised aggressively.** 3–8 GB per campaign is a ceiling, not a budget. Every choice below is
made in that order — first "does this lose anything?", then "is this the smallest encoding?".

Two implementers reading this must produce **identical bytes** from identical state. Where that is
not yet true the gap is marked **OPEN** rather than left to be discovered later, because
byte-identity is what the dedup floor depends on.

## 0. Conventions

Little-endian. `uN` is a fixed N-bit integer. `varint` is LEB128, unsigned. `zigzag` is
`(n << 1) ^ (n >> 63)` then LEB128, so small negatives cost one byte. **Every length prefix is a
varint.** Bitmaps are byte arrays; **field `i` is bit `i & 7` of byte `i >> 3`, LSB-first.**

## 1. Blocks and frames

The container compresses **domain-major blocks** spanning many hours. A block header carries what
every frame in it shares:

| Block header | Type |
|---|---|
| `domain` | `varint` |
| `branch` | `varint` |
| `schema_hash` | `u64` |
| `first_hour` | `varint` |

Hoisting `domain` and `branch` out of every frame saves 2–3 bytes on each of ~7 million frames.

Each frame:

| Field | Type | Meaning |
|---|---|---|
| `len` | `varint` | byte length of `kind` + `body` |
| `kind` | 3 bits, packed with the low bits of `hour_dd` | see below |
| `hour_dd` | `zigzag` | second difference of `hour`; **reset at every block start and every `BASELINE`** |
| `body` | bytes | per `kind` |

**`len` exists so an undecodable frame is skippable.** Without it, one domain a reader cannot
decode (unknown schema, §6) makes every later frame in the block unreachable — turning a
per-domain typed gap into an all-domains gap.

**`hour_dd` is signed and chains reset at block boundaries.** Reload branches move hours backwards
and cadence changes make second differences negative, so unsigned is unencodable. Resetting at
block starts is what keeps a block independently decodable.

| `kind` | Name | Body |
|---:|---|---|
| `0` | `BASELINE` | complete, **self-contained** canonical state |
| `1` | `DELTA` | changed rows and tombstones |
| `2` | `UNCHANGED` | `varint` run length in hours |
| `3` | `GAP` | typed gap record |
| `4` | `EVENTS` | append-only batch |

`UNCHANGED` carries a **run length**, so a domain static for a year is one frame, not 8,760. With
`domain` and `branch` hoisted, that frame is ~3 bytes.

**A `DELTA` or `UNCHANGED` frame is invalid before a `BASELINE` in its `(domain, branch)`.** Hard
error to the writer and CI; to the reader, a typed gap for that domain until its next baseline.
Every discontinuity — gap, restart, rollover, branch — forces a `BASELINE`.

## 2. Canonical form

`UNCHANGED` and the dedup floor rest on byte-identity, so canonical form must be fully determined:

- **Rows are sorted by stable key**, never by walk order. d64 swap-removes on dispatch, physically
  reordering survivors whose state did not change; walk order would make an unchanged hour differ
  and silently disable dedup.
- **Set- and map-valued fields are sorted by their key.**
- **Tombstone lists are sorted.**
- **Rounding is half-to-even** for `fixed(k)` and `ratio(d)`.

Any nondeterminism here is a size bug, not a cosmetic one.

## 3. Records: presence and unknown

```
  presence bitmap   ceil(O/8) bytes   one bit per OPTIONAL field, schema order
  unknown bitmap    ceil(P/8) bytes   present ONLY if the record's kind byte sets bit 0
                                      one bit per PRESENT field, in ascending field order
  [ bit-packed region ]               enums and booleans, schema order, LSB-first, byte-aligned end
  [ varint / zigzag values ]          remaining present, non-unknown fields, schema order
```

Fields the schema marks **required** carry no presence bit — a bit spent on a field that can never
be missing costs a byte per row on wide records for nothing. d56 has nine non-key fields of which
at least `fleet_id` and `owner_tag_db_id` are structurally required.

**Absent, null, zero and unknown are four different things.** This is a fidelity rule:

| State | Encoding |
|---|---|
| **absent** — not applicable, or unsupported by this schema | presence bit `0` |
| **null** — applicable, the relation genuinely points at nothing | presence bit `1`, unknown bit `0`, declared `nullable` marker |
| **zero** — applicable, value is 0 | presence bit `1`, unknown bit `0`, value `0` |
| **unknown** — applicable, not captured | presence bit `1`, unknown bit `1`, **no value bytes** |

**Known-null is not not-applicable.** d56 distinguishes a task force with no repair target (null,
and true) from a field this schema does not support (absent). Collapsing them loses the difference
between "we know there is nothing" and "we do not carry this". A `nullable` field encodes null as a
dedicated marker, never by clearing its presence bit.

Sentinels (`-1`, `u32::MAX`) are **forbidden** as encodings of absence or unknown: they collide
with real data, silently.

## 4. Scalars

| Kind | Encoding |
|---|---|
| unsigned | `varint` |
| signed | `zigzag` |
| **monotone counter** | in `BASELINE`: **absolute**. In `DELTA`: `zigzag` **delta-of-delta** against the reconstructed prior value |
| enum | bit-packed to `ceil(log2(n))` bits |
| boolean | one bit |
| content id | `varint` manifest id, §5 |
| continuous | §6 |

**Baselines are absolute — always.** A baseline whose counter is encoded against "the previous
frame" would, after a gap, be encoded against a value nobody has. That makes the baseline
non-self-contained and the counter unrecoverable, which is exactly the permanent-loss defect the
pattern names. The delta base is the **reconstructed prior value**, not "the previous frame" —
which may be an `UNCHANGED` run or a `DELTA` that did not carry this field.

**Monotone is an encoding hint, never a validity constraint.** d56's `sunk_convoys` is a
task-force aggregate of per-ship counters, and ships transfer between task forces, so it legally
decreases. A decoder that rejects a negative delta rejects valid data.

## 5. Symbols

Country tags, equipment names, province and state ids are content-defined. The stream carries
**manifest ids as varints**, never text. Symbols first seen mid-campaign go to an **append-only
overflow table** keyed by `(branch, first-seen hour)`, so seeking backwards never needs a table not
yet read. In-stream interning is forbidden — it must be fully read before any seek.

## 6. Continuous values, quantization, derived fields

Each continuous field declares one of:

- `exact` — IEEE bits preserved. **The default.**
- `fixed(k)` — value × 10^k, half-to-even, zigzag varint.
- `ratio(d)` — a 0..1 fraction as a `d`-bit unsigned where `1.0` maps to `2^d − 1`.

**Quantized fields carry an escape.** `fixed(k)` and `ratio(d)` cannot represent NaN, infinities,
signed zero, or out-of-range values. Each reserves one encoding meaning "exceptional", followed by
raw IEEE bits. Without it quantization is lossy for values it was never checked against, silently.
A field whose escape fires often is mis-declared and reverts to `exact`.

A field is `exact` unless its quantization is justified in the domain document. Quantization is
licensed by comprehension-grade fidelity, but **not** for a value whose exact magnitude explains a
threshold crossing.

An unquantized float that jitters in its low bits is never byte-identical, so it defeats
`UNCHANGED` and inflates every delta. This is where fidelity and size actually trade, per field,
in writing.

### Derived fields — the largest single saving available

A field that changes **deterministically with the clock** may be declared `derived(expr)` and
stored in a form that does not change. The motivating case is d64: `remaining_hours` decrements by
exactly one every hour, so every pending row emits a delta every hour forever. Storing
`due_hour = frame_hour + remaining_hours` is a **lossless bijection** given the frame hour — and it
is what replay already computes — but the row now stays constant until the event is created or
dispatched, collapsing the domain to `UNCHANGED` runs.

A domain author must check every field against this before accepting an hourly delta. A field that
changes every hour by a constant is a red flag, not a fact of life.

## 7. Delta frames

```
  changed row count   varint
  [ key, change mask, new presence bits, new unknown bits, values ]
  tombstone count     varint
  [ key ]             sorted
```

**A change mask alone cannot express a field becoming absent or unknown**, and that transition is
routine: d56's repair-target province pointer goes non-null → null the moment a repair completes.
One bitmap cannot both select changed fields and carry their new state. So a changed row carries
the change mask *plus* the new presence and unknown bits **for the changed fields only**; values
follow for those that are present and not unknown.

Without this a writer must either drop the transition — leaving a stale repair target in every
reconstruction until the next baseline — or emit a whole baseline for one cleared pointer.

### Collection shapes

**Not every collection is a keyed set, and treating one as such destroys data.** A schema declares
each collection as one of:

- **keyed set** — unique stable key, upsert and tombstone semantics. The default.
- **ordered occurrence list** — order is meaningful and duplicates are legitimate. d56's naval
  supremacy is exactly this: `world_fold.rs:22123` records that the game's naval-dominance array
  "can carry several distinct entries for the same `(region, controller)`; collapsing them would
  lose data", and that a delta frame against such a list would **double-append** rather than upsert.
  **Emitted as full replacement only — `DELTA` is forbidden for them**, which is what production
  already does.
- **ordered keyed list** — unique keys, meaningful physical order. Deltas carry an explicit
  position, never an implied one.

**A domain with legitimately duplicate identities is an occurrence list, not a broken keyed set.**
d64 is the case: two pending instances of one event for one owner differ only in their countdown,
so no unique stable key exists and duplicate-key rejection would gap valid hours forever. Declared
as an occurrence list it is emitted as full replacement, and both the duplicates and the physical
order survive.

This costs nothing here, which is why it is the right answer rather than a concession: combined
with the `derived` due-hour transform (§6) the replacement bytes are **constant between creation
and dispatch**, so `UNCHANGED` still fires and the domain stays near-silent. Full replacement is
only paid on the hours something actually happens, and d64 carries few enough rows that those
hours are cheap.

A domain that has not declared its collection shapes is not reviewable: the same bytes mean
different things under each.

**Keys may be composite.** A key is the schema's declared key field list, encoded in schema order;
d64's identity is `(owner_tag_db_id, event_id)`. Keys in both lists are **sorted and
delta-encoded** against the previous key, which cuts key bytes substantially in key-heavy frames.

**A delta may only be emitted from a walk proven complete** (pattern §3.2). An incomplete walk
makes missing keys look like deletions and the tombstone list would encode real data loss as
intent. An unproven walk emits `GAP` and the prior state stands.

## 8. Gap frames

Gaps are frames, not absences of frames. Per DECISIONS a gap carries range, reason, affected
domains and witness.

A gap in a domain-major block covers that block's domain; its `hour_dd` is the gap start.

| Field | Type |
|---|---|
| `hour_end_delta` | `varint` from gap start; `0` is a single hour |
| `reason` | `u8` — torn sweep, source unavailable, partial data, coalesced, skipped, bridge unavailable, worker panic, coverage shortfall, backpressure |
| `witness_kind` | `u8` discriminator |
| `witness` | per kind: counter id + observed value, or status bits |

A gap spanning **all** domains is emitted once per affected domain block. There is no
multi-domain gap frame — one frame, one domain, so a reader that refuses one domain still reads
the others' gaps.

## 9. Event frames

```
  high-water mark   varint   position AFTER this batch
  event count       varint
  [ event record ]           per §3
```

The mark is **after** the batch, so a restart resumes from it without re-reading or skipping. No
baselines, no tombstones. A gap means "events between A and B were not captured" — encoded with
its own reason so the viewer can say that rather than "state unknown".

## 10. Hybrid domains

A domain may declare **both** shapes — d64 is mutable pending state *plus* an append-only
lifecycle trailer. Its block then carries state frames and `EVENTS` frames under one `domain`, and
each part follows its own rules. **`UNCHANGED` and the dedup floor scope over the state part
only**; an event batch never makes the state part look changed, and a quiet state part never
suppresses events.

## 11. Schema, versioning, mods

A schema is an ordered field list — kind, quantization, presence index, key membership — plus its
declared shape(s).

**Schema definitions are stored in the container, append-only, keyed by hash.** Recording only the
hash would strand physically-present, losslessly-encoded data behind a version mismatch, turning a
versioning inconvenience into unreadable data. A reader that meets an unknown hash *and* cannot
find its definition refuses that domain and reports a typed gap; with the definition present it
decodes normally.

A schema change — game build, mod set, field added or retired — forces a `BASELINE` under the new
hash. **Field indices are append-only across a domain's schema revisions**: a retired field keeps
its index so older frames stay decodable.

## 12. What this format deliberately does not do

No field names. No self-describing types. No sentinels. No per-frame compression — that is the
container's job over multi-hour blocks, and a frame compressed alone forfeits the cross-hour
redundancy that dominates this workload.

## 13. Open

- **Column-major blocks** for large homogeneous tables (provinces, divisions) likely beat
  row-major before compression. **Decided: measured against row-major on the reference domain**,
  same corpus and harness, before the fan-out. The format commits to row-major only if it wins.
- **Per-domain zstd dictionaries** trained on baselines — 10–40% is an unverified target.
- **Block span** is unsettled: the committed benchmark shows 48-hour blocks beating 24-hour, and
  the trade is against a seek budget with no ratified number.
- **Column-major blocks** are measured as part of the reference domain (§10 of the pattern), using
  the same corpus and harness, before the fan-out commits ~80 encoders to a layout.
