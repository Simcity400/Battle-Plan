#!/usr/bin/env python3
"""the game save -> EQUIPMENT MARKET / TRADE oracle (``market.json``).

Deep-layer extension of the save oracle. In the game 1.19 the *equipment market* is
the engine's actual cross-country equipment-transfer mechanism (the modern
successor to / superset of classic "lend-lease": one country buys/leases
equipment from another, delivered by convoy). The save persists it in two
places, BOTH of which this module extracts:

  1. TOP-LEVEL ``equipment_market.contracts.purchase_contract[]`` — the live
     trade contracts. Per contract we extract:
       - ``contract_id``        : the contract (id,type).
       - ``seller`` / ``buyer`` : the two tags.
       - ``equipments``         : [{equip_id, equip_type, archetype, amount}]
                                  (the equipment being transferred).
       - ``speed``              : transfer-speed setting.
       - ``prices``             : [{equip_id, equip_type, archetype, amount}]
                                  (unit price level per equipment).
       - ``convoys``            : {assigned, total} from the delivery route.
       - ``delivery_country``   : tag operating the delivery convoys.
       - ``days``               : days the contract has run.
       - ``delivery_state``     : {factory_cic_progress, equipment_transfer_progress,
                                   collected} (current transfer progress).
       - ``completed``          : completed-counter from contract_meta.

  2. PER-COUNTRY ``countries.<TAG>.equipment_market.market_stockpile.equipments``
     — equipment the country has placed ON the market (its sellable stockpile),
     joined to archetype names. Plus the country's market-request automation
     flags (auto_accept_market_access / auto_send_market_access /
     auto_accept_purchase) when present.

Archetype names are resolved from the top-level ``equipments`` index (same join
parse_production.py uses); an unresolved (id,type) keeps ``archetype=null`` and
is counted in ``stats.unknown_equipment_ids`` (NO fabricated default).

WHY this is a rehost backstop: when the host crashes mid-session,
in-flight equipment transfers (who is supplying whom, how much, delivery
progress) are NOT reconstructable from the live-memory daily layer alone after a
rehost; the save is the authoritative record of the trade graph. parse_oob.py
only *mentions* lend-lease (expeditionary control); no module actually extracted
the market until now.

stdlib only. Reuses the parse_save tokenizer via deep_common.

Verified against a real 1.19 melt (paused 1946.5.18, DDR world) and refuted with
an adversarial Codex pass.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, to_int, to_float, run_cli, top_level_pass,
)



def _parse_equipments_index(tz, index):
    """equipments={ <archetype>={ id={id type} creator obsolete is_frame } }."""
    for arch_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        eid = etype = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    eid, etype = read_id_block(tz)
                else:
                    _skip_block_body(tz)
                continue
            atom_value(op)
        if eid is not None:
            index[(eid, etype)] = arch_name



def _parse_equipment_amounts(tz):
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "equipment":
                eid = etype = None
                amount = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "id":
                            eid, etype = read_id_block(tz)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "amount":
                        amount = to_float(v)
                if eid is not None:
                    out.append((eid, etype, amount))
            else:
                _skip_block_body(tz)
    return out



def _parse_delivery_route(tz):
    """delivery_route_handler={ convoy_client={ country convoys_subscriber } ... }."""
    rec = {"delivery_country": None, "convoys_assigned": None, "convoys_total": None,
           "efficiency": None, "days": None}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "convoy_client":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "convoys_subscriber":
                            for kk, oo in iter_block_entries(tz):
                                v = atom_value(oo) if oo is not LBRACE else None
                                if oo is LBRACE:
                                    _skip_block_body(tz)
                                    continue
                                if kk == "convoys":
                                    rec["convoys_assigned"] = to_int(v)
                                elif kk == "total":
                                    rec["convoys_total"] = to_int(v)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k == "country":
                        rec["delivery_country"] = v
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "efficiency":
            rec["efficiency"] = to_float(v)
        elif key == "days":
            rec["days"] = to_int(v)
    return rec


def _parse_contract_draft(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "equipments":
                rec["equipments_raw"] = _parse_equipment_amounts(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "seller":
            rec["seller"] = v
        elif key == "buyer":
            rec["buyer"] = v
        elif key == "speed":
            rec["speed"] = to_int(v)


def _parse_contract_definition(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "contract_draft":
                _parse_contract_draft(tz, rec)
            elif key == "prices":
                rec["prices_raw"] = _parse_equipment_amounts(tz)
            else:
                _skip_block_body(tz)


def _parse_delivery_state(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "factory_cic_progress":
            rec["delivery_state"]["factory_cic_progress"] = to_float(v)
        elif key == "equipment_transfer_progress":
            rec["delivery_state"]["equipment_transfer_progress"] = to_float(v)
        elif key == "collected":
            rec["delivery_state"]["collected"] = to_float(v)


def _parse_contract_meta(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "completed":
            rec["completed"] = to_int(v)
        elif key == "convoys":
            rec["meta_convoys"] = to_int(v)


def _parse_purchase_contract(tz):
    rec = {
        "contract_id": None, "seller": None, "buyer": None, "speed": None,
        "equipments_raw": [], "prices_raw": [],
        "delivery_country": None, "convoys_assigned": None, "convoys_total": None,
        "efficiency": None, "days": None,
        "delivery_state": {"factory_cic_progress": None,
                           "equipment_transfer_progress": None, "collected": None},
        "completed": None, "meta_convoys": None,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id":
                rec["contract_id"] = read_id_block(tz)
            elif key == "contract_definition":
                _parse_contract_definition(tz, rec)
            elif key == "delivery_route_handler":
                dr = _parse_delivery_route(tz)
                rec["delivery_country"] = dr["delivery_country"]
                rec["convoys_assigned"] = dr["convoys_assigned"]
                rec["convoys_total"] = dr["convoys_total"]
                rec["efficiency"] = dr["efficiency"]
            elif key == "contract_delivery_state":
                _parse_delivery_state(tz, rec)
            elif key == "contract_meta":
                _parse_contract_meta(tz, rec)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "days":
            rec["days"] = to_int(v)
    return rec


def _parse_contracts(tz, contracts):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "purchase_contract":
                contracts.append(_parse_purchase_contract(tz))
            else:
                _skip_block_body(tz)


def _parse_top_market(tz, contracts):
    """top-level equipment_market={ contracts={ purchase_contract[] } ... }."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "contracts":
                _parse_contracts(tz, contracts)
            else:
                _skip_block_body(tz)



def _parse_country_market(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "market_stockpile":
                rec["stockpile_present"] = True
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "equipments":
                            rec["stockpile_raw"] = _parse_equipment_amounts(tz)
                        else:
                            _skip_block_body(tz)
            elif key == "market_request_automation":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "option":
                            for kk, oo in iter_block_entries(tz):
                                if oo is LBRACE:
                                    _skip_block_body(tz)
                                    continue
                                v = atom_value(oo)
                                if v is None:
                                    continue
                                rec["automation"][kk] = (v == "yes")
                        else:
                            _skip_block_body(tz)
            else:
                _skip_block_body(tz)


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "market_present": False,
            "stockpile_present": False,
            "stockpile_raw": [],
            "automation": {},
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "equipment_market":
                    rec["market_present"] = True
                    _parse_country_market(tz, rec)
                else:
                    _skip_block_body(tz)
        if rec["market_present"]:
            by_tag[tag] = rec



def _join(raw, arch_index, unknown):
    out = []
    for eid, etype, amount in raw:
        name = arch_index.get((eid, etype))
        if name is None:
            unknown.add((eid, etype))
        out.append({"equip_id": eid, "equip_type": etype,
                    "archetype": name, "amount": amount})
    return out


def build(text, source, was_melted):
    arch_index: dict = {}
    contracts_raw: list = []
    by_tag: dict = {}

    meta = top_level_pass(
        text,
        {
            "equipments": lambda tz: _parse_equipments_index(tz, arch_index),
            "equipment_market": lambda tz: _parse_top_market(tz, contracts_raw),
            "countries": lambda tz: _parse_countries(tz, by_tag),
        },
        scalar_keys=("date", "version", "player",
                     "cached_active_trade_route_count"),
    )

    unknown: set = set()

    contracts = []
    for c in contracts_raw:
        contracts.append({
            "contract_id": c["contract_id"],
            "seller": c["seller"],
            "buyer": c["buyer"],
            "speed": c["speed"],
            "equipments": _join(c["equipments_raw"], arch_index, unknown),
            "prices": _join(c["prices_raw"], arch_index, unknown),
            "delivery_country": c["delivery_country"],
            "convoys": {"assigned": c["convoys_assigned"],
                        "total": c["convoys_total"]},
            "delivery_efficiency": c["efficiency"],
            "days": c["days"],
            "delivery_state": c["delivery_state"],
            "completed": c["completed"],
        })

    countries = {}
    for tag, rec in by_tag.items():
        stock = _join(rec["stockpile_raw"], arch_index, unknown)
        countries[tag] = {
            "market_stockpile": stock,
            "market_stockpile_entries": len(stock),
            "market_stockpile_complete": rec["stockpile_present"],
            "automation": rec["automation"] or None,
        }

    n_stock = sum(c["market_stockpile_entries"] for c in countries.values())
    n_stockpile_present = sum(c["market_stockpile_complete"] for c in countries.values())
    n_stockpile_empty = sum(
        c["market_stockpile_complete"] and c["market_stockpile_entries"] == 0
        for c in countries.values()
    )

    out = {
        "meta": {
            "source_filename": source.name,
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "cached_active_trade_route_count":
                meta.get("cached_active_trade_route_count"),
            "notes": {
                "equipment_market": "the game 1.19 equipment market = the engine's "
                "cross-country equipment-transfer mechanism (successor to/superset "
                "of classic lend-lease). Top-level purchase_contract[] are the live "
                "trade contracts; per-country market_stockpile is what each country "
                "has placed for sale.",
                "archetype_join": "(equip_id,equip_type) joined to the top-level "
                "equipments index, same as production.json. Unresolved ids keep "
                "archetype=null; counted in unknown_equipment_ids (never defaulted).",
                "peace_conference": "NOT here — see intel.json. peace_conference is "
                "EMPTY ({}) in this save (no active conference at pause).",
            },
            "stats": {
                "purchase_contracts": len(contracts),
                "countries_with_market_data": len(countries),
                "countries_with_market_stockpile": n_stockpile_present,
                "empty_market_stockpiles": n_stockpile_empty,
                "total_market_stockpile_entries": n_stock,
                "unknown_equipment_ids": len(unknown),
            },
        },
        "contracts": contracts,
        "countries": dict(sorted(countries.items())),
    }

    lines = ["=== EQUIPMENT MARKET / TRADE ORACLE VALIDATION ==="]
    lines.append(f"source           : {source.name} (melted={was_melted})")
    lines.append(f"save date        : {meta.get('date')}")
    lines.append(f"purchase contracts: {len(contracts)}")
    lines.append(f"countries w/ market data: {len(countries)}  "
                 f"stockpile entries={n_stock}  unknown ids={len(unknown)}")
    for c in contracts[:5]:
        eq = c["equipments"][0] if c["equipments"] else {}
        lines.append(f"   {c['seller']} -> {c['buyer']}: "
                     f"{eq.get('archetype')} x{eq.get('amount')} "
                     f"(convoys {c['convoys']['assigned']}/{c['convoys']['total']}, "
                     f"day {c['days']})")
    return {"market.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(run_cli(
        "market", "the game save -> equipment market / trade (lend-lease) oracle.",
        build, ["market.json"]))
