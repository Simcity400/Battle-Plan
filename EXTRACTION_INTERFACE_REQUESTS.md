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

---

## X-ENDEDCOMBAT (2026-07-16, extraction→replay): consume the new `ended_combats[]` field; render ended combats distinctly from active; key by (combat_id, generation)

**Context / D12 (zero data loss for land combat).** When domain 21 drops a land
combat from the active set the battle has ENDED, and its terminal domain-55
frame (phase `ended`, plus any per-side detail/attribution that still exists this
hour) must not be silently dropped. Independent review found the earlier approach
— emitting `lifecycle:"ended"` objects INTO the active `combats[]` array — is
unsafe on the replay side, because the typed `CombatRow` consumer discards the
`lifecycle` field and treats every element of `combats[]` as live. That would
make an hour with an empty active set plus one ended event report `available=true`
and display a finished battle as LIVE. Fabricating a live-looking row is worse
than dropping.

**What the extractor now emits (already shipped in `world_fold.rs`).** A new
top-level snapshot field, SEPARATE from the active-only `combats[]`:

```
"ended_combats": [
  {
    "combat_id": 42,
    "generation": 7,                       // TOP-LEVEL identity component
    "kind": "land",
    "lifecycle": "ended",
    "location_province": 1000,             // from the D55 terminal frame (or null)
    "attackers": null, "defenders": null,  // D21-sourced, unknown once ended — never zero-filled
    "combat_width": null, "terrain": null, "progress": null, "modifiers": null,
    "side_detail_availability": "present" | "unavailable",
    "land_combat_detail_events_live": { ... }   // present only when detail survives
  },
  ...
]
```

- `combats[]` now contains **only** combats D21 currently lists as active. No
  ended combat ever appears there.
- `ended_combats` is `null` when the active set is unknown this hour (fail
  closed), an empty array when known-with-no-ended-combats, and a populated array
  otherwise. Same absent-vs-null-vs-empty discipline as the rest of the schema.
- `side_detail_availability` badges whether per-side detail survived. In the
  **same-hour** end case the accumulated detail is `"present"`; in the
  **next-hour** end case only the terminal phase row exists, so per-side detail is
  genuinely GONE and is badged `"unavailable"` (never fabricated).

**Ask (replay layer, `replay_core`).**

1. **Consume `ended_combats[]`.** Add a typed reader for the new field. Ended
   combats MUST render distinctly from active combats: do **not** set
   `available=true` for them, and do **not** display them as live battles. They
   are terminal records for a completed battle.
2. **Key ended events by `(combat_id, generation)`.** Do NOT coalesce or index
   ended events (or reconcile them against active combats) by `combat_id` alone.
   `combat_id` is reused across generations — an active `(42, gen 8)` and an ended
   `(42, gen 7)` can coexist in one hour. They already live in separate fields;
   the ended element additionally carries its own top-level `generation` so the
   consumer can identify it collision-free. Keying by `combat_id` only would drop
   or merge one of the two.
3. Confirm the field name (`ended_combats`) and shape above are acceptable, or
   propose an alternative name/shape and extraction will accommodate it. Until
   the consumer lands, ended combats are still preserved in the snapshot (no data
   loss); they simply are not yet surfaced in the panel.
4. **UNION `ended_combats` across coalesced same-hour snapshots — do NOT
   first-wins or last-wins it (MAJOR zero-data-loss defect, found by independent
   review).** The replay-layer same-hour coalescer
   `diff_engine._merge_two_snapshots` (`replay_interface/replay/diff_engine.py`,
   ~line 1756, driven by `_coalesce_same_hour`/`_merge_snapshots` at ~1720) builds
   `merged = dict(a)` and then merges only a fixed set of keys (it unions
   `combats` by `combat_id`, merges `countries`/`states`/`divisions`/etc.), but it
   **never touches `ended_combats`** — so the merged snapshot silently retains only
   the FIRST snapshot's value. Reviewer's scenario, verbatim: *coalescing hour-100
   snapshots with `ended_combats:[]` followed by `ended_combats:[{combat_id:42,
   generation:7}]` returns `ended_combats:[]`* — a genuine terminal record is
   discarded, or a later empty/`null` overwrites an earlier populated set.

   This matters because, unlike active `combats[]`, an ended combat is a
   POINT-IN-TIME event: a combat that ends mid-hour appears in EXACTLY ONE
   sub-hour snapshot (the one whose fold first saw D21 drop it); a later same-hour
   snapshot no longer lists it in the active set AND no longer carries its terminal
   D55 frame, so its `ended_combats` for that combat is empty. Any plain supersede
   — whether first-wins or last-wins — can therefore drop a real end event.

   **Required semantics in `_merge_two_snapshots`:**
   - Add `ended_combats` to the merge as a **UNION keyed by
     `(combat_id, generation)`**, not by `combat_id` alone and not a scalar
     supersede. Merge `a["ended_combats"]` and `b["ended_combats"]` into the union
     of both by that composite key.
   - `null` means "active set unknown this hour" (fail closed) and must be
     preserved when BOTH sides are `null`; a populated/empty array on either side
     is real knowledge and must never be overwritten by the other side's
     `null`/`[]` (same `_is_empty` non-clobber discipline already applied to
     `designs`/`ideas_all`/etc.).
   - On a `(combat_id, generation)` key collision across the two snapshots, take
     the TERMINAL record — prefer the element whose `side_detail_availability` is
     `"present"` (same-hour detail) over `"unavailable"`; ties keep either (they
     describe the same terminal event). Never let a later `unavailable`/empty
     record blank out an earlier `present` one.

   **Contract asymmetry to encode:** active `combats[]` uses **last-observed**
   (union by `combat_id`, `b` wins — the latest active snapshot is authoritative);
   `ended_combats` uses **union by `(combat_id, generation)`** (an end event is
   observed once and must be retained). They are NOT the same merge rule.

   NOTE (extractor side is already correct — this is purely a replay-layer fix):
   the extractor emits EXACTLY ONE snapshot per game-hour, built once at
   `FRAME_END` (`world_fold::finish_hour` → `build_snapshot` → `build_combats`) and
   dedup'd by `emitted_hours`. `build_combats` iterates the whole-hour
   `combat_detail_events_this_hour` accumulator against the last-observed D21 active
   set, so a combat that ends mid-hour is detected at that single fold and the one
   emitted snapshot already carries the union of every ended combat for the hour
   (fold-gate:
   `ended_combats_are_unioned_across_all_sub_hour_folds_in_the_single_snapshot`).
   Multiple same-hour snapshots only arise on the replay side when reload/relaunch
   stitching concatenates snapshots that share an `hour_index` — which is exactly
   when `_merge_two_snapshots` runs and must union the field.
