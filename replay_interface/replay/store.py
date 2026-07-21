"""Snapshot store + writer for the replay backbone.

Ingests existing extractor/save_oracle output JSON (and, when present, extractor/memory_reader draft
outputs) READ-ONLY and normalizes them into a single canonical SNAPSHOT dict that
conforms to replay_interface/documents/REPLAY_SCHEMA.md, then persists it to a
per-game indexed store under replay_interface/replay/snapshots/.

Hard rules honored here:
  * READ-ONLY on every source file. We never write under extractor/save_oracle/
    or extractor/memory_reader/. The writer only touches
    replay_interface/replay/snapshots/.
  * No fabrication: a domain not present in the source stays null-but-present.
  * No personal paths: meta 'source_path_redacted' from the oracle is discarded;
    we keep only the basename in provenance.source_filename (SCHEMA §2 redaction).

Store layout (documented in replay_interface/replay/README.md):
    replay_interface/replay/snapshots/<game_id>/
        index.json                     # ordered manifest of snapshots in this game
        <hour_index>.snapshot.json     # one file per timestamp (zero-padded hour key)
        timeline.json                  # diff-engine output (written by diff_engine)

The hour_index is the canonical key/sort. Filenames use the integer hour_index so
they sort chronologically lexicographically when zero-padded.
"""
from __future__ import annotations

import json
import os
from typing import Any, Dict, List, Optional, Tuple

try:
    from . import schema
except ImportError:  # pragma: no cover - allow running as a loose script
    import schema  # type: ignore


# ---------------------------------------------------------------------------
# Low-level IO helpers (stdlib json only).
# ---------------------------------------------------------------------------
def _read_json(path: str) -> Any:
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def _read_json_opt(path: str) -> Optional[Any]:
    if not os.path.isfile(path):
        return None
    try:
        return _read_json(path)
    except (OSError, ValueError):
        return None


def _atomic_write_json(path: str, obj: Any, **dump_kwargs: Any) -> None:
    """Write JSON atomically: dump to a same-directory temp file, flush+fsync,
    then os.replace onto the destination. A crash mid-write can never leave a
    truncated file at ``path`` — the old content survives intact or the complete
    new content lands. On failure the temp file is removed (best-effort) and the
    error re-raised so a failed write is loud, never a stray partial file."""
    tmp = f"{path}.tmp.{os.getpid()}"
    try:
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(obj, fh, **dump_kwargs)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def _basename(meta: Optional[Dict[str, Any]]) -> Optional[str]:
    """Extract a *basename only* from oracle meta — never a path (SCHEMA §2)."""
    if not isinstance(meta, dict):
        return None
    name = meta.get("source_filename")
    if isinstance(name, str) and name:
        return os.path.basename(name)
    return None


# ---------------------------------------------------------------------------
# Oracle ingestion: build one canonical snapshot from a directory of oracle output.
# ---------------------------------------------------------------------------
class OracleBundle:
    """A loaded set of save_oracle output JSON files (all optional)."""

    FILES = {
        "country_snapshot": "country_snapshot.json",
        "economy": "economy.json",
        "production": "production.json",
        "tech": "tech.json",
        "focus": "focus.json",
        "diplomacy": "diplomacy.json",
        "state_ownership": "state_ownership.json",
        "state_control": "state_control.json",
        "state_detail": "state_detail.json",
        "province_control": "province_control.json",
        "divisions": "divisions.json",
        "combats": "combats.json",
        # deep domains added by the save-side parser family (all optional)
        "characters": "characters.json",
        "designs": "designs.json",
        "oob": "oob.json",
        "intel": "intel.json",
        # equipment market / trade (parse_market) + occupier-side governance
        # (parse_occupation). Both optional; ingested below.
        "market": "market.json",
        "occupation": "occupation.json",
        # WAVE-2 NEW save deep parsers (all optional; ingested below). market.json
        # is ALREADY wired (contracts/stockpile) — these add only the NEW state.
        "abilities": "abilities.json",         # aces + cmdr cooldowns + officer corps
        "projects": "projects.json",           # special projects/facilities/scientists
        "market_extra": "market_extra.json",   # price bands + subsidies + requests
        "deployment": "deployment.json",       # deploy conveyor + expeditionary
        # Tier-1 EXTRACT save domains (all optional; ingested below)
        "supply": "supply.json",
        "strategic_air": "strategic_air.json",
        "strategic_operatives": "strategic_operatives.json",
        "raids": "raids.json",
        "events": "events.json",
        "world": "world.json",
        "country_extra": "country_extra.json",
        "players": "players.json",
        "divisions_full": "divisions_full.json",
        "meta": "meta.json",
        "control_meta": "control_meta.json",
    }

    def __init__(self, output_dir: str):
        self.output_dir = output_dir
        self.data: Dict[str, Any] = {}
        for key, fname in self.FILES.items():
            self.data[key] = _read_json_opt(os.path.join(output_dir, fname))

    def get(self, key: str) -> Any:
        return self.data.get(key)


def _country_meta(bundle: OracleBundle) -> Tuple[Optional[str], Optional[str], Optional[str]]:
    """(save_date, save_version, source_filename) preferring the richest meta."""
    for src in ("meta", "control_meta"):
        meta = bundle.get(src)
        if isinstance(meta, dict):
            return (meta.get("save_date"), meta.get("save_version"), _basename(meta))
    # deep files carry their own meta block
    for key in ("economy", "production", "tech", "focus", "divisions"):
        obj = bundle.get(key)
        if isinstance(obj, dict) and isinstance(obj.get("meta"), dict):
            m = obj["meta"]
            return (m.get("save_date"), m.get("save_version"), _basename(m))
    return (None, None, None)


def _human_set(bundle: OracleBundle) -> set:
    """Tags flagged human (is_human / is_ai=no). country_snapshot is authoritative."""
    out = set()
    cs = bundle.get("country_snapshot")
    if isinstance(cs, dict):
        for tag, c in cs.items():
            if isinstance(c, dict) and c.get("is_human"):
                out.add(tag)
    return out


def build_snapshot_from_oracle(
    output_dir: str,
    game_id: str,
    *,
    memory_dir: Optional[str] = None,
) -> Dict[str, Any]:
    """Normalize one save_oracle output dir into a canonical SNAPSHOT dict.

    memory_dir is accepted for forward-compat; current memory_reader artifacts are
    *offset drafts*, not per-timestamp live reads, so no live values are ingested
    yet (they stay null-but-present). When a live reader emits a snapshot file we
    overlay it here without changing the schema.
    """
    bundle = OracleBundle(output_dir)
    save_date, save_version, source_filename = _country_meta(bundle)

    snap = schema.empty_snapshot(game_id, save_date)
    snap["save_version"] = save_version

    # --- countries ---
    _ingest_countries(bundle, snap)
    # --- diplomacy (wars/factions/subjects/operatives) ---
    _ingest_diplomacy(bundle, snap)
    # --- commanders (characters.json: leader + generals/FM/admirals + advisors) ---
    _ingest_commanders(bundle, snap)
    # --- designs (designs.json: templates + variant catalog + per-country MIOs) ---
    _ingest_designs(bundle, snap)
    # --- intel (intel.json: agency/operatives/operations/decryption/peace) ---
    _ingest_intel(bundle, snap)
    # --- market (market.json: trade contracts -> lend_lease + market stockpiles) ---
    _ingest_market(bundle, snap)
    # --- WAVE-2 market_extra (price bands + subsidies + requests) ---
    _ingest_market_extra(bundle, snap)
    # --- order of battle (oob.json: armies/fleets/wings/plans, cross-refs joined) ---
    _ingest_oob(bundle, snap)
    # --- states (ownership/control merged, then detail) ---
    _ingest_states(bundle, snap)
    _ingest_state_detail(bundle, snap)
    # --- occupier-side governance (occupation.json) folded onto occupied states ---
    _ingest_occupation(bundle, snap)
    # --- provinces ---
    _ingest_provinces(bundle, snap)
    # --- divisions ---
    _ingest_divisions(bundle, snap)
    # --- divisions_full: merge the additive per-division body onto divisions[] ---
    _ingest_divisions_full(bundle, snap)
    # --- combats (active at snapshot instant) ---
    _ingest_combats(bundle, snap)
    # --- Tier-1 EXTRACT save domains -------------------------------------------
    _ingest_supply(bundle, snap)               # per-country economy.logistics
    _ingest_strategic_air(bundle, snap)        # top-level strategic_air
    _ingest_strategic_operatives(bundle, snap) # top-level strategic_operatives
    _ingest_raids(bundle, snap)                # top-level raids
    _ingest_world_events(bundle, snap)         # top-level world_events
    _ingest_world(bundle, snap)                # top-level world (scenario/vars/flags)
    _ingest_country_extra(bundle, snap)        # per-country extra (nukes/aces/...)
    _ingest_players(bundle, snap)              # per-country human_name
    # --- WAVE-2 NEW save deep domains -----------------------------------------
    _ingest_abilities(bundle, snap)            # per-country aces/cooldowns/officer corps
    _ingest_projects(bundle, snap)             # per-country projects/facilities/scientists
    _ingest_deployment(bundle, snap)           # per-country deploy queue + expeditionary
    # --- map convenience layers (mirror state/province) ---
    _fill_map_layers(snap)
    # --- provenance ---
    _fill_provenance(bundle, snap, save_date, save_version, source_filename, memory_dir)

    return snap


def _ensure_country(snap: Dict[str, Any], tag: str) -> Dict[str, Any]:
    c = snap["countries"].get(tag)
    if c is None:
        c = schema.empty_country()
        snap["countries"][tag] = c
    return c


def _ingest_countries(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    humans = _human_set(bundle)

    # country_snapshot.json — merged scalars (the memory-aligned file).
    cs = bundle.get("country_snapshot")
    if isinstance(cs, dict):
        for tag, c in cs.items():
            if not isinstance(c, dict):
                continue
            dst = _ensure_country(snap, tag)
            pol = dst["politics"]
            pol["stability"] = c.get("stability")
            pol["war_support"] = c.get("war_support")
            pol["political_power"] = c.get("political_power")
            # command_power is a single source written to BOTH schema-defined keys
            # (politics.command_power and military.command_power); read it once.
            command_power = c.get("command_power")
            pol["command_power"] = command_power
            pol["ruling_party"] = c.get("ruling_party")
            pol["is_human"] = c.get("is_human")
            pol["is_major"] = c.get("major")
            pol["capital_state"] = c.get("capital")
            mil = dst["military"]
            mil["num_ships"] = c.get("num_ships")
            mil["command_power"] = command_power
            mil.setdefault("manpower", {})
            # manpower_ratio is a raw manpower figure in this oracle file.
            # CONTRACT NOTE: military.manpower.available is written here from
            # country_snapshot.manpower_ratio AND again in the economy.json pass
            # below (from economy.available_manpower). Order is last-writer-wins:
            # the economy pass runs after this one, so economy.available_manpower
            # silently overrides this value when both are present. Intentional —
            # economy.json is the authoritative manpower source. NO behaviour change.
            if c.get("manpower_ratio") is not None:
                mil["manpower"]["available"] = c.get("manpower_ratio")
            eco = dst["economy"]
            if isinstance(c.get("factories"), dict):
                eco["factories"].update(c["factories"])
            if c.get("fuel") is not None or c.get("max_fuel") is not None:
                eco["fuel"] = {"current": c.get("fuel"), "max": c.get("max_fuel")}

    # economy.json (deeper, authoritative for factories/resources/laws/ideas)
    eco_root = bundle.get("economy")
    if isinstance(eco_root, dict) and isinstance(eco_root.get("countries"), dict):
        for tag, e in eco_root["countries"].items():
            dst = _ensure_country(snap, tag)
            eco = dst["economy"]
            if isinstance(e.get("factories"), dict):
                eco["factories"].update(e["factories"])
            if isinstance(e.get("construction_queue"), list):
                eco["construction_queue"] = e["construction_queue"]
            if isinstance(e.get("resources"), dict):
                eco["resources"] = e["resources"]
            if isinstance(e.get("resource_trade"), dict):
                eco["resource_trade"] = e["resource_trade"]
            if isinstance(e.get("fuel"), dict):
                eco["fuel"] = e["fuel"]
            pol = dst["politics"]
            if isinstance(e.get("laws"), dict):
                pol["laws"] = e["laws"]
            if isinstance(e.get("ideas_all"), list):
                pol["ideas_all"] = e["ideas_all"]
            if isinstance(e.get("ideas_other"), list):
                pol["ideas_other"] = e["ideas_other"]
            # economy file scalars are a redundant source; only fill if absent
            for src_k, dst_k in (("stability", "stability"), ("war_support", "war_support"),
                                 ("political_power", "political_power"),
                                 ("command_power", "command_power"),
                                 ("ruling_party", "ruling_party")):
                if pol.get(dst_k) is None and e.get(src_k) is not None:
                    pol[dst_k] = e[src_k]
            mil = dst["military"]
            mil.setdefault("manpower", {})
            # CONTRACT NOTE: this is the authoritative manpower writer; it runs
            # AFTER the country_snapshot pass and intentionally last-writer-wins
            # over military.manpower.available set from manpower_ratio there.
            if e.get("available_manpower") is not None:
                mil["manpower"]["available"] = e["available_manpower"]

    # production.json -> military stockpile/convoys + equipment lines (for gross IC)
    prod_root = bundle.get("production")
    if isinstance(prod_root, dict) and isinstance(prod_root.get("countries"), dict):
        for tag, p in prod_root["countries"].items():
            dst = _ensure_country(snap, tag)
            mil = dst["military"]
            if isinstance(p.get("stockpile"), list):
                mil["equipment_stockpile"] = p["stockpile"]
            if isinstance(p.get("convoys"), list):
                mil["convoys"] = p["convoys"]
            if isinstance(p.get("equipment_lines"), list):
                # keep raw lines on economy for the gross-IC-delta pass later
                dst["economy"]["_production_lines"] = p["equipment_lines"]

    # tech.json
    tech_root = bundle.get("tech")
    if isinstance(tech_root, dict) and isinstance(tech_root.get("countries"), dict):
        for tag, t in tech_root["countries"].items():
            dst = _ensure_country(snap, tag)
            res = dst["research"]
            if isinstance(t.get("technologies"), list):
                res["technologies"] = t["technologies"]
            res["tech_count"] = t.get("tech_count")
            if isinstance(t.get("in_progress"), list):
                res["in_progress"] = t["in_progress"]
            res["research_slots"] = t.get("research_slots")
            res["original_research_slots"] = t.get("original_research_slots")
            if isinstance(t.get("research_bonuses"), list):
                res["research_bonuses"] = t["research_bonuses"]

    # focus.json
    focus_root = bundle.get("focus")
    if isinstance(focus_root, dict) and isinstance(focus_root.get("countries"), dict):
        for tag, f in focus_root["countries"].items():
            dst = _ensure_country(snap, tag)
            fc = dst["focus"]
            fc["focus_tree"] = f.get("focus_tree")
            fc["continuous_focus_palette"] = f.get("continuous_focus_palette")
            if isinstance(f.get("completed"), list):
                fc["completed"] = f["completed"]
            fc["completed_count"] = f.get("completed_count")
            fc["current"] = f.get("current")
            fc["progress"] = f.get("progress")
            fc["paused"] = f.get("paused")

    # division_count derived after divisions ingested (in _ingest_divisions epilogue)
    for tag in snap["countries"]:
        if tag in humans:
            snap["countries"][tag]["politics"]["is_human"] = True


def _unique_key(base: str, existing: Dict[str, Any]) -> str:
    """Deterministic collision-proof dict key: `base`, else 'base#2', '#3', ...
    in encounter order, so a key clash never silently overwrites a record."""
    if base not in existing:
        return base
    n = 2
    while f"{base}#{n}" in existing:
        n += 1
    return f"{base}#{n}"


def _ingest_diplomacy(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize diplomacy.json into per-country diplomacy + a world-level
    wars{}/factions{} block on the snapshot (Codex P1 #1, P2 #11)."""
    dip = bundle.get("diplomacy")
    if not isinstance(dip, dict):
        return

    # World-scoped, war-id-keyed wars (the proper backbone for war events).
    wars_by_id: Dict[str, Any] = {}
    for w in (dip.get("wars") or []):
        if not isinstance(w, dict):
            continue
        a, b = w.get("a"), w.get("b")
        if not (a and b):
            continue
        # Keys are opaque to every consumer; two distinct wars between the same
        # pair with the same start_date (or both missing) get a deterministic
        # ordinal suffix in encounter order instead of silently overwriting.
        war_id = _unique_key(f"{a}-{b}@{w.get('start_date')}", wars_by_id)
        wars_by_id[war_id] = {
            "a": a, "b": b, "start_date": w.get("start_date"),
            "a_casualties": w.get("a_casualties"), "b_casualties": w.get("b_casualties"),
            "war_score_a_vs_b": w.get("war_score_a_vs_b"),
            "war_score_b_vs_a": w.get("war_score_b_vs_a"),
        }
    # Factions keyed by engine id. An id-less record is preserved (never
    # silently dropped) under a synthetic 'noid#N' key and soft-annotated
    # id_missing so the anomaly stays visible; duplicate real ids get the same
    # ordinal disambiguation as wars.
    factions_by_id: Dict[str, Any] = {}
    noid_count = 0
    for f in (dip.get("factions") or []):
        if not isinstance(f, dict):
            continue
        if f.get("id") is None:
            noid_count += 1
            base = f"noid#{noid_count}"
            f = dict(f, id_missing=True)
        else:
            base = str(f["id"])
        factions_by_id[_unique_key(base, factions_by_id)] = f
    snap["diplomacy_world"] = {
        "wars": wars_by_id,
        "factions": factions_by_id,
        "subjects": dip.get("subjects") or [],
    }

    # Per-country: faction membership, wars (pairwise against[]), subjects, intel.
    faction_of: Dict[str, str] = {}
    faction_leader: Dict[str, str] = {}
    for f in (dip.get("factions") or []):
        if not isinstance(f, dict):
            continue
        fname = f.get("template") or f.get("icon") or str(f.get("id"))
        for mem in (f.get("members") or []):
            faction_of[mem] = fname
        if f.get("leader"):
            faction_leader[f["leader"]] = fname

    # build against-sets per tag from wars
    against: Dict[str, set] = {}
    for w in (dip.get("wars") or []):
        if isinstance(w, dict) and w.get("a") and w.get("b"):
            against.setdefault(w["a"], set()).add(w["b"])
            against.setdefault(w["b"], set()).add(w["a"])

    overlord_of: Dict[str, Dict[str, Any]] = {}
    for s in (dip.get("subjects") or []):
        if isinstance(s, dict) and s.get("subject"):
            overlord_of[s["subject"]] = s

    cdata = dip.get("countries") if isinstance(dip.get("countries"), dict) else {}
    tags = set(faction_of) | set(against) | set(overlord_of) | set(cdata or {})
    for tag in tags:
        dst = _ensure_country(snap, tag)
        d = dst["diplomacy"]
        d["wars"] = [{"against": x} for x in sorted(against.get(tag, ()))]
        if tag in faction_of:
            dst["politics"]["faction"] = {
                "name": faction_of[tag],
                "is_leader": faction_leader.get(tag) == faction_of[tag],
            }
        if tag in overlord_of:
            s = overlord_of[tag]
            dst["politics"]["autonomy"] = {
                "level": s.get("autonomy_state"), "overlord": s.get("overlord"),
            }
        cd = (cdata or {}).get(tag) or {}
        if isinstance(cd, dict):
            if isinstance(cd.get("operatives"), list):
                dst["intel"]["operatives"] = cd["operatives"]
            if cd.get("relations"):
                d["opinion"] = cd["relations"]
            if isinstance(cd.get("wargoals"), list) and cd["wargoals"]:
                # keep wargoals as a country-level field; only attach a wargoal to a
                # specific war when its target matches that enemy (Codex 2nd-pass P2 —
                # don't fabricate the same wargoal against every enemy).
                d["wargoals"] = cd["wargoals"]
                for w in d["wars"]:
                    matched = [g for g in cd["wargoals"]
                               if isinstance(g, dict) and g.get("target") == w["against"]]
                    if matched:
                        w["wargoals"] = matched


def _ingest_commanders(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize characters.json into per-country commanders (SCHEMA §3.9).

    The parser already resolved character ids -> names and joined military leaders
    to their formation assignment, so this is a direct (null-safe) copy of the
    leader / party_leaders / generals / field_marshals / admirals / advisors lists.
    """
    ch = bundle.get("characters")
    if not isinstance(ch, dict) or not isinstance(ch.get("countries"), dict):
        return
    for tag, c in ch["countries"].items():
        if not isinstance(c, dict):
            continue
        dst = _ensure_country(snap, tag)
        cmd = dst["commanders"]
        cmd["leader"] = c.get("leader")
        if isinstance(c.get("party_leaders"), list):
            cmd["party_leaders"] = c["party_leaders"]
        cmd["ruling_party"] = c.get("ruling_party")
        for k in ("generals", "field_marshals", "admirals", "advisors"):
            if isinstance(c.get(k), list):
                cmd[k] = c[k]
        if isinstance(c.get("counts"), dict):
            cmd["counts"] = c["counts"]


def _ingest_designs(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize designs.json into the top-level designs catalog + per-country MIOs.

    World-scoped: `division_templates` (list, each carries its owning `country`) and
    `equipment_variants` (dict keyed "id:type") go to snap["designs"]. Per-country
    `mios` / `scheduled_variants` / `policies` go to each Country (SCHEMA §3.10, §11).
    The catalog ids are the join target for division/ship/wing equipment refs (oob).
    """
    dsg = bundle.get("designs")
    if not isinstance(dsg, dict):
        return
    if isinstance(dsg.get("division_templates"), list):
        snap["designs"]["division_templates"] = dsg["division_templates"]
    if isinstance(dsg.get("equipment_variants"), dict):
        snap["designs"]["equipment_variants"] = dsg["equipment_variants"]
    cdata = dsg.get("countries")
    if isinstance(cdata, dict):
        for tag, c in cdata.items():
            if not isinstance(c, dict):
                continue
            dst = _ensure_country(snap, tag)
            if isinstance(c.get("mios"), list):
                dst["mios"] = c["mios"]
            if isinstance(c.get("scheduled_variants"), list):
                dst["scheduled_variants"] = c["scheduled_variants"]
            if isinstance(c.get("policies"), dict):
                dst["mio_policies"] = c["policies"]


def _ingest_intel(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize intel.json into per-country intel (SCHEMA §3.8, deepened).

    intel.json is authoritative for the espionage picture and OVERRIDES the thin
    operative summary that _ingest_diplomacy may have written. Maps the parser's
    agency scalars + capacity into a single `agency` object, `cryptology` -> the
    `decryption` field, and keeps operatives / recruitable / operations / peace.
    The top-level peace_conference lives in meta; surfaced per-country for the
    consumer that wants it co-located with intel.
    """
    it = bundle.get("intel")
    if not isinstance(it, dict):
        return
    peace = ((it.get("meta") or {}).get("peace_conference")
             if isinstance(it.get("meta"), dict) else None)
    cdata = it.get("countries")
    if not isinstance(cdata, dict):
        return
    for tag, c in cdata.items():
        if not isinstance(c, dict):
            continue
        dst = _ensure_country(snap, tag)
        intel = dst["intel"]
        intel["agency"] = {
            "name": c.get("agency_name"),
            "icon": c.get("icon"),
            "is_created": c.get("is_created"),
            "upgrade_progress": c.get("upgrade_progress"),
            "current_upgrade": c.get("current_upgrade"),
            "upgrades": c.get("upgrades"),
            "capacity": c.get("capacity"),
        }
        if isinstance(c.get("operatives"), list):
            intel["operatives"] = c["operatives"]
        if isinstance(c.get("recruitable_operatives"), list):
            intel["recruitable_operatives"] = c["recruitable_operatives"]
        if isinstance(c.get("operations"), dict):
            intel["operations"] = c["operations"]
        operation_tokens = c.get("operation_tokens")
        if isinstance(operation_tokens, dict):
            owned = operation_tokens.get("owned")
            if isinstance(owned, dict):
                # The save's outer intel_source tuple is a process-local
                # serializer reference, not portable identity.  Keep only the
                # exact operation-token -> target-tag membership set.
                intel["operation_tokens"] = {
                    token: list(targets)
                    for token, targets in owned.items()
                    if isinstance(token, str) and isinstance(targets, list)
                }
        if isinstance(c.get("cryptology"), dict):
            intel["decryption"] = c["cryptology"]
        if peace is not None:
            intel["peace_conference"] = peace


def _id_pair_str(cid: Any) -> Optional[str]:
    """Normalize an engine id (often an [id, type] pair) to a stable 'id:type' key.

    Shared by contract ids and combat ids — both are the same [id, type] shape.
    """
    if isinstance(cid, list):
        return ":".join(str(x) for x in cid)
    return str(cid) if cid is not None else None


def _id_pair_id(cid: Any) -> Optional[int]:
    """Return the numeric id component of an engine ``[id, type]`` handle.

    ``Combat.combat_id`` and the canonical participant arrays are integer
    fields.  The save oracle deliberately retains complete Clausewitz handles,
    so the store must project the first word instead of leaking an ``id:type``
    string (or a two-element list) into those typed replay leaves.
    """
    value = cid[0] if isinstance(cid, list) and cid else cid
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _id_pair_ids(handles: Any) -> List[int]:
    if not isinstance(handles, list):
        return []
    return [ident for handle in handles if (ident := _id_pair_id(handle)) is not None]


def _air_wing_handle_owners(snap: Dict[str, Any]) -> Dict[str, Optional[str]]:
    """Build an ambiguity-safe save-handle -> owner map for active air combat."""
    full: Dict[str, Optional[str]] = {}
    oob = snap.get("order_of_battle")
    wings = oob.get("air_wings") if isinstance(oob, dict) else None
    if not isinstance(wings, list):
        return full
    for wing in wings:
        if not isinstance(wing, dict) or wing.get("id") is None:
            continue
        key = str(wing["id"])
        candidate_owner = wing.get("owner")
        owner = candidate_owner if schema.valid_tag(candidate_owner) else None
        # A duplicate full handle is not a unique identity even if both rows
        # happen to carry the same owner.
        full[key] = owner if key not in full else None
    return full


def _combat_handle_owners(
    handles: List[Any], owners_by_handle: Dict[str, Optional[str]]
) -> tuple[List[str], bool]:
    owners: set[str] = set()
    complete = bool(handles)
    for handle in handles:
        full_key = (
            f"{handle[0]}:{handle[1]}"
            if isinstance(handle, list)
            and len(handle) == 2
            and all(
                isinstance(word, int) and not isinstance(word, bool)
                for word in handle
            )
            else None
        )
        owner = owners_by_handle.get(full_key) if full_key is not None else None
        if owner is None:
            complete = False
        else:
            owners.add(owner)
    return sorted(owners), complete


def _ingest_market(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize market.json into per-country lend_lease (equipment market) +
    market stockpiles, and a world-scoped trade-contract list (SCHEMA §3.7, §7b).

    The the game 1.19 equipment market IS the modern lend-lease mechanism. Each
    purchase_contract is a directed seller->buyer transfer; we attach it to BOTH
    parties' diplomacy.lend_lease with an explicit `direction` ("outgoing" for the
    seller, "incoming" for the buyer) so a consumer reads either side's view
    without re-walking the world list. The canonical contract objects live once,
    world-scoped, under diplomacy_world.trade_contracts (single source of truth).

    No fabrication: a contract with no resolvable seller/buyer is still listed
    world-scoped but only attached to a party whose TAG is a real string.
    """
    mk = bundle.get("market")
    if not isinstance(mk, dict):
        return

    contracts = mk.get("contracts")
    world_contracts: List[Dict[str, Any]] = []
    if isinstance(contracts, list):
        for c in contracts:
            if not isinstance(c, dict):
                continue
            cid = _id_pair_str(c.get("contract_id"))
            rec = {
                "contract_id": cid,
                "seller": c.get("seller"),
                "buyer": c.get("buyer"),
                "speed": c.get("speed"),
                "equipments": c.get("equipments") or [],
                "prices": c.get("prices") or [],
                "delivery_country": c.get("delivery_country"),
                "convoys": c.get("convoys"),
                "delivery_efficiency": c.get("delivery_efficiency"),
                "days": c.get("days"),
                "delivery_state": c.get("delivery_state"),
                "completed": c.get("completed"),
            }
            world_contracts.append(rec)
            seller, buyer = c.get("seller"), c.get("buyer")
            if isinstance(seller, str):
                ll = _ensure_country(snap, seller)["diplomacy"]["lend_lease"]
                ll.append(dict(rec, direction="outgoing", counterpart=buyer))
            if isinstance(buyer, str):
                ll = _ensure_country(snap, buyer)["diplomacy"]["lend_lease"]
                ll.append(dict(rec, direction="incoming", counterpart=seller))
    if world_contracts:
        snap["diplomacy_world"]["trade_contracts"] = world_contracts

    cdata = mk.get("countries")
    if isinstance(cdata, dict):
        for tag, c in cdata.items():
            if not isinstance(c, dict):
                continue
            dst = _ensure_country(snap, tag)
            if isinstance(c.get("market_stockpile"), list):
                dst["economy"]["market_stockpile"] = c["market_stockpile"]
            if c.get("automation") is not None:
                dst["economy"]["market_automation"] = c["automation"]


def _ingest_occupation(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold occupier-side governance (occupation.json) onto occupied STATES.

    state_detail.json already supplies each occupied state's own resistance /
    compliance scalars (ingested in _ingest_state_detail). occupation.json adds the
    occupier-side view: the per-state occupation LAW assignment and the occupier's
    default law. We only set fields that state_detail did not already provide (it is
    authoritative for the per-state resistance object); we never overwrite a measured
    value with None (no fabrication).
    """
    occ = bundle.get("occupation")
    if not isinstance(occ, dict) or not isinstance(occ.get("countries"), dict):
        return
    variants = snap.get("designs", {}).get("equipment_variants", {})

    def fnv1a32(token: str) -> int:
        value = 0x811C9DC5
        for byte in token.encode("utf-8"):
            value ^= byte
            value = (value * 0x01000193) & 0xFFFFFFFF
        return value or 1

    def resolve_equipment(rows: Any) -> Tuple[List[Dict[str, Any]], bool]:
        if not isinstance(rows, list):
            return [], False
        out = []
        complete = True
        for source_ordinal, row in enumerate(rows):
            if not isinstance(row, dict):
                complete = False
                continue
            equipment_id = row.get("equipment_id")
            equipment_type = row.get("equipment_type")
            variant = variants.get(f"{equipment_id}:{equipment_type}") if isinstance(variants, dict) else None
            token = variant.get("archetype") if isinstance(variant, dict) else None
            if not isinstance(token, str) or not token:
                complete = False
            out.append({
                "source_ordinal": row.get("source_ordinal", source_ordinal),
                "equipment_id": equipment_id,
                "equipment_type": equipment_type,
                "equipment_token": token,
                "amount": row.get("amount"),
            })
        return out, complete

    history = []
    history_complete = True
    for occupier, c in occ["countries"].items():
        if not isinstance(c, dict):
            continue
        template = c.get("garrison_template_id")
        template_id = template[0] if isinstance(template, (list, tuple)) and template else None
        for grp in (c.get("occupied") or []):
            if not isinstance(grp, dict):
                continue
            law_by_state = grp.get("occupation_law_by_state") or {}
            occupied_tag = grp.get("occupied_tag")
            garrison_by_state = grp.get("state_garrison_data") or {}
            for sid in (grp.get("states") or []):
                skey = str(sid)
                st = snap["states"].get(skey)
                if st is None:
                    st = schema.empty_state()
                    snap["states"][skey] = st
                law = law_by_state.get(skey)
                law_source = "save_occupation_law_by_state"
                if law is None:
                    law = c.get("default_law")
                    law_source = "save_controller_default_law"
                if law is not None and st.get("occupation_law") is None:
                    st["occupation_law"] = law
                # enrich the per-state occupation object (created by state_detail)
                # with the occupier-side tag, without clobbering measured fields.
                occ_obj = st.get("occupation")
                if not isinstance(occ_obj, dict):
                    occ_obj = {}
                    st["occupation"] = occ_obj
                if occupied_tag is not None and occ_obj.get("occupied_country_tag") is None:
                    occ_obj["occupied_country_tag"] = occupied_tag
                if occ_obj.get("occupier_tag") is None and isinstance(occupier, str):
                    occ_obj["occupier_tag"] = occupier
                if isinstance(law, str) and law:
                    occ_obj["occupation_law_token_id"] = fnv1a32(law)
                    occ_obj["occupation_law_token_encoding"] = "fnv1a32_of_law_token"
                    occ_obj["occupation_law_token_source"] = law_source

                garrison = garrison_by_state.get(skey)
                if garrison is None:
                    garrison = garrison_by_state.get(sid)
                if isinstance(garrison, dict):
                    rows, tokens_complete = resolve_equipment(garrison.get("equipment"))
                    equipment_meta = garrison.get("equipment_meta") or {}
                    raw_complete = equipment_meta.get("rows_complete") is True
                    occ_obj["garrison_template_id"] = template_id
                    occ_obj["garrison_army_manpower_need"] = garrison.get("army_manpower_need")
                    occ_obj["garrison_strength_ratio"] = garrison.get("strength_ratio")
                    occ_obj["garrison_required"] = garrison.get("garrison_required")
                    occ_obj["garrison_manpower_pool"] = garrison.get("manpower_pool") or []
                    occ_obj["garrison_equipment"] = rows
                    occ_obj["garrison_equipment_raw_complete"] = raw_complete
                    occ_obj["garrison_equipment_complete"] = raw_complete and tokens_complete
                    occ_obj["garrison_equipment_allow_zero_entries"] = equipment_meta.get(
                        "allow_zero_entries"
                    )
                    occ_obj["garrison_equipment_need"] = garrison.get("equipment_need") or {}
                    occ_obj["garrison_reinforcement_requests"] = (
                        garrison.get("reinforcement_requests") or []
                    )

        country_history = c.get("resistance_attack_log")
        if not isinstance(country_history, list):
            history_complete = False
            continue
        if c.get("resistance_attack_log_complete") is not True:
            history_complete = False
        for row in country_history:
            if not isinstance(row, dict):
                history_complete = False
                continue
            equipment_rows, tokens_complete = resolve_equipment(row.get("equipment_loss_rows"))
            equipment_meta = row.get("equipment_loss_meta") or {}
            row_complete = (
                row.get("row_complete") is True
                and equipment_meta.get("rows_complete") is True
                and tokens_complete
            )
            history_complete = history_complete and row_complete
            history.append({
                "stable_event_id": row.get("stable_event_id"),
                "source_ordinal": row.get("source_ordinal"),
                "controller_tag": occupier,
                "state_id": row.get("state_id"),
                "occupied_country_tag": row.get("occupied_country_tag"),
                "event_hour_or_engine_date": row.get("event_hour_or_engine_date"),
                "attack_blocked": row.get("attack_blocked"),
                "resistance_activity": row.get("resistance_activity"),
                "lost_manpower": row.get("lost_manpower"),
                "equipment_loss_rows": equipment_rows,
                "equipment_loss_children_complete": row_complete,
            })
    snap["occupation_history"] = history
    snap["occupation_history_complete"] = history_complete


def _ingest_oob(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize oob.json into top-level order_of_battle + per-country rollup.

    The parser already resolved the cross-refs (army member.unit -> division id,
    army-group -> army refs, navy-theater -> fleet refs, wing -> air_base -> state)
    and recorded resolution counts; this is a null-safe copy. Ship/wing equipment
    refs carry an (id,type) "id:type" key that joins the designs.equipment_variants
    catalog — left as keys so the consumer joins on demand (per the parser's note).
    The per-country `by_country` rollup is also mirrored onto Country.order_of_battle.
    """
    oob = bundle.get("oob")
    if not isinstance(oob, dict):
        return
    dst = snap["order_of_battle"]
    for k in ("army_groups", "armies", "fleets", "navy_theaters",
              "air_wings", "theatres"):
        if isinstance(oob.get(k), list):
            dst[k] = oob[k]
    by_country = oob.get("by_country")
    if isinstance(by_country, dict):
        dst["by_country"] = by_country
        for tag, roll in by_country.items():
            if isinstance(roll, dict):
                _ensure_country(snap, tag)["order_of_battle"] = roll


def _ingest_state_detail(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Merge state_detail.json (buildings/VP/manpower/occupation/category)."""
    sd = bundle.get("state_detail")
    if not isinstance(sd, dict) or not isinstance(sd.get("states"), dict):
        return
    for sid, det in sd["states"].items():
        if not isinstance(det, dict):
            continue
        st = snap["states"].get(sid)
        if st is None:
            st = schema.empty_state()
            snap["states"][sid] = st
        # owner/controller: detail file carries them but ownership/control files are
        # authoritative; only fill if still missing.
        if st.get("owner") is None and det.get("owner") is not None:
            st["owner"] = det.get("owner")
        if st.get("controller") is None and det.get("controller") is not None:
            st["controller"] = det.get("controller")
        if isinstance(det.get("buildings"), dict):
            st["buildings"] = det["buildings"]
        if isinstance(det.get("province_buildings"), dict):
            st["province_buildings"] = det["province_buildings"]
        # VP can be a scalar total or a per-province map — split per schema. Prefer
        # an explicit total field if the parser ever emits one (Codex 2nd-pass P3),
        # else derive from a validated {province_id: vp} map.
        if det.get("victory_points_total") is not None:
            st["victory_points_total"] = det.get("victory_points_total")
        vp = det.get("victory_points")
        if isinstance(vp, dict) and vp:
            # only treat as a province map if every value is numeric (and the
            # map is non-empty: an empty {} is meaningless, not a 0 VP total)
            if all(isinstance(v, (int, float)) for v in vp.values()):
                st["victory_points_by_province"] = vp
                if st["victory_points_total"] is None:
                    st["victory_points_total"] = sum(vp.values())
        elif isinstance(vp, (int, float)) and st["victory_points_total"] is None:
            st["victory_points_total"] = vp
        if isinstance(det.get("manpower"), dict):
            st["manpower_pool"] = det["manpower"]
        elif isinstance(det.get("manpower"), int):
            st["manpower_pool"] = {"total": det["manpower"]}
        if isinstance(det.get("resistance"), dict):
            st["occupation"] = det["resistance"]
        st["category"] = det.get("state_category") or st.get("category")
        if det.get("demilitarized") is not None:
            st["demilitarized"] = det.get("demilitarized")


def _ingest_combats(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Normalize combats.json active combats into the schema Combat shape."""
    cb = bundle.get("combats")
    if not isinstance(cb, dict) or not isinstance(cb.get("combats"), list):
        return
    out = []
    air_wing_owners = _air_wing_handle_owners(snap)
    for c in cb["combats"]:
        if not isinstance(c, dict):
            continue
        attacker = c.get("attacker") if isinstance(c.get("attacker"), dict) else {}
        defender = c.get("defender") if isinstance(c.get("defender"), dict) else {}
        attacker_handles = (
            attacker.get("units") if isinstance(attacker.get("units"), list) else []
        )
        defender_handles = (
            defender.get("units") if isinstance(defender.get("units"), list) else []
        )
        attacker_owners, attacker_owner_complete = _combat_handle_owners(
            attacker_handles, air_wing_owners
        )
        defender_owners, defender_owner_complete = _combat_handle_owners(
            defender_handles, air_wing_owners
        )
        combat_handle = c.get("combat_id")
        rec = {
            "combat_id": _id_pair_id(combat_handle),
            "kind": c.get("kind"),
            "location_province": c.get("location"),
            "attackers": _id_pair_ids(attacker_handles),
            "defenders": _id_pair_ids(defender_handles),
            "combat_width": None,
            "terrain": c.get("terrain"),
            "progress": attacker.get("progress"),
            # Preserve the complete save handles alongside the schema's
            # pointer-free integer projections.  This is especially important
            # for H-AIR: an eventual active-air fixture must retain the engine
            # object type words so opposing wing handles are not silently
            # collapsed into division-looking ids.
            "modifiers": {
                "day": c.get("day"),
                "duration": c.get("duration"),
                "save_combat_handle": combat_handle,
                "save_attacker_unit_handles": attacker_handles,
                "save_defender_unit_handles": defender_handles,
                "save_attacker_owners": attacker_owners if c.get("kind") == "air" else None,
                "save_defender_owners": defender_owners if c.get("kind") == "air" else None,
                "save_air_owner_join_complete": (
                    attacker_owner_complete and defender_owner_complete
                    if c.get("kind") == "air"
                    else None
                ),
            },
        }
        out.append(rec)
    snap["combats"] = out


def _merge_owner_controller(snap: Dict[str, Any], src: Any, set_owner: bool,
                            set_controller: bool) -> None:
    if not isinstance(src, dict):
        return
    for sid, rec in src.items():
        if not isinstance(rec, dict):
            continue
        st = snap["states"].get(sid)
        if st is None:
            st = schema.empty_state()
            snap["states"][sid] = st
        if set_owner and rec.get("owner") is not None:
            st["owner"] = rec.get("owner")
        if set_controller and rec.get("controller") is not None:
            st["controller"] = rec.get("controller")


def _ingest_states(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    # state_ownership.json carries owner (and a controller, but state_control is
    # the controller-authoritative file). Merge both; prefer control file's
    # controller, ownership file's owner.
    so = bundle.get("state_ownership")
    sc = bundle.get("state_control")
    _merge_owner_controller(snap, so, set_owner=True, set_controller=True)
    # state_control overrides controller (authoritative controller pass)
    _merge_owner_controller(snap, sc, set_owner=False, set_controller=True)


def _ingest_provinces(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    pc = bundle.get("province_control")
    if not isinstance(pc, dict):
        return
    for pid, tag in pc.items():
        prv = schema.empty_province()
        prv["controller"] = tag if isinstance(tag, str) else None
        snap["provinces"][pid] = prv


def _ingest_divisions(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    dv_root = bundle.get("divisions")
    if not isinstance(dv_root, dict):
        return
    div_list = dv_root.get("divisions")
    if not isinstance(div_list, list):
        return
    counts: Dict[str, int] = {}
    out: List[Dict[str, Any]] = []
    for d in div_list:
        if not isinstance(d, dict):
            continue
        rec = schema.empty_division()
        rec["div_id"] = d.get("div_id")
        rec["div_type"] = d.get("div_type")
        rec["owner"] = d.get("owner")
        rec["location_province"] = d.get("location_province")
        rec["state_id"] = d.get("state_id")
        rec["template_id"] = d.get("template_id")
        rec["template_type"] = d.get("template_type")
        rec["template_name"] = d.get("template_name")
        rec["template_role"] = d.get("template_role")
        rec["organisation"] = d.get("organisation")
        rec["strength"] = d.get("strength")
        if isinstance(d.get("equipment"), list):
            rec["equipment"] = d["equipment"]
        out.append(rec)
        if isinstance(rec["owner"], str):
            counts[rec["owner"]] = counts.get(rec["owner"], 0) + 1
    snap["divisions"] = out
    # division_count per country (derived)
    for tag, n in counts.items():
        c = _ensure_country(snap, tag)
        c["military"]["division_count"] = n


# ---------------------------------------------------------------------------
# Tier-1 EXTRACT ingestion (supply / strategic / events / world / country_extra /
# players / divisions_full). All save-sourced; no fabrication — a value is copied
# only when the parser emitted it; a genuine 0/0.0 is preserved (the Extract
# verifier confirmed real zeros exist, e.g. supply_gain=0, nuke amount=0).
# ---------------------------------------------------------------------------

# The per-country supply_system_2 logistics fields to fold onto economy.logistics.
# Listed explicitly so the parser's own meta/source keys never leak in, and so the
# block stays stable if the parser adds unrelated keys.
_SUPPLY_FIELDS = (
    "priority", "buffer", "wanted_supply_trucks", "capital",
    "last_supply_capital_move", "daily_losses_index", "last_lost_train_province",
    "truck_count", "truck_damage_avg", "trucks", "node_count", "node_motorization",
    "disrupted_count", "disrupted_supply", "foreign_homebase_nodes",
    "lost_railways_total", "lost_trains_total", "lost_trucks_attrition_total",
    "lost_trucks_killed_total", "lost_railways", "lost_trains",
    "lost_trucks_attrition", "lost_trucks_killed",
)


def _ingest_supply(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold supply.json per-country logistics onto countries.*.economy.logistics."""
    sup = bundle.get("supply")
    if not isinstance(sup, dict) or not isinstance(sup.get("countries"), dict):
        return
    for tag, c in sup["countries"].items():
        if not isinstance(c, dict):
            continue
        dst = _ensure_country(snap, tag)
        # copy only the present fields; absent keys stay absent (no fabrication)
        dst["economy"]["logistics"] = {k: c[k] for k in _SUPPLY_FIELDS if k in c}


def _ingest_strategic_air(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold strategic_air.json into the top-level strategic_air block."""
    sa = bundle.get("strategic_air")
    if not isinstance(sa, dict):
        return
    dst = snap["strategic_air"]
    if isinstance(sa.get("air_bases"), list):
        dst["air_bases"] = sa["air_bases"]
    if isinstance(sa.get("indices"), dict):
        dst["indices"] = sa["indices"]
    if isinstance(sa.get("per_country_region_blocks"), dict):
        dst["per_country_region_blocks"] = sa["per_country_region_blocks"]


def _ingest_strategic_operatives(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold strategic_operatives.json into top-level strategic_operatives.pools."""
    so = bundle.get("strategic_operatives")
    if not isinstance(so, dict):
        return
    if isinstance(so.get("operatives"), dict):
        snap["strategic_operatives"]["pools"] = so["operatives"]


def _ingest_raids(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold raids.json into the top-level raids block."""
    rd = bundle.get("raids")
    if not isinstance(rd, dict):
        return
    dst = snap["raids"]
    if isinstance(rd.get("counters"), dict):
        dst["counters"] = rd["counters"]
    if isinstance(rd.get("targets"), list):
        dst["targets"] = rd["targets"]
    if isinstance(rd.get("countries"), list):
        dst["countries"] = rd["countries"]


def _ingest_world_events(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold events.json into the top-level world_events block."""
    ev = bundle.get("events")
    if not isinstance(ev, dict):
        return
    dst = snap["world_events"]
    if isinstance(ev.get("fired_event_names"), list):
        dst["fired_events"] = ev["fired_event_names"]
    if isinstance(ev.get("pending_events"), list):
        dst["pending_events"] = ev["pending_events"]
    if isinstance(ev.get("threat"), list):
        dst["threat"] = ev["threat"]
    if isinstance(ev.get("sunk_convoys_history"), list):
        dst["sunk_convoys"] = ev["sunk_convoys_history"]
    if isinstance(ev.get("convoy_losses_by_killer"), dict):
        dst["convoy_losses_by_killer"] = ev["convoy_losses_by_killer"]
    if isinstance(ev.get("convoy_losses_by_owner"), dict):
        dst["convoy_losses_by_owner"] = ev["convoy_losses_by_owner"]


def _ingest_world(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold world.json (scenario / vars / flags / tech-share / power-balance /
    faction influence / tension inputs / saved targets) into top-level world."""
    w = bundle.get("world")
    if not isinstance(w, dict):
        return
    dst = snap["world"]
    if isinstance(w.get("scenario"), dict):
        dst["scenario"] = w["scenario"]
    if isinstance(w.get("variables"), dict):
        dst["variables"] = w["variables"]
    if isinstance(w.get("flags"), dict):
        dst["flags"] = w["flags"]
    if isinstance(w.get("tech_sharing_groups"), list):
        dst["tech_sharing_groups"] = w["tech_sharing_groups"]
    if isinstance(w.get("power_balances"), list):
        dst["power_balances"] = w["power_balances"]
    # parser key is faction_system; surface under the schema's faction_influence name
    if isinstance(w.get("faction_system"), list):
        dst["faction_influence"] = w["faction_system"]
    if isinstance(w.get("world_tension_inputs"), dict):
        dst["world_tension_inputs"] = w["world_tension_inputs"]
    if isinstance(w.get("saved_event_targets"), list):
        dst["saved_event_targets"] = w["saved_event_targets"]


# The per-country country_extra fields to surface under countries.*.extra. Listed
# explicitly so parser meta keys never leak in.
_EXTRA_FIELDS = (
    "nukes_total", "nukes_ready_total", "nukes", "dynamic_modifiers",
    "ace_count", "aces", "reinforcement_priority", "deployment_blocks",
    "special_projects", "experience_status", "script_flags", "convoys_destroyed",
    "accidents_score", "cosmetic_tag", "pride_of_the_fleet",
    "collaboration_present", "strategic_navy",
)


def _ingest_country_extra(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold country_extra.json onto its canonical country namespaces.

    ``cores`` and ``claims`` are authoritative state memberships, not misc
    metadata.  Keep them distinct under ``identity`` so save baselines and the
    live compact carrier converge on the same replay paths.
    """
    ce = bundle.get("country_extra")
    if not isinstance(ce, dict) or not isinstance(ce.get("countries"), dict):
        return
    for tag, c in ce["countries"].items():
        if not isinstance(c, dict):
            continue
        dst = _ensure_country(snap, tag)
        dst["extra"] = {k: c[k] for k in _EXTRA_FIELDS if k in c}
        identity = dst.setdefault("identity", {})
        for field in ("cores", "claims"):
            value = c.get(field)
            if isinstance(value, list) and all(
                isinstance(state_id, int) and not isinstance(state_id, bool)
                for state_id in value
            ):
                identity[field] = sorted(set(value))


def _ingest_players(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold players.json persona name onto countries.*.human_name (MP only)."""
    pl = bundle.get("players")
    if not isinstance(pl, dict) or not isinstance(pl.get("players"), dict):
        return
    for tag, p in pl["players"].items():
        if not isinstance(p, dict):
            continue
        name = p.get("user")
        if isinstance(name, str) and name:
            _ensure_country(snap, tag)["human_name"] = name


# ---------------------------------------------------------------------------
# WAVE-2 NEW save deep ingests (market_extra / abilities / projects / deployment).
# All save-sourced (S); no fabrication — a missing file leaves the field absent.
# ---------------------------------------------------------------------------
# Per-country projects fields to surface under countries.*.projects (allow-list so
# parser meta never leaks; mirrors _EXTRA_FIELDS).
_PROJECT_FIELDS = (
    "projects_total", "projects_started", "projects_completed", "projects_active",
    "active_projects", "completed_projects", "projects", "facilities",
    "breakthrough",
)


def _ingest_market_extra(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold market_extra.json: per-country price bands + subsidies (economy.*),
    contract-scoped price levels + pending requests (diplomacy_world.*). All S.

    NON-DUPLICATION: market.json (already ingested by _ingest_market) carries the
    contracts/stockpile/automation; this adds ONLY the price-level/subsidy/request
    state that parser skipped. Empty price_levels (auto-price world) are skipped."""
    mx = bundle.get("market_extra")
    if not isinstance(mx, dict):
        return
    cdata = mx.get("countries")
    if isinstance(cdata, dict):
        for tag, rec in cdata.items():
            if not isinstance(rec, dict):
                continue
            dst = _ensure_country(snap, tag)["economy"]
            pls = rec.get("price_levels")
            if isinstance(pls, list) and pls:
                dst["market_price_levels"] = pls
            subs = rec.get("subsidies")
            if isinstance(subs, list) and subs:
                dst["market_subsidies"] = subs
    contracts = mx.get("contracts")
    if isinstance(contracts, list):
        pl_contracts = [
            # contract_id normalized via _id_pair_str so these price bands share
            # the 'id:type' key space with diplomacy_world.trade_contracts.
            {"contract_id": _id_pair_str(c.get("contract_id")),
             "seller": c.get("seller"), "buyer": c.get("buyer"),
             "price_levels": c.get("price_levels")}
            for c in contracts
            if isinstance(c, dict) and c.get("price_levels")
        ]
        if pl_contracts:
            snap["diplomacy_world"]["market_price_levels"] = pl_contracts
    reqs = mx.get("requests")
    if isinstance(reqs, list) and reqs:
        snap["diplomacy_world"]["market_requests"] = reqs


def _ingest_abilities(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold abilities.json into countries.*.abilities (aces + commander cooldowns +
    officer corps). All S except the two officer-corps capture tallies (D, but the
    parser already aggregated them — copied as-is). Missing file -> field absent."""
    ab = bundle.get("abilities")
    if not isinstance(ab, dict):
        return
    aces = ab.get("aces") if isinstance(ab.get("aces"), dict) else {}
    cmdr = (ab.get("commander_abilities")
            if isinstance(ab.get("commander_abilities"), dict) else {})
    oc = ab.get("officer_corps") if isinstance(ab.get("officer_corps"), dict) else {}
    tags = set(aces) | set(cmdr) | set(oc)
    for tag in tags:
        # skip unresolvable placeholder keys (e.g. "?") so no junk tag is
        # created: schema.valid_tag is the single source of truth for TAGs.
        if not schema.valid_tag(tag):
            continue
        dst = _ensure_country(snap, tag)
        dst["abilities"] = {
            "aces": (aces.get(tag, {}) or {}).get("aces", []),
            "commander_cooldowns": cmdr.get(tag, []),
            "officer_corps": oc.get(tag),
        }


def _ingest_projects(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold projects.json: per-country projects/facilities/breakthrough body onto
    countries.*.projects (allow-list), and the per-TAG scientist roster onto
    countries.*.scientists. All S except two cross-ref join fields (D). Missing
    file -> fields absent (no fabrication)."""
    pj = bundle.get("projects")
    if not isinstance(pj, dict):
        return
    cdata = pj.get("countries")
    if isinstance(cdata, dict):
        for tag, c in cdata.items():
            if not isinstance(c, dict):
                continue
            dst = _ensure_country(snap, tag)
            dst["projects"] = {k: c[k] for k in _PROJECT_FIELDS if k in c}
    scientists = pj.get("scientists")
    if isinstance(scientists, dict):
        for tag, lst in scientists.items():
            if isinstance(lst, list):
                _ensure_country(snap, tag)["scientists"] = lst


def _ingest_deployment(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Fold deployment.json: per-country deploy conveyor + reinforcement priority
    onto countries.*.military, and the owner!=controller expeditionary pairing onto
    diplomacy_world.expeditionary (+ per-owner diplomacy.expeditionary). All S.

    ROBUSTNESS: the expeditionary list is [] in many saves (no lent forces); the
    ingest no-ops cleanly on the empty list."""
    dp = bundle.get("deployment")
    if not isinstance(dp, dict):
        return
    cdata = dp.get("countries")
    if isinstance(cdata, dict):
        for tag, rec in cdata.items():
            if not isinstance(rec, dict):
                continue
            mil = _ensure_country(snap, tag)["military"]
            conv = rec.get("conveyors")
            # Empty is authoritative only when the parser positively completed
            # this country's full conveyor vector.  A missing completeness bit
            # must remain unreadable/None rather than erase a prior queue.
            if rec.get("queue_complete") is True and isinstance(conv, list):
                mil["deployment_queue"] = conv
            rp = rec.get("reinforcement_priority")
            if rp is not None:
                mil["reinforcement_priority"] = rp
    exped = dp.get("expeditionary")
    if isinstance(exped, list) and exped:
        snap["diplomacy_world"]["expeditionary"] = exped
        by_owner: Dict[str, List[Dict[str, Any]]] = {}
        for e in exped:
            if not isinstance(e, dict):
                continue
            owner = e.get("owner")
            if not isinstance(owner, str):
                continue
            by_owner.setdefault(owner, []).append(
                {"controller": e.get("controller"),
                 "divisions": e.get("divisions", [])})
        for owner, lst in by_owner.items():
            _ensure_country(snap, owner)["diplomacy"]["expeditionary"] = lst


# divisions_full additive fields to merge onto an existing division (the 9 already
# in divisions.json are NOT re-copied — they are authoritative from _ingest_divisions).
_DIVFULL_FIELDS = (
    "script_id", "division_name", "experience", "max_supply",
    "army_current_supply_ratio", "supply_gain", "motorization_level",
    "move_priority", "last_combat_date", "dig_in", "dig_in_cap",
    "acclimatization", "army_manpower", "killed", "killer_definition",
    "equipment_archetypes", "has_officer",
)


def _ingest_divisions_full(bundle: OracleBundle, snap: Dict[str, Any]) -> None:
    """Merge divisions_full.json's additive per-division body onto divisions[].

    Joined by full Clausewitz identity (div_id, div_type). divisions.json was already
    ingested into snap["divisions"]; this enriches each matched record with the extra
    LITERAL save fields. A divisions_full record with no matching divisions.json entry
    is appended as a new division (it still carries div_id/div_type/owner/location).
    No fabrication: only present fields are copied; a genuine 0/0.0 is preserved."""
    dvf = bundle.get("divisions_full")
    if not isinstance(dvf, dict):
        return
    full_list = dvf.get("divisions")
    if not isinstance(full_list, list):
        return

    def _uid(d: Dict[str, Any]):
        return (d.get("div_id"), d.get("div_type"))

    index = {_uid(d): d for d in snap["divisions"] if isinstance(d, dict)}
    counts_dirty = False
    for f in full_list:
        if not isinstance(f, dict):
            continue
        key = _uid(f)
        if key[0] is None:
            continue
        rec = index.get(key)
        if rec is None:
            # a division present only in divisions_full -> create a fresh record so its
            # body is not dropped (carries identity + the additive fields).
            rec = schema.empty_division()
            rec["div_id"] = f.get("div_id")
            rec["div_type"] = f.get("div_type")
            rec["owner"] = f.get("owner") if isinstance(f.get("owner"), str) else None
            rec["location_province"] = f.get("location_province")
            rec["template_id"] = f.get("template_id")
            rec["template_type"] = f.get("template_type")
            rec["organisation"] = f.get("organisation")
            rec["strength"] = f.get("strength")
            snap["divisions"].append(rec)
            index[key] = rec
            counts_dirty = True
        for fld in _DIVFULL_FIELDS:
            if fld in f:
                rec[fld] = f[fld]

    # If divisions_full introduced divisions absent from divisions.json, refresh the
    # per-country division_count so it stays a measured truth (Codex-style honesty).
    if counts_dirty:
        recount: Dict[str, int] = {}
        for d in snap["divisions"]:
            o = d.get("owner")
            if isinstance(o, str):
                recount[o] = recount.get(o, 0) + 1
        for tag, n in recount.items():
            _ensure_country(snap, tag)["military"]["division_count"] = n


def _fill_map_layers(snap: Dict[str, Any]) -> None:
    owner_by_state = {sid: st.get("owner") for sid, st in snap["states"].items()
                      if st.get("owner") is not None}
    control_by_province = {pid: prv.get("controller")
                           for pid, prv in snap["provinces"].items()
                           if prv.get("controller") is not None}
    snap["map"]["owner_by_state"] = owner_by_state or None
    snap["map"]["control_by_province"] = control_by_province or None


def _fill_provenance(bundle: OracleBundle, snap: Dict[str, Any],
                     save_date: Optional[str], save_version: Optional[str],
                     source_filename: Optional[str],
                     memory_dir: Optional[str]) -> None:
    save_domains = []
    domain_map = {
        "country_snapshot": "country_snapshot",
        "economy": "economy",
        "production": "production",
        "tech": "tech",
        "focus": "focus",
        "diplomacy": "diplomacy",
        "state_ownership": "state_ownership",
        "state_control": "state_control",
        "state_detail": "state_detail",
        "province_control": "province_control",
        "divisions": "divisions",
        "combats": "combats",
        "characters": "commanders",
        "designs": "designs",
        "oob": "order_of_battle",
        "intel": "intel",
        "market": "market",
        "occupation": "occupation",
        # Tier-1 EXTRACT save domains
        "supply": "supply_logistics",
        "strategic_air": "strategic_air",
        "strategic_operatives": "strategic_operatives",
        "raids": "raids",
        "events": "world_events",
        "world": "world",
        "country_extra": "country_extra",
        "players": "players",
        # WAVE-2 NEW save deep domains
        "abilities": "abilities",
        "projects": "projects",
        "market_extra": "market_extra",
        "deployment": "deployment",
    }
    for fkey, dom in domain_map.items():
        obj = bundle.get(fkey)
        if obj:  # present and non-empty
            save_domains.append(dom)
    # divisions_full enriches the existing 'divisions' domain (already listed when
    # divisions.json is present); it does not add a separate provenance domain.

    prov = snap["provenance"]
    prov["save"] = {
        "captured": bool(save_domains),
        "source_filename": source_filename,  # basename only
        "save_date": save_date,
        "domains": save_domains,
    }
    # No live-memory snapshot ingested yet (offset drafts only). Stay honest.
    prov["memory"] = None
    notes = []
    if memory_dir and os.path.isdir(memory_dir):
        notes.append(
            "memory_reader present as offset DRAFTS only; no per-timestamp live "
            "reads ingested — memory-source fields remain null (no fabrication)."
        )
    prov["notes"] = notes


# ---------------------------------------------------------------------------
# Snapshot store (per-game indexed directory).
# ---------------------------------------------------------------------------
def _hour_key(hour_index: Optional[int]) -> str:
    """Zero-padded hour_index for chronological filename sort. None -> 'unknown'."""
    if hour_index is None:
        return "unknown"
    return f"{hour_index:012d}"


class SnapshotStore:
    """Per-game on-disk store under a snapshots root (default replay_interface/replay/snapshots)."""

    def __init__(self, root: Optional[str] = None):
        if root is None:
            root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "snapshots")
        self.root = root

    def game_dir(self, game_id: str) -> str:
        return os.path.join(self.root, game_id)

    def index_path(self, game_id: str) -> str:
        return os.path.join(self.game_dir(game_id), "index.json")

    def write_snapshot(self, snap: Dict[str, Any]) -> str:
        """Persist one snapshot and update the game index. Returns the file path."""
        problems = schema.validate_snapshot(snap)
        if problems:
            raise ValueError("refusing to write invalid snapshot:\n  " +
                             "\n  ".join(problems[:20]))
        game_id = snap["game_id"]
        gdir = self.game_dir(game_id)
        os.makedirs(gdir, exist_ok=True)
        key = _hour_key(snap.get("hour_index"))
        fname = f"{key}.snapshot.json"
        path = os.path.join(gdir, fname)
        _atomic_write_json(path, snap, separators=(",", ":"))
        self._update_index(game_id, snap, fname)
        return path

    def _update_index(self, game_id: str, snap: Dict[str, Any], fname: str) -> None:
        idx_path = self.index_path(game_id)
        idx = _read_json_opt(idx_path) or {"game_id": game_id, "schema_version":
                                            schema.SCHEMA_VERSION, "snapshots": []}
        entry = {
            "hour_index": snap.get("hour_index"),
            "date": snap.get("date"),
            "file": fname,
            "save_version": snap.get("save_version"),
            "counts": {
                "countries": len(snap.get("countries", {})),
                "states": len(snap.get("states", {})),
                "provinces": len(snap.get("provinces", {})),
                "divisions": len(snap.get("divisions", [])),
            },
        }
        # replace any existing entry with the same file/hour, then re-sort.
        # (Filename is derived solely from hour_index, so hour_index is None iff
        # file == "unknown.snapshot.json"; the file check already subsumes the
        # None case, making the old `or entry["hour_index"] is None` clause dead.)
        idx["snapshots"] = [e for e in idx["snapshots"]
                            if e.get("file") != fname and
                            e.get("hour_index") != entry["hour_index"]]
        idx["snapshots"].append(entry)
        idx["snapshots"].sort(key=lambda e: (e.get("hour_index") is None,
                                             e.get("hour_index") or 0))
        _atomic_write_json(idx_path, idx, indent=2)

    def list_snapshots(self, game_id: str) -> List[Dict[str, Any]]:
        idx = _read_json_opt(self.index_path(game_id))
        return (idx or {}).get("snapshots", [])

    def load_snapshot(self, game_id: str, file_or_hour: Any) -> Optional[Dict[str, Any]]:
        gdir = self.game_dir(game_id)
        if isinstance(file_or_hour, int):
            fname = f"{_hour_key(file_or_hour)}.snapshot.json"
        else:
            fname = str(file_or_hour)
        return _read_json_opt(os.path.join(gdir, fname))

    def load_all_ordered(self, game_id: str) -> List[Dict[str, Any]]:
        out = []
        for e in self.list_snapshots(game_id):
            snap = self.load_snapshot(game_id, e["file"])
            if snap is not None:
                out.append(snap)
        return out

    def write_timeline(self, game_id: str, events: List[Dict[str, Any]]) -> str:
        gdir = self.game_dir(game_id)
        os.makedirs(gdir, exist_ok=True)
        path = os.path.join(gdir, "timeline.json")
        _atomic_write_json(
            path,
            {"game_id": game_id, "schema_version": schema.SCHEMA_VERSION,
             "event_count": len(events), "events": events}, indent=2)
        return path


def ingest_oracle_dir(output_dir: str, game_id: str, *,
                      store_root: Optional[str] = None,
                      memory_dir: Optional[str] = None) -> Tuple[Dict[str, Any], str]:
    """Convenience: build a snapshot from an oracle output dir and write it.

    Returns (snapshot, written_path).
    """
    snap = build_snapshot_from_oracle(output_dir, game_id, memory_dir=memory_dir)
    store = SnapshotStore(store_root)
    path = store.write_snapshot(snap)
    return snap, path


__all__ = [
    "OracleBundle", "SnapshotStore", "build_snapshot_from_oracle",
    "ingest_oracle_dir",
]
