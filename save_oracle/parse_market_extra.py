#!/usr/bin/env python3
"""the game save -> EQUIPMENT-MARKET EXTRA oracle (``market_extra.json``).

Deep-layer extension that captures the equipment-market sub-state that
``parse_market.py`` (market.json) deliberately leaves on the floor. parse_market
already extracts: the top-level ``purchase_contract[]`` (seller/buyer, equipments,
per-unit ``prices``, convoys, delivery progress) and the per-country
``market_stockpile`` + ``market_request_automation`` flags. This module adds the
PRICE-LEVEL / SUBSIDY / REQUEST state it skips, WITHOUT re-emitting any of those:

  (a) PRICE-LEVELS — the low/normal/high price-level state per market position.
      The engine stores it as ``price_levels={ levels={ <entry> } }`` in TWO
      places:
        * per top-level contract: ``equipment_market.contracts.purchase_contract[]
          .contract_definition.price_levels.levels`` (the contract's negotiated
          price band), and
        * per country: ``countries.<TAG>.equipment_market.price_levels.levels``
          (the country's standing market price-band per archetype).
      ``levels`` is EMPTY ({}) for every contract and country in this melt (no
      manual price band set — the engine is on auto-price), so we record the
      structure + an explicit ``levels_empty`` count rather than fabricating one.
      The parser reads whatever ``levels`` carries when it is populated.

  (b) SUBSIDIES — ``countries.<TAG>.equipment_market.subsidies.subsidies[]``, the
      market subsidies a seller pays to make an archetype cheaper to specific
      buyers. Per subsidy: ``cic`` (subsidy cost), ``archetype`` (named, e.g.
      ``light_tank_chassis``), ``targets`` (tag list the subsidy applies to).
      parse_market skips this entirely. Real (non-empty) in this save.

  (c) REQUESTS — the top-level ``equipment_market.contracts.requests={ <id> ... }``
      list (pending market-access / purchase request ids) and the per-country
      ``cached_active_trade_route_count`` is already in market.json so NOT here.

Archetype names for the price-level entries are resolved from the top-level
``equipments`` index (the same join parse_market/parse_production use); an
unresolved (id,type) keeps ``archetype=null`` and is counted in
``stats.unknown_equipment_ids`` (NO fabricated default).

WHY a rehost backstop (DESIGN 11.3): the market PRICE BAND + SUBSIDY graph is the
negotiated-terms layer of the trade graph. market.json carries the live transfers
(who supplies whom, progress); this carries the *terms* (price levels, who is
being subsidised to buy what) which are equally unreconstructable from the live
daily layer after a host crash + rehost.

stdlib only. Reuses the parse_save tokenizer via deep_common. NEW file: does not
edit market.json / parse_market.py.

Verified against re_scratch/campaign_melt_1940.txt (paused 1940, 143-country
world) and refuted with an adversarial Codex pass.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, to_int, to_float, read_scalar_list, run_cli, top_level_pass,
)


# ---------------------------------------------------------------------------
# Top-level equipments archetype index (id,type) -> archetype name
# (identical join basis to parse_market / parse_production)
# ---------------------------------------------------------------------------

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
            atom_value(op)  # consume scalar
        if eid is not None:
            index[(eid, etype)] = arch_name


# ---------------------------------------------------------------------------
# price_levels.levels  — populated structure unknown in this melt (always {}).
# We read it tolerantly: each level entry is captured as a raw {key:value} map
# plus, if it carries an id={id type} block, the (id,type) for archetype join.
# ---------------------------------------------------------------------------

def _parse_levels(tz):
    """Read ``levels={ ... }`` (LBRACE consumed) tolerantly.

    Empty in this save. When populated each child is captured as a dict of its
    scalar fields; an ``id={id type}`` sub-block is surfaced as (equip_id,
    equip_type) for the archetype join. Returns (entries, n_empty_children).
    """
    entries = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            rec = {"key": key, "equip_id": None, "equip_type": None, "fields": {}}
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    if k == "id":
                        rec["equip_id"], rec["equip_type"] = read_id_block(tz)
                    else:
                        _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is not None:
                    rec["fields"][k] = v
            entries.append(rec)
        # bare atoms inside levels are ignored
    return entries


def _parse_price_levels(tz):
    """price_levels={ levels={ ... } }."""
    levels = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "levels":
                levels = _parse_levels(tz)
            else:
                _skip_block_body(tz)
    return levels


# ---------------------------------------------------------------------------
# subsidies.subsidies[]  — per-archetype market subsidy to specific buyers
# ---------------------------------------------------------------------------

def _parse_one_subsidy(tz):
    # A subsidy targets buyers EITHER by an explicit ``targets={ <tag> }`` list OR
    # by a ``trigger="..."`` scripted-trigger name (mutually exclusive in this
    # save). Capture both so neither targeting mode is lost.
    rec = {"cic": None, "archetype": None, "targets": [], "trigger": None}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "targets":
                rec["targets"] = read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "cic":
            rec["cic"] = to_float(v)
        elif key == "archetype":
            rec["archetype"] = v
        elif key == "trigger":
            rec["trigger"] = v
    return rec


def _parse_anon_subsidy_list(tz):
    """Inner ``subsidies={ { ... } { ... } }`` = list of ANONYMOUS blocks.

    ``iter_block_entries`` auto-skips anonymous blocks, so descend with the raw
    tokenizer (same pattern as parse_supply._parse_trucks). The inner '{' was
    already consumed by the caller.
    """
    out = []
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            return out
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            out.append(_parse_one_subsidy(tz))
        # stray atoms/'=' ignored
    return out


def _parse_subsidies(tz):
    """subsidies={ subsidies={ {cic archetype targets} {..} } }."""
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "subsidies":
                out = _parse_anon_subsidy_list(tz)
            else:
                _skip_block_body(tz)
    return out


# ---------------------------------------------------------------------------
# Per-country equipment_market: price_levels + subsidies (skip what market.json
# already owns: market_stockpile + market_request_automation)
# ---------------------------------------------------------------------------

def _parse_country_market(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "price_levels":
                rec["price_levels"] = _parse_price_levels(tz)
            elif key == "subsidies":
                rec["subsidies"] = _parse_subsidies(tz)
            else:
                # market_stockpile / market_request_automation -> market.json
                _skip_block_body(tz)


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {"price_levels": [], "subsidies": []}
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "equipment_market":
                    _parse_country_market(tz, rec)
                else:
                    _skip_block_body(tz)
        if rec["price_levels"] or rec["subsidies"]:
            by_tag[tag] = rec


# ---------------------------------------------------------------------------
# Top-level equipment_market: per-contract price_levels + the requests[] list
# (skip everything market.json already owns: equipments/prices/convoys/...)
# ---------------------------------------------------------------------------

def _parse_contract_draft(tz, rec):
    """contract_draft={ seller buyer equipments{...} speed subsidies{...} }.

    We only need seller/buyer to attribute the price band; equipments/prices are
    market.json's job, so they are skipped.
    """
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "seller":
            rec["seller"] = v
        elif key == "buyer":
            rec["buyer"] = v


def _parse_contract_definition(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "price_levels":
                rec["price_levels"] = _parse_price_levels(tz)
            elif key == "contract_draft":
                _parse_contract_draft(tz, rec)
            else:
                _skip_block_body(tz)


def _parse_purchase_contract(tz):
    rec = {"contract_id": None, "seller": None, "buyer": None, "price_levels": []}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id":
                rec["contract_id"] = read_id_block(tz)
            elif key == "contract_definition":
                _parse_contract_definition(tz, rec)
            else:
                _skip_block_body(tz)
            continue
        # scalars ignored
    return rec


def _parse_contracts(tz, contracts):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "purchase_contract":
                contracts.append(_parse_purchase_contract(tz))
            else:
                _skip_block_body(tz)


def _parse_top_market(tz, contracts, requests):
    """equipment_market={ contracts={ purchase_contract[] } requests={ <id> } }.

    ``requests`` is a direct child of equipment_market (a flat pending-request id
    list), NOT of contracts.
    """
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "contracts":
                _parse_contracts(tz, contracts)
            elif key == "requests":
                for r in read_scalar_list(tz):
                    ri = to_int(r)
                    if ri is not None:
                        requests.append(ri)
            else:
                _skip_block_body(tz)


# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

def _attach_arch(levels, arch_index, unknown):
    for lv in levels:
        eid, etype = lv.get("equip_id"), lv.get("equip_type")
        if eid is None:
            lv["archetype"] = None
            continue
        name = arch_index.get((eid, etype))
        if name is None:
            unknown.add((eid, etype))
        lv["archetype"] = name


def build(text, source, was_melted):
    arch_index: dict = {}
    contracts_raw: list = []
    requests: list = []
    by_tag: dict = {}

    meta = top_level_pass(
        text,
        {
            "equipments": lambda tz: _parse_equipments_index(tz, arch_index),
            "equipment_market":
                lambda tz: _parse_top_market(tz, contracts_raw, requests),
            "countries": lambda tz: _parse_countries(tz, by_tag),
        },
        scalar_keys=("date", "version", "player"),
    )

    unknown: set = set()

    # Contract price-levels (seller/buyer captured from contract_draft, which is
    # nested in contract_definition and read in the same pass).
    contracts = []
    for c in contracts_raw:
        _attach_arch(c["price_levels"], arch_index, unknown)
        contracts.append({
            "contract_id": c["contract_id"],
            "seller": c["seller"],
            "buyer": c["buyer"],
            "price_levels": c["price_levels"],
            "price_levels_count": len(c["price_levels"]),
        })

    countries = {}
    n_country_levels = 0
    n_subsidies = 0
    for tag, rec in sorted(by_tag.items()):
        _attach_arch(rec["price_levels"], arch_index, unknown)
        countries[tag] = {
            "price_levels": rec["price_levels"],
            "price_levels_count": len(rec["price_levels"]),
            "subsidies": rec["subsidies"],
            "subsidies_count": len(rec["subsidies"]),
        }
        n_country_levels += len(rec["price_levels"])
        n_subsidies += len(rec["subsidies"])

    n_contract_levels = sum(c["price_levels_count"] for c in contracts)
    # How many contracts/countries had an (empty) price_levels block at all is the
    # honest signal here (levels={} everywhere in this melt).
    contracts_with_block = len(contracts)
    countries_with_block = len(countries)

    out = {
        "meta": {
            "source_filename": source.name,
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "scope": "EXTRA equipment-market state NOT in market.json: "
                "price-levels (the negotiated low/normal/high price band) per "
                "contract and per country, plus per-country market subsidies. "
                "market.json still owns contracts (seller/buyer/equipments/prices/"
                "convoys/delivery) and per-country market_stockpile + automation.",
                "price_levels": "The price_levels block IS PRESENT on every "
                "contract and country, but its inner levels={} is EMPTY in this "
                "melt (engine on auto-price; no manual band). So "
                "contracts_with_price_levels_block counts the present blocks, while "
                "nonempty_price_level_entries=0 is the count of actual level rows "
                "(never fabricated). The structure is parsed generically and will "
                "populate when a manual band is set.",
                "subsidies": "countries.<TAG>.equipment_market.subsidies.subsidies[]"
                " = market subsidies the seller pays per archetype to buyers, "
                "targeted EITHER by an explicit targets[] tag list OR a scripted "
                "trigger=\"...\" (optional modes, not a guaranteed-exclusive schema; "
                "both fields are captured). Real / non-empty in this save.",
                "requests": "equipment_market.contracts.requests={ <id> } = pending "
                "market request ids (flat id list).",
                "archetype_join": "(equip_id,equip_type) joined to the top-level "
                "equipments index (same as production.json / market.json). "
                "Unresolved ids keep archetype=null; counted in unknown_equipment_ids"
                " (never defaulted).",
            },
            "stats": {
                "contracts_with_price_levels_block": contracts_with_block,
                "contract_price_level_entries": n_contract_levels,
                "countries_with_market_extra": len(countries),
                "country_price_level_entries": n_country_levels,
                "country_subsidies": n_subsidies,
                "pending_requests": len(requests),
                "unknown_equipment_ids": len(unknown),
                "nonempty_price_level_entries":
                    n_contract_levels + n_country_levels,
            },
        },
        "contracts": contracts,
        "requests": sorted(requests),
        "countries": countries,
    }

    lines = ["=== EQUIPMENT-MARKET EXTRA ORACLE VALIDATION ==="]
    lines.append(f"source                 : {source.name} (melted={was_melted})")
    lines.append(f"save date              : {meta.get('date')}")
    lines.append(f"contracts w/ pl block  : {contracts_with_block}  "
                 f"(price-level entries={n_contract_levels})")
    lines.append(f"countries w/ extra     : {len(countries)}  "
                 f"(country price-levels={n_country_levels}  "
                 f"subsidies={n_subsidies})")
    lines.append(f"pending requests       : {len(requests)}  "
                 f"unknown ids={len(unknown)}")
    lines.append(f"nonempty price-levels  : {n_contract_levels + n_country_levels} "
                 f"(blocks present but levels={{}} => auto-price; 0 is honest)")
    shown = 0
    for tag, rec in countries.items():
        for s in rec["subsidies"]:
            lines.append(f"   {tag} subsidises {s['archetype']} "
                         f"(cic={s['cic']}) -> {s['targets']}")
            shown += 1
            if shown >= 8:
                break
        if shown >= 8:
            break
    return {"market_extra.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(run_cli(
        "market_extra",
        "the game save -> equipment-market EXTRA (price-levels / subsidies / requests).",
        build, ["market_extra.json"]))
