# fold_check — fold-equivalence gate (G1)

`fold_check` is the **lossless gate (G1)** for extraction optimization. Every later optimization
— bulk-read collapse, delta emission, the `--profile production` switch, codec
changes — must prove it did not change the *decoded* game state. This tool is the
one-command check: it decodes a collector compact stream, runs the **production
fold**, and emits a canonical per-game-hour hash. Certification additionally
binds every hour to the caller-selected production roster; a stream's own
`FRAME_END emitted == scheduled` claim is never treated as its coverage authority.

## Why it is trustworthy

There is exactly **one** domain decode+fold in the codebase, and this tool reuses
it through `replay_decoder`: H4CR framing comes from `compact_contract`, logical
hour closure comes from the production `stream_fold::BoundaryTracker`, payload
exactness comes from the production structural validator, and state is applied
by `recorder_host::world_fold::WorldFold`. `DOMAIN_PERF` carries no folded state,
but its exact once-per-expected-domain presence and normalized status are part of
certification identity; unassigned diagnostic probes remain outside that identity. The hash is over
the shipped snapshot bytes, which are canonical by construction (the fold's
accumulators are all `BTreeMap`/`BTreeSet`, and `serde_json` — with no
`preserve_order` — writes object keys sorted), so identical input yields
byte-identical output.

If the tool used a second decoder or a second fold, it could not certify the
production fold; reusing the real one is the whole point.

## Usage

```
fold_check hash    <input> --expected-profile production [--out report.json]
fold_check compare <a> <b> --expected-profile production [--out report.json] [--sample N]
```

`--expected-profile production` is mandatory. Its roster comes from the shared
`compact_contract` production descriptor registry, independently of the bytes
being checked. For every closed hour, the FRAME_END emitted and scheduled counts
must both equal that roster's size, every expected domain must have exactly one
PERF record, every domain marked `requires_data_record` must have exactly one DATA
record, and no assigned domain outside the roster may appear.

`<input>` is a `collector_stream.bin` **or** a `.battleplan` container. The kind
is auto-detected from leading bytes. Raw H4CR, zstd-compressed H4CR, and
`PK\x03\x04` replay containers are accepted transparently; for a container the
manifest-verified `collector_compact_stream`
entry is extracted and folded. Standalone inputs are decoded in bounded chunks, so
even a legacy multi-GB raw stream never resides fully in memory (only the fold's working world
state, plus one hour's snapshot at a time, is held). The general decoder can
recover complete prefix hours for diagnostics, but this certification tool
fails closed for a framing stop, truncated/corrupt tail, malformed DOMAIN_DATA
payload, or non-closed hour boundary.

### `hash`

Prints a deterministic JSON report:

- `totals` — stream/folded byte counts, record counts by type (`perf_records_ignored`
  is counted but not folded), `malformed_payload_records`, `hours_emitted`, and
  `ends_at_frame_end`. A successful report has a clean certification verdict.
- `hour_range` — `[first, last]` folded `hour_index`.
- `per_domain` — DOMAIN_DATA record count per domain id (record-level accounting).
- `hours[]` — per game-hour: `hour_index`, `date`, `snapshot_hash` (SHA-256 of the
  folded snapshot), `snapshot_bytes`, and the per-domain data-record counts that
  fed the hour.
  Each domain also carries `data_payload_status_flags_or`: the status its
  DOMAIN_DATA payloads carried INSIDE THEMSELVES, which is the status the
  fold actually decides on. It is read through the single payload authority
  that already decodes every domain, so it cannot drift from the fold. A
  payload declaring a non-tolerated failure does not certify even when its
  DOMAIN_PERF record is clean — the fold withholds that domain's rows on
  that status, so the hour is not complete.
  A domain whose terminal outcome for the hour was a `DOMAIN_GAP` rather than a
  `DOMAIN_PERF` carries `gap_records`, `gap_reasons` and `gap_parts`: the absence
  is accounted, and an hour with one cannot certify under a roster that requires
  an extracted outcome for that domain.
- `refused_hours` — every game-hour that was assembled and REFUSED. A refused
  hour published nothing, so it has no snapshot and no hash; it is listed with
  the offending domains the collector named (raw status bits, the non-tolerated
  subset, and the reader's typed reason) and the terminal outcomes the refusal
  discarded. An attempted-and-refused hour is therefore distinguishable from an
  hour the stream never reached. A run that publishes NO hour does not certify:
  a fold-equivalence check over zero hours certifies nothing.
- `expected_roster` — the external profile name and exact domain requirements
  used for certification.
- `chain_hash` — a single SHA-256 over the ordered snapshot hashes plus normalized
  per-hour completeness. Its equality implies identical hour range, folded state,
  and observed record/status evidence under the same required roster.

### `compare`

Exits **0** iff the two inputs fold to the identical hour range with identical
per-hour hashes; exits **1** on any divergence. On divergence it reports the
first divergent hour and, for a same-hour hash mismatch, a bounded drill-down:
which top-level snapshot sections differ (with row counts), a sample of differing
leaf paths (`--sample N`, default 24), and the per-domain data-record-count deltas.

Two runs that publish identical hours but REFUSED different ones are not
identical: `compare` reports a `refused_hour_mismatch` divergence. Fold
equivalence over the published hours alone would lose the refusals, and a
refused hour is precisely the hour a consumer must not assume it has.

## Gate protocol (G1)

For an A/B/A shadow run (old collector vs optimized collector on the same live
game, same-state bracketed), fold both captures and:

```
fold_check compare old_run/collector_stream.bin new_run/collector_stream.bin \
  --expected-profile production
```

Exit 0 is the pass. This is what "no lever ships without G1" means in the plan.

## Tests

`cargo test -p fold_check` builds a small synthetic `H4CR` stream in memory
(a few domains, three game-hours, including a `frame_end` and a `perf` record)
and asserts stable hashing, folded-state independence from PERF, that a
single mutated payload byte changes the hash of exactly its hour, and that torn
tails, malformed payloads, open hours, obsolete boundary grammar, and forged
self-consistent FRAME_END counts with a missing expected domain fail closed.
