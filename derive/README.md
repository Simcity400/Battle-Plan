# derive — offline derived ("D"-source) replay layers

Pure offline Python. Computes the schema's **D-tagged** fields (derived by
join/compute, per `replay/SCHEMA.md` §0.1) that ship `null` until a compute pass
exists, from data already in a stored snapshot plus the static map assets. **No
game, no network, no fabrication** — a field stays `null` when its inputs are
absent.

```
from derive import derive_layers
out = derive_layers(snapshot)                       # single snapshot
out = derive_layers(curr, prev_snapshot=prev)       # series → gross_ic delta too
```

`derive_layers` returns a **deep copy** with the layers overlaid (it never
mutates its input, so a caller importing the live snapshot is unaffected).

## What it fills

| schema field | how it is derived | honesty boundary |
|---|---|---|
| `map.front_lines[]` | control-adjacency edges: raster 4-neighbour **land** adjacency where two adjacent provinces have **different** controllers. `{a,b,controller_a,controller_b}`, `a<b`, unordered-unique. | `null` if the snapshot carries no province control; `[]` if no front exists. |
| `map.controlled_industry_share` | `{TAG: fraction}` of total factory levels (`industrial_complex+arms_factory+dockyard`) attributed to each state's **controller** (not owner). Richer breakdown in `map.controlled_industry_detail` (`by_type`, `by_tag_factories`, totals). | `null` if no state has building data. Shares sum to ~1.0 minus any uncontrolled. |
| division `encircled` | same-controller **supply-graph estimate** (see below). | `True` land-pocket / `False` supplied / `None` uncertain-or-unknown. |
| `military.ships_by_hull` | per-country hull census from `order_of_battle.fleets→task_forces→ships` grouped by ship `definition` (already a hull token). `{hull:{count,tonnage}}`. | `tonnage` is always **`null`** — the save carries no displacement; we do **not** fabricate a nominal table. |
| `military.casualties` | per-tag war losses from `diplomacy_world.wars` (`a_casualties`→side a, `b_casualties`→side b, summed across the tag's wars). `{total,killed,captured,war_count}`. | `killed`/`captured` stay **`null`** — the save lumps them into `total`. |
| `military.manpower.deployed_strength_proxy` | sum of the tag's divisions' `strength`. Also sets `military.division_count`. | the real **`manpower.deployed`** (people) stays `null` — `strength` is a proxy, not a head-count. |
| `economy.gross_ic_by_equipment` | **two-snapshot** per-archetype monotone delta of `_production_lines[].produced`. | **`null`** on a single snapshot (one snapshot cannot give production-over-interval, and `produced` is a per-run accumulator, not a lifetime cumulative). |

The overlay also records a `_derived` block (per-layer fill counts + the model
notes) and appends a note to `provenance.notes`.

## Encirclement model (read this before trusting the layer)

A controlled land province belongs to a **connected component** of provinces
that are land-adjacent (raster) **and** share the same controller. A component
is **supplied** iff it contains a land **supply source** for that controller — a
province whose state the controller controls and which is the controller's
**capital** OR carries a **`supply_node` / `naval_supply_hub`** building
(province-folded to the state in the oracle).

- component supplied → `encircled = False`
- component unsupplied **but** touching a controlled **port** (`naval_base` /
  `naval_supply_hub`) → `encircled = None` — we cannot see convoys / sea routes /
  blockade, so we abstain rather than confidently say `False` *or* `True`.
- component unsupplied with **no** port → `encircled = True` (a land pocket).
- a division whose `location_province` has no controller, is absent from the
  adjacency graph, or whose snapshot lacks province control → `None`.

**Known model limits (documented, by design — not bugs):**

- **Same-controller only.** Allied / subject / military-access / through-port
  supply is *not* modelled (would need the diplomacy graph + access). The
  estimate can therefore **over-flag** a pocket that is in fact allied-supplied.
- **State-folded supply sources.** The oracle folds `province_buildings` to the
  state, so a supplied state credits *all* its controlled provinces, not the
  exact hub province.
- **Raster contact only.** Scripted straits / sea crossings (`adjacencies.csv`)
  and impassable-terrain borders are not represented; island-via-strait land
  looks non-adjacent.

Validated on the real 1946 save: every `encircled=True` component is small
(≤ 7 provinces) — contiguous major powers (SOV/USA/GER) show ~0 encircled, while
fragmented holders (BRM/ITA) show plausible pockets. No whole-nation false
positive.

## Province adjacency (`adjacency.py`)

Built from the authoritative `map_pipeline/assets/province_ids*.png` raster (each
pixel's RGB *is* the province id). 4-neighbour (right/down) **slice** compare —
never `np.roll`, so the map seam never fabricates a wrap adjacency. **Land only**:
sea/lake ids (non-zero in the raster but absent from `province_to_state.json`) are
dropped by land-set membership, not just `id == 0`. Result is cached on disk under
`derive/_cache/` (gitignored; the read-only map assets dir is never written to).
Half-res is the default (lossless at ×2 per the pipeline) and ~4× faster; pass
`half_res=False` for full-res.

## CLI

```
python -m derive.cli <game_id> [--root <snapshots_root>] [--assets <dir>]
                     [--half-res|--full-res] [--dry-run]
python -m derive.cli --file <snapshot.json> [--prev <prev.json>] [--out <path>]
```

- `<game_id>` recomputes every stored snapshot under
  `replay/snapshots/<game_id>/` (read via the replay `SnapshotStore`), pairing
  adjacent snapshots for the `gross_ic` delta, and writes them back
  (re-validated by the store). `--dry-run` prints the fill summary only.
- `--file` recomputes one loose snapshot (optionally with `--prev` for the
  delta), writing in place or to `--out`.

## Tests

```
python -m derive.tests.test_derive
```

A synthetic 4-province toy world exercises every layer and the no-mutation /
no-fabrication contracts; the real-snapshot and campaign-series tests assert the
validation invariants (shares ~1.0, ships totals == raw OOB, small encircled
components, tonnage/killed null, gross_ic null on a single snapshot). Real tests
self-skip if the snapshots are absent.

## Boundaries

This package reads `replay/`, `save_oracle/output`, `map_pipeline/assets`, and
`replay_render` **read-only**. It edits nothing outside `derive/`. The on-disk
adjacency cache is the only file it writes outside the package, and only next to
the static map assets.
