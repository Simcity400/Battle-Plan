"""supply_number.py -- per-province SUPPLY NUMBER estimate (confidence-badged).

Produces a best-effort province -> supply estimate, plus state-level and global
rollups, for the Battle Plan tool. READ-ONLY: consumes parsed save_oracle JSON
(+ an optional live division proxy); never touches the game or the network.

==============================================================================
WHY AN *ESTIMATE*, NOT THE EXACT NUMBER (wall W2)
==============================================================================
The per-province "supply available" NUMBER that the in-game Supply map mode
shows (the "20/20" tooltip) is a SOLVER OUTPUT. It is proven NOT stored in the
save as a per-province slot, and NOT readable from live memory without UI/code
anchoring -- a documented wall (DESIGN B2 / RESEARCH_logistics_intel.md
"Supply [residual]" / supply_live.py module docstring; multiple negative,
Codex-confirmed pinning passes). A parallel Ghidra effort is decompiling the
solver and will land it as ``memory_reader.logistics_intel.supply_solver``;
THIS module import-guards that future module and PREFERS it when present.

Until the solver lands, the best we can honestly do is a layered ESTIMATE that
combines the signals we *can* read, and BADGES every output row so nothing is
ever rendered as ground truth:

  * ``provenance`` in {solver, proxy, stored_logistics, estimate}
  * ``confidence`` in {high, med, low}
  * ``value_kind`` -- what the number actually IS (so the UI never shows a T2
    structural-potential estimate as if it were the exact "20/20"):
        - "solver"           : exact solver output (future; authoritative)
        - "division_proxy"   : validated live per-division supply (where troops are)
        - "structural_potential" : stored hub/rail/infra heuristic (NOT delivered)
        - None               : no value (unknown; we refuse to invent one)
  * ``exact`` -- True only for the solver tier.
  * ``value_scope`` -- "province" for a genuine per-province value (solver/proxy)
    vs "state" for a T2 structural estimate that is really a per-STATE number
    repeated across the state's provinces (so a consumer never mistakes the
    repeated value for independent per-province readings -- Codex impl-4).

==============================================================================
THE COMBINATION (priority order; first hit wins per province)
==============================================================================
T0  SOLVER  (future, import-guarded): if supply_solver imports and yields a
    value for the province -> provenance=solver, confidence=high, exact=True,
    value_kind="solver". This is the clean seam the Ghidra crack plugs into.

T1  PROXY   (live, validated): the province holds >=1 land division and the
    division-province proxy (supply_live.enumerate_supply / a passed-in
    ``proxy_by_province`` dict) gives {current, capacity}. ->
    provenance=proxy, confidence=high, exact=False, value_kind="division_proxy".
    Caveat (Codex E): the proxy is a per-DIVISION derived/clamped value, NOT a
    verbatim copy of the province tooltip field, so ``exact=False`` ALWAYS and
    the label is explicitly "validated division proxy" -- never "the 20/20".

T2  ESTIMATE (stored): the province has no proxy. We estimate from the STORED
    signals of its STATE: supply-hub (supply_node) level, rail level, and
    infrastructure -- this is STRUCTURAL SUPPLY POTENTIAL, i.e. roughly how much
    supply the local logistics *could* push if connected, NOT delivered supply.
        base = K_NODE*supply_node_level
             + K_RAIL*min(rail_level, RAIL_CAP)
             + K_INFRA*infrastructure_level                 (then clamped)
    MODIFIERS (each only ever LOWERS the estimate -- we never inflate):
        - cut-off: controller != owner (occupied/contested) -> *CUTOFF_MULT.
        - disruption: if the state's supply hub province appears in
          supply_system_2.disrupted_supply, scale the HUB-DERIVED component by
          (1 - disruption_value) (Codex C: disruption is NODE-keyed; we apply it
          to the node contribution, not blindly to rail/infra).
    value_kind="structural_potential". provenance="stored_logistics" when the
    estimate is hub-dominated (a real stored supply_node level drives it), else
    "estimate". confidence is CAPPED at "med": med only when the state has a
    supply hub AND infrastructure>0 AND is not cut off; otherwise "low".
    *** Codex B (critical): controller==owner does NOT prove rail connectivity
        to the capital. An owned-but-rail-severed province is a real FALSE-HIGH
        we cannot detect from stored data alone. Hence T2 is "structural
        potential" (never "delivered"), is capped at confidence=med, and the
        WIRE note + UI guidance say to surface the band, not the bare number. A
        connectivity/distance-to-source signal would be needed to do better and
        is explicitly out of scope until the solver lands. ***

T3  NO DATA: province not joined to any state, or the state has no logistics
    buildings at all -> supply=None, value_kind=None, provenance="estimate",
    confidence="low". We DO NOT invent a number.

ROLLUPS (state + global): a state/global rollup reports BOTH the aggregate
estimate AND honest coverage (Codex F):
    - ``min_confidence``     : worst contributor (so one low drags it down)
    - ``dominant_provenance``: the most COMMON provenance among contributors
                               (NOT "best present" -- a state with 1 proxy + 99
                               estimates is NOT proxy-backed; saying so would lie)
    - ``coverage``           : province counts by provenance and by confidence
A state's ``estimated_supply`` is the MEAN of its provinces' numeric estimates
(None-valued provinces excluded but counted in coverage).

VALIDATION (documented, not an exact match -- the exact number is the wall):
  * Against the proxy on troop provinces: T1 reproduces the proxy exactly (it IS
    the proxy) -- a tautology, but it confirms wiring. The real test is that
    cut-off / no-hub provinces estimate LOW (see tests).
  * Plausibility: high-infra, big-hub, uncontested core states estimate higher
    than cut-off / hubless / occupied ones.
  * Codex (Mode A) adversarially reviewed the combination + confidence rules;
    its false-high findings are folded in above (B/C/D/E/F). The headline
    finding -- "hub capacity treated as delivered supply without connectivity"
    -- is mitigated by labelling T2 structural_potential and capping confidence,
    and is flagged as the residual the solver must close.

All weights live in named constants below. Every container is always present;
an unobtainable value is None, never guessed (DESIGN no-fabrication).
READ-ONLY. stdlib only.
"""
from __future__ import annotations

import json
import os
from dataclasses import dataclass, field, asdict
from typing import Any, Dict, List, Optional, Tuple

# ---------------------------------------------------------------------------
# T0 SEAM: optional import of the FUTURE exact supply solver (Ghidra D3).
# IMPORT-GUARDED so this module never hard-depends on it. When it lands, it
# must expose either:
#   * supply_solver.province_supply(output_dir) -> {province_id(int): float}, or
#   * supply_solver.SOLVER with a .province_supply(output_dir) method,
# returning the exact per-province supply NUMBER. Either shape is accepted by
# ``_load_solver_values`` below. Anything else -> we fall back to the estimate.
# ---------------------------------------------------------------------------
try:  # pragma: no cover - presence depends on the parallel Ghidra crack
    from memory_reader.logistics_intel import supply_solver as _supply_solver  # type: ignore
except Exception:  # noqa: BLE001 - any import failure -> estimate-only fallback
    _supply_solver = None


# ===========================================================================
# TUNABLES (named, justified; tweak in one place)
# ===========================================================================
# Structural-potential weights. These are deliberately rough -- the stored
# signal is weak (Codex D) -- and the OUTPUT is badged accordingly. They are
# scaled so a fully-developed hub state (hub 3, rail capped, infra 5) lands in
# the ballpark of the kind of double-digit supply the map mode shows for a
# well-connected industrial region, WITHOUT claiming that as exact.
K_NODE = 5.0          # per supply-hub level (hub capacity is the dominant term)
K_RAIL = 1.0          # per rail level (capped -- summed rail saturates fast)
K_INFRA = 1.2         # per infrastructure level (0..5)
RAIL_CAP = 10         # rail contribution saturates (a state-sum, not a path)
SUPPLY_CLAMP = 60.0   # hard ceiling on the structural estimate (sanity bound)

CUTOFF_MULT = 0.30    # controller != owner -> contested/occupied -> low delivered

# Bands for the coarse classification (Codex D: surface the band, not the bare
# number, for T2). Thresholds are on the final numeric estimate.
BAND_EDGES: List[Tuple[float, str]] = [
    (0.5, "none"),
    (5.0, "low"),
    (15.0, "med"),
    (float("inf"), "high"),
]

# Provenance / confidence / value-kind vocabularies (single source of truth).
PROVENANCE = ("solver", "proxy", "stored_logistics", "estimate")
CONFIDENCE = ("high", "med", "low")
VALUE_KIND = ("solver", "division_proxy", "structural_potential", None)

_CONF_RANK = {"high": 3, "med": 2, "low": 1}
_PROV_RANK = {"solver": 4, "proxy": 3, "stored_logistics": 2, "estimate": 1}

_REQUIRED = ("state_detail",)
_OPTIONAL = ("supply", "province_control")


# ===========================================================================
# Input loading (mirrors models/victory_score.py conventions)
# ===========================================================================
def _load_json(output_dir: str, name: str) -> Optional[Any]:
    path = os.path.join(output_dir, name + ".json")
    if not os.path.exists(path):
        return None
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def _load_province_to_state(output_dir: str) -> Dict[int, int]:
    """province_id -> state_id. Looked up first beside the JSON inputs, then in
    the repo's map_pipeline assets (same source parse_state_detail uses). Missing
    -> {} (province-level rows then degrade to state=None, all confidence=low)."""
    candidates = [
        os.path.join(output_dir, "province_to_state.json"),
        os.path.join(
            os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
            "map_pipeline", "assets", "province_to_state.json",
        ),
    ]
    for path in candidates:
        if not os.path.exists(path):
            continue
        try:
            with open(path, "r", encoding="utf-8") as fh:
                raw = json.load(fh)
        except (OSError, ValueError):
            continue
        out: Dict[int, int] = {}
        for k, v in raw.items():
            try:
                out[int(k)] = int(v)
            except (TypeError, ValueError):
                continue
        return out
    return {}


def load_inputs(output_dir: str) -> Dict[str, Any]:
    """Load JSON inputs. Raises FileNotFoundError if a *required* input is absent;
    optional inputs missing -> None (the model degrades, never fabricates)."""
    data: Dict[str, Any] = {}
    for name in _REQUIRED:
        d = _load_json(output_dir, name)
        if d is None:
            raise FileNotFoundError(
                f"required input '{name}.json' not found in {output_dir!r}"
            )
        data[name] = d
    for name in _OPTIONAL:
        data[name] = _load_json(output_dir, name)
    data["province_to_state"] = _load_province_to_state(output_dir)
    return data


# ===========================================================================
# Output row dataclasses (badge travels WITH the value)
# ===========================================================================
@dataclass
class ProvinceSupply:
    province: int
    state: Optional[int]
    supply: Optional[float]          # the estimate (None = unknown, never guessed)
    capacity: Optional[float]        # capacity where known (proxy/solver), else None
    band: str                        # coarse none|low|med|high
    provenance: str                  # solver | proxy | stored_logistics | estimate
    confidence: str                  # high | med | low
    value_kind: Optional[str]        # solver|division_proxy|structural_potential|None
    exact: bool                      # True only for the solver tier
    value_scope: str = "province"    # "province" (solver/proxy) | "state" (T2 estimate)
    cutoff: bool = False             # controller != owner (contested/occupied)
    notes: Optional[str] = None


@dataclass
class StateSupply:
    state: int
    owner: Optional[str]
    controller: Optional[str]
    estimated_supply: Optional[float]   # mean of numeric province estimates
    supply_node_level: int
    rail_level: int
    infrastructure: int
    min_confidence: str
    dominant_provenance: str
    coverage: Dict[str, Any]            # counts by provenance + by confidence
    cutoff: bool = False


@dataclass
class SupplyModel:
    provinces: List[Dict[str, Any]] = field(default_factory=list)
    states: List[Dict[str, Any]] = field(default_factory=list)
    global_rollup: Dict[str, Any] = field(default_factory=dict)
    meta: Dict[str, Any] = field(default_factory=dict)


# ===========================================================================
# Helpers
# ===========================================================================
def _finite_nonneg(v: Any) -> Optional[float]:
    """Coerce to a finite, non-negative float, else None (Codex impl-3).

    NaN/inf must never slip through: NaN fails every ``< edge`` band check and
    would otherwise band "high"; inf bands high too. Bad strings would crash a
    downstream consumer. Anything not finite-and->=0 -> None (treated as unknown,
    never as a high-supply value)."""
    try:
        f = float(v)
    except (TypeError, ValueError):
        return None
    if f != f or f in (float("inf"), float("-inf")):  # NaN or +/-inf
        return None
    if f < 0.0:
        return None
    return f


def _band(value: Optional[float]) -> str:
    # None = unknown (no proven value) -> a DISTINCT band, never "none" (which is a
    # measured near-zero). Conflating the two would hide the difference between
    # "cut off, ~0 supply" and "we don't know" (Codex impl2-3).
    if value is None:
        return "unknown"
    for edge, name in BAND_EDGES:
        if value < edge:
            return name
    return "high"  # pragma: no cover - inf edge guarantees a hit


def _proxy_value(pr: Dict[str, Any], key_primary: str, key_alt: str) -> Optional[float]:
    """Read a proxy field tolerating BOTH live-reader shapes (Codex impl2-4):
    ``enumerate_supply`` -> {"current","capacity"}; ``enumerate_supply_full`` ->
    {"supply","capacity"}. Returns a finite, non-negative float or None."""
    v = pr.get(key_primary)
    if v is None:
        v = pr.get(key_alt)
    return _finite_nonneg(v)


def _state_buildings(state: Dict[str, Any]) -> Tuple[int, int, int]:
    """(supply_node_level, rail_level, infrastructure) for a state_detail entry."""
    pb = state.get("province_buildings") or {}
    b = state.get("buildings") or {}
    node = int(pb.get("supply_node", 0) or 0)
    # naval_supply_hub also delivers supply at the coast; fold it in at par.
    node += int(pb.get("naval_supply_hub", 0) or 0)
    rail = int(pb.get("rail_way", 0) or 0)
    infra = int(b.get("infrastructure", 0) or 0)
    return node, rail, infra


def _structural_estimate(node: int, rail: int, infra: int) -> Tuple[float, float]:
    """Return (total_estimate, node_component) BEFORE cut-off/disruption mods.

    node_component is returned separately so the disruption multiplier can be
    applied to the hub-derived term only (Codex C)."""
    node_c = K_NODE * node
    rail_c = K_RAIL * min(rail, RAIL_CAP)
    infra_c = K_INFRA * infra
    total = node_c + rail_c + infra_c
    if total > SUPPLY_CLAMP:
        # scale node_c down proportionally so the split stays consistent
        scale = SUPPLY_CLAMP / total
        node_c *= scale
        total = SUPPLY_CLAMP
    return total, node_c


def _disrupted_node_provinces(supply_json: Optional[Dict[str, Any]]) -> Dict[int, float]:
    """node_province -> max disruption value across all countries (0..1).

    supply_system_2.disrupted_supply is NODE-keyed (Codex C). We take the worst
    disruption seen on each node province (a node can be listed by >1 country)."""
    out: Dict[int, float] = {}
    if not supply_json:
        return out
    for _tag, rec in (supply_json.get("countries") or {}).items():
        for d in (rec.get("disrupted_supply") or []):
            pid = d.get("node_province")
            val = d.get("value")
            if pid is None or val is None:
                continue
            try:
                pid_i = int(pid)
                val_f = float(val)
            except (TypeError, ValueError):
                continue
            if val_f > out.get(pid_i, 0.0):
                out[pid_i] = val_f
    return out


# ===========================================================================
# T0 solver seam
# ===========================================================================
def _load_solver_values(output_dir: str) -> Optional[Dict[int, float]]:
    """If the future Ghidra solver module is importable, ask it for the exact
    per-province supply numbers. Returns {province_id: float} or None.

    Accepts either ``supply_solver.province_supply(output_dir)`` or
    ``supply_solver.SOLVER.province_supply(output_dir)``. Any failure (module
    absent, wrong shape, raises) -> None, and we fall back to the estimate. This
    is the ONLY place that knows the solver's call shape; the rest of the model
    is solver-agnostic."""
    if _supply_solver is None:
        return None
    fn = getattr(_supply_solver, "province_supply", None)
    if fn is None:
        solver_obj = getattr(_supply_solver, "SOLVER", None)
        fn = getattr(solver_obj, "province_supply", None) if solver_obj else None
    if fn is None:
        return None
    try:  # pragma: no cover - exercised only once the real solver exists
        raw = fn(output_dir)
    except Exception:  # noqa: BLE001
        return None
    if not isinstance(raw, dict):
        return None
    out: Dict[int, float] = {}
    for k, v in raw.items():
        try:
            pid = int(k)
        except (TypeError, ValueError):
            continue
        fv = _finite_nonneg(v)        # drop NaN/inf/negative/garbage (Codex impl-3)
        if fv is not None:
            out[pid] = fv
    return out or None


# ===========================================================================
# Core build
# ===========================================================================
def compute_supply_number(
    output_dir: str,
    proxy_by_province: Optional[Dict[int, Dict[str, Any]]] = None,
) -> Dict[str, Any]:
    """Build the province/state/global supply estimate from an oracle output dir.

    ``proxy_by_province`` is the optional live division proxy
    (``{province_id: {"current": float, "capacity": float, ...}}`` as produced by
    ``memory_reader.logistics_intel.supply_live.enumerate_supply``). When the live
    game is unavailable (the normal offline replay case) it is None and T1 is
    simply skipped -- the model degrades to T2/T3, never fabricates.

    Returns a plain dict (asdict of SupplyModel) ready to serialize.
    """
    data = load_inputs(output_dir)
    states_json: Dict[str, Any] = (data["state_detail"] or {}).get("states", {}) or {}
    supply_json: Optional[Dict[str, Any]] = data.get("supply")
    province_control: Dict[str, Any] = data.get("province_control") or {}
    prov_to_state: Dict[int, int] = data.get("province_to_state") or {}

    solver_values = _load_solver_values(output_dir)
    disrupted = _disrupted_node_provinces(supply_json)
    proxy = {int(k): v for k, v in (proxy_by_province or {}).items()}

    # Pre-compute the per-state structural estimate + cut-off flag once.
    # state_supply[state_id] = (total_est, node_component, node, rail, infra,
    #                           owner, controller, cutoff, disruption_mult)
    state_meta: Dict[int, Dict[str, Any]] = {}
    for sid_str, st in states_json.items():
        try:
            sid = int(sid_str)
        except (TypeError, ValueError):
            continue
        node, rail, infra = _state_buildings(st)
        total, node_c = _structural_estimate(node, rail, infra)
        owner = st.get("owner")
        controller = st.get("controller")
        cutoff = bool(owner and controller and owner != controller)
        state_meta[sid] = {
            "total": total, "node_c": node_c,
            "node": node, "rail": rail, "infra": infra,
            "owner": owner, "controller": controller, "cutoff": cutoff,
        }

    # Which node provinces are disrupted, mapped to their state so the multiplier
    # can hit that state's hub component.
    disrupted_state: Dict[int, float] = {}
    for node_pid, val in disrupted.items():
        sid = prov_to_state.get(node_pid)
        if sid is not None:
            disrupted_state[sid] = max(disrupted_state.get(sid, 0.0), val)

    # ---- per-province pass ----
    # We iterate the union of: provinces that have a proxy/solver value, and
    # every province in the province->state map (so the map can be coloured).
    pids: set = set(prov_to_state.keys())
    pids |= set(proxy.keys())
    if solver_values:
        pids |= set(solver_values.keys())

    province_rows: List[ProvinceSupply] = []
    # group accumulator for state rollups
    per_state_rows: Dict[int, List[ProvinceSupply]] = {}

    for pid in sorted(pids):
        sid = prov_to_state.get(pid)
        sm = state_meta.get(sid) if sid is not None else None
        # cut-off flag prefers province-level control vs state owner when known.
        ctrl = province_control.get(str(pid))
        owner = sm["owner"] if sm else None
        cutoff = bool(owner and ctrl and owner != ctrl)
        if sm and not cutoff:
            cutoff = sm["cutoff"]

        row = _classify_province(
            pid=pid, sid=sid, sm=sm, cutoff=cutoff,
            solver_values=solver_values, proxy=proxy,
            disrupted_state=disrupted_state,
        )
        province_rows.append(row)
        if sid is not None:
            per_state_rows.setdefault(sid, []).append(row)

    # ---- per-state rollup ----
    state_rows: List[StateSupply] = []
    for sid in sorted(state_meta.keys()):
        sm = state_meta[sid]
        rows = per_state_rows.get(sid, [])
        state_rows.append(_rollup_state(sid, sm, rows))

    # ---- global rollup ----
    global_rollup = _rollup_global(province_rows, state_rows)

    proxy_accepted = sum(1 for r in province_rows if r.provenance == "proxy")
    model = SupplyModel(
        provinces=[asdict(r) for r in province_rows],
        states=[asdict(r) for r in state_rows],
        global_rollup=global_rollup,
        meta=_build_meta(data, solver_values, proxy, disrupted_state, proxy_accepted),
    )
    return asdict(model)


def _classify_province(
    pid: int,
    sid: Optional[int],
    sm: Optional[Dict[str, Any]],
    cutoff: bool,
    solver_values: Optional[Dict[int, float]],
    proxy: Dict[int, Dict[str, Any]],
    disrupted_state: Dict[int, float],
) -> ProvinceSupply:
    """Apply the T0->T3 priority ladder for one province."""
    # ---- T0 solver (authoritative) ----
    if solver_values is not None and pid in solver_values:
        val = _finite_nonneg(solver_values[pid])  # already guarded on load; belt+braces
        if val is not None:
            return ProvinceSupply(
                province=pid, state=sid, supply=val, capacity=None,
                band=_band(val), provenance="solver", confidence="high",
                value_kind="solver", exact=True, value_scope="province",
                cutoff=cutoff, notes="exact supply solver output",
            )
        # non-finite solver value -> do NOT trust it; fall through to estimate.

    # ---- T1 proxy (validated, live; where troops are) ----
    pr = proxy.get(pid)
    if pr is not None:
        # tolerate enumerate_supply ("current") AND enumerate_supply_full ("supply")
        cur_f = _proxy_value(pr, "current", "supply")   # NaN/inf/garbage -> None
        cap_f = _proxy_value(pr, "capacity", "capacity")
        if cur_f is not None:
            return ProvinceSupply(
                province=pid, state=sid, supply=cur_f, capacity=cap_f,
                band=_band(cur_f), provenance="proxy", confidence="high",
                value_kind="division_proxy", exact=False, value_scope="province",
                cutoff=cutoff,
                notes="validated division proxy (per-division derived; NOT the "
                      "exact map number)",
            )
        # unusable proxy value -> fall through to the structural estimate.

    # ---- T3 no state / no data ----
    if sm is None:
        return ProvinceSupply(
            province=pid, state=sid, supply=None, capacity=None,
            band=_band(None), provenance="estimate", confidence="low",
            value_kind=None, exact=False, value_scope="province",
            cutoff=cutoff,
            notes="no state join / no logistics data; value unknown (not invented)",
        )

    # ---- T2 stored structural-potential estimate (a per-STATE value) ----
    total = sm["total"]
    node_c = sm["node_c"]
    node = sm["node"]
    infra = sm["infra"]

    # disruption hits the HUB-derived component only (Codex C).
    if sid is not None and sid in disrupted_state:
        d = disrupted_state[sid]
        non_node = total - node_c
        node_c = node_c * (1.0 - d)
        total = non_node + node_c

    # cut-off scales the WHOLE delivered estimate -- including the hub term, so
    # the later hub_dominated test compares consistent (post-cutoff) units and a
    # cut-off province is never mis-badged stored_logistics (Codex impl-1).
    if cutoff:
        total *= CUTOFF_MULT
        node_c *= CUTOFF_MULT

    # if the state has no logistics signal at all -> unknown, not a fake 0.
    if node == 0 and infra == 0 and sm["rail"] == 0:
        return ProvinceSupply(
            province=pid, state=sid, supply=None, capacity=None,
            band=_band(None), provenance="estimate", confidence="low",
            value_kind=None, exact=False, value_scope="state",
            cutoff=cutoff,
            notes="state has no hub/rail/infra; supply unknown (not invented)",
        )

    est = round(total, 2)
    hub_dominated = node_c >= (total - node_c)  # hub term carries the estimate
    provenance = "stored_logistics" if (node > 0 and hub_dominated) else "estimate"
    # confidence capped at med. med REQUIRES the row to be hub-backed
    # (provenance==stored_logistics) AND supplied AND uncontested -- so a hub that
    # disruption knocked below the rail/infra terms (-> provenance flipped to
    # estimate) correctly drops to low, not a dishonest hub-backed med (Codex impl-2).
    if provenance == "stored_logistics" and infra > 0 and not cutoff:
        confidence = "med"
    else:
        confidence = "low"

    note = "structural supply potential (stored hub/rail/infra; NOT delivered, " \
           "NOT connectivity-checked; value_scope=state)"
    if cutoff:
        note += "; cut-off (controller != owner) -> reduced"
    if sid is not None and sid in disrupted_state:
        note += "; hub disrupted"

    return ProvinceSupply(
        province=pid, state=sid, supply=est, capacity=None,
        band=_band(est), provenance=provenance, confidence=confidence,
        value_kind="structural_potential", exact=False, value_scope="state",
        cutoff=cutoff, notes=note,
    )


def _rollup_state(
    sid: int, sm: Dict[str, Any], rows: List[ProvinceSupply]
) -> StateSupply:
    numeric = [r.supply for r in rows if r.supply is not None]
    est = round(sum(numeric) / len(numeric), 2) if numeric else None

    if rows:
        min_conf = min(rows, key=lambda r: _CONF_RANK[r.confidence]).confidence
        dominant = _dominant_provenance([r.provenance for r in rows])
    else:
        min_conf = "low"
        dominant = "estimate"

    # state cutoff = state-level control mismatch OR any province cut off
    # (province_control can flip a province even when state owner==controller;
    # the state flag must reflect that, Codex impl2-2).
    prov_cutoff = sum(1 for r in rows if r.cutoff)
    coverage = {
        "provinces": len(rows),
        "by_provenance": _count(r.provenance for r in rows),
        "by_confidence": _count(r.confidence for r in rows),
        "by_value_scope": _count(r.value_scope for r in rows),
        "numeric": len(numeric),
        "cutoff_provinces": prov_cutoff,
    }
    return StateSupply(
        state=sid, owner=sm["owner"], controller=sm["controller"],
        estimated_supply=est, supply_node_level=sm["node"],
        rail_level=sm["rail"], infrastructure=sm["infra"],
        min_confidence=min_conf, dominant_provenance=dominant,
        coverage=coverage, cutoff=bool(sm["cutoff"] or prov_cutoff),
    )


def _rollup_global(
    province_rows: List[ProvinceSupply], state_rows: List[StateSupply]
) -> Dict[str, Any]:
    numeric = [r.supply for r in province_rows if r.supply is not None]
    return {
        "provinces": len(province_rows),
        "states": len(state_rows),
        "provinces_with_value": len(numeric),
        "mean_estimated_supply": (
            round(sum(numeric) / len(numeric), 2) if numeric else None),
        "min_confidence": (
            min(province_rows, key=lambda r: _CONF_RANK[r.confidence]).confidence
            if province_rows else "low"),
        "dominant_provenance": _dominant_provenance(
            [r.provenance for r in province_rows]),
        "by_provenance": _count(r.provenance for r in province_rows),
        "by_confidence": _count(r.confidence for r in province_rows),
        "by_value_kind": _count(
            (r.value_kind or "none") for r in province_rows),
        "cutoff_provinces": sum(1 for r in province_rows if r.cutoff),
    }


def _dominant_provenance(provs: List[str]) -> str:
    """Most COMMON provenance, ties broken by authority (Codex F: a single proxy
    province must NOT make a whole state read as proxy-backed)."""
    if not provs:
        return "estimate"
    counts = _count(provs)
    return max(counts.items(), key=lambda kv: (kv[1], _PROV_RANK.get(kv[0], 0)))[0]


def _count(it) -> Dict[str, int]:
    out: Dict[str, int] = {}
    for x in it:
        out[x] = out.get(x, 0) + 1
    return out


def _build_meta(
    data: Dict[str, Any],
    solver_values: Optional[Dict[int, float]],
    proxy: Dict[int, Dict[str, Any]],
    disrupted_state: Dict[int, float],
    proxy_accepted: int,
) -> Dict[str, Any]:
    sd_meta = (data.get("state_detail") or {}).get("meta", {}) or {}
    return {
        "model": "supply_number",
        "save_date": sd_meta.get("save_date"),
        "save_version": sd_meta.get("save_version"),
        "solver_present": solver_values is not None,
        "proxy_provinces_in": len(proxy),       # records the caller supplied
        "proxy_provinces": proxy_accepted,      # records actually USED as T1 (Codex impl2-4)
        "disrupted_states": len(disrupted_state),
        "wall": "W2 -- exact per-province supply is a solver output, not stored; "
                "this is a confidence-badged ESTIMATE. T2 is structural potential "
                "(hub/rail/infra), NOT delivered supply; no connectivity check.",
        "confidence_legend": {
            "high": "solver (exact) or validated live division proxy",
            "med": "stored hub-backed STRUCTURAL estimate, uncontested land "
                   "(structural potential, NOT proven delivered/connected)",
            "low": "cut-off / no-hub / no-data / unknown / disruption-flipped",
        },
        "provenance_legend": list(PROVENANCE),
        "value_kind_legend": [v for v in VALUE_KIND if v is not None] + ["null"],
        "tunables": {
            "K_NODE": K_NODE, "K_RAIL": K_RAIL, "K_INFRA": K_INFRA,
            "RAIL_CAP": RAIL_CAP, "SUPPLY_CLAMP": SUPPLY_CLAMP,
            "CUTOFF_MULT": CUTOFF_MULT,
        },
    }


# ===========================================================================
# CLI
# ===========================================================================
def main(argv: Optional[List[str]] = None) -> int:  # pragma: no cover - thin CLI
    import argparse

    ap = argparse.ArgumentParser(
        description="Estimate per-province the game supply numbers (confidence-badged; "
                    "exact value awaits the Ghidra supply solver).")
    ap.add_argument("output_dir",
                    help="save_oracle output dir (state_detail.json [+ supply.json, "
                         "province_control.json]).")
    ap.add_argument("--out", default=None,
                    help="write the JSON model here (default: print summary).")
    args = ap.parse_args(argv)

    model = compute_supply_number(args.output_dir)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(model, fh, indent=1)
        print(f"wrote {args.out}")
    g = model["global_rollup"]
    print("=== SUPPLY-NUMBER ESTIMATE (badged; exact value awaits solver) ===")
    print(f"provinces        : {g['provinces']} "
          f"({g['provinces_with_value']} with a numeric estimate)")
    print(f"states           : {g['states']}")
    print(f"mean est supply  : {g['mean_estimated_supply']}")
    print(f"by provenance    : {g['by_provenance']}")
    print(f"by confidence    : {g['by_confidence']}")
    print(f"by value_kind    : {g['by_value_kind']}")
    print(f"cut-off provinces: {g['cutoff_provinces']}")
    print(f"solver present   : {model['meta']['solver_present']}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
