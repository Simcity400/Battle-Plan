# replay/ — the replay data backbone

Turns raw **save-oracle** and (future) **memory-reader** outputs into a queryable
**replay**: an ordered series of canonical **snapshots** plus a derived **event
timeline**. Pure offline Python, **stdlib only** — no game, no network, import-clean.

This package is the backbone every other layer writes into (DESIGN §3, §9 build step
1–2 and 6). The map/time-lapse renderer and the dashboard read the snapshot store and
the timeline produced here.

## Files

| file | what |
|------|------|
| `SCHEMA.md` | **The contract.** Canonical snapshot + event schema, per-field source tags (M/M*/S/D). Read this first. |
| `schema.py` | Python mirror of SCHEMA.md: dataclasses, `FIELD_SOURCES`, `date_to_hour_index`, empty-skeleton builders, `validate_snapshot`. |
| `store.py` | Ingests save-oracle output (read-only) into canonical snapshots; the on-disk `SnapshotStore`. |
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
  count rollup — all save-sourced (**S**), `null` until their parser feeds them.
- **`hour_index`** (int, Clausewitz `((year*365+day_of_year)*24+hour)`, 365-day no-leap)
  is the canonical sort/diff key. `date` is the human `"Y.M.D.H"` string.
- **Events** are derived by diffing adjacent snapshots, sharpened by exact mod-log
  markers. See `SCHEMA.md §9` for the event record and the full type enum.
- **Source tags** (M live-memory / M\* memory-planned / S save / D derived) are
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
chronological. `index.json` is the manifest the dashboard/scrubber iterates; each
snapshot file is self-contained and schema-valid (`store` refuses to write an invalid
one). `unknown` is used for a snapshot whose date couldn't be parsed.

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

To ingest a series, run step 1 once per save-oracle output you have (each is one
timestamp), then step 2 once. The memory reader, when it produces live per-timestamp
reads, overlays into the same snapshot shape via `build_snapshot_from_oracle`'s
`memory_dir` hook (today the memory artifacts are offset *drafts* only, so M-source
fields stay `null` and `provenance.memory` is `null` — honest, not fabricated).

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
**memory** is `"exact"` (tick resolution); a save-backed diff is `"interval"` (the
change happened in `(a, b]`; `detail.interval_start_hour` gives the earlier bound). The
engine reads this from each snapshot's `provenance`, so events auto-sharpen from
interval to exact when the live memory layers come online — no engine change needed.
