> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This file describes the H4CR collector stream's changed-only emission as built, and the
> contract a domain must satisfy before it may use it.
>
> The downstream portable `.battleplan` container has its own, separate delta design in
> `extractor/documents/WIRE_FORMAT.md` (§1.3–§1.5: `BASELINE` / `DELTA` / `UNCHANGED` frames).
> That document governs the file the recorder writes. This one governs the H4CR record stream
> the collector writes to the recorder. They are different formats with the same problem, and
> the vocabulary here is deliberately borrowed from there.

# Delta emission on the H4CR collector stream

## 1. What this is for

The collector stream is the largest artifact the product produces and almost all of it is
retransmission. Measured over `extractor/native/runs/live_unlock/t19/collector_stream.bin`
(4.78 GB, 81,734 records, 691 game-hours, 67 domains):

| Measurement | Value |
|---|---|
| Whole-record dedup | worthless — 99.5% of bytes survive it |
| Actually-changed bytes between equal-length adjacent hours, whole file | 3.7 MB |
| Long-window LZMA marginal cost per additional hour, d53 | 1.7 KB/h (raw 5.5 MB/record) |
| …d41 | 0.26 KB/h (raw 1.46 MB/record) |
| …d20 divisions, the genuinely-changing case | 22 KB/h |
| Whole-stream information content | ≈ 30 MB (≈150× smaller than the raw stream) |

Emission cost is coupled to completeness, not only to disk: a whole-frame encode writes 5–9 MB
per frame against an 86.8 ms hour budget at speed five, and an hour that misses the budget
becomes a backpressure gap. Smaller emissions buy hours, not just bytes.

## 2. The scheme

Delta emission is **keyed per-entity diffing performed by the domain encoder**, not a byte diff
of the payload. This is settled, not a proposal: the var-length domains (d44, d26, d48, d45,
d55, d11) change payload length nearly every hour, so a same-offset XOR degenerates immediately,
while the LZMA numbers above show a real keyed diff collapses these payloads by two orders of
magnitude.

### 2.1 Framing: a status bit, not a record type

A delta payload is an ordinary `DOMAIN_DATA` record (`record_type` 3). It is marked by
`COMPACT_STATUS_DELTA_DATA` (bit 10) inside the payload's own status word, not by a new
`record_type` and not by a header flag.

This is the right seam and should stay:

* `record_type` is frame grammar. `compact_record_counts_toward_terminal_outcome` and the
  PERF-cardinality rule are written over it, and a domain's outcome accounting must not change
  because its encoder chose a patch this hour.
* The H4CR header's `record_flags` are validated by
  `compact_record_allowed_flags(domain_id, record_type)`, which is a per-domain allowlist. Delta
  is not per-domain policy; it is a property of one payload.
* The status word already travels with the payload to every consumer that decodes it, and every
  consumer already rejects unassigned status bits.

`COMPACT_STATUS_DELTA_DATA` is **not** a failure bit and is not in
`COMPACT_STATUS_FAILURE_MASK`.

### 2.2 Delta against what

Against the **producer's pinned baseline** for that domain — the per-domain tracker in
`CompactPerfGateTrackers` — and never against "the previous record on the wire".

The pin is what both sides agree on. The consumer keeps a mirror of it (see §4), so the rule
that governs every decision below is:

> **The baseline may only advance to a read the consumer will have applied, and only to a read
> that covered its whole extent.**

The first clause keeps the two sides in step. The second stops an incomplete read from deleting
the rows it merely failed to reach from the reference.

### 2.3 Keyframes

A keyframe is simply a `DOMAIN_DATA` record without `DELTA_DATA`: the whole state. Keyframes are
forced, never scheduled:

| Forced by | Mechanism |
|---|---|
| Capture start / first read of a domain | `tracker.is_empty()` seeds the pin and emits whole |
| Campaign reset or logical-stream change | `CompactPerfGateTrackers::default()` at the reset macro |
| A rejected clock bracket for that domain | `reset_trackers_after_rejected_bracket` |
| A silent reader for that domain | folded into the same rejected set — a reader that ran and published nothing may still have moved its own cursors |
| Explicit `full` emit mode | `tracker.clear()` |

There is deliberately **no periodic keyframe cadence**. The `.battleplan` container needs one for
seekability and gets it per block (`WIRE_FORMAT.md` §1.5); the H4CR stream is read start-to-end by
the recorder in one pass, is not seeked, and a periodic keyframe on this stream would cost 1.4 MB
per domain per cadence to buy nothing. If a future consumer needs to seek this stream, that is
the change that introduces cadence, and it should be argued from that requirement.

Note what is **not** on the list: an ordinary domain gap. A gap for domain *D* publishes no
`DOMAIN_DATA` for that hour, and both sides retain what they had, so the chain does not
desynchronise. `WIRE_FORMAT.md` §1.5 forces a baseline after a gap for a different reason —
there, silence is a positive claim ("we looked and nothing changed") that unobserved time cannot
support. Here silence is not a claim at all; the hour is explicitly typed as a gap record.

### 2.4 Deletions

A key the pin holds and the read did not return is emitted as a **tombstone**: the row's
identity with its `deleted` flag set and every live field zeroed. Absence alone never means
deletion — the consumer merges what it is given and prunes only what is tombstoned.

Tombstones are what make the scheme lossless under incomplete coverage (§3).

### 2.5 Versioning and old readers

The stream is versioned in three independent places and a new emission mode touches only one:

* **`STREAM_HEADER` (`record_type` 4)** identifies the campaign and the logical stream. It is not
  where emission modes are announced, and must not become so: a stream's records are individually
  self-describing by design, and a header-level mode claim would be a second source of truth that
  a mid-stream mode change could contradict.
* **`COMPACT_RECORD_VERSION`** (H4CR header) is framing. Unchanged by anything here.
* **The payload status word** is where a delta announces itself, and
  `compact_status_unknown_flags_v1` is the safety net. Bits 0..=14 are assigned; **16..=31 are
  unassigned and every consumer must reject a record carrying one**. That is the mechanism by
  which an old reader refuses a stream it cannot decode instead of misreading it: a future status
  bit is a hard rejection, never a silent skip. Bit 15 is the next free global bit.

A consumer that understands `DELTA_DATA` but has no mirror baseline for a domain must refuse that
domain's deltas rather than apply them to its projected state —
`reject_delta_before_baseline` is that check, and it types the result as a domain-unknown hour.

## 3. The fold / replay contract

For a domain in delta mode the consumer keeps **two** pieces of state:

1. **the delta-baseline mirror** — an exact copy of what the producer diffs against, e.g.
   `domain48_delta_state` in `world_fold.rs`;
2. **the projected state** — what the hour's answer is built from.

They are separate on purpose. The mirror must track the producer's pin regardless of what verdict
the hour earns, because the pin is a decoding key, not a claim. The projected state carries the
admission verdict, and a domain-unknown hour retains it untouched.

Applying one hour:

```
seed  := DELTA_DATA ? mirror : empty
apply := for each carried row: tombstone ? remove(key) : insert(key, row)
mirror := seed ∘ apply          (only if the producer advanced its pin — §2.2)
```

**Reconstruction property.** For every hour, the rows a delta payload decodes to are exactly the
rows the same read would have emitted whole. This is the gate that must be green before a domain
is switched on; d53's is
`compact_d53_reduced_partial_folds_identically_to_full_emission`, d41's is
`domain41_partial_emit_tests::delta_partial_reconstructs_every_all_partial_hour_exactly`.

**Interaction with the binding rulings.**

* *Partial data still seals.* A partial read is emitted, accounted, and — under §3.1 — encoded as
  a patch. It never voids a capture and never voids a hour that other domains carried.
* *One uniform domain outcome.* Delta emission adds no outcome kind. A delta record is a
  `DOMAIN_DATA` payload like any other and its domain's terminal outcome is still exactly one
  `DOMAIN_PERF` or one `DOMAIN_GAP`.
* *A failing domain states its own reason on the wire.* Unchanged: the status word and the H4DG
  reason still carry it, and a delta payload's status word carries the same failure bits a full
  payload would have carried.
* *Hour misattribution is a stop condition.* A delta record's `game_hour_raw` is the hour its READ
  happened in, exactly as a full record's is. The patch is against a previous *state*, never
  against a previous *hour*, so no delta record ever borrows another hour's label. A
  `HOUR_LABEL_MISMATCH` refusal produces no `DOMAIN_DATA` at all and the pin does not move.

### 3.1 Incomplete coverage

This is the part that decides whether the scheme is worth anything, because the domains that
dominate the stream are the ones whose reads are permanently incomplete.

`PARTIAL_DATA` is a member of `COMPACT_STATUS_FAILURE_MASK`, and the historical rule was that any
failure flag suppresses diffing outright. For a domain that is partial in *most* hours that is a
rare fallback. For a domain that is partial in *every* hour it is the only path the code ever
takes — and it is the reason the two largest domains on the wire have never emitted a single
delta record.

The resolution, already shipped for d53 and d48 and now available for d41, is:

* diff against the pin, **do not advance it** — a partial read must not become the reference,
  because that would drop the rows it failed to reach from the baseline for good;
* **tombstone every absent key anyway**. The producer cannot tell a torn row from a deleted one,
  so it reports the only thing it knows: this key was not in this read. The patch therefore
  reconstructs precisely the rows the read observed — which is exactly what a full payload for
  that read would have carried, so the hour's content is unchanged and the encoding is lossless;
* seed the pin from the first read of the capture **whatever its coverage**, or an all-partial
  domain never acquires a baseline and delta stays permanently dead.

The remaining wire-visible difference between a partial hour and a clean one is unchanged and
lives where it always did: the status word says `PARTIAL_DATA`, and what that costs the hour is
the fold's admission question, not the encoder's.

## 4. Per-domain preconditions

A domain may be switched to delta emission when all of these hold. They are listed because two of
the largest domains fail one of them today.

1. **A stable key exists for every row family**, unique across the whole domain and not merely
   within an owning object. (d41's bare `CProject` local ids were not unique across countries;
   the owner-joined `(tag_db_id, local_id)` key is what replaced them.)
2. **A tombstone row can be encoded for that key** with every live field zeroed and no ambiguity
   against a live row.
3. **The consumer keeps a delta-baseline mirror for the domain**, separate from its projected
   state.
4. **A reconstruction gate exists and is green** (§3).
5. **The per-frame diff cost is affordable.** This is a real constraint and not a formality: the
   d53 lane measured its diff at **+46–68 ms of capture CPU per frame** against an 86.8 ms hour
   budget, which is why d53's partial-delta mode ships off. A diff over hundreds of thousands of
   rows can cost more hours than the smaller writes buy back.

## 5. Status

| Domain | Delta today | Notes |
|---|---|---|
| d42, d46 | 100% of t19 records | working as designed |
| d20, d22, d23, d24, d26, d44, d52 | 38–40% | the rest are keyframes forced by rejected brackets and silent readers at speed five |
| d27, d28, d51 | 19–31% | as above |
| d41 `projects_live` | **0 of 474** in t19, **157 of 256** in t21 | 692 MB in t19, `PARTIAL_DATA` on every record; one country's facility walk, since repaired. See §6 |
| d43 | **0 of 457** | 189 MB, and `SOURCE_UNAVAILABLE` on every record — a completeness problem, not an encoding one |
| d45, d55, d11, d12, d29, d37, d32 | 0% | not yet assessed against §4 |
| d53, d48, d91, d93, d83 | not measured here | their payloads do not use the shared 20-byte data header, so the scan below cannot read their status word. d53 and d48 both have partial-delta implemented; d53's is off because of §4.5 |

The percentages above come from a linear scan of t19's H4CR records reading the status word at
payload offset 16, which is the shared `DOMAIN_DATA` header layout (`elapsed_wall_ms` u64,
`row_count` u32, `changed_rows` u32, `status_flags` u32). Domains with their own payload envelope
are excluded rather than guessed at; a status word read at the wrong offset is not a measurement.

## 6. What refused d41, and what fixed it

**d41 is `PARTIAL_DATA` in 474 of 474 t19 records because of one country.** That much was right.
The cause named in the first draft of this section was not, and the correction matters because it
moved the repair from a domain that was not broken to one that was.

The counters are byte-identical in all 474 records, read straight off
`runs/live_unlock/t19/collector_stream.bin` at the offsets
`recorder_host/src/domain_data_authority.rs` already parses:

| Counter | t19, all 474 records |
|---|---|
| `invalid_facility_countries` | **1** |
| `invalid_facility_candidate_vtable_read_fail` | **1** |
| `invalid_project_pool_countries` | 0 |
| `invalid_program_status_countries` | 0 |
| every other `root_reasons` field | 0 |
| `facility_count` / `scientist_count` | 27 / 153 |

The refusal is in the **facility** walk, not the project pool. (`facility_count = 27` and
`scientist_count = 153` are what confirm the offset mapping: they match the counts reported
independently for this capture.)

**The mechanism.** The facility container hangs off `CProgramStatus+0x20`. Our walk had no count
for it: it ran to a fixed cap of 64 slots and inferred the end of the array from the first slot
that did not look like a facility — zero, outside the heap range, or a vtable mismatch. All three
of those tests can only fire on memory *past the end*, where the bytes are whatever the previous
larger occupant left. So the outcome depended on stale capacity: harmless when the stale word was
zero or obviously foreign, fatal when it happened to be a heap-aligned address that is not mapped,
which fails the read and refuses the country.

artifacts hash-verified) each bound the walk with the i32 at `CProgramStatus+0x2C`:
element — the facility's unread-prototype-rewards field — which is what identifies the elements.
Bounding our walk by that count is the fix; the constant carries the decompilation and two
corroborations.

**t21 is the control.** The same decode over `runs/live_unlock/t21` shows every invalid counter at
**0** across all 256 records, and `DELTA_DATA` already set on 157 of them — keyframe 1,458,417
bytes, delta 420 bytes. So the delta path was never the problem: it fires by itself the moment a
read comes back clean, exactly as §3.1 says. t21 also carries one fewer facility than t19 (26 vs
27), which is why the same code was clean there — the defect is state-dependent, not a structural
singleton, and it can recur.

That leaves the second option, which is **not** being taken:

* **Tolerate the partial.** Giving d41 an entry in `COMPACT_EXPECTED_PARTIAL_TOLERANCES` would
  admit the hours and let `DeltaPartial` (§3.1) make them cheap. It was rejected: it papers over a
  reader defect rather than fixing it, and it is only sound with a per-country identity for what
  was missed. A counter of `1` names no country, and "exclude and account" needs an identity to
  account with.

That identity now exists regardless. A refused facility or pool root latches the country and the
guard into `d41_facility_failure_stage` / `_tag_db_id` / `_slot_index` / `_count_sample` and
`d41_pool_root_failure_stage` / `_tag_db_id`. The stage word is the presence witness — 0 means
nothing refused, and the other words must not be read when it is 0. `_slot_index` against
`_count_sample` is what decides, without a second capture, whether a future refusal is a read that
ran off the end or a live facility that genuinely could not be read.

`D41EmitMode::DeltaPartial` still exists, is tested, and is off. With the root repaired the
ordinary `Delta` path is what carries d41, as it already does in t21.
