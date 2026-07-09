# Extraction ↔ Replay interface requests (repo root)

This file carries coordination requests **from the extraction lane to the replay
lane** (`replay_interface/native/replay_core`). It is the mirror of the
replay→extractor request logs under `replay_interface/` (R-SNOWMUD, R-COSMETIC,
…). Extraction owns `extractor/`; replay owns `replay_interface/`. Neither edits
the other's tree — proposals land here.

---

## X-DECODER (2026-07-09, extraction→replay): adopt the shared `replay_decoder` crate; then containers can drop snapshot JSONs

**Context / D1.** The Extraction Optimization Plan resolved D1: the compact typed
**stream** (`collector_compact_stream`) is the canonical full-detail record, and
production should migrate to *stream-only* containers "once a shared decoder crate
exists for `replay_core`". Today a `.battleplan` ships two representations of every
run — the raw stream (~28% of bytes, which `replay_core` never reads) **and** the
per-hour folded snapshot JSONs (~72%, the only thing `replay_core/source.rs`
consumes). That decoder crate now exists.

### What landed

A new library crate **`extractor/native/replay_decoder`** (workspace member). It is
a *thin facade* — it adds **no** new decode or fold logic. One decoder, one fold:

- H4CR record framing is the single primitive
  `compact_contract::parse_compact_record_header` (also used by the production
  streaming fold and the `fold_check` gate).
- The fold is the **existing production** `recorder_host::world_fold::WorldFold`
  (`ingest_record` / `take_finished`) — the same code the recorder host and
  `fold_check` drive.
- Containers are opened with the existing verified `snapshot::container::Container`
  reader (zstd method-93 + zip64 aware).

The world-state JSON bytes it yields are **byte-identical** to the snapshot entries
a container ships today (proven: `fold_check`-on-container, `fold_check`-on-raw-
stream, and the production `stream_fold` all fold the legacy fixture to the same
per-hour hash `a640d272…` / chain `9d17ca9d…`).

### API surface

```rust
// Open a source (auto-detects a raw collector_stream.bin vs a .battleplan).
let mut cursor = replay_decoder::open("recording.battleplan")?;   // or open_with / open_stream_file / open_container / open_reader

// Session metadata without folding a snapshot.
let meta = cursor.metadata();          // StreamMeta { schema_version, game_id, domains: Vec<DomainInfo{id,name}> }

// Pull one game-hour at a time, bounded memory (no previous hour retained).
while let Some(hour) = cursor.next_hour() {
    let hour = hour?;                   // WorldHour { hour_index, date, snapshot: Vec<u8>, domain_data_counts }
    let world = hour.to_json()?;        // the SAME canonical world-state JSON the snapshot entries carry
    // feed `world` (or hour.snapshot bytes) to the existing world.rs parser
}
// HourCursor also implements Iterator<Item = Result<WorldHour, DecodeError>>.

let stats = cursor.stats();            // DecodeStats: byte/record/hour totals, truncated tail, stop_reason
```

Random access (documented honest cost model):

```rust
let source = replay_decoder::Source::detect(path)?;
let index  = replay_decoder::build_index(&source, &opts)?;   // hour_index -> byte offset, built on one pass (or take_index() during a live pass)
let hour   = replay_decoder::seek_to_hour(&source, &index, target_hour, &opts)?;
// index.to_json()/from_json() persist it next to the container.
```

**Seek cost model (honest).** Delta-emitting domains (map_control, divisions,
d41/d44/d48/d51/d53, …) carry a baseline once, then only changed rows. To
reconstruct an arbitrary hour you must fold from a point where every such domain
still had its baseline ahead of it. The only point guaranteed to satisfy that for
*all* domains is the **start of the stream** (every delta tracker begins empty
there). So today exactly one hour — the first emitted — is a "cheap seek point"
(`index.cheap_seek_points()`), and an arbitrary hour's honest seek cost is "fold
from the first hour up to it" (`O(bytes)` from stream start). A future explicit
keyframe / global baseline-reset marker in the stream
(`COMPACT_STATUS_TRANSITION_BASELINE_RESET`) would add more cheap points; the
`replay_decoder` machinery already keys off that flag. For scrubbing, prefer a
single linear pass (bounded memory) over many independent seeks.

### Minimal consumption example for `replay_core`

`replay_core/source.rs` currently reads folded snapshot entries out of the
container. To consume the stream instead, replace the per-hour snapshot load with
a `replay_decoder` cursor and keep `world.rs` unchanged:

```rust
// Cargo.toml:  replay_decoder = { path = "../../../extractor/native/replay_decoder" }
let mut cursor = replay_decoder::open(container_path)?;
for hour in cursor.by_ref() {
    let hour = hour?;
    let world: serde_json::Value = hour.to_json()?;   // identical shape to today's snapshot entry
    self.ingest_hour(hour.hour_index, &hour.date, world);   // your existing world.rs path
}
```

`open_container(path, &OpenOptions::default())` uses the container's manifest
`game_id`; the yielded `hour.snapshot` bytes carry the same top-level shape
(`game_id`, `date`, `hour_index`, `schema_version`, `countries`, `states`,
`provinces`, `divisions`, `diplomacy_world`, `map`, `provenance`, …) that the
snapshot entries carry today — because they ARE the same fold output.

### Migration proposal

1. **`replay_core` adopts `replay_decoder`** for its per-hour world states (the
   example above), keeping `world.rs` parsing exactly as-is.
2. Once adopted and validated, the **production profile stops packing the folded
   snapshot JSONs** — containers become **stream-only** (drops ~72% of container
   bytes; the stream is already the canonical record). The stream + zstd-93 codec
   already ship.
3. The folded **snapshot entries move to the debug profile** only (still emitted
   for the dev/`fold_check` workflow and for any consumer not yet on the decoder).
   Legacy containers that still carry snapshots remain readable unchanged.

### What stays stable (contract guarantees)

- **The canonical world-state JSON schema is unchanged.** `replay_decoder` yields
  the identical fold output today's snapshots contain, so `replay_core`'s `world.rs`
  parsing keeps working on the yielded values with no schema change. `schema_version`
  is exposed via `cursor.metadata().schema_version` (currently `"1"`).
- **No-fabrication semantics are preserved** (absent key vs explicit null distinct;
  the fold is untouched).
- **One fold, forever.** `replay_decoder` drives the same `WorldFold` as the recorder
  host; there is no second decoder to drift. Any future stream/codec change that
  keeps the fold output identical is transparent to `replay_core`.

### Ask

- Confirm `replay_core` can depend on `extractor/native/replay_decoder` (a
  path dependency inside this repo) — or state the preferred coupling (vendored
  crate, published path, FFI) and extraction will accommodate it.
- Flag any world-state field `world.rs` needs that is NOT already in the folded
  snapshot (there should be none — it is the same output), so it can be added
  before the snapshot term is dropped from production containers.
