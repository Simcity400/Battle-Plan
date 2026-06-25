"""Compact per-game SERIES builder for the replay UI (READ-ONLY on the store).

Country Detail charts need a value-over-time series for every country across the
whole snapshot list. Each full snapshot is ~5 MB, so the UI must NOT fetch them
all just to draw a line chart (that recreates the inline-size problem at runtime,
Codex review #1). Instead this module walks the store once and emits a *compact*
``series.json`` (a few KB-to-MB) holding only the scalar/aggregate values the
charts plot, keyed by country TAG and indexed by snapshot.

It is pure stdlib and READ-ONLY: it reads ``replay/snapshots/<game_id>/`` via
``replay.store.SnapshotStore`` and writes only under ``replay_ui/output/``.

No fabrication: a value the snapshot did not capture stays ``null`` in the series
(distinct from a measured ``0``). The series also carries, per snapshot, the
``provenance`` block so the UI can show which domains/channel backed each frame,
and a coverage count per metric so the UI can say "N of M snapshots captured this".
"""
from __future__ import annotations

import json
import os
import sys
from typing import Any, Dict, List, Optional

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
if REPO not in sys.path:
    sys.path.insert(0, REPO)

try:  # store is in replay/ — read-only import
    from replay.store import SnapshotStore
except Exception:  # pragma: no cover
    SnapshotStore = None  # type: ignore


# Scalar metrics pulled per (snapshot, country). dotted accessor -> chart label.
# Kept deliberately small: these are what Country Detail plots over time. Anything
# heavier (stockpiles, full division lists) is loaded lazily from the snapshot the
# scrubber is parked on, not pre-serialized for every frame.
COUNTRY_METRICS: Dict[str, str] = {
    "economy.factories.civ": "Civilian factories",
    "economy.factories.mil": "Military factories",
    "economy.factories.dockyard": "Dockyards",
    "economy.factories.under_construction": "Factories under construction",
    "economy.fuel.current": "Fuel",
    "military.num_ships": "Ships",
    "military.division_count": "Divisions",
    "military.manpower.available": "Available manpower",
    "military.command_power": "Command power",
    "military.casualties.killed": "Casualties (killed)",
    "politics.stability": "Stability",
    "politics.war_support": "War support",
    "politics.political_power": "Political power",
    "research.tech_count": "Technologies researched",
    "focus.completed_count": "Focuses completed",
}


def _dig(obj: Any, dotted: str) -> Any:
    cur = obj
    for part in dotted.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(part)
        if cur is None:
            return None
    return cur


def _controlled_states(snap: Dict[str, Any]) -> Dict[str, int]:
    """Derived metric (D): how many states each TAG owns this snapshot."""
    counts: Dict[str, int] = {}
    for st in (snap.get("states") or {}).values():
        if isinstance(st, dict):
            owner = st.get("owner")
            if isinstance(owner, str):
                counts[owner] = counts.get(owner, 0) + 1
    return counts


def build_series(game_id: str, store_root: Optional[str] = None) -> Dict[str, Any]:
    """Walk the store for ``game_id`` and return the compact series dict.

    store_root lets the demo store (replay_ui/demo_snapshots) reuse this exact
    code path (Codex review #4 — configurable store root).
    """
    if SnapshotStore is None:
        raise RuntimeError("replay.store unavailable; cannot build series")
    store = SnapshotStore(store_root)
    index = store.list_snapshots(game_id)
    snaps = store.load_all_ordered(game_id)

    frames: List[Dict[str, Any]] = []
    # tag -> metric_key -> list aligned with frames (None where not captured)
    series: Dict[str, Dict[str, List[Any]]] = {}
    tags_seen: set = set()
    save_versions: set = set()
    metric_keys = list(COUNTRY_METRICS) + ["derived.controlled_states"]

    for fi, snap in enumerate(snaps):
        frames.append({
            "hour_index": snap.get("hour_index"),
            "date": snap.get("date"),
            "save_version": snap.get("save_version"),
            "provenance": snap.get("provenance"),
            "counts": {
                "countries": len(snap.get("countries") or {}),
                "states": len(snap.get("states") or {}),
                "provinces": len(snap.get("provinces") or {}),
                "divisions": len(snap.get("divisions") or []),
                "combats": len(snap.get("combats") or []),
            },
            "file": (index[fi]["file"] if fi < len(index) and "file" in index[fi]
                     else None),
        })
        if snap.get("save_version"):
            save_versions.add(snap["save_version"])
        controlled = _controlled_states(snap)
        countries = snap.get("countries") or {}
        # was state ownership captured this frame? (so a country with zero owned
        # states is a MEASURED 0, not "not captured" — Codex #4). True if any
        # channel listed an ownership domain, or simply if states are present.
        prov = snap.get("provenance") or {}
        dom = set((prov.get("save") or {}).get("domains") or []) | \
              set((prov.get("memory") or {}).get("domains") or [])
        ownership_captured = bool(snap.get("states")) or \
            ("state_ownership" in dom) or ("state_control" in dom)
        # union of tags in this frame plus the derived controlled-states set
        frame_tags = set(countries) | set(controlled)
        # all tags ever seen so far PLUS this frame's tags — we append exactly one
        # element per metric per frame for EVERY known tag, so the lists stay
        # strictly index-aligned with `frames`. A tag absent this frame appends
        # None (= not captured). A tag first seen now gets its earlier frames
        # back-padded to fi Nones (Codex review: a mid-series gap must NOT shift
        # later values onto the wrong frame).
        for tag in (tags_seen | frame_tags):
            c = countries.get(tag) or {}
            tser = series.setdefault(tag, {})
            for key in metric_keys:
                lst = tser.setdefault(key, [])
                if len(lst) < fi:                 # tag is new -> back-pad to fi
                    lst.extend([None] * (fi - len(lst)))
                if key == "derived.controlled_states":
                    # measured 0 when ownership was captured AND this tag is a real
                    # country present this frame; else not captured (None).
                    if tag in controlled:
                        val = controlled[tag]
                    elif ownership_captured and tag in countries:
                        val = 0
                    else:
                        val = None
                else:
                    val = _dig(c, key)
                    # an empty dict (e.g. casualties == {}) is not captured, not 0
                    if isinstance(val, dict):
                        val = None
                lst.append(val)
        tags_seen |= frame_tags

    # every list now has exactly nframes elements (asserted-by-construction).
    nframes = len(frames)
    for tag, tser in series.items():
        for key in metric_keys:
            lst = tser.setdefault(key, [])
            if len(lst) < nframes:                # a tag seen earlier, gone now
                lst.extend([None] * (nframes - len(lst)))

    # per-tag identity (human/major/name help) from the LAST frame that has it
    identity: Dict[str, Dict[str, Any]] = {}
    for snap in reversed(snaps):
        for tag, c in (snap.get("countries") or {}).items():
            if tag in identity:
                continue
            pol = (c or {}).get("politics") or {}
            identity[tag] = {
                "is_human": pol.get("is_human"),
                "is_major": pol.get("is_major"),
                "ruling_party": pol.get("ruling_party"),
            }
    for tag in tags_seen:
        identity.setdefault(tag, {"is_human": None, "is_major": None,
                                  "ruling_party": None})

    # coverage: for each tag/metric, how many frames were non-null
    coverage: Dict[str, Dict[str, int]] = {}
    for tag, tser in series.items():
        coverage[tag] = {k: sum(1 for v in vs if v is not None)
                         for k, vs in tser.items()}

    return {
        "game_id": game_id,
        "schema_version": "1",
        "n_frames": nframes,
        "frames": frames,
        "metrics": COUNTRY_METRICS,
        "derived_metrics": {"derived.controlled_states": "States owned"},
        "tags": sorted(tags_seen),
        "identity": identity,
        "series": series,
        "coverage": coverage,
        "save_versions": sorted(save_versions),
        "single_snapshot": nframes <= 1,
    }


def write_series(game_id: str, store_root: Optional[str] = None,
                 out_dir: Optional[str] = None) -> str:
    out_dir = out_dir or os.path.join(HERE, "output")
    os.makedirs(out_dir, exist_ok=True)
    data = build_series(game_id, store_root)
    path = os.path.join(out_dir, f"{game_id}.series.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, separators=(",", ":"))
    return path


__all__ = ["build_series", "write_series", "COUNTRY_METRICS"]
