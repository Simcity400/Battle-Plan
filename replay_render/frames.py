"""Generic frame interface for the replay time-lapse + the schema adapter layer.

The renderer (``render.py``), time-lapse builder (``timelapse.py``) and scrubber
(``scrubber.py``) consume ONLY the ``Frame`` dataclass defined here. Adapting to
the canonical ``replay/SCHEMA.md`` (owned by another agent) is confined to the
adapter functions at the bottom of this file; nothing else moves.

A Frame is:
  * ``timestamp``       str label (Clausewitz ``"YYYY.M.D.H"``).
  * ``hour_index``      int|None canonical sort key (SCHEMA §0.3); used to order
                        a series correctly (numeric, not lexical on the string).
  * ``owner``           {state_id(int): owner_tag(str) | (r,g,b)}.
  * ``controller``      {state_id(int): controller_tag(str)} | None  (state-level).
  * ``control_by_prov`` {province_id(int): controller_tag(str)} | None
                        (PROVINCE-level control; SCHEMA §5/§8). When present it
                        OVERRIDES the state-level controller for that province,
                        so intra-state fronts and partial occupations render
                        correctly. ``None`` == fall back to the state controller.
  * ``divisions``       [DivisionMarker] | None.

State/province ids are ints; tags are uppercase strings matched against
``country_colors.json``. An owner value may instead be a literal ``(r,g,b)``
tuple (used by the synthetic demo).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple, Union

OwnerVal = Union[str, Tuple[int, int, int]]


# --------------------------------------------------------------------------
# hour_index: numeric canonical sort key. Use replay.schema.date_to_hour_index
# if importable (the canonical impl), else a vendored copy of the SAME formula
# (SCHEMA §0.3: ((year*365 + day_of_year)*24 + hour), 365-day years, no leap).
# Kept self-contained so replay_render has no hard cross-package import.
# --------------------------------------------------------------------------

# Vendored copy of replay.schema's constants, matched EXACTLY (0-based
# day-of-year, per-month day-max + hour validation, 365-day no-leap). Kept
# byte-for-byte equivalent so the fallback never produces a non-canonical key.
_DAYS_IN_MONTH = (31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)  # no leap
_DAYS_BEFORE_MONTH = (0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334)


def date_to_hour_index(date_str: Optional[str]) -> Optional[int]:
    """Clausewitz 'Y.M.D[.H]' -> hours-since-epoch (canonical sort key), or None.

    Prefers replay.schema.date_to_hour_index when importable; otherwise uses this
    vendored copy of the SAME formula (0-based day-of-year, validated, 365-day).
    """
    try:  # prefer the canonical implementation if the package is importable
        from replay.schema import date_to_hour_index as _canon  # type: ignore
        return _canon(date_str)
    except Exception:
        pass
    if not date_str or not isinstance(date_str, str):
        return None
    parts = date_str.strip().split(".")
    if len(parts) < 3:
        return None
    try:
        year = int(parts[0]); month = int(parts[1]); day = int(parts[2])
        hour = int(parts[3]) if len(parts) > 3 else 0
    except (ValueError, IndexError):
        return None
    if not (1 <= month <= 12):
        return None
    if not (1 <= day <= _DAYS_IN_MONTH[month - 1]):
        return None
    if not (0 <= hour <= 23):
        return None
    day_of_year_index = _DAYS_BEFORE_MONTH[month - 1] + (day - 1)  # 0-based
    return (year * 365 + day_of_year_index) * 24 + hour


@dataclass
class DivisionMarker:
    province_id: int
    owner: str              # TAG (for marker colour)
    strength: float = 1.0   # count or strength -> marker size
    label: str = ""         # optional (e.g. template role)


@dataclass
class Frame:
    timestamp: str
    owner: Dict[int, OwnerVal]
    controller: Optional[Dict[int, str]] = None
    control_by_prov: Optional[Dict[int, str]] = None
    divisions: Optional[List[DivisionMarker]] = None
    hour_index: Optional[int] = None
    meta: dict = field(default_factory=dict)

    def __post_init__(self):
        if self.hour_index is None:
            self.hour_index = date_to_hour_index(self.timestamp)

    def state_ids(self):
        return self.owner.keys()


# --------------------------------------------------------------------------
# Adapters: external data -> Frame. These are the ONLY places that know a
# concrete on-disk shape.
# --------------------------------------------------------------------------

def _div_strength(d: dict) -> Optional[float]:
    """Strength for marker sizing. SCHEMA §0.2: null != measured zero.

    Returns None when strength is genuinely unmeasured (so the caller can skip
    the marker or size it as a count), and the float value otherwise (including
    a real 0.0). Never fabricates 1.0 from a null.
    """
    s = d.get("strength", None)
    if s is None:
        return None
    try:
        return float(s)
    except (TypeError, ValueError):
        return None


def frame_from_ownership_dict(timestamp: str,
                              ownership: Dict,
                              divisions: Optional[List[dict]] = None,
                              control_by_prov: Optional[Dict] = None,
                              hour_index: Optional[int] = None) -> Frame:
    """Adapt the save_oracle ``state_ownership.json`` shape -> Frame.

    ``ownership`` = {state_id: {"owner": TAG, "controller": TAG}} (the shape
    ``map_view/app.js`` consumes). ``divisions`` = the ``divisions`` list from
    ``divisions.json``. ``control_by_prov`` = optional {province_id: TAG}
    (``province_control.json``) for province-level fronts.
    """
    owner: Dict[int, OwnerVal] = {}
    controller: Dict[int, str] = {}
    for sid_str, rec in ownership.items():
        sid = int(sid_str)
        own = rec.get("owner")
        ctrl = rec.get("controller") or own
        if own:
            owner[sid] = own
        if ctrl:
            controller[sid] = ctrl
    marks: Optional[List[DivisionMarker]] = None
    if divisions:
        marks = []
        for d in divisions:
            prov = d.get("location_province")
            if not prov:
                continue
            s = _div_strength(d)
            # unmeasured strength -> size as a unit count (1.0), but never claim
            # a fabricated magnitude; measured 0.0 is honoured.
            marks.append(DivisionMarker(
                province_id=int(prov),
                owner=d.get("owner", "") or "",
                strength=(s if s is not None else 1.0),
                label=str(d.get("template_role", "") or ""),
            ))
    cbp: Optional[Dict[int, str]] = None
    if control_by_prov:
        cbp = {int(p): t for p, t in control_by_prov.items() if t}
    return Frame(timestamp=timestamp, owner=owner,
                 controller=controller or None, control_by_prov=cbp,
                 divisions=marks, hour_index=hour_index)


def frame_from_replay_snapshot(snapshot: dict) -> Frame:
    """Adapt one canonical ``replay/SCHEMA.md`` snapshot -> Frame.

    Conforms to SCHEMA v1:
      * ``date`` (§0.3) + ``hour_index`` (canonical numeric sort key).
      * owner: ``map.owner_by_state`` if present (the raster-convenience mirror,
        §8), else built from ``states[*].owner`` (§4).
      * state controller: ``states[*].controller`` (§4).
      * province control: ``map.control_by_province`` (§8) if present, else
        ``provinces[*].controller`` (§5) -> drives intra-state fronts.
      * divisions: ``divisions[]`` (§6) with ``location_province``/``owner``/
        ``strength``/``template_role`` (strength-null honoured, §0.2).

    Falls back gracefully to the older flat shapes (``ownership``/``states`` with
    nested ``{owner,controller}``) so the pre-schema oracle output still works.
    """
    date = (snapshot.get("date") or snapshot.get("timestamp")
            or snapshot.get("save_date") or "?")
    hour = snapshot.get("hour_index")

    states = snapshot.get("states") or {}
    smap = snapshot.get("map") or {}

    # ---- owner ----
    owner: Dict[int, OwnerVal] = {}
    obs = smap.get("owner_by_state")
    if isinstance(obs, dict) and obs:
        for sid, tag in obs.items():
            if tag:
                owner[int(sid)] = tag
    elif states:
        for sid, st in states.items():
            tag = st.get("owner") if isinstance(st, dict) else st
            if tag:
                owner[int(sid)] = tag
    else:
        # legacy {state:{owner,controller}} under "ownership"/"owner"
        legacy = snapshot.get("ownership") or snapshot.get("owner") or {}
        for sid, rec in legacy.items():
            tag = rec.get("owner") if isinstance(rec, dict) else rec
            if tag:
                owner[int(sid)] = tag

    # ---- state controller ----
    controller: Dict[int, str] = {}
    if states:
        for sid, st in states.items():
            if isinstance(st, dict):
                c = st.get("controller")
                if c:
                    controller[int(sid)] = c
    else:
        legacy = snapshot.get("ownership") or snapshot.get("owner") or {}
        for sid, rec in legacy.items():
            if isinstance(rec, dict) and rec.get("controller"):
                controller[int(sid)] = rec["controller"]

    # ---- province-level control (preferred for fronts) ----
    control_by_prov: Dict[int, str] = {}
    cbp = smap.get("control_by_province")
    if isinstance(cbp, dict) and cbp:
        for pid, tag in cbp.items():
            if tag:
                control_by_prov[int(pid)] = tag
    else:
        provs = snapshot.get("provinces") or {}
        for pid, pr in provs.items():
            if isinstance(pr, dict) and pr.get("controller"):
                control_by_prov[int(pid)] = pr["controller"]

    # ---- divisions ----
    marks: Optional[List[DivisionMarker]] = None
    divs = snapshot.get("divisions")
    if divs:
        marks = []
        for d in divs:
            prov = d.get("location_province", d.get("province"))
            if not prov:
                continue
            s = _div_strength(d)
            marks.append(DivisionMarker(
                province_id=int(prov),
                owner=d.get("owner", "") or "",
                strength=(s if s is not None else 1.0),
                label=str(d.get("template_role", "") or ""),
            ))

    return Frame(
        timestamp=str(date), owner=owner,
        controller=controller or None,
        control_by_prov=control_by_prov or None,
        divisions=marks, hour_index=hour,
    )


def frames_from_replay_series(series) -> List[Frame]:
    """Adapt a time-series of canonical snapshots -> [Frame], ordered by hour_index.

    ``series`` may be a list of snapshot dicts, or a dict {key: snapshot}. The
    canonical sort key is ``hour_index`` (SCHEMA §0.3), NOT the date string and
    NOT the dict key order — date strings sort wrong lexically (``1936.10`` <
    ``1936.2``) and dict keys are not guaranteed numeric.
    """
    if isinstance(series, dict):
        items = list(series.values())
    else:
        items = list(series)
    frames = [frame_from_replay_snapshot(s) for s in items]
    # stable sort by hour_index; frames with an unknown hour_index keep input order
    frames.sort(key=lambda f: (f.hour_index is None, f.hour_index or 0))
    return frames
