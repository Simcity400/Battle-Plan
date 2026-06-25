# live_capture — the live snapshot WRITER

Turns the cracked, **read-only** memory readers under `memory_reader/` into
**schema-conformant snapshots** (`replay/SCHEMA.md`) written **durably** to the
replay store, with **MP-continuity tagging** built in from the start. Pure stdlib
Python. The only memory access is through the existing read-only readers — this
package **never** edits or reads memory itself, and edits nothing outside
`live_capture/`.

```
from live_capture import CaptureLoop, DateInfo

loop = CaptureLoop(game_id="ger_live", store_root="replay/snapshots")

# single shot (opens + closes one hoi4.exe handle):
res = loop.capture_once()

# interval (one handle reused; holds a single-writer lock for the whole run):
loop.run(interval_s=30, max_captures=20)
```

## Pipeline (one capture)

```
build-gate ─▶ attach ─▶ resolve live layers ─▶ run reader plugs ─▶ assemble
   │            │         (tag table / bridge /     (registry)        snapshot
   │            │          province control)                             │
   │            │                                                        ▼
   └────────────┴───────────────────────────── continuity-tag ─▶ derive overlay
                                                      │                  │
                                                      ▼                  ▼
                                            DURABLE write ◀── (write IMMEDIATELY)
                                             snapshot + index + continuity state
```

Each snapshot is written the instant it is assembled+derived, so a crash loses at
most the in-flight read.

## The five pieces

| module | role |
|---|---|
| `process_identity.py` | Pins a live `hoi4.exe` to a stable, verifiable identity: PID + creation-time (PID-reuse safe) + exe sha256. `build_ok()` is the build gate; `instance_id` is the memory-only stable grouping key. |
| `registry.py` | The modular **reader-plug registry**. Each domain is an optional, import-guarded plug. A plug that doesn't import or raises at read time → its domain is **omitted** from provenance and its fields stay **null** (no fabrication). |
| `assembler.py` | Folds plug results into a `schema.empty_snapshot`, resolving every tag-db-id → TAG. Only **captured** domains are filled and listed in `provenance.memory.domains`. |
| `continuity.py` | Stamps each snapshot with `campaign_id` / `segment_id` / `capture_seq` / `hour_index` (+ break reason + confidence) in the `_capture` sidecar. |
| `durable_store.py` | Atomic (temp + fsync + `os.replace`) snapshot/index writes, the continuity-state file, and a single-writer lock. Wraps the read-only `replay.SnapshotStore`. |
| `capture.py` | The `CaptureLoop`: ties it all together; single-shot + interval. |

## No fabrication (the hard rule)

A domain is **captured** — folded into the snapshot and listed in
`provenance.memory.domains` — **iff** its plug ran to completion without raising.
Otherwise every field that domain would have filled stays at the schema null.

- `0` / `0.0` = **measured zero**. `null` = **not measured**. The assembler never
  coerces a missing read into `0`.
- A tag-db-id that won't resolve to a TAG becomes `null`, **never** a numeric id
  in a `*_tag` / `owner` / `controller` field (schema §0.4). The *record* is kept
  (a state with `controller=null`, a division with `owner=null`) — partial
  resolution is not fabricated absence.
- Plug reads are **transactional**: a plug builds its data in a local dict and the
  assembler only ever sees a result produced without raising. A plug that mutates
  then raises can't leak partial data while provenance claims the domain was
  omitted.

## Continuity tagging

The writer **emits** the tags a future cross-segment **stitch** needs; the stitch
itself is a follow-up. Tags land in `snap["_capture"]` (a non-schema sidecar — the
schema validator ignores unknown top-level keys, so the canonical contract is
intact).

| tag | meaning |
|---|---|
| `campaign_id` | grouping key. From a save `game_unique_seed` (`source="save_seed"`) or a user id (`"user"`) when supplied; else the **process-instance id** (`"process_instance"`). The fallback is **process-derived, not wallclock-derived**, so restarting the capture tool mid-game keeps the **same** campaign_id for the same ongoing game. |
| `campaign_id_source` | `save_seed` \| `user` \| `process_instance`. |
| `process_instance_id` | `sha256(exe_sha + pid + create_time)[:16]`; survives PID reuse. |
| `capture_run_id` | one capture-tool invocation. |
| `segment_id` | monotonic within a campaign. **New segment** on a process-instance change or a confirmed time **rollback**. |
| `capture_seq` | monotonic across the whole run; never resets. |
| `hour_index` | the in-game hour (mirrors the snapshot key). |
| `continuity_break` | `null` \| `process_changed` \| `campaign_changed` \| `time_rollback` \| `time_jump_forward` \| `unknown_time`. |
| `confidence` | `exact` \| `degraded`. |

**Segment-detection rules** (reviewed adversarially with Codex):

- A `None` (unknown) hour **never** starts a new segment and **never** overwrites
  the last-known hour. The next **known** hour is compared against the last
  **known** hour across the unknown gap, so a rollback hidden behind unknown reads
  is still caught.
- Same known hour as last = a re-read at the same tick (paused / fast loop), **not**
  a rollback.
- A known hour strictly **less** than the last known hour = **rollback** → new
  segment.
- A process-instance change = always a new segment.
- A **large forward jump** in the same process can't be told apart from a brand-new
  campaign loaded into the same process (no live `game_unique_seed`). We don't
  silently call it a new campaign: we start a new **segment**, keep the
  `campaign_id`, and flag `time_jump_forward` + `degraded`. The stitch resolves it.

## Date / hour sourcing (why it matters)

`date` → `hour_index` is the snapshot's **filename, sort key, and diff key**, so it
must be **trusted**. The live clock read costs ~150 s (full arena scan + watch
loop) and the weather package is in flux, so by default the loop accepts an
externally-supplied date **only if the caller marks it trusted** (e.g. it came from
the in-game mod log at the same tick — `DateInfo(date, trusted=True,
source="mod_log")`). A date from a stale save meta is **not** trusted: such a
capture is still recorded, but **non-canonically** (to `_noncanonical/`, with the
`hour_index` withheld so it can't poison the canonical replay). No trusted date →
unknown-time → non-canonical.

## Durability

Both the snapshot and the index are written **atomically** (temp file in the same
directory → `fsync` → `os.replace`). Unknown-hour captures all collide on
`unknown.snapshot.json` in the canonical store, so by default they go to a sibling
`_noncanonical/` directory (sequence-numbered) and are kept out of the index. A
single-writer **lock** (`O_CREAT | O_EXCL`) covers the whole run so two loops can't
clobber the continuity state.

## Store layout

```
<root>/<game_id>/s<seg>.<zeropad-hour>.snapshot.json  canonical snapshots (atomic)
<root>/<game_id>/index.json                            index (atomic; carries continuity tags)
<root>/<game_id>/_noncanonical/...                     unknown-hour / untrusted-date captures
<root>/_continuity/<game_id>.state.json                continuity state (atomic)
<root>/_continuity/<game_id>.lock                      single-writer lock
```

The canonical filename carries the **segment**, so a rollback that revisits an
already-captured hour in a new segment writes a **distinct** file instead of
overwriting the earlier segment (index uniqueness is file-based, not hour-based).
`game_id` is validated as a safe slug — a `..` / path-separator id is refused, so
nothing can escape the store root.

All of this is **gitignored** (`live_capture/.gitignore` + the repo root rules).

## Plugs (the default registry)

| plug → provenance domain | reader | notes |
|---|---|---|
| `state_control` | `live_layers/province_control.enumerate_state_controllers` | per-state controller TAG |
| `province_control` | `live_layers/province_control.ProvinceControl` | per-province controller TAG |
| `divisions` | `division_live/division_live_reader.DivisionLiveReader` | live divisions (org/str/move/combat/supply) |
| `country_scalars` | `live_layers/country_index_bridge` + `economy_offsets.draft.json` | politics scalars + num_ships |
| `supply` | `logistics_intel/supply_live.enumerate_supply` | per-province supply proxy (sidecar `_live_supply`) |
| `naval_air` | `naval_air/naval_air_reader` | sea-zone dominance + air superiority (sidecar `_live_naval_air`) |
| `land_combats` | `combat/combat_enum.CombatReader.enumerate_land_combats` | **solved, save-validated 13/13** land battles → promoted into the canonical typed `combats[]` (`kind="land"`) **and** kept raw in sidecar `_live_land_combats`. div-ids / province-ids are raw ints (not tags). |
| `combats` | `naval_air/naval_air_reader` *(naval/air combat — being cracked)* | **import-guarded**; raw crack in sidecar `_live_combats`, **not** folded into the typed `combats[]` (unstable) |
| `buildings` | `logistics_intel/buildings_save` | **SAVE-backstop, opt-in**: activates only when `ctx.extras["melt_path"]` / `["state_detail_path"]` is supplied; else **raises → omitted** (absent by default). Sidecar `_save_buildings`. Not a live read. |
| `intel` | `logistics_intel/intel_save` | **SAVE-backstop, opt-in**: activates only on `ctx.extras["melt_path"]`; else omitted. Sidecar `_save_intel`. Target indices stay **raw ints** — the index→TAG map is **unconfirmed**, never fabricated into TAGs. Not a live read. |

> **Land combat** (`combat_enum`) is the one combat domain that is solved and
> stable (13/13 vs the save for both attacker AND defender sets), so it is the only
> combat source promoted into the canonical typed `combats[]`. The **naval/air
> combat** methods are still being cracked; the `combats` plug import-guards them
> and keeps their (currently empty) output in `_live_combats` only.
> The **save-backstop** plugs (`buildings`, `intel`) parse a melted save on disk,
> not live memory — they are sidecars named `_save_*` (never `_live_*`, so a
> consumer can't mistake them for a fresh read) and are **absent by default**: a
> pure-live capture supplies no save path, so they raise and are omitted (no
> fabricated freshness).
> The **weather** reader (`memory_reader/weather/`) is also in flux: its only live
> entry point does a ~150 s clock scan and returns a *date*, not weather — so there
> is **no honest weather plug yet** and the weather domain is simply **absent**
> from the default registry (absent ≠ a fabricated "captured"). **No edits** to
> `memory_reader/` are needed here. Sidecar domains (`naval_air`, `supply`,
> `combats`) that have no first-class v1 schema home are surfaced under
> `snap["_live_*"]`, present only when captured.

## Tests (no game required)

```
python -m live_capture.tests.test_live_capture
```

STUB readers + a fake process identity prove: snapshot assembly conforms to the
schema; null-safety for absent/failed plugs (measured-zero ≠ null); the full
continuity decision tree (first/forward/rollback/pid-change/forward-jump/
unknown-time, campaign_id stable across a simulated tool restart); the derive
overlay runs and `_capture` survives it; durable store writes with index update +
temp cleanup; unknown-hour → `_noncanonical`; the lock is exclusive.

## Live end-to-end capture — DEFERRED (dry-run seam)

The **live** path (`CaptureLoop.capture_once` / `.run`) attaches to a running
`hoi4.exe`, resolves the live layers, runs the plugs, and writes durably. It is
**deferred here**: the live weather/clock workflow owns live process reads, so this
track does **not** attach to the game or run an arena scan. Everything is exercised
**offline** through the documented dry-run seam instead:

- **`CaptureLoop.capture_from_results(results, identity=..., date_info=...)`** — feed
  canned `DomainResult`s (real or stubbed reader output) straight into
  assemble → continuity-tag → derive → durable-write, with **no live process**.
  This is the seam the test suite drives end to end.
- To dry-run with a fake process (no game) but the real registry, construct
  `CaptureLoop(..., open_proc=lambda: <fake/None>, plugs=<stub plugs>)`; the loop
  imports readers only via the registry, so it stays importable with no game.

When a session is ready to capture live, call `capture_once`/`run` with a
`date_provider` that supplies a **trusted** in-game date (from the mod log at the
same tick); an untrusted/absent date is written non-canonically by design. The
save-backstop plugs additionally take `extras={"melt_path": ..., "state_detail_path": ...}`.

## Boundaries

Reads `replay/`, `derive/`, and `memory_reader/*` **read-only**; writes only under
`live_capture/` (this package) and the configured snapshots `store_root`. Never
touches `save_oracle/`, `dashboard/`, `mod/`, `replay_game/`, `replay_ui/`,
`game_assets/`. Never commits. Never writes a personal/user path into a tracked
file (basenames only, per schema §2 redaction).
```
