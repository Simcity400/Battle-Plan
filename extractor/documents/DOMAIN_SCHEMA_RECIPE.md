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
  a keyed set.
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
- **The manifest-id assignment rule does not exist yet** (`WIRE_FORMAT.md` §13). Today's
  boundary mappers cast the source `u32` straight into `Value::ContentId`
  (`bench_real_dynamics.rs:114`). Do the same, and record it once in your open questions;
  do not invent a per-domain mapping.
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

(Multiple collections in one domain: `record_from_<collection>_row` each.) This is where the
step-2 semantics and the step-1 scales live. Derived fields (§6) apply at this boundary,
never inside the codec — which is exactly why the tick is a parameter.

The mapper is not the only public boundary. Also export a **key extractor**
(`key_from_row`) and a **payload-header splitter** (`split_payload`, carrying the fold's
*structural* checks: header length, exact row-count arithmetic with overflow-checked
multiplication, and the invariants YOUR authorities actually state — `changed_rows ==
row_count` is unconditional for full-replace domains (d63), mode-dependent for
baseline-plus-delta ones (d26's fold requires `<=` always and equality exactly when
`DELTA_DATA` is set — a clean full baseline may be unequal), and absent for pure
incremental payloads (asserting it on d10 was a round-1 finding); per-row semantic
checks such as unknown-flag rejection stay in `record_from_row`). Where an invariant is mode-dependent or header-relative, take the
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
   raw-bytes case; a schema that declares none of them says so in a test comment instead of
   inventing cases. Hand-building a `Vec<Value>` does not count: d20's current four-state
   test predates the mapper contract and would pass with a wrong mapper; yours must not.
4. Quantization actually saves bytes versus an all-`Exact` variant — **if any field is
   quantized**. A schema with no quantized field instead records in a comment why no
   continuous field is drawn-rather-than-interpolated (or has no continuous fields at all).
5. Shape/key test asserting the key fields and their kinds.
6. **Every field's offset, with scale as the special case**: a raw row with known bytes at
   each offset maps to the expected `Value` per field — for scaled fields with known
   fixed-point words (this is the test that catches the 32768-vs-100000 class), and for
   unscaled domains still field-by-field, because transposition is the same failure class
   (d62's `owner`/`actor` are adjacent in the row and separated in the key).

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
the stream passes.** Three rules the pilot's review rounds established:

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

**Large-state and shipping-incremental domains cannot hold per-hour snapshots** (d22, d23,
d24, d26 — up to ~24k rows/hour × 903 hours). Use a two-pass harness: validate everything
while retaining only record spans, then re-fold through the same code path while encoding.
Put the frame-level fold in `dNN.rs` as a public, tested `State` + `fold_payload` boundary —
the fourth public boundary, beside the step-7 trio, whenever one record is not one state.
Enumerate every OTHER writer of your domain's accumulator and prove it inert on your capture
or model it (the recorder prunes d26's accumulator from D20 deletion rows).

**Dependency ruling (already made — do not re-decide per domain):** the harness needs the
shipping framing contract, so `wire_format` carries `compact_contract` (a leaf crate) as a
dependency used only by measurement bins, documented as such in `Cargo.toml`. Never copy its
constants. For compression, `scripts/bench_real_dynamics.py` hardcodes the d20 arm name; use
`scripts/compress_arms.py <out_dir> <ship_arm_basename> [hours_per_span]` instead.

Report: raw and compressed sizes both arms, the ratio at BOTH the whole-stream and 48-h
block shapes, the wire arm's baseline/delta raw split, real churn per hour, and hours
verified. The split is not optional: five of six wave-1 domains showed the block ratio
measures *re-baseline policy*, not grammar — shipping lanes baseline once per campaign
while this arm re-baselines every 48 h, so a large low-churn table can lose at block shape
(d22: 42% whole-stream vs 709% at blocks) while its delta path wins by 6–50×. Block cadence
is the container's open question; report the split so the grammar comparison stays
recoverable. (`compress_arms.py` requires both arms to cover the same hour count — append
an offset for hours that emit nothing.)

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
    -c service_tier=priority -o "$OUT" - > "$AJ" 2> "$ERR" &
  CDX=$!
  # Persist the thread id AS SOON AS the first JSONL line lands — the
  # resumability law: a hard kill skips exit-time code, so extraction after
  # the command returns is too late. This survives cancellation mid-review.
  until [ -s "$AJ" ] || ! kill -0 $CDX 2>/dev/null; do sleep 1; done
  TID=$(python -c "import json,sys;print(json.loads(open(sys.argv[1]).readline()).get('thread_id',''))" "$AJ" 2>/dev/null)
  echo "$TID review-<domain>-round-<n>-attempt-$ATTEMPT" >> "$LEDGER"
  wait $CDX; CODE=$?
  # Usage-limit check comes FIRST — never trust exit status alone; an exit-0
  # reply carrying the signature is a dead thread, not a verdict.
  if grep -qiE "usage limit|out of credits|purchase more credits" "$ERR" "$OUT" 2>/dev/null; then
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
- [ ] Measurement with per-hour reader replay verified, numbers reported — or a stop-condition
      report for ordered/event/hybrid domains
- [ ] Codex APPROVE, with the thread id recorded
- [ ] Open questions listed — every `Exact`-because-unsure field, every Null/Unknown
      tie-break, every fold-vs-benchmark conflict, and the ContentId raw-cast note
