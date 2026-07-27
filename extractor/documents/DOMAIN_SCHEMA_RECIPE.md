# Domain schema recipe

How to port one domain onto the wire format, as actually executed for d20 and then, as the
pilot of this recipe, for d63. `extractor/native/wire_format/src/d63.rs` is the
recipe-conformant worked example — its mapper, `key_from_row`/`split_payload` helpers, tests
and measurement bin (`bench_real_d63.rs`) implement this recipe's contracts as written.
`extractor/native/wire_format/src/d20.rs` is the original example; it predates several of
these contracts and its retrofit items are flagged where they matter below. `WIRE_FORMAT.md`
is the authority both conform to.

**This is deliberately narrower than `DOMAIN_CAPTURE_PATTERN.md` §9.** That checklist covers
acquisition, hooks, budgets, shedding and cadence for a collector re-architecture that does
not exist yet. Do not attempt it. Nothing in this recipe changes the collector, the recorder,
or any capture path.

## Scope

**In:** a schema, the boundary mapping from today's compact row to a `Record`, tests, and a
size measurement against a real capture stream.

**Out:** collector changes, hook work, acquisition redesign, cadence changes, replay-layer
work, anything in `world_fold.rs` other than *reading* it.

**Stop conditions — report instead of building:**

- **Ordered shapes.** `Shape::OrderedKeyedList` and `Shape::OrderedOccurrenceList` are full
  replacement and **are not implemented** — every encoder and decoder in `body.rs`
  hard-rejects a non-`KeyedSet` shape. If the domain needs one, stop and report; do not force
  a keyed set. **Read step 5's ordinal device first**: an ordered array whose order is a total
  function of the carried fields stays a keyed set — the position rides in the key or beside
  it — and only the arrays that fail that test fall through to this stop (d11's convoy
  priority reservations, d16's `timed_live`).
- **Player-authored free text.** Text no content manifest can ever resolve stops the
  collection carrying it: d11's conveyor `name` is a player-typed division name, so §5's
  symbol rule will never reach it whatever assignment rule lands (`d11.rs:224`–`:231`).
  Content-defined text is **not** this case and is not a stop — see step 3.
- **Event streams and hybrids.** If the domain's data is a happened-at-tick history rather
  than a state photographed on a cadence — or both at once, like d64's delayed-events chains
  (`WIRE_FORMAT.md` §9) — the event side has **no implementation** (`wire_format/src/lib.rs`
  says so explicitly: §8 gaps and §9 events have no code). Stop and report the domain's
  shape; do not force events into state records and do not invent an event encoding.

A stopped domain with a clear report of what it needs is a completed assignment.

## Authorities, in precedence order

1. **`recorder_host/src/world_fold.rs`, the domain's `fold_*` function.** This is the real
   authority for the payload grammar: row length, field offsets, per-field scale factors, and
   which flag gates which field. d20's entire row layout came from `fold_divisions`
   (`:18923`), not from prose. Two limits, both load-bearing:
   - The fold answers **null vs value**, never null vs absent vs unknown — it emits JSON
     `Null` uniformly whenever a gate is clear (see step 2).
   - Where the fold and the benchmark mapper disagree on a field's semantics, **the fold
     wins** and the disagreement goes in your open questions. Known instance: `fold_divisions`
     emits `move_progress` raw (`:19183`) while the benchmark divides it by `FIXED_SCALE`
     (`bench_real_dynamics.rs:136`); the benchmark is an example of *form*, not an authority
     on field semantics.
2. **`compact_contract`** — the domain's descriptor (id, wire name, `requires_data_record`)
   and any payload codec the domain already has under `compact_contract/src/`.
3. **`extractor/documents/domains/dNN_*.md`** — condensed knowledge, where it exists.
   **Roughly 20 production domains have no dossier** (`domains/UNDOCUMENTED_DOMAINS.md`);
   that is expected and is not a blocker. The fold function is sufficient.

If the fold function and a dossier disagree, the fold function wins and the disagreement is
reported as a finding.

## Steps

### 1. Read the fold and write down the grammar

Find `fn fold_<domain>` in `world_fold.rs`. Record, as a table: every field, its byte offset,
its width, **its scale constant**, and the flag bit(s) that gate it — one flag may gate
several fields (d63's single experience flag gates seven channels). Note the header length and
row length constants, and whether a legacy shorter row exists (d20: 120-byte row, 104-byte
legacy).

**Scales are per-field, not global.** `FIXED_SCALE = 100_000.0` is only the general case;
the fold also defines `FUEL_SCALE = 32768.0`, `TECH_PROGRESS_SCALE = 1000.0`,
`COUNTRY_CORE_EXPERIENCE_SCALE = 32768.0`, function-local scales like
`EFFICIENCY_SCALE = 10_000_000.0`, and fields emitted raw with no scale at all (d20's
`move_progress`). Take each field's scale from the code that folds *that field* — names do
not choose scales, code does: the field named `fuel` in d20 folds at `FIXED_SCALE`
(`world_fold.rs:19313`) while d10's fuel folds at `FUEL_SCALE` (`:9216`). Assuming the wrong
scale produces values wrong by ~3× (a 32768-scaled field read as 100_000), 100× (1000- or
10_000_000-scaled fields), or 100,000× (a raw-emitted field divided anyway) — and every
round-trip test still passes, because the error is in the mapping, not the codec.

**One array can carry two divisors — carry RAW values plus scale metadata, never a chosen
projection (d61).** `air_loss_stats_live`'s 24 channels share one value array with two display
divisors in a single constant table: channels 0 and 12 divide by 1,000, the other 22 by
100,000 (`lanes/air_loss_stats_live/src/lib.rs:102`–`:127`). The fold's own emitter carries
raw, divisor and display side by side (`world_fold.rs:9162`–`:9178`), the display being a pure
function of the other two — so the wire carries the raw `i64` with no divisor applied at the
boundary, the per-channel mapping pinned in exactly one place (`d61::channel_display_value`),
and the derived float recomputed downstream. Encoding a chosen projection instead would bake
the divisor choice into the bytes, and the hazard is measured: `our_buildings_bombed` peaks at
raw 56,600,000 — 56,600 buildings at the correct divisor and 566 at the wrong one, both
plausible-looking, and no round-trip test can catch the wrong choice because the error is in
the mapping, not the codec.

**Then write down how each hour's payload relates to the state before it.** Five shapes are
measured, and the difference decides your harness (step 9), your `split_payload` invariants
(step 7) and whether you need an accumulator at all:

1. **Full replace** — every record carries the whole table; the prior state is discarded
   (d63, d18).
2. **Baseline + delta** — a mode word says which (d26's `DELTA_DATA`, `FrameMode`); d26's fold
   requires `changed_rows <= row_count` always and equality exactly on a **delta**
   (`d26.rs:424`–`:434`), while a clean full baseline may be unequal.
3. **Pure incremental** — every payload is a patch, there is no baseline record and no mode
   word (d10; asserting `changed_rows == row_count` on it was a round-1 finding).
4. **Replace-or-sparse-after-first** — only the FIRST record replaces; every later record is a
   sparse patch even though nothing in it says so (d31, d34, d36 — three separate survey
   corrections). Reading records 2..n as snapshots silently drops every row they omit.
5. **Two-accumulator, mode-selected seed** — the fold keeps a *projected state* and a separate
   *delta baseline*, and the frame mode chooses which one the payload is applied to (d48).
   `fold_construction_buildings_state_damage` (`world_fold.rs:20252`) seeds its working state
   three ways — a DELTA frame from the delta baseline, a tolerated PARTIAL from the projected
   state with a sparse merge, a CLEAN frame from empty (`:20339`–`:20355`, with a comment
   naming the defect this exists to prevent) — and the two accumulators then advance on
   **different conditions**: the projected state takes every folded frame, the delta baseline
   only a clean or delta frame, never a partial (`:20618`–`:20625`). This is not shape 2 with
   extra steps: shape 2 has one accumulator, so "apply the delta to the prior state" is
   unambiguous, while here the two tables diverge the moment a tolerated partial lands.
   Your `fold_payload` boundary takes the frame mode **and** the partial flag as typed inputs
   (step 7's table), holds both accumulators and reproduces both advance conditions; seeding a
   delta from the projected state is the defect this shape exists to name — it silently
   re-introduces every phantom and reverted row a partial ever merged, for the rest of the
   stream. **Implement the seed even where the tolerance table makes one arm unreachable**:
   d48 has no `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry
   (`compact_contract/src/lib.rs:136` lists domains 26, 27, 28 and 54 only), so its partial
   arm is structurally dead today — it is implemented and tested anyway under step 7's
   mirror-the-fold rule, and the measurement harness asserts the tolerance is still absent so
   a later entry fails the run rather than silently invalidating the coverage caveat.
   Otherwise the next porter reads "unreachable" as "omit it".

**The cue for "do I need an accumulator" is the fold, not the payload:** *does the fold ever
leave prior state untouched?* — and a second question: **does the fold keep more than one
prior state?** If it does, which one a frame reads is part of the grammar (shape 5) and must
be a typed input, never something the harness infers.
Not payload size, not the presence of a `DELTA_DATA` bit — d13's
packer omits countries whose read failed and d34 is a tiny table that is nonetheless
sparse-after-first. A survey's row-count claim is guidance; the fold and your capture are the
evidence, and a falsified survey line is corrected in the survey document, not worked around
(d35's "constant 204 rows/hour" was `observed_variant_rows`, and d35 is pure incremental).

### 2. Decide each field's four-state semantics

This is where most of the judgment is. §3 of `WIRE_FORMAT.md` defines four states, and they
are **not** interchangeable. Its own words:

- **absent** — "not applicable, or unsupported by this schema" (presence bit `0`)
- **null** — "applicable, the relation genuinely points at nothing" (present, null bit set)
- **unknown** — "applicable, not captured" (present, unknown bit set)
- **zero** — "applicable, value is 0" — a real measured value, never collapsed into null

**The fold cannot tell you which of these to pick.** `fold_divisions` emits JSON `Null`
uniformly for *every* flag-gated field whose gate is clear — including
`move_target_province` (`world_fold.rs:19175`), which d20 encodes as **Absent**. The fold
decides null-vs-value only. The three-way choice is yours, made field by field:

1. Does the row's own state make the concept **meaningless**? Then **Absent**. d20:
   `move_target_province` is Absent when the row's `F_HAS_TARGET` flag is clear, and
   `move_progress` when `F_MOVING` is clear — a division with no movement has no destination
   and no progress; the value does not exist, rather than being unavailable.
2. Does the gate being clear mean the relation **genuinely points at nothing** — no owner
   assigned, no order linked? Then **Null**. d20: `owner` and `template_id` fold to Null on a
   clear gate.
3. Does the gate being clear mean the value existed but **was not captured** — unreadable,
   source too old, subsystem not sampled? Then **Unknown**, and the field declares
   `.unknown_capable()`. d20: the three A15 extension fields on a **legacy 104-byte row** are
   Unknown — the row physically cannot carry them. On a full 120-byte row with the flag
   clear, those same fields are Null.
4. Otherwise it is a real value, including a real **Zero**.

**A gate is not always a flag bit in the row, and finding the real one is the work.** Four
shapes are measured, and each changes the answer for a whole class of fields:

- **Kind-dependent gates (d28).** *Which* gates can even be clear depends on a key component.
  The collector builds a different row per command kind and hard-codes `None` for the concepts
  a kind does not have, so the same clear gate is **Absent** on the kind that never attempts
  the read and **Unknown** on the kind whose read failed — recoverable only because `kind` is
  key component 0 (`d28.rs:84`–`:101`). Decide per (field, kind), not per field.
- **Sibling-status-word gates (d28's `leader_trait_count`).** A field with no flag of its own
  may still have an empty state carried by a *neighbouring* word — and the obvious neighbour
  can be the wrong one. The count is written `unwrap_or(0)` with no gate bit, so a `None` is
  an indistinguishable zero; `leader_trait_status` looks like the disambiguator, but the
  collector returns it as `None` on every one of its six leader-read failure paths, so mapping
  `leader_trait_status == NONE` to Absent published 122 live army rows whose leader read had
  failed as "not applicable". The state is a function of `leader_status` — the same word
  `leader_id` and `leader_skill` use — swept over all 36 status pairs (`d28.rs:157`–`:175`,
  `:805`–`:819`). Look for the sibling before concluding "no gate, always a value", then check
  that the sibling you found means what its name says.
- **Frame-status-driven Unknown (d17).** A frame-level status bit can decide *which* empty
  state a clear row gate means. d17's `at_war_count`/`casualties` carry their value whenever
  their own row flag is set, whatever the frame says; when the flag is **clear**, the frame's
  `WAR_SEGMENT_ABSENT` bit is the only thing that separates "the segment found no relation" —
  a measured **Zero** — from "the segment did not capture" — **Unknown** (`d17.rs:464`–`:475`,
  `gated` at `:492`–`:500`). It is lossless only because the emitter packs a zero under a clear
  gate, which the mapper enforces rather than assumes (`d17.rs:437`–`:451`; step 7's divergence
  rule says why refusing the contradicting row is the honest move). This is a fidelity gain
  over the fold, which fabricates zeros for both cases — and it is why the frame's status word
  is a typed mapper input (step 7), not something the mapper re-derives.
- **Value-derived gates, and the Zero-that-might-be-Unknown (d24, d16, d32).** Where presence
  is carried by the value itself (`raw != 0` meaning present), a true zero is indistinguishable
  from an unset one. That ambiguity is collector-side: **record it, never paper over it.** d16
  reads three counters unconditionally and 58.50% of counter cells are zero, so mapping `0` to
  Unknown would assert "not captured" over the majority of real data — the counters ship as
  plain values with no capability anywhere, and the loss is reported upstream (`d16.rs:100`,
  `:789`). d32's `air_base_id` is the tie-break in the wild: value-gated, no flag, and the
  collector drops the whole wing on a failed read, so Null-vs-Unknown is undecidable →
  **Unknown**, recorded.

**Never gate against the fold; document the band instead (d36).** A mapper that refuses a row
the fold accepts is data loss of exactly the kind this recipe exists to prevent — d36's round-1
finding was a mapper rejecting `amount_raw` above the band where `Fixed(5)` reproduces the raw
integer. The fix maps every `i64` the fold maps and states the reproduction band as a measured
property: exact to `|raw| <= 2^51`, first failing inside `2^52`, pinned by a dense sweep
(`d36.rs:137`–`:139`, `:953`). A precision claim is a tested band, not a validity constraint.

**Known conflict in the worked example — the rule wins, not the example.** The fold's comment
for `supply_gain` and `disrupted_supply` reads "Null when the field was absent (unreadable)"
(`world_fold.rs:19224`–`19226`). "Unreadable" is the spec's **unknown**, yet d20 declares both
fields `.optional().nullable()` with no `.unknown_capable()` (`supply_gain` in the loop at
`d20.rs:103`–`116`, `disrupted_supply` at `:132`–`137`) and its mapper emits Null on a clear
gate. That is a recorded d20 retrofit item, not a precedent.
When your field's evidence says uncaptured or unreadable, choose Unknown even where d20's
current schema chose Null.

**When the fold is uninformative and the dossier is silent, read the collector's read path
for the domain in `collector/src/attach_log.rs` (read-only) before reaching for the
tie-break.** It is the only authority on *why* a gate is clear. d63's decision came from
exactly there: "Fail-closed per field: an unreadable/out-of-range value folds ABSENT (null),
never a fabricated 0" (`attach_log.rs:60210`) — the per-field reads return `None` only on
unreadable memory or a range rejection, which is §3's Unknown, not Null. Corroborate
empirically against your capture where you can (d63: across 396,417 real rows no gate is
ever clear — a clear gate is a read-failure path, not a fact about the world).

**A value the fold hard-enforces as a function of carried state is not a field.** d62's
witness-flags word is constant over every row its codec accepts; d24 omitted four derived
witness bits (two activity bits, two driver bits) and reinterpreted `MUD_PRESENT` as the
*gate* on `mud_active`. Enforce the invariant in the mapper, test the identity, and leave
it off the wire — re-encoding it would be a second copy of state the decoder must then keep
consistent. Distinct rule, same domain: mutually-exclusive flag bits (d24's named-weather
set) collapse into one `Enum` field — the information is kept, in a form where the overlap
the fold rejects becomes inexpressible. Do not confuse the two: exclusivity justifies an
Enum, never dropping the information.

**Tie-break: if the fold gives no evidence and you cannot decide between Null and Unknown,
choose Unknown.** Null asserts "definitively empty," and asserting it about a value that was
merely uncaptured is irreversible fidelity loss; Unknown never lies. Every tie-broken field
goes in your open questions.

Declare **exactly** what your mapper can emit: Absent needs `.optional()`, Null needs
`.nullable()`, Unknown needs `.unknown_capable()`; a field that can reach several states
declares several, and a field that cannot reach a state must not declare it — every declared
capability costs real bits (the presence bitmap is sized by optional fields, the null bitmap
by nullable present fields). These are different bytes on the wire, not a stylistic choice.
Known d20 retrofit here too: `move_target_province` declares `.optional().nullable()`
(`d20.rs:89`) but its mapper only ever emits value-or-Absent, and `owner`/`template_id`
declare `.optional()` their mapper never uses — declaration and mapper drifted apart before
this rule existed. Yours must match exactly; the states test in step 8 is what enforces it.

**Legacy rows are mapped, not rejected.** If your domain has a shorter legacy row, fields the
legacy row cannot carry are Unknown on that row. Unknown-on-legacy is d20's *declared intent*
(the `unknown_capable` rationale in `d20.rs`), not yet implemented behavior — the d20
benchmark detects legacy tables and asserts none are present
(`bench_real_dynamics.rs:216`–`:237`) because that one capture was all-extended. That is a
property of one stream, not a license to reject legacy input; your mapper handles both row
lengths.

### 3. Choose kinds

`Unsigned`, `Signed`, `Monotone`, `Continuous`, `Boolean`, `Enum`, `ContentId`. Notes:

- **`ContentId` is for references into mod-set content — ids whose *meaning* a content
  manifest must resolve, and which would point at something different under another mod set.**
  d20 uses it for exactly three fields: `owner` (country), `location_province` and
  `move_target_province` (provinces). **Every other id-like field is `Unsigned`**, and those
  come in two flavors that are easy to conflate: *runtime instance identities*, created
  during play — d20's `div_id`, `template_id`, `army_id`, `order_instance_id` — and *engine
  constants/discriminators*, stable everywhere but still not content — d20's `div_type`, the
  engine's identity-type code. (Id-like only: measured quantities stay `Continuous` and
  counters stay `Monotone` per the rules below.) When unsure, the deciding question is:
  **must a content manifest resolve this id for it to keep its meaning across mod sets?**
  Yes → `ContentId` (province 1234 is whatever province the mod set defines). No →
  `Unsigned`. The campaign test is a one-way corollary for instance ids — division 5 is a
  different division next campaign, so it is an instance — but it does **not** run in
  reverse: `div_type` is identical in every campaign *and every mod set*, no manifest
  resolves it, and it is `Unsigned`. Stability alone does not make an id content; being
  defined *by* mod content does.
  **A hashed content-defined token is still `ContentId`, and the test runs in both
  directions.** Where the id is a *hash* of content-defined text — d52's
  `occupation_law_token_id` is an FNV-1a-32 of the law's name (the fold labels the encoding
  `fnv1a32_name_hash`, `world_fold.rs:17014`–`:17022`) — the deciding question is whether a
  manifest MUST resolve the id for it to keep its meaning across mod sets, not whether one
  *can* today. Today none can: such a field's §13 retrofit needs a name-hash **pre-image**
  table that nothing currently ships (the collector has the string in hand and discards it
  after hashing, `lanes/occupation_resistance_compliance_live/collector_module.rs:546`–`:554`)
  — and classing the field `ContentId` is exactly what makes that gap findable by the
  retrofit sweep. The converse holds too: d95's `type_id`/`stable_id` are read verbatim from
  one packed engine word (`lanes/spotted_units_live/src/a729.rs:333`–`:338`), hash nothing on
  their read path, and stay `Unsigned` on d20's `div_type` precedent. The rule's scope is a
  content-defined string actually hashed on the field's read path — not any id that merely
  looks hash-like.
- **The manifest-id assignment rule does not exist yet** (`WIRE_FORMAT.md` §13). Where the
  source already carries a numeric id, today's boundary mappers cast the source `u32` straight
  into `Value::ContentId` (`bench_real_dynamics.rs:114`). Do the same, and record it once in
  your open questions; do not invent a per-domain mapping. Where the source carries **text**
  there is no raw id to cast — that is the next bullet.
- **Content-defined text is a `ContentId` resolved through a symbol table — not a stop
  condition.** §4 declares no string kind and `Kind::legal_as_key` admits only `Unsigned`,
  `Signed` and `ContentId` (`schema.rs:29`–`:31`), but §5 already answers the question for the
  text these payloads actually carry: "Country tags, equipment names, province and state ids
  are content-defined. The stream carries **manifest ids as varints, never text**"
  (`WIRE_FORMAT.md:346`–`:351`). A text field whose vocabulary comes from mod content —
  equipment and ship definitions, focus and ideology tokens, law/idea/advisor tokens, country
  and cosmetic tags — is therefore a `ContentId` carried through a **symbol resolver the
  caller supplies**, and because `ContentId` is a legal key kind the same rule turns a
  text-KEYED collection into a `ContentId`-keyed one with no new shape.
  **d29 is the worked example:** a `Symbols` table (`d29.rs:205`) taken as a typed mapper
  parameter (`d29.rs:604`–`:609`), holding a sorted-vocabulary stand-in labelled as a stand-in,
  with the two properties that make it safe stated exactly: nothing downstream computes from
  the id's value, and the measurement is invariant under any assignment over **this domain's
  own vocabulary** of fewer than 128 symbols, which is a one-byte varint either way
  (`d29.rs:190`–`:199`). §5's ids are file-global, so an assignment over the whole content set
  can still widen the varint — claim the invariance you measured, not more. Do the same, label
  the stand-in, list the field in the ContentId retrofit sweep, and note the vocabulary may
  only exist after pass 1 of the harness (step 9).
  **Two things this rule does not unblock**, and neither is an argument about kinds:
  player-authored free text (Scope), and a collection carrying a *second, independent*
  blocker. d17's war graph is the measured instance of the second: even with a symbol id, a
  keyed set cannot say "this collection's membership was not measured this hour", which is the
  state of **903 of 903 hours** in the only capture (`d17.rs:71`–`:77`). That is a gap scope
  narrower than §8's `(domain, part)` record and it has no implementation
  (`WIRE_FORMAT.md` §13). The collection stops on that blocker, not on the text.
- **`Monotone`** only when the value is a counter that *usually* rises. §4 makes monotone a
  hint, never a validity constraint — d20's `cumulative_losses` is `Monotone` even though
  transfers can decrease it.
- **`Enum`** requires `enum_card(n)`, and codes are rejected at or above the cardinality.

### 4. Choose quantization — the standing policy

Continuous fields only. **Default `Quant::Fixed(2)`** (d20's `BAR_PRECISION`) for
bar-like, percentage-like or gauge-like values that the viewer *draws* rather than
interpolates.

**Ship `Quant::Exact`** for any field that is: interpolated between samples (d20:
`move_progress`, from which the viewer derives on-map position), a position or coordinate, an
identifier-like number, or **anything you are unsure about**.

Do not guess. Every `Exact` chosen for the "unsure" reason goes in the deliverable's open
questions list with one line saying what evidence would settle it. Under-quantizing costs
bytes; over-quantizing silently corrupts what the replay draws, and only the second one is
unrecoverable.

**One more lossless case (d24): a decimal fixed-point source.** An int32 stored over a
100_000 denominator is exactly `Fixed(5)` — not a precision budget but the source's own
grid — and it also recovers the raw integer downstream consumers require, which `Exact`
would spend 8 bytes/value to do. A `/32768` source has NO lossless `Fixed(k)` under the
current codec: the identity `r/32768 = r·5¹⁵/10¹⁵` holds mathematically, but the codec
scales through binary64, and rounding there produces an adjacent integer that passes the
stability check and decodes to the neighbouring float (e.g. raw 295149 → …624 instead of
…625). So `Exact` is the only lossless form for power-of-two sources today — d10's choice
stands on that alone, beside its unjustified precision requirements — and its measured
+10.5% zstd penalty compares `Exact` f64 bytes against the shipping i64 words. Closing
that gap needs a codec-level exact path (an integer-domain `Fixed`, or a declared
power-of-two quantizer), a `WIRE_FORMAT.md` §13 item, not a schema-side choice.

### 5. Declare key and shape

`Shape::KeyedSet` when rows have a stable identity that persists across hours — only keyed
sets have a delta grammar. Ordered shapes and event streams are stop conditions (see Scope).

Key fields: concrete, totally ordered, present in every row, never
optional/nullable/unknown-capable/derived. d20's key is the `(div_id, div_type)` pair the
fold builds its uid from.

**An ordered array can still be a keyed set — the ordinal device.** The test is
**the array's order must be a total function of the carried fields on every payload the FOLD
accepts**, not merely on the payloads this capture happens to contain. Where it passes, the
order survives in one of two forms, and which one is decided by whether the rows already have
an identity:

- **A carried field already ranks the array** — nothing to add. d11's `production_licenses`
  v2 carries `visible_order` and the fold requires it to equal the item's position; v1's ids
  are sorted ascending by the fold, which is the keyed set's own order (`d11.rs:173`–`:176`).
- **The rows have a fold-enforced identity, but nothing ranks them** — keep the fold's key and
  carry the position as a **value field**. d11's production lines keep the fold's
  `(tag_db_id, kind, line_id)` key (`d11.rs:508`–`:516`) and carry `line_ordinal`, the dense
  base-table position within the row's `(tag_db_id, kind)` run, as a required concrete field
  (`d11.rs:537`–`:542`) — the only order witness on a frame that carries lines and no `PPR1`,
  which the fold accepts. Convoy variants do the same beside their fold-enforced key, and their
  redundancy with key order on this capture is reported rather than relied on, because "this
  stream happens to be sorted" is not a property of the payloads the fold accepts
  (`d11.rs:145`–`:161`, `:178`–`:186`).
- **The rows have no identity of their own, and the position IS one** — then the position goes
  **in the key**, generated at the boundary as a dense ordinal. d11's construction queue is
  keyed `(tag_db_id, construction_ordinal, line_id)`: the fold enforces no key there and pushes
  rows in wire order, so two rows can share every identity word; the ordinal is unique by
  construction, refuses nothing, and leads `line_id` so the keyed set's own order **is** the
  fold's array order (`d11.rs:162`–`:172`, `key_from_construction_row(row, ordinal)` at
  `:1941`). **Under a `Vec`-valued fold with no key this is the only faithful reading**: a
  `Vec` is a map from position to element, so the ordinal reproduces the container
  element-for-element. d39/d40's round-1 finding was a key built from the row's own
  `upgrade_index` word, which refused payloads the fold folds, collapsed duplicates, and would
  have silently re-sorted descending ranges (`variant_upgrades.rs:84`–`:86`, `:271`–`:277`).

**The question that decides a positional key is whether the position persists across hours as
the same thing** — step 5's own requirement, applied to the ordinal. A queue slot and a
variant's upgrade slot do. Two measured cases where it does not, and both stop:

- d11's `convoy_priority_reservations`. The order is a function of no carried field — 2,633
  consecutive samples are a *pure reorder* of an identical `{priority, count}` multiset — and
  the rows carry an identity of their own: `priority` is unique within a country in every
  country-hour of the capture, though the fold checks no uniqueness, so generality is unproven.
  Keying by source index would replace whatever identity they have with a positional one, which
  every reorder rewrites, and a positional index is not the "stable identity that persists
  across hours" a keyed set requires (`d11.rs:191`–`:216`). It is an ordered list either way —
  `OrderedKeyedList` if that uniqueness holds in general, `OrderedOccurrenceList` if duplicate
  priorities are legitimate — and both are unimplemented, so the stop is the same.
- d16's `timed_live`. Duplicates are legitimate, there is no key at all, and an entry's index
  shifts as neighbours expire, so no position persists — §7.3's `OrderedOccurrenceList`
  (`d16.rs:142`–`:153`), which §7.4 says carries no key.

Both shapes are unimplemented, so both are reported. Where the two readings are both
defensible, the choice is a recorded decision with its evidence, not a default: d32 read a
loadout line's identity as its variant rather than its position — the ordinal there is
transport, re-packed from emission order every frame, so a positional key would churn every row
of every hour — and refused the frames its keyed set cannot express instead, an input its own
producer cannot generate (`d32.rs:146`–`:170`; see step 7).

A generated ordinal is counted at exactly one place: the mapper always takes it as a parameter
(`record_from_line_row(collection, row, joins, ordinal, frame_tick)`, `d11.rs:1987`), and the
key extractor takes it too where it is a key component (`d11.rs:1941`). **Every caller must
count it the same way**, which is why it belongs to the shared boundary and is never a number a
harness re-derives.

### 6. Write `wire_format/src/dNN.rs`

Mirror `d20.rs`: a `DOMAIN` const, collection id consts, a `field` module of stable field-id
constants (append-only — §11 forbids reusing an id for a different field), and a `schema()`
returning the `Schema`. Comment every non-obvious choice with the `world_fold.rs:LINE` it came
from. Register the module in `lib.rs`.

### 7. Write the boundary mapping — location and signature are fixed

The mapper is part of `dNN.rs`, **public**, one function per collection:

```rust
/// One compact row of the `<collection>` table, as the collector emits it
/// today, onto its stored Record. `row` is the raw row slice (the caller has
/// already consumed the table header); length selects legacy vs extended
/// where a legacy form exists. `frame_tick` is §6's derived-field input
/// (stored = v + c * (frame_tick >> u)); a domain with no derived fields
/// takes it and ignores it, so every domain's mapper has the same shape.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> Result<Record, String>
```

(Multiple collections in one domain: `record_from_<collection>_row` each. A child row whose
identity includes its parent's takes the parent id as its first argument:
`key_from_<child>_row(parent_id, entry)` — d23's and d83's convention (`d23.rs:576`,
`d83.rs:627`), so a caller that already parsed the parent does not parse it twice.) This is
where the step-2 semantics and the step-1 scales live. Derived fields (§6) apply at this
boundary, never inside the codec — which is exactly why the tick is a parameter.

**The signature is those three fixed arguments PLUS whatever typed inputs the fold's semantics
require** — one named parameter for each input the fold reads that the row itself does not
carry. Take it; never re-derive it in a caller, and never hard-code one frame's shape into the
mapper. The measured family:

| Input | First instance | What the fold needs it for |
|---|---|---|
| `frame_tick: u64` | d63 | §6's derived-field input. A domain with no derived field names it and ignores it, so every mapper has the same shape |
| `mode: FrameMode` | d26 (`d26.rs:373`) | which structural invariant holds on this frame |
| `war_segment: WarSegment` | d17 (`d17.rs:208`, `:413`) | a frame status bit that decides one field's four-state answer for every row in the frame |
| `symbols: &Symbols` | d29 (`d29.rs:604`) | §5 symbol resolution for content-defined text (step 3) |
| `scales: RowScales` | d35 (`d35.rs:247`, `:446`) | header-carried divisors — a **value** input, not merely an invariant |
| `frame_game_hour: Option<i32>` | d13 (`d13.rs:753`) | the fold's **own clock**, which is not `frame_tick` |
| `ordinal: u32` | d11 (`d11.rs:1987`) | the array position — whether it rides in the key (`key_from_construction_row`, `d11.rs:1941`) or is carried as a value field beside a fold-enforced key, as `record_from_line_row` does; step 5 |
| `record_sequence: u64` | d70 (`d70.rs::fold_payload`; supplied by `apply_domain_data` at `world_fold.rs:4803`–`:4807`, `None` from the generic dispatcher at `:4737`) | the producer sequence the payload must be bound to — the fold refuses a payload whose stored sequence disagrees with the H4CR record that carried it (`:10268`–`:10274`), so a mapper derived from the generic dispatcher alone is wrong here |

**A fold-level clock is a separate input from §6's `frame_tick`.** d13's A29 acceptance
predicate reads `game_hour_raw`; conflating the two produced a round-1 finding. It gets its own
named parameter, and `None` reproduces the fold's own skip of the same-hour clause
(`d13.rs:745`–`:756`). d13 exposes both forms — the fixed three-argument mapper, and an
`_at_hour` variant carrying the fold's clock **in place of** the tick, since the tick is not
what that fold reads; the three-argument form refuses the rows whose semantics need the clock
rather than guessing one.

**`parse_header` (or `peek_header`) and `split_payload` are separate boundaries where a
non-folding exit sits between the checks the fold runs first and the body it reads last, and
the split is YOUR fold's call order — not a universal shape.** In d34, d35, d36 and d32 the
fold runs *some* header checks, returns without folding on a failure status — and, where its
grammar has one, on a delta-before-baseline — and only *then* checks the payload length and
reads rows. Which checks come first differs per fold and is not guessable: d34's are the
36-byte minimum and its `reserved != 0` rejection (`d34.rs:227`–`:240`), d35's are the minimum
and both scale words being non-zero, with no delta-before-baseline exit at all
(`d35.rs:352`–`:364`), d36's are the 48-byte minimum, four count invariants and the scale
assertion (`d36.rs:288`–`:340`), and d32's are the 44-byte minimum, four count relationships,
the fixed-scale word and a reserved word (`d32.rs:421`–`:488`). Enforcing an
invariant your fold checks *after* its early exit — the payload length being the usual one —
would refuse records the recorder accepts and folds nothing from, so the header read a caller
needs in order to *classify* a record must be reachable without paying for the body. Other
folds are shaped differently and get a different split or none: d83 decodes the whole payload
**before** consulting status, so a malformed payload is a refused stream whatever its status
(`bin/bench_real_d83.rs:54`–`:61`), and d10, d13 and d16 never read the status word at all
(`d13.rs:116`–`:124`, `d16.rs:189`–`:191`) — which is the same rule step 9 states for
exclusions, applied one layer earlier. d29 and d11 need the same split for a second reason:
their rows are variable-length, so the body walk needs header values before it can find a row
boundary at all. Structural checks that couple header words to each other belong in the same
place — the shape is not always `row_count` arithmetic (d83 couples authority, status, receipt
and snapshot-present, `d83.rs:414`–`:430`).

**Key-only collections are legal — and need one explicit check.** A collection that is pure key
membership with zero value fields is precedented (d28's `MEMBERS`, d39/d40's `variants`), but
its field loop iterates nothing, so a record built for the *wrong* collection would come out
silently empty instead of failing. Verify `collection.collection_id` before the loop
(`variant_upgrades.rs:633`).

**Mirror the fold's permissiveness — refuse only where mapping the input would publish
something false.** The default is d22's ruling: a port may not refuse what the fold accepts
merely because the emitter provably never produces it, and the mirrored permissiveness is
pinned by tests (this is the same rule as step 2's "never gate against the fold"). Mirroring is
right there because the fold's own permissive collapse is last-writer-wins, so the keyed set
reproduces its stored state exactly (`d32.rs:151`–`:156`).

Two measured shapes of legitimate divergence, each named by the loss it prevents:

- **The target shape cannot represent the input.** d32's fold stores a `Vec` in which two
  same-variant loadout lines coexist; the keyed set's `(owner, wing, generation, variant_id)`
  cannot express that, so mirroring would silently keep one line and drop the other — the
  data-loss defect itself. The alternatives were an unimplemented `OrderedOccurrenceList` (a
  stop) or an ordinal key component, which here is transport re-packed from emission order
  every frame and would churn every row of every hour. So `split_payload` **rejects** a frame
  carrying a duplicated variant inside one wing's range (`d32.rs:146`–`:170`).
- **The input contradicts the invariant your four-state mapping rests on.** d17's
  Unknown-on-a-clear-gate mapping is lossless only because the emitter packs a zero under a
  clear gate; a clear-gate row carrying a non-zero word would have its captured value replaced
  by Unknown, so the mapper refuses that row rather than mapping it (`d17.rs:437`–`:451`). d28
  refuses a non-zero `leader_trait_count` under a non-`EXACT` status for the same reason — the
  status-driven mapping would drop the count silently (`d28.rs:731`–`:741`).

The test is one question: **would mapping this input lose or falsify data?** If yes, refusing
is the honest option, and it is recorded with three things — the shape limit or invariant that
makes the mapping unable to carry the input, the producer evidence that the input is
unreachable in practice (d32: the collector rejects a duplicate variant,
`attach_log.rs:57114`–`:57115`, and that rejection drops the whole wing row, `:56844`), and a
test pinning the refusal. If no — if the input is merely surprising, or merely never emitted —
mirror the fold. "The emitter never does this" is the reason a divergence is unreachable, never
the reason it is right.

The mapper is not the only public boundary. Also export a **key extractor**
(`key_from_row`) and a **payload-header splitter** (`split_payload`, carrying the fold's
*structural* checks: header length, exact row-count arithmetic with overflow-checked
multiplication, and the invariants YOUR authorities actually state — `changed_rows ==
row_count` is unconditional for full-replace domains (d63), mode-dependent for
baseline-plus-delta ones (d26's fold requires `<=` always and equality exactly when
`DELTA_DATA` is set — a clean full baseline may be unequal), absent for pure
incremental payloads (asserting it on d10 was a round-1 finding), and **not an invariant at
all where the fold never reads the word** — d17, d34 and d16 all carry a `changed_rows`
header word their folds ignore (d16's is literally the same value as `country_count`), so it
is parsed and reported, never enforced. The test is whether YOUR fold reads it, not which
grammar class the domain is in; per-row semantic checks such as unknown-flag rejection stay in
`record_from_row`). Where an invariant is mode-dependent or header-relative, take the
mode or header value as a parameter (d26's `FrameMode`, d24's
`check_province_id(id, observed_province_rows)`) instead of hard-coding one frame shape.
The measurement bin, the tests, and any future
caller must share ONE derivation of the table header and the key. Re-derivation drifts: the
pilot's round-2 Codex finding was exactly that class — a bespoke re-implementation of the
H4CR stream framing that skipped version, header-length, sequence, record-flag and
closed-EOF checks the shared contract enforces. `d63.rs` is the pattern.

Known retrofit, not the pattern to copy: d20's mapper currently lives as a private
`ShipRow::to_record` inside `bench_real_dynamics.rs` — it predates this contract. Yours goes
in `dNN.rs` so the measurement bin, the tests, and any future caller share one mapping.

### 8. Tests — in `dNN.rs`'s test module

1. `schema().validate(grid_log2)` succeeds.
2. A full row round-trips **byte-identically** (encode → decode → re-encode).
3. **States through the mapper**: build raw row *bytes* and call your public
   `record_from_row`, asserting the exact `Value` per case — a real zero and full values
   always, plus a case for **each state the schema actually declares**: every capability the
   schema uses (`.optional()`, `.nullable()`, `.unknown_capable()`, legacy row) gets its
   raw-bytes case; a collection that declares none of them says so in a test comment instead
   of inventing cases (the record is per collection, so the "no capabilities" note is too).
   Hand-building a `Vec<Value>` does not count: d20's current four-state test predates the
   mapper contract and would pass with a wrong mapper; yours must not.
4. Quantization actually saves bytes versus an all-`Exact` variant — **if any field is
   quantized**. A schema with no quantized field instead records in a comment why no
   continuous field is drawn-rather-than-interpolated (or has no continuous fields at all).
5. Shape/key test asserting the key fields and their kinds.
6. **Every field's offset, with scale as the special case**: a raw row with known bytes at
   each offset maps to the expected `Value` per field — for scaled fields with known
   fixed-point words (this is the test that catches the 32768-vs-100000 class), and for
   unscaled domains still field-by-field, because transposition is the same failure class
   (d62's `owner`/`actor` are adjacent in the row and separated in the key). **The header is
   covered by the same test**: every word `peek_header`/`parse_header` reads gets its own
   known-bytes case (`d31.rs:1054`), because a transposed header word mis-slices every row
   behind it and no round-trip or replay assertion can see that — both arms agree on the
   wrong slice.

Run `cargo test -p wire_format` — the whole suite, not just yours.

### 9. Measure against a real capture

Captures are **centralized and serialized** — one game process, single-injection per PID, and
the crash knee sits between 20 and 40 scheduled domains
(`extractor/native/runs/hookdose/README.md`). **Do not launch the game.** You will be given a
`collector_stream.bin` containing your domain.

Generalize the measurement approach for your domain, calling **your** `record_from_row` /
`key_from_row` / `split_payload`: walk the H4CR framing, keep your domain's DATA records,
fold them into per-hour states, encode 48-hour blocks of BASELINE + stored-minimal DELTA, and
compare against the shipping records verbatim at zstd 3. `bench_real_d63.rs` is the
recipe-conformant example; `bench_real_dynamics.rs` is the original form. **The reader replay
is mandatory**: decode what you encoded against the reader's own reconstructed prior and
assert stored-equality with the fold, every hour. A size number without that assertion is not
a result.

**Validate the stream, not just the magic.** `bench_real_dynamics.rs` checks only record
magic and completeness, so it would silently measure a truncated or header-corrupt stream and
report a plausible number — the pilot copied that faithfully and Codex caught it (that bin
still carries the defect; it is form, not gospel). Your harness parses every record through
`compact_contract::parse_compact_record_header`, runs the whole stream through
`compact_contract::stream_validator::StatefulStreamValidator`, and additionally enforces the
hour-boundary and EOF rules of `recorder_host::stream_fold`'s `BoundaryTracker`/`eof_check`
— those two live in `recorder_host`, which `wire_format` must NOT depend on, so the harness
implements their documented subset itself, exactly as `bench_real_d63.rs` does (its module
header lists the rules with their `stream_fold.rs` line anchors). **Write no artifact until
the stream passes.** Rules the review rounds established:

- **Exclude every path on which YOUR fold returns without folding** — failure status is only
  the common one (use the fold's own predicate,
  `compact_status_failure_flags_after_expected_partial_tolerance(domain_id, …)`, never a
  hand-copied mask literal). d26 has four such paths (failure, delta-before-baseline,
  partial-before-baseline, rowless-partial); every domain whose fold implements a
  baseline-plus-delta grammar has delta-before-baseline (a pure incremental payload with no
  status guard, like d10, has neither). And derive the exclusion's POSITION from your fold's call order:
  d63 checks status before its row loop, d62 decodes rows first — copying the pilot's order
  into d62 was a Codex round-1 finding. A fold that never reads status at all (d10) forces
  an explicit, recorded refuse-or-exclude decision; never choose silently.
- Enforce hour uniqueness across ALL of your domain's records — including failure-status
  ones — BEFORE status exclusion, or a success+failure pair in one hour slips through.
- A stream cut exactly at a FRAME_END boundary is a well-formed shorter stream: accept it.
- **Reject an unassigned status bit BEFORE classifying the record.** The failure predicate and
  its partial-tolerance sibling are classifiers over the *assigned* v1 status bits
  (`compact_contract/src/status_flags_base.rs:21`–`:24`); an unassigned bit has no defined
  classification and is not evidence of a future informational status, so a record carrying one
  refuses the stream rather than falling through to "success" (`bin/bench_real_d16.rs:305`–
  `:330`, test `an_unassigned_status_bit_is_refused_not_classified` at `:1141`). This defect was
  present in every bench bin written before d16 and is flagged for a sweep — do not copy an
  earlier bin's status handling without checking it.
- **A third outcome exists beside "excluded hour" and "measured hour": a stream rejection.** A
  fold `Err` that says the payload is not this grammar — d83's non-failure status word forcing
  the unavailable authority, a magic or authority-hash mismatch, a trailing-byte payload
  (`d83.rs:82`–`:104`) — is not an hour to skip; it invalidates the run. Say which of the three
  each of your fold's non-folding paths is, in the harness, by name.
- **Where an independent authority predicate is STRICTER than the fold, measure what the fold
  produces — and document the split (d49).** Production's status-independent payload authority
  runs on every DOMAIN_DATA record and the harness runs it fail-closed; but d49's
  `validate_d49_promoted_location_region` (`domain_data_authority.rs:8692`, applied at
  `:8666`) is byte-for-byte the fold's own location/region refusal, and the two authorities
  disagree about what it means: the certifier makes it fatal to the stream, while the
  production fold path marks the domain-hour unknown, retains the accumulator and returns
  `Ok` (`world_fold.rs:20079`–`:20091`; the `Err` arm at `:20080`–`:20085` fires only in the
  separate validation-mode fold, not in production). The harness measures what the fold
  produces, so it takes the fold's classification — excluded hour — counts it by cause,
  applies every OTHER authority predicate fail-closed, and reports the certification
  consequence as a named deviation rather than absorbing it
  (`runs/batch3/d49_naval_combat_lifecycle/measurement.md`, where 72 of 514 hours are exactly
  this case).
- **Per-bin validation rules are NOT portable: re-derive every rule from YOUR domain's own
  authorities, never copy a sibling bin's (d65).** The same header word carries different
  rules per domain — `changed_rows` is `== row_count` where the fold reads the word (d63),
  only `<= row_count` where the payload authority bounds it and the fold never reads it (d65)
  — and the same defect class is enforced at different layers: d65's payload authority checks
  `(owner, program_id)` duplicates status-independently, so a duplicate inside a
  failure-status record refuses the stream, while d62 has no such authority check and the
  same input there is merely an excluded hour. Copying in either direction either refuses
  payloads the recorder folds or measures streams production refuses; pin each rule with its
  own test.
- **Before modelling an expected-partial tolerance branch, check the domain is actually in
  `COMPACT_EXPECTED_PARTIAL_TOLERANCES`** (`compact_contract/src/lib.rs:136` — domains 26,
  27, 28 and 54 only), **and check the branch's position.** `handle_compact_failure` applies
  the tolerance-adjusted failure mask, so for an UNLISTED domain
  `COMPACT_STATUS_PARTIAL_DATA` stays inside the mask it checks
  (`COMPACT_STATUS_FAILURE_MASK`, `lib.rs:110`–`:118`) and the gate returns on exactly that
  status — a tolerance branch positioned after it is dead code. d65's fold carries exactly
  that: the tolerance call at `world_fold.rs:12299` sits behind the failure gate at
  `:12292` and its predicate is constantly false for a domain the table does not list, so
  the branch is unreachable twice over. Do not implement a dead branch in the harness; pin
  the live classification with a test so a future tolerance entry breaks the test rather
  than silently changing what is measured. The flip side is step 1's shape-5 rule: where the
  FOLD's tolerance arm is live grammar (d48's mode-selected seed), implement it even though
  the table makes it unreachable today.
- **A gap forces a baseline (`WIRE_FORMAT.md` §1.5).** An excluded hour or non-contiguous
  step is a state-affecting gap, and the next state observation after one must be a
  `BASELINE` (`WIRE_FORMAT.md:187`–`:190`) — state may have moved and returned while nobody
  was looking, and where the two states happen to be equal a delta arm would emit nothing at
  all, silently asserting "unchanged" over an interval nobody measured. The encoder opens a
  fresh baseline after ANY such gap, mid-block if that is where it lands (§1.5 permits it),
  and the reader replay restarts its chain there. The rule decides the ENCODING only, not the
  churn accounting: d50's regression test pins that the churn counters are unaffected by a
  gap-forced baseline (`bench_real_d50.rs:1743` — "the gap rule decides the ENCODING, not
  what changed"), and both bins count a transition between every pair of consecutive measured
  states, across an excluded hour included (`bench_real_d50.rs:733`, `bench_real_d95.rs:983`)
  — churn stays "changed rows over measured-hour transitions" exactly as defined above.
  `bench_real_d50.rs` and `bench_real_d95.rs` enforce the baseline rule; bins written before
  them chose baselines from
  `hours_seen % BLOCK_HOURS` alone, which under-reports the wire arm's size and over-claims
  its replay verification on any capture with an excluded hour — a standing sweep item,
  flagged like d16's status-validity sweep.

**Large-state and shipping-incremental domains cannot hold per-hour snapshots** (d22, d23,
d24, d26 — up to ~24k rows/hour × 903 hours). Use a two-pass harness: validate everything
while retaining only record spans, then re-fold through the same code path while encoding.
**A domain with content-defined text needs the two-pass harness whatever its size**, because
the symbol vocabulary is what pass 1 builds and pass 2's mapping needs it (step 3). Pass 1
therefore cannot call the mapper at all: d29 runs the fold's own accumulation rules over a
**key-only mirror** of the accumulator, which is enough to collect the vocabulary and to
*prove* pass 2 can never need a symbol pass 1 did not see (`bin/bench_real_d29.rs:32`,
`:205`–`:212`, `:415`). Write that proof down; without it the stand-in table is an assumption.
Put the frame-level fold in `dNN.rs` as a public, tested `State` + `fold_payload` boundary —
the fourth public boundary, beside the step-7 trio — **whenever the harness would otherwise
re-derive a multi-table or nested walk**, not only when one record is not one state: d83 is one
record = one state and still needed it, because its two-level walk is grammar and grammar has
exactly one home.

**The `fold_payload` error contract is your fold's, not a fixed one.** d22's fold is
all-or-nothing; d35's mutates part of the accumulator before erroring. Mirror whichever yours
does and state it in the boundary's docs — a harness that assumes the wrong one silently
disagrees with the recorder on exactly the frames that fail.

Enumerate every OTHER writer of your domain's accumulator and choose one of three treatments,
explicitly: **model it** (the recorder prunes d26's accumulator from D20 deletion rows),
**prove it inert** on your capture, or — where the writer is another domain this assignment does
not port — **assert it inert per capture**, reading only the key columns the check needs and
failing the harness louder than the recorder would if it ever fires. d32 is the assert-inert
instance: d30's end-of-hour orphan prune genuinely removes rows, modelling it would mean porting
d30's payload grammar, and the harness instead fails on any drop (0 drops in 146 hours,
`d32.rs:179`–`:191`, `bin/bench_real_d32.rs:60`–`:68`).

**Dependency ruling (already made — do not re-decide per domain):** the harness needs the
shipping framing contract, so `wire_format` carries `compact_contract` (a leaf crate) as a
dependency used only by measurement bins, documented as such in `Cargo.toml`. Never copy its
constants. For compression, `scripts/bench_real_dynamics.py` hardcodes the d20 arm name; use
`scripts/compress_arms.py <out_dir> <ship_arm_basename> [hours_per_span]` instead.

**Where the library half must transcribe a shipping codec, the duplication is machine-checked,
never trusted (d67, d70).** The ruling above gives `compact_contract` to measurement bins
only, so a domain whose row grammar lives in a codec crate is transcribed by hand into
`dNN.rs` — d67's DMI2 (`compact_contract/src/dynamic_modifiers.rs`) and d70's SCH1
(`compact_contract/src/naval_history.rs`) are the first two. Three checks hold the copy to
the original: the harness dual-decodes **every** real capture record with both decoders and
requires identical frames — header scalars, authority, ordering and every row, ordinal
included (d67's `check_authority_payload`, 1,033/1,033 agree; d70's
`cross_check_against_codec`, all 750); a fuzz-equivalence test requires identical
accept/reject verdicts over random byte strings and single-bit mutations of a well-formed
payload (d67's `the_two_decoders_agree_on_every_fuzzed_payload`, 20,000 cases; d70 sweeps
every single-byte corruption and truncation of a good payload); and transcribed constants
are asserted equal to their shipping originals where the port declares that test — d67's
`every_transcribed_constant_equals_the_shipping_one` covers all 14 plus the calendar
predicate, while d70 asserts its restated `FAILURE_STATUS_MASK` against
`compact_contract`'s (`bench_real_d70.rs`, `assert_status_predicates_agree`) but has no
per-constant equality test over its other restated constants — an open sweep item, not a
license to skip the check in a new port. A drifted mirror fails the run, not the review.

Report: raw and compressed sizes both arms, the ratio at BOTH the whole-stream and 48-h
block shapes, the wire arm's baseline/delta raw split, real churn per hour, and hours
verified. The split is not optional: five of six wave-1 domains showed the block ratio
measures *re-baseline policy*, not grammar — shipping lanes baseline once per campaign
while this arm re-baselines every 48 h, so a large low-churn table can lose at block shape
(d22: 42% whole-stream vs 709% at blocks) while its delta path wins by 6–50×. Block cadence
is the container's open question; report the split so the grammar comparison stays
recoverable. (`compress_arms.py` requires both arms to cover the same hour count — append
an offset for hours that emit nothing.)

**Churn is reported twice, because one number cannot answer both questions.** Report it
**cadence-independent** — changed rows over *measured-hour transitions*, taken before the block
decision so a change landing on a block boundary still counts — and separately along the
**delta path**, changed rows and tombstones over the hours that actually emitted a delta
(`bin/bench_real_d16.rs:435`–`:441`, `:674`–`:695`; `bin/bench_real_d17.rs:431`). The
denominator matters: dividing an idle domain's churn by emitted-delta hours made d17 look ~900×
busier, a round-2 review finding.

**A stopped segment is accounted for in the shipping arm, never silently charged to yours.**
Where part of the domain is stop-reported, the shipping records still carry its bytes, so the
comparison charges your arm for data it was never allowed to encode. Two accountings, and both
are acceptable:

- **A byte split of the shipping arm** — say what each slice of the shipping stream is: the
  slice your wire arm replaces, the per-record framing no per-collection port can claim, and
  the stop-reported slice nobody claims. d16 reports its 136,422 shipping bytes as 52.9%
  country rows (claimed), 37.1% framing and 10.1% stopped `timed_live`
  (`runs/batch1/d16_decisions_live/measurement.md`).
- **A second, cut shipping arm** — where the segment occupies a cuttable span, subtract its
  bytes and report the wire ratio against both arms. d11 does this for the `PDQ1` trailer,
  which the fold requires to be the final one, so the cut is a suffix: `ship_d11_no_pdq1.bin`
  is each shipping record with that trailer removed and its payload length left as written — a
  size-accounting artifact, not a valid record, and labelled as one
  (`bin/bench_real_d11.rs:26`–`:34`, `:411`–`:415`, `:602`–`:611`).

Where the segment is interleaved rather than cuttable, the byte split is the only option:
cutting it would fabricate a stream the recorder never wrote. d11's `PCV1` reservation rows sit
inside their trailer at 8 bytes a row, so they stay charged to the shipping arm and are
reported as a named count, byte total and share (`bin/bench_real_d11.rs:591`–`:596`). Either
way the number is stated, not absorbed; and where you do cut an arm, say what the cut arm *is*
where you report it.

**State what the capture never exercised.** A coverage caveat is mandatory, not a courtesy, and
it names the path precisely. d83 encoded and replayed 19 baselines of 1,317 rows an hour, so
its value path is exercised — but every stockpile is empty on this world, so **no non-zero
amount was ever encoded and the delta path has zero coverage at all**: 0 changed rows over 884
non-baseline hours (`runs/batch1/d83_nuke_stockpile_live/bench_real_d83.txt`). Its
0.35%-of-shipping figure therefore measures a static table and nothing else. Name the path,
name the reason, name the capture that would exercise it.

## Two domains, one fold — the pair pattern

Where `world_fold.rs` dispatches the **same** fold function for two domain ids, they are one
grammar and must not become two implementations that drift. d39 `air_variant_upgrades` and d40
`stockpile_variant_upgrades` are the worked example: `world_fold.rs:4684` and `:4685` both
dispatch `fold_variant_upgrades` (`:15431`), and the only divergences inside it are the
accumulator it writes and the domain's name in its error strings.

- **The whole grammar lives in one shared module** (`variant_upgrades.rs`) parameterized by a
  plain `Domain` **descriptor value** — id and name — which nothing but the two domain modules
  constructs.
- **`dNN.rs` is an identity binding.** §11 assigns field ids per domain, so each domain needs
  its own `Schema` carrying its own domain value, and each needs the recipe's fixed boundary
  (`record_from_*_row`, `key_from_*_row`, `split_payload`, `fold_payload`) reachable at its own
  name. Every function forwards to the shared module; there is no logic in those files, by
  construction (`d39.rs:12`–`:17`).
- **The tests loop over both descriptors, and a `schema_hash` equality test machine-checks
  "byte-identical grammar."** §11's canonical bytes cover the collections and their fields, not
  the domain id, so `d39::schema().schema_hash() == d40::schema().schema_hash()` is a real
  assertion — and it is what fails if a later edit gives one domain a field the other lacks
  (`variant_upgrades.rs:244`–`:249`, `:980`–`:993`).
- **Measure each domain separately.** One grammar does not mean one dataset: the pair measured
  15.2% and 26.5% of shipping whole-stream on the same capture.

## Known-open spec items — and why they do not block this work

`WIRE_FORMAT.md` §13 states the spec "is not yet the fan-out contract" and lists what is
open. The items that touch this recipe:

- **Symbol/manifest-id assignment** is unspecified. Handled above: cast raw ids, record the
  open item. Schemas survive it outright (field ids are append-only), and mappers survive
  *structurally* — but **every mapper that emits `ContentId` will change** when §5's
  assignment rule lands: today's raw cast becomes a manifest-resolver lookup, and the
  signature gains a resolver input. That is a planned cross-domain retrofit; listing each
  `ContentId` field in your open questions is what lets the retrofit sweep find them.
  **ContentId *keys* are a heavier retrofit class than ContentId values:** key components
  sort and delta-encode by raw value (`body.rs`), so a manifest remap reorders keys and
  changes the byte layout of every baseline and key list, not just one field's value.
  d63's `tag_db_id` key is the first instance; flag any ContentId key separately.
  **A domain whose source carries content-defined text already pays part of that retrofit**:
  its mapper takes the symbol resolver today (step 3), so what the rule lands on is the
  stand-in table's assignment, not the mapper's shape. And **"this domain has no `ContentId`"
  is a sweep result, not a silence** — assert it, as d35 does with
  `no_field_is_a_content_id`, so the retrofit sweep can tell "checked, none" from "not
  checked".
- **Row-major vs columnar** was the headline §13 question; it is now **closed by decision**
  (2026-07-26): the row-major grammar this recipe targets is retained. The basis is
  evidence, not a direct columnar-delta comparison (no columnar delta grammar exists to
  measure): real changed-row churn measured 21–72%/hour, a quantity and regime the synthetic
  surface never measured (its few-percent figures were movement/combat occupancy, not row
  churn), which invalidates the synthetic margin without locating the ratio at real rates
  (the surface is non-monotone in churn); the one real columnar-vs-row measurement (a single
  baseline) favored row-major; and most real hours are deltas. Reopenable only by new
  evidence.
- **Golden cross-implementation vectors** do not exist yet. Your byte-identity and replay
  tests are per-domain evidence, not cross-implementation proof; that gate arrives later and
  is not yours to build.

The deliverable is schemas, mappings, tests and measurements — **not shipped bytes**. Byte
layout details may still move under §13's remaining items; field-id assignments and four-state
decisions will survive, which is why they, not the bytes, are the review surface.

## Codex review loop — the completion gate

Your work is not done when it compiles. It is done when Codex approves it.

Read `C:\Users\Example\.claude\CODEX_PROTOCOL.md`. Mode A, one-shot, fresh thread per call:

```
# ABSOLUTE path — agent shells reset cwd between calls; a relative ledger
# scatters silently, which violates the resumability law.
RUNDIR="extractor/native/runs/<batch>/<domain>"
LEDGER="$RUNDIR/codex_thread_ledger.txt"
VERDICT=""
for ATTEMPT in 1 2; do
  OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl); ERR=$(mktemp /tmp/cdx_XXXXXX.err)
  printf '%s' "$PROMPT" | codex exec --json -s danger-full-access -c approval_policy=never \
    --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=high \
    -o "$OUT" - > "$AJ" 2> "$ERR" &
  CDX=$!
  # Persist the thread id AS SOON AS the first JSONL line lands — the
  # resumability law: a hard kill skips exit-time code, so extraction after
  # the command returns is too late. This survives cancellation mid-review.
  until [ -s "$AJ" ] || ! kill -0 $CDX 2>/dev/null; do sleep 1; done
  TID=$(python -c "import json,sys;print(json.loads(open(sys.argv[1]).readline()).get('thread_id',''))" "$AJ" 2>/dev/null)
  echo "$TID review-<domain>-round-<n>-attempt-$ATTEMPT" >> "$LEDGER"
  wait $CDX; CODE=$?
  # Usage-limit check comes FIRST — never trust exit status alone; an exit-0
  # reply carrying the signature is a dead thread, not a verdict. The signature
  # can arrive ONLY on the --json stdout JSONL stream ($AJ) with stderr quiet
  # and the -o file empty — a lane that grepped only $ERR and $OUT retried
  # straight into a limit (d49 lane, 2026-07-27). Grep all three.
  if grep -qiE "usage limit|out of credits|purchase more credits" "$ERR" "$OUT" "$AJ" 2>/dev/null; then
    # NEVER retry into a usage limit. Queue durably; resume later with
    #   codex exec -s danger-full-access resume "$TID"  (re-check state; continue, don't redo)
    echo "USAGE_LIMIT $TID review-<domain>-round-<n>" >> "$RUNDIR/codex_resume_queue.txt"
    exit 2
  fi
  if [ $CODE -eq 0 ] && [ -s "$OUT" ]; then VERDICT="$OUT"; break; fi
  # otherwise: transient failure — loop runs one fresh-thread retry
done
if [ -z "$VERDICT" ]; then
  echo "codex review failed twice; a missing review is never an approval" >&2
  exit 1
fi
# read the verdict from "$VERDICT"
```

Read the reply from `$OUT`, not stdout, and only after the exit-0 and non-empty checks — an
empty `$OUT` with a nonzero exit is a failed review, not an approval. Never `--ephemeral`,
never `resume --last`.

**A 401 is NOT a usage limit** — it is a transient auth failure, invisible to
`codex login status` (which can report "Logged in" throughout), and the single fresh-thread
retry the loop already runs is the right response; it has been observed to succeed (d49 lane,
2026-07-27). Only the explicit usage-limit/credits signatures take the queue-and-resume path.
`C:\Users\Example\.claude\CODEX_PROTOCOL.md` carries the same two rules.

Prompt rules that make the loop converge:

- **Neutral engineering language.** Codex's content filter refuses "attack", "adversarial",
  "hostile", "forged". Say "review", "check", "find defects".
- **Define a closed defect class**, and list what is out of scope. The class: data loss or
  silently wrong data on input a real capture produces; a wrong scale factor; a panic
  reachable through a public entry point; an encoder emitting bytes its own decoder rejects;
  a four-state or kind decision that contradicts the fold function or §3. Out of scope: the
  collector, hooks, budgets, benchmark realism, unimplemented ordered shapes and events,
  cosmetics.
- State that **a clean APPROVE is the expected outcome** if nothing in the class exists.
- Codex has **no memory of this project**. Every prompt carries the full context: what the
  domain is, what the fold function says, what you built, and where the files are.
- **The count-pin rule.** Every review prompt pins the worktree path AND the tree's current
  test counts — total, per-crate, per-module — and a verdict whose observed counts do not
  match the tree is REJECTED, whatever it says: a review that validated another checkout is
  not a review. Two wave-5 wrong-checkout reviews were caught exactly this way. d61's round-1
  APPROVE reported 1,253 tests across 36 binaries — the shared checkout without the change;
  the worktree held 1,290, and the 37-test difference was exactly the two new files under
  review (`runs/batch3/d61_air_loss_stats/measurement.md`). d48's round-4 reviewer
  self-disclosed compiling the wrong checkout, discarded its own first run, and re-ran
  against the worktree with its own observed counts (1,309, 37 d48 tests) before approving.
  An APPROVE with mismatched counts is rejected and the round re-run pinned to the worktree.
- **Give the review lane a private `CARGO_TARGET_DIR`.** Parallel lanes building one
  workspace contend on the cargo lock, and a shared target dir wedged a wave-5 review for
  minutes; a per-lane target dir removes the contention and keeps the reviewer's observed
  test counts attributable to its own tree.

**Round-8 rule (standing owner directive, 2026-07-27).** If round 8 still does not
APPROVE, resume the round-8 thread (`codex exec -s danger-full-access resume "$TID" ...`,
same model/effort/tier flags) and instruct Codex to apply its remaining fixes ITSELF in
your worktree; re-run the full suite and your measurement, verify its diff scope with
`git status --porcelain` + `git diff`, record the outcome as "round-8 self-fix", and
complete. Do not loop past round 8.

Fix findings **by property, not by the reported input** — sweep the whole kind/value space the
finding implies, and carry the reviewer's own failing input into a regression test. Re-review
after every fix round. Loop until Codex returns APPROVE with no in-class findings.

## Definition of done

- [ ] `dNN.rs` schema, registered in `lib.rs`, every non-obvious choice cited to `world_fold.rs`
- [ ] Public `record_from_row` mapper(s) in `dNN.rs` implementing the four-state semantics
      and per-field scales
- [ ] The step-8 tests passing (a capability-conditional test that does not apply is recorded
      with its reason, not silently missing); full `cargo test -p wire_format` green, zero
      warnings
- [ ] Measurement with per-hour reader replay verified, numbers reported — churn both
      cadence-independent and delta-path, any stop-reported segment accounted for in the
      shipping arm (a byte split, or a second size-accounting arm where the segment is
      cuttable), and every path the capture never exercised named as a coverage caveat — or a
      stop-condition report for the collections that stop
- [ ] Codex APPROVE, with the thread id recorded
- [ ] Open questions listed — every `Exact`-because-unsure field, every Null/Unknown
      tie-break, every fold-vs-benchmark conflict, and the ContentId raw-cast note
