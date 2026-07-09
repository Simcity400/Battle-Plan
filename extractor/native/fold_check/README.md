# fold_check — fold-equivalence gate (G1)

`fold_check` is the **lossless gate (G1)** for the extraction optimization plan
(`extractor/documents/EXTRACTION_OPTIMIZATION_PLAN.md`). Every later optimization
— bulk-read collapse, delta emission, the `--profile production` switch, codec
changes — must prove it did not change the *decoded* game state. This tool is the
one-command check: it decodes a collector compact stream, runs the **production
fold**, and emits a canonical per-game-hour hash. Two streams that fold to the
same hashes are certified fold-identical.

## Why it is trustworthy

There is exactly **one** decode+fold in the codebase, and this tool reuses it:
`recorder_host::world_fold::WorldFold`. `fold_check` only frames the 32-byte
`H4CR` record envelope (a stable wire contract in `compact_contract`) and hands
each record to `WorldFold::ingest_record` — the identical dispatch the recorder
host performs in `ingest_stream`. `DOMAIN_PERF` and read-probe records carry no
folded state and are ignored, exactly as the host ignores them. The hash is over
the shipped snapshot bytes, which are canonical by construction (the fold's
accumulators are all `BTreeMap`/`BTreeSet`, and `serde_json` — with no
`preserve_order` — writes object keys sorted), so identical input yields
byte-identical output.

If the tool used a second decoder or a second fold, it could not certify the
production fold; reusing the real one is the whole point.

## Usage

```
fold_check hash    <input> [--out report.json]
fold_check compare <a> <b> [--out report.json] [--sample N]
```

`<input>` is a `collector_stream.bin` **or** a `.battleplan` container. The kind
is auto-detected from the leading bytes (`H4CR` = raw stream, `PK\x03\x04` = zip
container); for a container the manifest-verified `collector_compact_stream`
entry is extracted and folded. The raw-stream path is streamed in 8 MiB chunks so
a multi-GB stream never resides fully in memory (only the fold's working world
state, plus one hour's snapshot at a time, is held). Streams that end mid-hour
are handled gracefully: complete hours are reported and the incomplete tail is
noted in `totals.truncated_tail_bytes`.

### `hash`

Prints a deterministic JSON report:

- `totals` — stream/folded byte counts, record counts by type (`perf_records_ignored`
  is counted but not folded), `hours_emitted`, and `truncated_tail_bytes`.
- `hour_range` — `[first, last]` folded `hour_index`.
- `per_domain` — DOMAIN_DATA record count per domain id (record-level accounting).
- `hours[]` — per game-hour: `hour_index`, `date`, `snapshot_hash` (SHA-256 of the
  folded snapshot), `snapshot_bytes`, and the per-domain data-record counts that
  fed the hour.
- `chain_hash` — a single SHA-256 over the ordered `(hour_index, snapshot_hash)`
  pairs. Its equality implies identical hour range **and** identical per-hour state.

### `compare`

Exits **0** iff the two inputs fold to the identical hour range with identical
per-hour hashes; exits **1** on any divergence. On divergence it reports the
first divergent hour and, for a same-hour hash mismatch, a bounded drill-down:
which top-level snapshot sections differ (with row counts), a sample of differing
leaf paths (`--sample N`, default 24), and the per-domain data-record-count deltas.

## Gate protocol (G1)

For an A/B/A shadow run (old collector vs optimized collector on the same live
game, same-state bracketed), fold both captures and:

```
fold_check compare old_run/collector_stream.bin new_run/collector_stream.bin
```

Exit 0 is the pass. This is what "no lever ships without G1" means in the plan.

## Tests

`cargo test -p fold_check` builds a small synthetic `H4CR` stream in memory
(a few domains, three game-hours, including a `frame_end` and a `perf` record
that must be ignored) and asserts stable hashing, PERF-record ignoring, that a
single mutated payload byte changes the hash of exactly its hour, and that a
truncated tail yields only the complete hours.
