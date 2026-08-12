# The ≤50 MB target, priced against a measured recording

Task #186. This is a measurement and a decision table. It does not propose an encoder and
it does not promise the target.

Every figure is either measured in this lane over sealed recordings, or **cited** with its
source and its own basis. The two are never mixed in one arithmetic chain, and the cited
ones are labelled as citations every time they appear. Evidence, provenance and
re-runnable instruments: `extractor/native/runs/gw_size_floor/` (start with
`PROVENANCE.txt`; `CHECK.txt` recomputes every number below from the JSON).

**Two denominators are in use and every percentage says which.** "the cache entry" is
`checkpoints.h4cp` compressed; "the file" is the whole `.battleplan`. In the four
recordings this document decomposes entry by entry the cache entry is **78.55–82.72%** of
the file; across all 19 current-format recordings on disk it spans **57.22%** (`t30`) to
**83.66%** (`t31`), because the hour-independent term is a fixed ~9.9 MB and a short
recording is mostly that. The two denominators must never be conflated.

---

## 0. The deciding numbers, first

| | bytes | basis |
|---|---:|---|
| The target | 50,000,000 | one shared recording, post-compression |
| Hour-independent floor already inside the file | **9,912,361** | measured; 9,130 bytes of it (ZIP framing, `index.json`, `manifest.json`) are t49-scale placeholders whose campaign values are unpriced (§2.2) |
| Left for the whole campaign's hours | 40,087,639 | 50,000,000 − 9,912,361 |
| Budget per game hour over a 105,192-hour campaign | **381.1** | 40,087,639 ÷ 105,192 |
| What one cache record costs today | **88,385** | measured marginal slope, §3 |
| What one cache record costs after every priced lever | **45,193** | measured marginal slope of the transformed candidate, §7 |

A campaign holds **one cache record per game hour** — that is what zero loss at speed five
means, and it is the product's own bar — so the two units coincide in the projection and
differ in the captures, which lost hours. §3 states that assumption where it is made.

**Answer to the question asked: on every measurement available, no — ≤50 MB per campaign
recording is not reachable with plain codecs.**

* Today a 1936–1948 campaign projects to **9.31 GB — 186× the target**.
* After every lever this lane can price — dropping a provably redundant field, one
  checkpoint per campaign, and zstd level 19 instead of level 3, all applied to one byte
  stream and compressed once — it projects to **4.77 GB, 95× the target**. Two of those
  three levers are ablation *proxies* rather than built artifacts, with errors that are
  neither signed nor bounded; §7 says exactly which. **4.77 GB is a two-proxy projection,
  and it stays one until an L1 fold and an L2 cache are built and packed.**
* The largest cache-encoder rewrite anyone has measured on this artifact is **1.349×**
  (`DELTA_WIRE_DESIGN.md` §9.3, on a different capture, expressly *not* a proven ceiling,
  and not re-measured here). It is **not** applied to the verdict — it was never measured
  jointly with these levers, and §2.3 shows this artifact does not compose that way.

**What would have to be true for the answer to be yes**, stated as a number rather than a
hedge. Of the 50,000,000 bytes, 9,912,361 is hour-independent, 5,516,746 is the projected
`hour_index.h4ix` (its own measured marginal, 52.44 per record, plus a 354-byte intercept),
and 1,571,334 is the cache's measured one-off cost — the fitted intercept, the seed
checkpoint and zstd's window warming, paid once by any recording. That leaves **313.71
bytes per game hour for the cache**, the only term a cheaper game hour affects. The
measured post-lever record costs 45,192.9 bytes. Every recording on disk is late-war and a
1936 hour is plausibly much cheaper (Trap 3, §3), so for 50 MB to come into reach at
today's fidelity **the campaign-average hour would have to be 144× cheaper than the
measured late-war hour**. N1 is the experiment that would test it; this document does not
claim to know the answer, it states the threshold.

That threshold is **before L4**. If the owner also drops the static catalogs, their
9,847,142 bytes of payload return to the hourly budget, raising the cache allowance to
407.32 bytes per game hour and lowering the threshold to **≈111×** — and slightly further,
by the ZIP framing L4 also removes, which is unmeasured (§7 L4). Either way the answer is
two orders of magnitude, not a near miss.

Barring that, **the only route to 50 MB this lane can price is recording less: fewer hours,
a coarser cadence, or fewer domains** — §8 prices the first, N4 is the experiment for the
second, and `DECISIONS.md` currently forbids the third. That is a statement about what has
been measured, not a proof that no encoder exists: N3 could still move the floor, and the
cited information estimate in §6 is explicitly not a lower bound.

The brief's premise — a 14.465 GB artifact, a ~290× ask — is stale in the product's
favour. The raw collector stream is no longer packaged (owner ruling 2026-08-05;
`--package-raw-stream` is opt-**in**, `recorder_host/src/main.rs:4875,4905`), deleting
6.002 GB of it. The correction came from a packaging ruling, not an encoder.

The largest new lever found here: **`map._r2_weather_live` is 28–35% of the file**, and
inside it the four derived float spellings are **10.13–18.35% of the file** while being a
pure arithmetic function of the `*_raw` integers emitted beside them under the same
presence bit (§2.5). Removing them costs no fidelity — and does not approach 50 MB.

---

## 1. What "the recording" is, and what the working files are

The target binds the **sealed `.battleplan` container** — the single file a player drops
into Discord. On the current tip that file holds twelve entries. Everything else the
recorder touches is a working file that never leaves the machine.

| file | role | in the recording? |
|---|---|---|
| `native_session.battleplan` | **THE recording.** One campaign, one file, for its whole life (`documents/DECISIONS.md`). | it *is* the artifact |
| `payload/<id>/replay_decoder/checkpoints.h4cp` | H4CP v2 checkpoint+delta cache: one record per sealed game hour, carrying the world document | yes — 78.6% of t49's file |
| `payload/<id>/replay_decoder/hour_index.h4ix` | authenticated per-hour offsets and digests | yes |
| `payload/<id>/static/*.json` | six catalogs resolved from the player's own content at capture time | yes — 21.2% |
| `payload/<id>/recorder/native_session_manifest.json` | capture provenance, gap accounting, per-domain outcomes | yes — one per session (N6) |
| `manifest.json`, `index.json`, `timeline.json` | container identity and hour timeline | yes |
| ZIP local headers and central directory | container framing | yes — 2,972 bytes |
| `collector_stream.bin` (the spool) | the raw H4CR wire stream the fold consumes | **no** — working file, opt-in |
| `.replay_decoder_artifacts-*` | finalize spill directories | no |

Two things the size discussion must keep out of the budget but not out of mind:

* **The spool.** t48's is 1,080,523,256 bytes for 580 advanced hours
  (`stream_storage.stored_bytes`; these measurement captures ran
  `--stream-compression none`). A disk and throughput concern, not a Discord one.
* **The cache before packing.** t49's is 1,014,774,624 bytes over its 598 cache records —
  **1,696,948 bytes per cache record** uncompressed. (Per *advanced game hour* it is
  1,365,780, because that capture advanced 743 hours and sealed 598 records; the two units
  differ exactly as §3 describes.) Projected at one record per game hour — the zero-loss
  assumption §3 states — 105,192 records is **~178 GB** written and read during finalize to
  produce a ~9 GB container. Out of scope here; R2 in §10.

---

## 2. The measured decomposition of a real sealed recording

### 2.1 The compressor, named, and the controls that hold it

Every compressed figure is produced by the container's own encoder: **zstd, level 3,
windowLog 27, long-distance matching on, content checksum on, pledged source size** —
`configured_zstd_encoder` and `CompressionProfile::default()` in
`portable_replay/native/snapshot/src/pack.rs:230-296, 1919-1928`, cross-checked against
each recording's own `replay_packaging.compression` manifest field. Entries below
`ZSTD_MIN_ENTRY_BYTES` (16 KiB) stay STORE. **No exotic codec appears anywhere in this
document**; §7 L3 prices a higher *level* of the same codec and says what it costs.

**BASELINE.** Recompressing the untouched `checkpoints.h4cp` extracted from a sealed
container reproduces that container's own stored `compress_size` **exactly, to the byte**:
t49_pricing 36,415,870; t48_authoritative 39,206,934; t40b_probe 24,859,196. Both sides
link libzstd **1.5.7** (Rust `zstd-sys 2.0.16+zstd.1.5.7`; Python `zstandard 0.25.0`).
This is an **assertion**, not a printed note — `gw_cache_sections.py`, `gw_levers.py`,
`gw_derived_twins.py` and `gw_floor_stack.py` all exit non-zero if it ever drifts.

**PARITY.** The H4CP v2 record-grammar walk is total *and ordered*: framing plus every
attributed span reconstructs the cache byte for byte, in sequence. Asserted in
`gw_cache_sections.py` (coalesced spans must equal `[(0, len(cache))]`) and independently in
`gw_derived_twins.py`, where every span must begin exactly where the previous one ended —
so a walk that double-counted one span and omitted another of the same size fails, which a
bare byte total would not catch. It accounts for all 1,014,774,624 bytes of t49's cache.
The leaf walker behind L1 carries its own control: it must consume each record's payload
to exactly the declared end, and refuses any record whose kind it does not know.

### 2.2 The container, per entry

`t49_pricing`, sealed 2026-08-11, 598 cache-hours covering 1944.4.15.13 → 1944.5.16.12.
All four recordings are in `runs/gw_size_floor/TABLES.txt`.

| entry | compressed | uncompressed | % of file | scales with |
|---|---:|---:|---:|---|
| `replay_decoder/checkpoints.h4cp` | **36,415,870** | 1,014,774,624 | 78.55% | game hours |
| `static/localisation_join/localisation_catalog.json` | 5,418,061 | 50,669,056 | 11.69% | fixed |
| `static/gfx_icon_asset_manifest_static/…json` | 3,093,948 | 52,653,142 | 6.67% | fixed |
| `static/decisions_static_definitions/…json` | 638,046 | 19,257,139 | 1.38% | fixed |
| `static/idea_tech_loc_xref_static/…json` | 341,055 | 6,032,976 | 0.74% | fixed |
| `static/templates_equipment_catalog/…json` | 330,906 | 6,316,160 | 0.71% | fixed |
| `recorder/native_session_manifest.json` | 55,626 | 1,087,248 | 0.12% | **sessions** (N6) |
| `replay_decoder/hour_index.h4ix` | 30,473 | 57,118 | 0.07% | game hours |
| `static/ship_definitions_static/…json` | 25,126 | 483,671 | 0.05% | fixed |
| `index.json` | 3,249 | 17,759 | 0.01% | hour count and entry lengths |
| `manifest.json` | 2,909 | 2,909 | 0.01% | entry byte lengths |
| `timeline.json` | 463 | 463 | 0.00% | fixed |
| ZIP framing | 2,972 | — | 0.01% | entry count, and ZIP64 above 4 GiB |
| **file on disk** | **46,358,704** | | 100% | |

**The hour-independent term is 9,912,361 bytes, of which 9,903,231 is genuinely
hour-independent** — the six static catalogs (9,847,142), the recorder manifest (55,626)
and `timeline.json` (463). The other **9,130** are t49-scale **placeholders whose campaign
values are unpriced**: the ZIP framing (2,972 — both projected cache entries exceed ZIP32's
4,294,967,295-byte field limit, so production would write ZIP64 descriptors,
central-directory extras and end records), `index.json` (3,249 — it serialises the hour
count and the index and cache lengths) and `manifest.json` (2,909 — it carries every
entry's byte length). **9,130 bytes against a 50,000,000-byte budget**: they move nothing,
and they are named so the arithmetic is complete rather than quietly wrong.

The term has four parts that behave differently, which matters because L4 in §7 removes
only one of them:

```
  six static catalogs                          9,847,142     L4 removes this payload, plus
                                                             their ZIP framing (unmeasured)
  index.json + manifest.json + timeline.json       6,621     NOT actually fixed: index.json
                                                             serialises the hour count and
                                                             the index/cache lengths, and
                                                             manifest.json carries every
                                                             entry's byte length, so both
                                                             change at campaign scale. t49's
                                                             sizes are a PLACEHOLDER; the
                                                             campaign values are unpriced
  ZIP framing                                      2,972     twelve entries' local headers
                                                             and central-directory records.
                                                             L4 removes SIX entries' share
                                                             of this; the rest stays. The
                                                             removed subset is unmeasured
  one recorder manifest                           55,626     per SESSION, not per hour (N6)
                                              ------------
                                               9,912,361     19.8% of the 50 MB budget
```

The six static catalogs are **byte-identical across all four of t46/t47/t48/t49** —
different captures, different hour counts, 9,847,142 bytes every time. They are a function
of the player's content set, so a modded campaign has a different floor (N7). The recorder
manifest is a **per-session** term and a campaign stitches many sessions, so 9,912,361 is a
one-session floor and the campaign totals below are correspondingly a lower bound on that
term until N6 is measured.

The cache entry's compression ratio across the four recordings is **27.87× – 47.26×**,
which brackets the 41.1× that task #100 and `DELTA_WIRE_DESIGN.md` §8.5 report. That ratio
has been read as headroom; §9.3 of that document already measured that it is not — the
best cache-encoder rewrite anyone has built recovers **1.349×** of it, because zstd took
the rest first.

### 2.3 The cache entry, per world-document section

Method: walk the H4CP v2 record grammar, assign every payload byte to exactly one top-level
member, then for each member recompress the cache entry with that member's bytes removed
and read the difference in container bytes. This is the instrument
`DELTA_WIRE_DESIGN.md` §8.1 used on the stream, moved to the cache. The ablated artifact is
not a valid cache: this attributes where the disk sits, it does not propose deleting
anything.

Ranked by t49's disk cost, against **both** denominators.

| section | content (t49) | **disk (t49)** | % cache entry | % file | **disk (t48)** | % cache entry | % file |
|---|---:|---:|---:|---:|---:|---:|---:|
| **`map`** | 121,035,876 | **17,556,892** | 48.21% | **37.87%** | **15,205,929** | 38.78% | **30.94%** |
| `divisions` | 225,565,134 | 7,871,476 | 21.62% | 16.98% | 11,375,366 | 29.01% | 23.15% |
| `countries` | 193,623,012 | 5,244,895 | 14.40% | 11.31% | 5,102,842 | 13.02% | 10.38% |
| `supply` | 37,276,984 | 3,471,678 | 9.53% | 7.49% | 5,364,985 | 13.68% | 10.92% |
| `order_of_battle` | 175,073,093 | 1,583,765 | 4.35% | 3.42% | 2,336,801 | 5.96% | 4.75% |
| `provinces` | 73,320,181 | 805,992 | 2.21% | 1.74% | 1,016,611 | 2.59% | 2.07% |
| `states` | 37,706,084 | 736,298 | 2.02% | 1.59% | 879,347 | 2.24% | 1.79% |
| `naval_air` | 51,790,641 | 183,786 | 0.50% | 0.40% | 382,666 | 0.98% | 0.78% |
| `strategic_air` | 51,919,683 | 161,685 | 0.44% | 0.35% | 354,512 | 0.90% | 0.72% |

Three properties that govern how this table may be used:

* **Content ranking inverts disk ranking.** `order_of_battle` carries 45% more content than
  `map` and costs 9% of what `map` costs. Rank encoder work by container bytes or rank it
  wrong.
* **These are individual marginals and they do not add.** The 41 sum to 37,924,557 against
  a 36,415,870 cache entry on t49 (+4.1%), and 42,441,737 against 39,206,934 on t48
  (+8.3%). **Every grouped figure in this document is a separate joint ablation, never a
  sum**; where a sum is quoted it is labelled as a sum and is not called a share.
* **Removing content can enlarge the artifact.** Ten sections on t49 and thirteen on t48
  have negative marginals (most negative: `_r2_dynamic_modifier_lifecycle_live` −20,731 on
  t49, `measured_countries` −29,459 on t48). This reproduces `DELTA_WIRE_DESIGN.md` §8.2's
  non-monotonicity on different material and a different artifact.

Groups, each measured by one joint ablation:

| group | sum of marginals | **JOINT** | % cache entry | % file | joint ÷ sum |
|---|---:|---:|---:|---:|---:|
| **t49** top 2 (`map`, `divisions`) | 25,428,368 | 25,149,619 | 69.06% | 54.25% | 0.99 |
| **t49** top 4 (+ `countries`, `supply`) | 34,144,941 | 33,241,654 | 91.28% | 71.71% | 0.97 |
| **t49** top 8 | 37,454,782 | 35,473,633 | 97.41% | 76.52% | 0.95 |
| **t49** `naval_air` + `strategic_air` | 345,471 | **1,185,504** | 3.26% | 2.56% | **3.43** |
| **t49** the 32 sections outside the named nine | 308,090 | **660,437** | 1.81% | 1.42% | **2.14** |
| **t49** the whole "converging" set of §5 (37 sections) | — | **4,690,493** | 12.88% | 10.12% | — |
| **t48** top 4 | 37,049,122 | 35,634,264 | 90.89% | 72.51% | 0.96 |
| **t48** `naval_air` + `strategic_air` | 737,178 | **1,459,043** | 3.72% | 2.97% | **1.98** |
| **t48** the 32 sections outside the named nine | 422,678 | **784,047** | 2.00% | 1.60% | **1.85** |
| **t48** the whole "converging" set of §5 (37 sections) | — | **5,549,056** | 14.15% | 11.29% | — |

**The small-group rows are why grouped figures must be measured, not summed.** For the
large head the sum is a passable approximation of the joint; for the small groups it is off
by 1.9–3.4×, always in the direction that makes a small group look smaller. Two sections
that individually cost 0.44% and 0.50% of t49's cache entry cost **3.26% together**,
because each was standing in as the other's match source — the air sections share four of
their five members (`world_fold.rs`, `build_strategic_air` / `build_naval_air`), so
ablating one leaves the other to supply the matches. An earlier draft of this document
quoted these sums as shares; they are shown here only beside the joints that replaced them.

### 2.4 `map` is `weather`

`map` is a bag. Drilling it one level with the same instrument:

| `map` member | content (t49) | **disk (t49)** | % cache | % file | **disk (t48)** | % cache | % file |
|---|---:|---:|---:|---:|---:|---:|---:|
| **`_r2_weather_live`** | 69,936,597 | **16,448,057** | 45.17% | **35.48%** | **13,839,043** | 35.30% | **28.16%** |
| `rail_replay_state_live` | 49,525,561 | 877,793 | 2.41% | 1.89% | 1,237,349 | 3.16% | 2.52% |
| `map_detail_live` | 908,561 | 184,400 | 0.51% | 0.40% | 131,129 | 0.33% | 0.27% |
| `control_by_province` | 615,032 | 179,994 | 0.49% | 0.39% | 124,028 | 0.32% | 0.25% |
| `owner_by_state` | 49,457 | 9,012 | 0.02% | 0.02% | 23,458 | 0.06% | 0.05% |

**One domain — d24 `weather_live` — is 28–35% of the entire sealed file, on both recordings
measured.** Its own frame is **15,305,338** bytes against a 16,448,057 marginal — **93.1%**
— so it shares almost nothing with the rest of the artifact: independent content, not
repetition zstd is already collapsing. It is the same domain `DELTA_WIRE_DESIGN.md` §8.1
found at 2.11% of the stream's payload and 15.62% of its disk; the inversion is not new,
its size in the cache is.

### 2.5 Part of `weather` is arithmetic

Drilling `_r2_weather_live` to leaf fields. These are **individual marginals** and do not
add; the joint below is what the lever is worth.

| leaf field | content | individual marginal | % cache entry |
|---|---:|---:|---:|
| `provinces/*/temperature` | 10,346,603 | 2,987,181 | 8.20% |
| `provinces/*/temperature_raw` | 9,719,331 | 2,336,529 | 6.42% |
| `provinces/*/temperature_change` | 13,467,893 | 2,629,156 | 7.22% |
| `provinces/*/temperature_change_raw` | 12,774,263 | 2,175,908 | 5.98% |
| `provinces/*/water` | 5,444,598 | 2,146,473 | 5.89% |
| `provinces/*/water_raw` | 5,080,339 | 1,636,942 | 4.50% |
| `provinces/*/snow` | 1,651,978 | 424,204 | 1.16% |
| `provinces/*/snow_raw` | 1,535,188 | 302,074 | 0.83% |
| the 28 remaining weather fields | 6,183,459 | each ≤133,362 | each ≤0.37% |
| **sum of the 36 labels** | **66,203,652** | | |

Those 36 labels are 66,203,652 of the member's 69,936,597 content bytes from §2.4. The
3,732,945-byte remainder is object framing *between* the labelled depths — the tag and
member count of `_r2_weather_live`, of its `regions` and `provinces` maps, and of each
per-province and per-region object — which this instrument charges to no field. The
whole-cache PARITY control in `gw_derived_twins.py` accounts for it separately.

`WeatherProvinceState::to_json` (`recorder_host/src/world_fold.rs`, at :29163 on this tree)
emits each of those four quantities **twice**: once through `optional_i32(flags, bit, raw)`
and once through `optional_scaled_i32(flags, bit, raw, scale_raw)`, which is
`json!(raw as f64 / scale_raw as f64)` behind the **same presence bit** (both helpers at
:31758-31772). `scale_raw` is emitted once at the weather root — 56 content bytes for the
whole capture. So `temperature` ≡ `temperature_raw / scale_raw`, and both are `null`
together when absent. **The derived spelling carries no information the document does not
already hold.** (Line numbers move between lanes; the symbol names are the durable
citation.)

Measured jointly, **path-scoped** to exactly
`map/_r2_weather_live/provinces/*/{temperature, temperature_change, water, snow}`:

| recording | cache content | content removed | % content | **JOINT disk** | **% cache entry** | **% file** |
|---|---:|---:|---:|---:|---:|---:|
| t49_pricing | 1,014,774,624 | 30,911,072 | 3.05% | **8,508,484** | **23.36%** | **18.35%** |
| t40b_probe | 572,582,901 | 12,422,302 | 2.17% | **3,528,976** | **14.20%** | **10.13%** |

Two to three percent of the content, ten to eighteen percent of the file. That is the
signature `DELTA_WIRE_DESIGN.md` §9.1 predicted from the other end: float spellings are the
expensive class precisely because they are the one class zstd cannot collapse.

**The four individual marginals sum to 8,187,014; jointly they are 8,508,484** — the joint
is 3.9% *larger* than the sum, non-additivity in the opposite direction from §2.3's. Only
joint figures are used anywhere in this document.

Two scoping properties, both stated because both changed an answer:

* **Path scope, not name scope.** A name-scoped drop of the same four keys also takes
  `map.regions.*.snow`, a boolean flag with no raw twin. Every figure above and every stack
  cell in §7 is path-scoped.
* **The walk descends through replacements.** An earlier version of this instrument charged
  a `PATCH_REPLACE` whole to its parent label without descending, so in the hours where a
  delta replaced the whole `_r2_weather_live` member the derived spellings inside were never
  labelled and never dropped. That silently excluded ~1.0 MB of content on each recording
  and understated the lever by about a point of the file. Found in review; the walker now
  descends, and every number in this section and in §7 is from the corrected run.

A whole-document census finds **59 field names** carrying both `<name>` and `<name>_raw`;
**40** of the 59 occur an identical number of times. The bytes attributed to their derived
halves are 86,931,509 against 72,526,680 for the raw halves.

**How much those 55 further names are worth is NOT established here.** The obvious
experiment — drop all 59 names jointly — is a *name-keyed* ablation: it removes every
member carrying one of those names anywhere in the document, including whole
container-valued subtrees and unrelated same-named fields, so the candidate deletes
**126,090,943** source bytes rather than the 86,931,509 the census attributes. Its
10,040,305-byte marginal therefore prices that broader deletion, not the derived halves,
and no "the other 55 add X" inference can be drawn from it. Only the weather four have been
priced with a candidate whose removed bytes match its attributed spans (§2.5's path-scoped
drop). Extending that treatment to the other 55 is N2-shaped work.

(That census had the same replacement-descent defect as the leaf walker and was corrected in
the same review: before the fix it reported 49 pairs and 32,908,021 derived bytes, because
every field inside a wholesale-replaced subtree was charged to the subtree's own key. The
figures above are from the corrected walk. The §7 verdict never used this census — it uses
the separately path-scoped weather walk — so the correction moves nothing in the floor.) **Only the weather four are proven derivations, by reading
the producer**; the other 55 name pairs are structural evidence, not proof, and 19 have
unequal occurrence counts, so they are certainly not all one field emitted twice. The lever
priced in §7 is the four, not the fifty-nine.

---

## 3. The basis, and three traps in it

A campaign projection multiplies a per-hour rate by ~10⁵. Three things make the obvious
rate wrong.

**Trap 1 — a per-capture average is not a per-hour rate, and neither is a whole-capture
compression ratio.** Each capture pays one-off costs (the seed checkpoint, zstd's window
warming) that a campaign amortises to almost nothing. Compressing successive prefixes of one
cache separates them (`gw_marginal_hour.py`): the intercept is 1.49 – 2.69 MB per capture,
and a projection must not *multiply* it — but it must still **add it once**, because a real
recording pays it. Every projection in §4, §7 and §8 does. It is 0.03% of a campaign and
3.9% of the whole 50 MB budget, so ignoring it would be invisible in one place and decisive
in the other. **The same objection applies to a ratio**: a whole-capture factor like
`13,586,533 / 24,859,196` carries that intercept in its denominator, so it is not a marginal
factor and may not be multiplied by a marginal rate. §7 therefore measures the *transformed*
artifact's own marginal slope rather than scaling the untransformed one.

**Trap 2 — loss deflates the rate.** An hour that lost domain data emits a smaller record.
The manifest metric is `game_hours_without_data.total`: **advanced game hours that placed no
expected-domain data on the wire** (`hour_data_accounting` / `hour_data_accounting_json` in
`recorder_host/src/main.rs`, at :24455 on this tree), divided here by
`game_hours_advanced`. It is not a count of domain-hours. Across the 19 sealed recordings on
disk the per-hour rate is anti-correlated with it:

| capture | cache records | advanced hours with no expected-domain data | share | avg B/cache record | **marginal B/cache record** (OLS / 2nd half) | intercept |
|---|---:|---:|---:|---:|---|---:|
| `t30` | 129 | 13 of 128 | **10.2%** | 103,026 | **91,120 / 87,815** | 1,760,433 |
| `t40b_probe` | 266 | 25 of 265 | **9.4%** | 93,456 | **88,385 / 93,906** | 1,489,337 |
| `t31` | 602 | 199 of 601 | 33.1% | 86,721 | 80,788 / 85,949 | 2,688,407 |
| `t49_pricing` | 598 | 642 of 743 | 86.4% | 60,896 | 58,219 / 55,693 | 2,150,435 |
| fleet, n=19 | — | — | 9.4–88.1% | min 43,038 · median 70,710 · mean 70,433 · max 103,026 | — | — |

**The x axis is cache RECORDS, not advanced game hours.** `gw_marginal_hour.py` regresses
compressed prefixes against the record count, and the two differ whenever a capture loses
hours: t49 holds 598 records for 743 advanced hours. **The campaign projection equates one
record with one game hour**, which is what the product's own bar asserts — zero loss at
speed five means every game hour seals a record — so the projection is of a *zero-loss*
campaign at a *late-war* per-record cost. Both halves of that sentence are stated because
both are assumptions: N5 for the first, N1 for the second.

The fleet minimum (43,038, `t38_4c`) is a four-core run with 78% of its advanced hours
carrying no expected-domain data. **Reading the fleet minimum as a floor reads a gap as a
saving.** The product bar is zero loss at speed five (`documents/DECISIONS.md`), so a
zero-loss production capture would plausibly cost *more* per hour than any rate here.

That is a **directional expectation, not a bound**. The table is an observational pairing
across captures that differ in more than their loss, not a zero-loss A/B; and §2.3 shows on
this very artifact that removing content can make the compressed result *larger*, so "fewer
fields on the wire" does not by itself determine which way the compressed bytes move. N5 is
the experiment that would measure it.

**The basis this document projects on: the marginal per-cache-record cost of `t40b_probe`, the
most complete recording long enough to fit a curve — 88,385 B (OLS), intercept 1,489,337.**
It still carries 9.4% residual loss (N5).

**Trap 3, unresolved — every sealed recording on disk is late-war.** All 19 sit between
1944.3.22 and 1944.6.4, from one save. A 1936 hour holds a fraction of 1944's divisions,
combats and air wings, so the campaign integral is plausibly well below late-war-rate ×
hours — again a directional expectation, and nothing on disk quantifies it. N1 names the
experiment.

**So §4's and §7's campaign totals carry three unquantified effects: Trap 3 pushing down,
Trap 2 and the per-session manifest term pushing up.** This document does not assert they
are too small to matter. It states the threshold instead: **Trap 3 would have to make the
campaign-average hour 144× cheaper than the measured late-war hour** for 50 MB to come into
reach (§0), and N1 is the experiment that would settle it.

---

## 4. The campaign projection, today

Campaign length **105,192 game hours** (1936-01-01 → 1948-01-01), the default in
`extractor/native/scripts/Get-FullGameSizeProjection.ps1` and the basis
`DELTA_WIRE_DESIGN.md` §8.5 used.

```
file(H) = 9,912,361            hour-independent, §2.2   (a ONE-SESSION term; N6)
        +    52.44 x H + 354    hour_index.h4ix          MARGINAL slope and intercept,
                                                        median of four recordings
        +    rate  x H         checkpoints.h4cp, the fitted slope
        + intercept            checkpoints.h4cp, the fitted one-off, added ONCE
```

**Both** per-record terms are fits with their own intercept, added once rather than
multiplied — see Trap 1. The index term used to be a per-capture average (whole entry ÷
record count, 53.17); it is now its own prefix fit, because H4IX has a fixed header before
its per-hour loop and zstd has a startup cost, and multiplying an average that carries both
by 10⁵ is the same error. The two intercepts are invisible here and decisive in §8.

**Three terms inside the fixed 9,912,361 are measured at t49's scale and are not valid at
campaign scale** — the ZIP framing (production would write ZIP64 descriptors, extras and end
records once the cache entry passes 4,294,967,295 bytes), `index.json` (it serialises the
hour count and the index and cache lengths) and `manifest.json` (it carries every entry's
byte length). Their campaign values are **unpriced**; together they are 9,130 bytes and move
nothing below. They are named because the arithmetic should be complete (§2.2).

| basis | rate B/cache record | intercept | cache entry | file | vs 50 MB |
|---|---:|---:|---:|---:|---:|
| **marginal, t40b_probe, OLS** | **88,385** | 1,489,337 | 9.299 GB | **9.314 GB** | **186×** |
| marginal, t40b_probe, second-half slope | 93,906 | 1,489,337 | 9.880 GB | 9.895 GB | 198× |
| marginal, t30, second-half slope | 87,815 | 1,760,433 | 9.239 GB | 9.255 GB | 185× |

**Two more rates exist and are deliberately NOT projected**, because both are whole-capture
averages with no intercept decomposition and multiplying one by 10⁵ is the Trap 1 error
this document rejects: the fleet mean, **70,433 B per cache record** across all 19
recordings at every completeness level, and `DELTA_WIRE_DESIGN.md` §8.5's cited t26 cache
term, **79,624.6 B per cache record**. They are quoted as rates, for scale, and they
bracket the marginal basis from below — which is the only thing they are good for.

Reading 50 MB as 52,428,800 (MiB) moves 186× to 178×. The `hour_index.h4ix` term deserves
its own line: 52.44 bytes per record plus a 354-byte intercept projects to **5,516,746 bytes (5.52 MB) — 11.03% of the entire 50 MB
budget, for an index.**

Two cross-checks that this is the same world the prior measured:

* §8.5's cache term (51,278,269 over 644 t26 records = 79,624.6 B per record) sits inside
  this lane's fleet range of averages, between the loss-flattered mean and the marginal.
  Average against average — the comparison is like for like, which is why it is a
  cross-check and not a projection.
* §8.1's stream term is reproduced exactly: `collector_raw/collector_stream.bin` in the
  on-disk t26 container is **36,747,703** compressed bytes, the figure §8.1 publishes.

**Where the brief's 14.465 GB went.** §8.5's table is a synthesis of a measured stream term
and a separately built cache; the t26 container on disk is a per-hour-JSON-snapshot
generation and holds no `checkpoints.h4cp` at all. Of that 14.465 GB the 6.002 GB stream
term is no longer in a production recording, and the 8.376 GB cache term re-measures on
current material to 9.31 GB when the basis is the least-lossy sufficiently long recording
rather than a fleet average. That basis still carries 9.4% residual loss (N5); it is the
best available, not a loss-free one.

---

## 5. The funded levers, priced

**Delta emission (task #55) — landed; contributes nothing further.** It is inside every
number in this document; `DELTA_WIRE_DESIGN.md` §5 is its status table. Counting its
30–100× again would be double-counting a saving already taken, and it was taken on the
**stream**, which a production recording no longer ships.

**H4CP v3, journal-is-cache — a finalize-time lever, not a size lever.** The funded evidence
(`runs/ge_v3_stage3/RESULTS.md`, `DEPTH_SWEEP_TABLE.txt`) prices journal bytes against the v2
patch bytes they replace: **1.03–1.10× at the argmin grain depth 2**, 1.34–1.42× at depth 3,
1.34–1.73× at depth 1. At the funded depth the journal is at or slightly **above** v2 parity
in bytes. What it buys is time: 0.0858 s and 0.0890 s per game hour at full 41-of-41-section
coverage against a 0.20 s threshold. Those are **pre-compression journal bytes for individual
sections**; no v3 cache has ever been packed and measured in container bytes — the stage-3
report says the container writing is itself unpriced (N3).

**So v3's effect on container size is unpriced, in magnitude and in sign, and it is
excluded from the verdict.** The floor arithmetic carries no v3 factor at all. That is not
the same as crediting it with 1.00×, and this document does not claim the omission is
conservative: §2.3 shows compressed size on this artifact is non-monotone, so a journal
1.03–1.10× larger before compression could land either side of v2 after it. N3 is the only
thing that settles it.

**Keyframe / checkpoint policy — measured here.** In the cache, keyframes are the checkpoint
records. `DELTA_WIRE_DESIGN.md` §8.4 modelled the interval from t26 record lengths and
projected −2.38% of the cache for giving up seeking entirely, pricing promoted and demoted
hours at observed means over a 1,209-to-27,357,593 spread. This lane measures it directly in
container bytes:

| recording | checkpoints | **all checkpoints deleted** | % cache | % file | **keep only the seed** (the policy) | % cache |
|---|---:|---:|---:|---:|---:|---:|
| t49_pricing | 4 | 5,396,354 | 14.82% | 11.64% | **5,408,886** | **14.85%** |
| t48_authoritative | 3 | 3,686,839 | 9.40% | 7.50% | — | — |
| t40b_probe | 2 | — | — | — | **1,907,042** | **7.67%** |

**"Keep only the seed" is the policy, not an approximation.** In every recording the first
checkpoint record is ~1.2 KB against tens of megabytes for the rest — t49's four are
**1,209 / 17,250,341 / 17,405,053 / 27,421,086** and t40b's two are **1,208 / 32,972,885** —
so retaining the first and dropping the rest removes every whole-world image of any size,
which is what one checkpoint per campaign does. No 626/627 scaling is needed and none is
applied. (On t49, dropping three of four moves marginally *more* disk than dropping all four
— 5,408,886 against 5,396,354 — §2.3's non-monotonicity again, and confirmation the seed
record costs nothing.)

**The candidate is a PROXY for the policy, not a bound on it.** An hour demoted from
checkpoint to delta would really be re-encoded as a patch against its predecessor; this
instrument cannot produce that patch, so it drops the record instead. It is tempting to
call that a lower bound — a real cache would carry those bytes, so it must be bigger — but
**this artifact's own evidence refuses that argument**: deleting all four of t49's
checkpoints yields 31,019,516 bytes, while *keeping* the 1,209-byte seed yields the
**smaller** 31,006,984. Adding content shrank the frame. Compressed size is not monotone
in its input here (§2.3), so "the real thing has more bytes, therefore it compresses
larger" does not follow.

What the candidate is: the closest artifact this lane can build without re-encoding, and
the direction of its error is unknown. Closing that needs a cache actually built at a
different interval — the same experiment N4 needs, run at interval = campaign length.

For composition: drilling `divisions` shows its checkpoint occurrences carry 146,359,210
content bytes for a 4,632,732 individual marginal and its delta occurrences 79,204,305 for
3,098,092. Those are two individual marginals of the same section, so they are a comparison
and **not** a decomposition of the 5,396,354 checkpoint cost; what they establish is the
ordering — the checkpoint half of `divisions` costs more disk than its whole delta chain.

### Which sections converge, and which have a structural floor

| class | sections | evidence |
|---|---|---|
| **converging** — 37 sections, none individually worth an encoder | `order_of_battle`, `naval_air`, `strategic_air`, `provinces`, `states`, and the 32-section tail | **the whole set ablated as ONE candidate**: 4,690,493 on t49 (12.88% of the cache entry, 10.12% of the file) and 5,549,056 on t48 (14.15% / 11.29%). That is more than the sum of its parts suggests and is the honest size of the group — but it is 37 producers deep, so reaching it means 37 encoders, not one (§2.3) |
| **structural — keyframes** | the checkpoint records | 14.85% of t49's cache entry, 7.67% of t40b's, from a deletion proxy (§5). A campaign needs at least one whole-world image; above that the count is a seek-policy choice |
| **structural — accounting** | `hour_index.h4ix`, `manifest.json`, `index.json`, `timeline.json`, ZIP framing, the recorder manifest | authenticated per-hour offsets and gap accounting; the index alone is **5,516,746 bytes (11.03% of the 50 MB budget)** at campaign scale, plus the fixed terms, and *partial data still seals* requires the accounting to exist |
| **structural — closing images** | the six static catalogs, 9,847,142 bytes | byte-identical in four recordings; resolved from the player's content (§2.2) |
| **NOT structural, and the largest single item** | `map._r2_weather_live`, and inside it the four derived float spellings | 28–35% of the file; 10.13–18.35% of the file is arithmetic the same document already carries (§2.5) |

---

## 6. The cited information estimate, and what it is not

`DELTA_WIRE_DESIGN.md` §1 reports, over `runs/live_unlock/t19/collector_stream.bin`
(4.78 GB, 81,734 records, **691 game hours**, 67 domains): whole-stream information content
**≈ 30 MB**, "≈150× smaller than the raw stream", derived from long-window LZMA marginal
cost per additional hour per domain.

**Read the basis before the number, and then read what the basis will not support.**

30 MB is a **whole-capture** figure for one capture: 691 advanced game hours, of which
**198 (28.7%) placed no expected-domain data on the wire** (t19's own
`game_hours_without_data.total`). Read as "a campaign carries 30 MB of information" it
inverts the verdict, which is the trap worth naming. But the obvious repair —
`30,000,000 / 691 = 43,415` bytes per hour, times 105,192 — **is also wrong, and this
document does not do it**:

* It is a capture average, and Trap 1 is precisely that a capture average is not a marginal
  rate. The cited source publishes no intercept decomposition, so the marginal cannot be
  recovered from it.
* The capture it averages lost 28.7% of its hours, so the average is loss-deflated in
  exactly the way Trap 2 describes.
* It measures a *different artifact* (the wire stream, not the cache), for a *different*
  domain set (67 against today's 68), on another late-war capture.

An earlier version of this document projected it to 4.567 GB and reported the floor as
"4.1% above the information content". **Both are withdrawn.** They compared a loss-deflated
whole-capture average against a marginal per-record slope, which is the basis error this
document exists to catch.

**What the citation is still worth.** It remains the only independent attempt anyone has
made to measure how much of this data is information rather than retransmission, and its
headline — a stream compressing ~150× under a long-window LZMA — is the reason delta
emission was funded and landed. It is a **capture-level benchmark and a direction**, not a
rate, not a bound, and not an input to any number in §0 or §7. The verdict is built from the
measured levers and the measured gap and would read the same if this section did not exist.
Recovering a usable information *rate* means redoing that analysis with a marginal fit on a
low-loss capture — which is N9.

---

## 7. The decision table

Baseline **9.31 GB** per 105,192-hour campaign (§4). Target 50,000,000 bytes. Gap **186×**.
**Nothing in the "worth" column may be summed**, and the verdict below multiplies nothing.

| # | lever | measured worth (of the file) | cost class | decides | status |
|---|---|---|---|---|---|
| **L1** | **drop the four derived float spellings** at `map/_r2_weather_live/provinces/*/{temperature,temperature_change,water,snow}`; keep `*_raw` and the shared `scale_raw` | **−10.13% (t40b) / −18.35% (t49)**, measured jointly, path-scoped | **encoding change**, information-preserving: a consumer recomputes `raw / scale_raw`. Costs a fold change, a `fold_check` G1 hash rebase, and a replay-side derivation | engineering | measured; the replay consumer change is unscoped |
| **L2** | **one checkpoint per campaign** instead of one per 168 hours | **−7.67% of t40b's cache entry / −14.85% of t49's**, from a **deletion proxy** — the real policy re-encodes the demoted hours as deltas and was not built | **policy** — max seek becomes the whole campaign. Replay already eagerly folds the complete certified stream before opening (`DECISIONS.md`) | owner | **proxy measured, policy not built** (§5); the seek cost is unmeasured |
| **L3** | **zstd level 19**, same codec, same windowLog 27 | **×0.6911 (t40b) / ×0.6946 (t49) / ×0.6762 (t48)** of the cache entry | **owner trade-off on finalize time** — 135 s vs 2.5 s for 1 GB on this machine; ~178 GB of campaign cache implies hours, not minutes. Level 22 buys a further ×0.94 for ~2.4× the time | owner | measured; campaign finalize budget unmeasured |
| **L4** | **drop the static catalogs**, resolve from the player's content at replay time (`DECISIONS.md` already does this for mod graphics) | **−9,847,142 bytes of entry payload** — the catalogs only, not the container's own identity entries. Dropping six entries also removes their ZIP local headers and central-directory records (`pack.rs` writes both per entry), so the real file reduction is **slightly larger** and is not measured here | **product decision** — the recording stops being self-contained; viewing already refuses without the required content | owner | payload measured; a repacked L4 candidate would settle the framing |
| **L5** | **cache encoder v3: global dictionaries + class-major layout** | *cited* ×1.349 on t26 (`DELTA_WIRE_DESIGN.md` §9.3), **not re-measured here** | **encoding change** — needs the whole cache buffered or a second pass; breaks the v2 record format | engineering | stopped at measurement against a ~1.5× bar |
| — | windowLog above 27 | ×0.8821 (t49) / ×0.8849 (t48) at wl30–31 | **not shippable**: `CompressionProfile::validate` caps at the reader contract's 27 | — | quoted only to show long-range repetition is nearly exhausted |
| — | coarser sampling cadence (e.g. daily) | **not 1/24, and not priced.** A delta spanning *k* hours saturates toward a whole checkpoint rather than staying a delta, so the saving is far below 24×. Two illustrative anchors: an hour's average today is ~70.7 KB, and t49's four checkpoint records compress **together** to 4,190,957 bytes — ~1.05 MB per record *within that joint frame*, where the four can match each other, so not an isolated-record cost either. Neither anchor is a marginal cost for a 24-hour delta, and §2.3 forbids treating differently-sampled measurements as bounds. **No range is quoted here** | **fidelity trade** — `DECISIONS.md` sets hourly as the baseline cadence | owner | **needs measurement** (N4) |

### The floor: the transformed artifact's own marginal

A whole-capture compression ratio cannot be multiplied by a marginal rate (Trap 1). So the
transformation is applied first and the slope is measured **on the result**:
`gw_floor_curve.py` drops L1 (path-scoped), applies L2, compresses successive prefixes at
level 19, and fits the slope of container bytes against **retained records** on
`t40b_probe`.

**The x axis is retained records, not original hours.** A record L2 removes contributes no
bytes, so counting it as an x unit would measure "bytes per original hour of a deletion
candidate", which is not a rate a campaign can use — under the real policy every one of
those hours still carries a delta. Regressing against retained records gives the cost of a
record that exists, and the campaign then needs no separate demotion term at all: the
one-checkpoint policy leaves exactly one record per game hour (1 seed checkpoint +
105,191 deltas).

| prefix, game hours | retained records | transformed source | compressed at level 19 | B/retained |
|---:|---:|---:|---:|---:|
| 22 | 22 | 60,109,078 | 2,602,108 | 118,278 |
| 66 | 66 | 137,484,736 | 4,794,846 | 72,649 |
| 133 | 133 | 293,836,325 | 7,432,498 | 55,883 |
| 200 | 199 | 405,128,507 | 10,608,123 | 53,307 |
| 266 | 265 | 528,197,861 | 13,586,533 | 51,270 |

**Transformed marginal: 45,192.9 bytes per retained record (OLS), 46,621.5 (second-half
slope), intercept 1,571,334.** Against the untransformed 88,385 the whole stack is
**×0.5113 on the marginal basis** — *better* than the ×0.5465 whole-capture ratio, because
the intercept the ratio carries in its denominator does not shrink with the levers. The
verdict uses the marginal.

For the record, the whole-capture cells (**not** used in the verdict, and not multipliable
by a rate) on `t40b_probe`: baseline 24,859,196; L1 21,330,220 (×0.8580); L2 22,952,154
(×0.9233); L1+L2 19,712,281 (×0.7930); L1+L2+L3 13,586,533 (×0.5465); level 19 alone
17,180,010 (×0.6911). Multiplying the three separate factors would have given ×0.5475
against the jointly measured ×0.5465. On t49 the same comparison is ×0.4532 against ×0.4368
— 3.8% apart, in the other direction.

### Verdict

```
45,192.9 B per record  measured marginal slope of the transformed candidate
x 105,192 records      one per game hour under the one-checkpoint policy  = 4.754 GB
+ 1,571,334            the same fit's intercept, added once (Trap 1)      = 4.755 GB
+ 9,912,361 hour-independent + 5,516,746 index                            = 4.771 GB    95x
L4 then removes the 9,847,142 static catalog payloads                     = 4.761 GB    95x
THE TARGET                                                                  0.050 GB     1x
```

**Nothing in that chain is multiplied.** 45,192.9 is the slope of the transformed candidate
itself, so L1, L2 and L3 are already inside it. The earlier draft's separate "626 demoted
checkpoint hours" term is gone: it existed only because the slope had been fitted against
original hours, and fitting against retained records removes both the term and the basis
mismatch it was papering over.

**What kind of number 4.77 GB is. Two of its three levers are proxies, and neither proxy's
error is signed or bounded.**

* **L3 is exact.** Compressing the same bytes at level 19 is exactly what that lever does.
* **L1 is a span-deletion proxy.** The instrument removes the labelled byte spans and
  compresses what is left; it does not decrement the object member counts, the patch change
  counts or the record payload lengths that the H4CP grammar requires
  (`replay_cache.rs` `encode_prepared`, and the decoders that read those counts back). So
  the candidate is not a valid cache and not byte-identical to what a fold change would
  produce. The rewrite it skips is not small in count — the four dropped fields occur
  **1,387,590** times in t49 over 598 records, **2,320 per record**
  (`leaf_attrib_…json` `occurrences_removed`), each inside an object or patch whose count
  would change — and its compressed effect is **unmeasured and unsigned**. An earlier draft
  argued it was negligible by comparing uncompressed source bytes against compressed output
  bytes; that comparison was invalid and is withdrawn.
* **L2 is a deletion proxy.** The candidate *deletes* the demoted checkpoint records where a
  real one-checkpoint cache would *re-encode* them as deltas, and §5 shows this artifact
  refuses the obvious "so the real one must be bigger" argument.

So **4.77 GB is a two-proxy projection**, not a measurement of a built artifact. The
BASELINE and PARITY controls certify the instrument against the *untouched* cache; they
say nothing about a constructed candidate, and this document does not claim they do. The
same objection applies to every ablation in §2.3 — that is what an ablation instrument is,
and it is the instrument `DELTA_WIRE_DESIGN.md` §8.1 used for the same purpose. For
attribution ("where is the disk") that is exactly right; for construction ("what would this
artifact weigh") it is a proxy, and the honest way to close it is to build the thing: a fold
without the derived spellings, and a cache at interval = campaign length, packed and
weighed. That is N3-and-N4-shaped work and is not in this lane.

**`DELTA_WIRE_DESIGN.md` §9.3's 1.349× cache rewrite is deliberately NOT in the chain.** It
was measured on a different capture, on the untransformed v2 cache, against a cache-only
denominator, and its own source says it is the best of the constructions measured there and
"not a proven ceiling". Applying it here would be exactly the compounding §2.3 spends its
length refusing. Counting it needs a re-measurement on the transformed current cache.

**So: ≈4.77 GB, 95× the target, as a two-proxy projection.** The **×0.5113** factor is
directly measured — but measured *on the two-proxy candidate*, not on a built artifact, so
it inherits both proxies. Everything downstream of it does too. The gap it has to close is
a factor of 95 and the whole stack is a factor of **1.96**. The proxies' errors are
unsigned and unbounded and this document does not assert they are small; what it asserts is
that L1 and L2 were the two largest levers it
could find and that pulling both, plus the compressor, moved a factor of two.

**No corroboration from the cited information estimate is claimed.** An earlier version of
this document reported the floor as 4.1% above it; §6 explains why that comparison was
invalid — a loss-deflated whole-capture average against a marginal per-record slope — and
withdraws it. The verdict stands on the measured levers alone.

**The nearest point this lane can price is ≈4.8 GB per full 1936–1948 campaign**, which is
inside the 3–8 GB ceiling `DECISIONS.md` already states (R1). Getting there costs one
encoding change, one seek-policy ruling, and finalize time measured in hours rather than
minutes. It is a floor for *these levers on this material*, not a proven lower bound for
any encoder — N1 and N3 are the two experiments that could move it.

**The only routes to the gap this lane can price** are recording less — fewer hours (§8), a
coarser cadence (N4, entirely unpriced), or fewer domains. Each is an owner decision about
the product, not an encoder change, and `DECISIONS.md` currently forbids the third. An
encoder route is not ruled out — N3 is unmeasured and §6's estimate is not a bound — but
nothing measured here or in the prior work points at one worth a factor of 95.

---

## 8. What 50 MB actually buys

The budget is not empty — it is small. The same arithmetic, solved for hours. **Every row
below the first is a fitted two-proxy projection, not a hard artifact limit**: the
post-lever rate comes from §7's candidate, which deletes spans and records rather than
re-encoding them (§7), and both the hour-independent and index terms carry the placeholders
§2.2 and §4 name. The first row — today, at today's fidelity — rests only on a marginal fit
over untouched recordings.

```
50,000,000 - 9,912,361 hour-independent - 1,489,337 intercept = 38,598,302
/ (88,385 + 52.44) B per record, less the 354-byte index intercept = 436 hours = 18.2 days
```

| at | game hours in 50 MB | game time |
|---|---:|---|
| today, 88,385 B per record (intercept 1,489,337) | 436 | 18.2 days |
| after L1 + L2 + L3, 45,192.9 B per record (intercept 1,571,334) | 851 | 35.5 days |
| after L1 + L2 + L3, and L4 freeing the catalog payloads | 1,068 | 44.5 days |

Each row solves `slope × H + intercept + 52.44 × H + 354 + fixed = 50,000,000` for *H* and
**floors** the result — a capacity is the largest whole number of records that fits, and
rounding 1,068.9 up would name a 50,004,143-byte recording as fitting in
50,000,000. The artifact's one-off cost is paid out of the budget once, not ignored
(Trap 1); it costs 17 hours today and 35 at the floor, which is why it is carried here and
barely visible in §4. The L1+L2+L3 rows carry no separate checkpoint-demotion term for the same reason §7 does
not: the rate is per retained record, and a recording of *H* hours under the one-checkpoint
policy holds exactly *H* records.

**A 50 MB recording at today's fidelity covers about two and a half weeks of game time, and
about five weeks after every lever.** The captures in this lane each advanced 580–743 game
hours in **sixty seconds** of speed-five play, so 50 MB is roughly forty seconds of unpaused
max-speed the game today, or eighty after the levers.

That is the shape of the reachable product: 50 MB is a battle, a front, a highlight — not a
campaign. If the owner wants both a shareable file and a whole campaign, those are two
different artifacts, and choosing that is a product decision this document cannot make.

---

## 9. Needs measurement

| # | question the material on disk cannot answer | the experiment |
|---|---|---|
| N1 | **The campaign integral.** Every sealed recording is 1944.3–1944.6, from one save. A late-war rate over 105,192 hours overstates by an unknown factor (Trap 3). | Two 60 s captures on the current tip from a 1936-01-01 save and a 1939 or 1941 save; then `gw_survey.py` + `gw_marginal_hour.py`. Three points give the rate-vs-date curve. |
| N2 | **Whether L1 generalises.** 10.13% of t40b's file against 18.35% of t49's. | Run `gw_derived_twins.py` and `gw_floor_stack.py` over the remaining 17 sealed recordings. Read-only, no build. |
| N3 | **What a v3 journal cache costs in CONTAINER bytes.** Every v3 byte figure to date is pre-compression journal bytes for one section. | Build a full-coverage v3 artifact for one existing capture and pack it with `configured_zstd_encoder`, against the v2 artifact for the same capture. |
| N4 | **What a coarser cadence really costs.** Completely unpriced: a delta over 24 hours is neither 1/24 of an hourly delta nor a whole checkpoint, and nothing on disk measures one. | Rebuild the cache from one capture's H4CR keeping every 24th hour via `build_certified_replay_artifacts`, and pack both. The same build at interval = campaign length also settles L2's proxy (§5). |
| N5 | **The zero-loss rate.** The best recordings still had 9–10% of advanced hours carrying no expected-domain data. The plausible direction is that a zero-loss capture costs more per hour, but the completeness table is an observational cross-capture pairing, not an A/B, and §2.3 shows removing content can enlarge the compressed result — so the direction is untested. | One capture meeting the graded production receipt's zero-loss-at-speed-five bar, measured with these scripts, against a deliberately lossy one from the same save. |
| N6 | **The recorder manifest across a campaign.** ~56 KB per capture with no clear hour dependence; a campaign stitches many sessions, so the 9,912,361 fixed term is a one-session lower bound. | Seal a stitched multi-session container and read the manifest entry. |
| N7 | **The modded floor.** The 9,847,142 catalog term is one vanilla content set. | One capture on a large modset; re-read §2.2. |
| N8 | **Whether the transformed marginal generalises.** It is measured on one recording. | Run `gw_floor_curve.py` on t30 and on a high-completeness capture from N1. |
| N9 | **A usable information rate.** The cited ~30 MB is a loss-deflated whole-capture average with no intercept decomposition, so it cannot be turned into a per-hour rate or a campaign figure (§6). | Redo the long-window analysis of `DELTA_WIRE_DESIGN.md` §1 as a prefix curve on a low-loss capture, so it yields a marginal and an intercept rather than an average. |

---

## 10. Residuals

| # | residual | owner |
|---|---|---|
| R1 | `documents/DECISIONS.md` states "**3–8 GB per campaign is a ceiling**"; `AGENTS.md` (2026-08-11) states ≤50 MB per recording; this lane measures 9.31 GB today and 4.77 GB at the measured floor. Three numbers are current intent at once, and today's product is outside all three at the top. | owner — one number wins, the others get deleted |
| R2 | The cache is ~178 GB uncompressed at campaign scale (1.70 MB/game-hour × 105,192), all written and read during finalize to produce a ~9 GB container. Not a Discord problem; possibly a shipping one. | engineering (finalize / spool) |
| R3 | L1 is a defect-shaped finding, not only a lever: the fold emits a derived projection beside the exact value it is derived from, under the same presence bit. `DELTA_WIRE_DESIGN.md` §9.1 measured the `number` token class at **8.24% of the DELTA content** and **72.4% of the sum of the four separately compressed class frames** (43,083,504 bytes — explicitly *not* the cache baseline, which §9.1 says isolated frames do not decompose). This lane names the producer those float spellings come from. Staffing it needs N2 first. | engineering, after N2 |
| R4 | `divisions` could not be drilled per member: it is an array section and this lane's instrument splits objects only. Its 17.0–23.2% of the file is attributed at section granularity, not per field. | this lane's instrument — extend with a keyed array walk if a `divisions` lever is funded |
| R5 | The recordings predate lanes GT/GU/GX (merged here at `caa4034c`). Their world-document effect is two added `null` members and no change to any producer this decomposition ranks — `git diff e09295b0 HEAD -- world_fold.rs` has zero "weather" hits. The decomposition was not re-taken on the current tip. | this lane — closed by N1's captures, which run on the current tip anyway |
| R6 | `stack.json` predates this document's lever numbering: its `joint_factor_L1_and_L3` key denotes the checkpoint lever with level 19, which this document calls L2+L3, and its D is name-scoped. It is retained only because its K@19 cells (15,858,547 on t40b, 21,350,270 on t49) reproduce `floor_stack.json`'s exactly, which is the determinism control. Nothing in this document cites it. | this lane — superseded by `floor_stack.json` and `floor_curve_*.json` |
| R7 | The leaf instrument's array handling is not general: `Attributor.value` skips array elements without looking for the prefix inside them, so a prefix path that ran through an array would be missed. It cannot affect this document — `map` and `_r2_weather_live` are both object members — but the instrument is only correct for object-rooted prefixes and says so. | this lane's instrument — generalise if a lever under an array is funded |

---

## 11. Re-running everything

```
cd extractor/native/runs/gw_size_floor/scripts
python gw_container_decompose.py <out> <run_dir>...          # 2.2
python gw_cache_sections.py      <out> <run_dir> --own-frame # 2.3  (asserts BASELINE, PARITY)
python gw_section_drill.py       <out> <run_dir> map         # 2.4
python gw_leaf_attrib.py         <out> <run_dir> map/_r2_weather_live 3   # 2.5 + the joint
python gw_derived_twins.py       <out> <run_dir>             # 2.5 twin census (asserts both)
python gw_levers.py              <out> <run_dir>...          # 5, 7 L2/L3, joint groups
python gw_floor_stack.py         <out> <run_dir>...          # whole-capture stack cells
python gw_floor_curve.py         <out> <run_dir> 12          # 7 THE FLOOR (transformed marginal)
python gw_groups.py              <out> <run_dir>...          # 2.3, 5 joint group ablations
python gw_survey.py              <out> <live_unlock_dir>     # 3 fleet
python gw_completeness.py        <out> <live_unlock_dir>     # 3 loss column
python gw_marginal_hour.py       <out> <run_dir>...          # 3 untransformed marginals
python gw_index_curve.py         <out> <run_dir>...          # 4 the H4IX marginal
python gw_tables.py              <out>                       # TABLES.txt
python gw_check.py               <out>                       # CHECK.txt: every figure, from the JSON
```

Every script is read-only with respect to the recordings and writes only into `<out>`.
`gw_check.py` selects every measured input from the JSON by name and fails if one is
absent — no measured rate is hard-coded in it, and the handful of cited constants are
declared at the top of that file and labelled as citations wherever they print.
