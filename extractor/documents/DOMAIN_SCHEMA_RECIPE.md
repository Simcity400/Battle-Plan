# Domain schema recipe

How to port one domain onto the wire format, as actually executed for d20. This is the
recipe; `extractor/native/wire_format/src/d20.rs` is the worked example, and
`WIRE_FORMAT.md` is the authority this conforms to.

**This is deliberately narrower than `DOMAIN_CAPTURE_PATTERN.md` §9.** That checklist covers
acquisition, hooks, budgets, shedding and cadence for a collector re-architecture that does
not exist yet. Do not attempt it. Nothing in this recipe changes the collector, the recorder,
or any capture path.

## Scope

**In:** a schema, the boundary mapping from today's compact row to a `Record`, tests, and a
size measurement against a real capture stream.

**Out:** collector changes, hook work, acquisition redesign, cadence changes, replay-layer
work, anything in `world_fold.rs` other than *reading* it.

## Authorities, in precedence order

1. **`recorder_host/src/world_fold.rs`, the domain's `fold_*` function.** This is the real
   authority for the payload grammar: row length, field offsets, presence flags, and the
   `has_*` gating that decides null vs absent. d20's entire row layout came from
   `fold_divisions` (`:18923`), not from prose.
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
its width, its scale (fixed-point fields divide by `FIXED_SCALE = 100_000.0`), and the flag
bit that gates it. Note the header length and row length constants, and whether a legacy
shorter row exists (d20: 120-byte row, 104-byte legacy).

### 2. Decide each field's four-state semantics

This is where most of the judgment is, and where d20's schema earns its keep. §3 of
`WIRE_FORMAT.md` distinguishes four states and they are **not** interchangeable:

- **Absent** — not applicable, or unsupported by this schema. d20: `move_target_province` on a
  stationary division.
- **Null** — applicable but not carried this row, i.e. the `has_*` flag is clear.
- **Unknown** — the schema supports it but this *source* could not carry it. d20: the A15
  extension fields on a legacy 104-byte row. Set `unknown_capable()` only for fields with a
  real "older row could not carry this" story; it costs a bitmap bit per capable field.
- **Zero** — a real measured value. Never collapse zero into null.

A field whose flag is clear is **Null**, not Absent, unless the fold itself treats the whole
concept as inapplicable in that state.

### 3. Choose kinds

`Unsigned`, `Signed`, `Monotone`, `Continuous`, `Boolean`, `Enum`, `ContentId`. Notes:

- **`ContentId`** for anything that references game content by id (province, country, entity
  ids) — not `Unsigned`. It is a distinct kind because content ids rebase across mod sets.
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

### 5. Declare key and shape

`Shape::KeyedSet` when rows have a stable identity that persists across hours — only keyed
sets have a delta grammar. Ordered shapes (`OrderedKeyedList`, `OccurrenceList`) are full
replacement and **are not implemented**; if the domain needs one, stop and report that rather
than forcing a keyed set.

Key fields: concrete, totally ordered, present in every row, never optional/nullable/derived.
d20's key is the `(div_id, div_type)` pair the fold builds its uid from.

### 6. Write `wire_format/src/dNN.rs`

Mirror `d20.rs`: a `DOMAIN` const, collection id consts, a `field` module of stable field-id
constants (append-only — §11 forbids reusing an id for a different field), and a `schema()`
returning the `Schema`. Comment every non-obvious choice with the `world_fold.rs:LINE` it came
from. Register the module in `lib.rs`.

### 7. Write the boundary mapping

A `to_record(&Collection) -> Record` that turns one compact row into a `Record`, applying the
step-2 semantics. Follow `ShipRow::to_record` in
`wire_format/src/bin/bench_real_dynamics.rs`. Derived fields (§6) apply at this boundary, never
inside the codec.

### 8. Tests — the four d20 tests, minimum

In `wire_format/src/dNN.rs`'s test module, mirroring `d20.rs`:

1. `schema().validate(grid_log2)` succeeds.
2. A full row round-trips **byte-identically** (encode → decode → re-encode).
3. All four states survive a round-trip on a realistic row, including a real zero.
4. Quantization actually saves bytes versus an all-`Exact` variant of the same schema.

Plus a shape/key test asserting the key fields and their kinds. Run
`cargo test -p wire_format` — the whole suite, not just yours.

### 9. Measure against a real capture

Captures are **centralized and serialized** — one game process, single-injection per PID, and
the crash knee sits between 20 and 40 scheduled domains
(`extractor/native/runs/hookdose/README.md`). **Do not launch the game.** You will be given a
`collector_stream.bin` containing your domain.

Generalize `bench_real_dynamics.rs`'s approach for your domain: walk the H4CR framing, keep
your domain's DATA records, fold them into per-hour states, encode 48-hour blocks of
BASELINE + stored-minimal DELTA, and compare against the shipping records verbatim at zstd 3.
**The reader replay is mandatory**: decode what you encoded against the reader's own
reconstructed prior and assert stored-equality with the fold, every hour. A size number
without that assertion is not a result.

Report: raw and compressed sizes both arms, the ratio, real churn per hour, and hours verified.

## Codex review loop — the completion gate

Your work is not done when it compiles. It is done when Codex approves it.

Read `C:\Users\Example\.claude\CODEX_PROTOCOL.md`. Mode A, one-shot, fresh thread per call:

```
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
printf '%s' "$PROMPT" | codex exec --json -s danger-full-access -c approval_policy=never \
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=high \
  -c service_tier=priority -o "$OUT" - > "$AJ"
```

Read the reply from `$OUT`, not stdout. Capture `thread_id` from the first JSONL line and keep
it. Never `--ephemeral`, never `resume --last`.

Prompt rules that make the loop converge:

- **Neutral engineering language.** Codex's content filter refuses "attack", "adversarial",
  "hostile", "forged". Say "review", "check", "find defects".
- **Define a closed defect class**, and list what is out of scope. The class: data loss or
  silently wrong data on input a real capture produces; a panic reachable through a public
  entry point; an encoder emitting bytes its own decoder rejects; a four-state semantic that
  contradicts the fold function. Out of scope: the collector, hooks, budgets, benchmark
  realism, unimplemented ordered shapes, cosmetics.
- State that **a clean APPROVE is the expected outcome** if nothing in the class exists.
- Codex has **no memory of this project**. Every prompt carries the full context: what the
  domain is, what the fold function says, what you built, and where the files are.

Fix findings **by property, not by the reported input** — sweep the whole kind/value space the
finding implies, and carry the reviewer's own failing input into a regression test. Re-review
after every fix round. Loop until Codex returns APPROVE with no in-class findings.

## Definition of done

- [ ] `dNN.rs` schema, registered in `lib.rs`, every non-obvious choice cited to `world_fold.rs`
- [ ] Boundary mapping implementing the four-state semantics
- [ ] Five tests passing; full `cargo test -p wire_format` green, zero warnings
- [ ] Measurement with per-hour reader replay verified, numbers reported
- [ ] Codex APPROVE, with the thread id recorded
- [ ] Open questions listed — every `Exact`-because-unsure field, and what would settle it
