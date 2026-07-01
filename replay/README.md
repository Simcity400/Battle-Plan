# replay/ — the replay data backbone

Turns captured world-state records into a queryable **replay**: an ordered series
of canonical **snapshots** plus a derived **event timeline**. Pure offline
Python, **stdlib only** — no game, no network, import-clean.

This package is the backbone every capture and replay layer writes into. Runtime
capture is moving to in-process collector frames; save-oracle output remains a
fixture and offline import path, not a live replay requirement.

## Files

| file | what |
|------|------|
| `SCHEMA.md` | **The contract.** Canonical snapshot + event schema, per-field source tags (C/M/M*/S/D). Read this first. |
| `schema.py` | Python mirror of SCHEMA.md: dataclasses, `FIELD_SOURCES`, `date_to_hour_index`, empty-skeleton builders, `validate_snapshot`. |
| `store.py` | Ingests fixture/capture output into canonical snapshots; the on-disk `SnapshotStore`. |
| `diff_engine.py` | Derives events from adjacent snapshots; merges a series into one chronological timeline. |
| `tests/` | `unittest` suite: schema/date math, ingestion vs the real oracle output, every event type, merge/dedupe. |
| `snapshots/` | The on-disk store (one subdir per game). |

## Data model in one breath

- A **snapshot** is the whole world at time `T` (countries, states, provinces,
  divisions, combats, map, `designs`, `order_of_battle`). Every container is always
  present; an unobtained leaf is `null` (no fabrication — DESIGN §1/§4).
- Per country the snapshot now also carries `commanders` (leader + generals/FM/admirals
  + advisors), `mios`/`scheduled_variants`/`mio_policies`, a deepened `intel` (agency /
  operatives / operations / decryption / peace conference), and an `order_of_battle`
  count rollup. Many rows are currently save-fixture-proven (**S**) and stay `null`
  until a parser or collector feeds them; runtime closure requires collector output
  or an explicit unavailable badge.
- **`hour_index`** (int, Clausewitz `((year*365+day_of_year)*24+hour)`, 365-day no-leap)
  is the canonical sort/diff key. `date` is the human `"Y.M.D.H"` string.
- **Events** are derived by diffing adjacent snapshots, sharpened by exact mod-log
  markers. See `SCHEMA.md §9` for the event record and the full type enum.
- **Source tags** (C collector / M legacy live-memory / M\* live-planned /
  S save fixture / D derived) are
  documented per field in `SCHEMA.md` and machine-readable via
  `schema.source_of("countries.GER.politics.stability")`.

## Store format

```
replay/snapshots/<game_id>/
    index.json                       # ordered manifest: [{hour_index,date,file,counts}]
    <012d hour_index>.snapshot.json  # one compact JSON snapshot per timestamp
    timeline.json                    # diff-engine event feed for the whole game
```

Filenames are the zero-padded `hour_index`, so a lexical directory sort is
chronological. `index.json` is the manifest replay readers iterate; each
snapshot file is self-contained and schema-valid (`store` refuses to write an
invalid one). `unknown` is used for a snapshot whose date couldn't be parsed.

## Usage

```python
from replay import store, diff_engine

# 1. Ingest one save-oracle output dir into a snapshot + write it to the store.
snap, path = store.ingest_oracle_dir("save_oracle/output", game_id="ger_1941")

# 2. Build the event timeline over a whole game's snapshots.
st = store.SnapshotStore()                 # default root = replay/snapshots
snaps = st.load_all_ordered("ger_1941")    # chronological
timeline = diff_engine.build_timeline(snaps, milestone_steps={"factory": 50,
                                                              "production": 1000})
st.write_timeline("ger_1941", timeline)
```

To ingest a fixture series, run step 1 once per save-oracle output you have
(each is one timestamp), then step 2 once. Runtime collector frames should be
adapted into the same snapshot shape and recorded under collector provenance.
Fields stay `null` until a real source feeds them — honest, not fabricated.

## Provenance & redaction

- Each snapshot records, once, which **domains** came from which **channel** in
  `provenance` — consumers reason about coverage from that, not per-value tags.
- `provenance.save.source_filename` is a **basename only**. The writer discards the
  oracle's `source_path_redacted` and never stores an absolute/user path anywhere in a
  snapshot (verified by a test that greps the serialized snapshot for personal-path
  needles).

## Tests

```
python -m unittest discover -s replay/tests -p "test_*.py"
```

Tests write only to a temp scratch dir (`%TEMP%/re_scratch/replay_*`), never into the
repo. The real-oracle tests skip automatically if `save_oracle/output` is absent.

## Cadence note (per DESIGN §5)

Event `confidence` is **per-domain**: a diff on a domain both snapshots captured from
the live collector can be `"exact"` at the collector's cadence; a fixture/imported
diff is `"interval"` (the change happened in `(a, b]`; `detail.interval_start_hour`
gives the earlier bound). The engine reads this from each snapshot's `provenance`,
so events sharpen when live collector layers feed the schema.
