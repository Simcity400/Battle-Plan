"""Canonical save-oracle output contract.

Every parser-owned JSON output belongs here.  The cross-file validator imports
this registry and therefore fails closed when any declared parser output is
missing.  Keeping this separate from ``validate.py`` makes omissions visible in
review and gives offline fixture audits one authoritative output inventory.
"""

from __future__ import annotations


OUTPUTS_BY_PRODUCER = {
    "parse_abilities.py": ("abilities.json",),
    "parse_campaign_identity.py": ("campaign_identity_oracle.json",),
    "parse_characters.py": ("characters.json",),
    "parse_combats.py": ("combats.json",),
    "parse_control.py": (
        "state_control.json", "province_control.json", "control_meta.json",
    ),
    "parse_country_extra.py": ("country_extra.json", "players.json"),
    "parse_deployment.py": ("deployment.json",),
    "parse_designs.py": ("designs.json",),
    "parse_diplo.py": ("diplomacy.json",),
    "parse_divisions_full.py": ("divisions_full.json",),
    "parse_doctrine.py": ("doctrine.json",),
    "parse_economy.py": ("economy.json",),
    "parse_events.py": ("events.json",),
    "parse_focus.py": ("focus.json",),
    "parse_intel.py": ("intel.json",),
    "parse_market.py": ("market.json",),
    "parse_market_extra.py": ("market_extra.json",),
    "parse_occupation.py": ("occupation.json",),
    "parse_oob.py": ("oob.json",),
    "parse_production.py": ("production.json",),
    "parse_projects.py": ("projects.json",),
    "parse_raids.py": ("raid_instances.json",),
    "parse_save.py": (
        "country_snapshot.json", "state_ownership.json", "meta.json",
    ),
    "parse_state_detail.py": ("state_detail.json",),
    "parse_strategic.py": (
        "strategic_air.json", "strategic_operatives.json", "raids.json",
    ),
    "parse_supply.py": ("supply.json",),
    "parse_tech.py": ("tech.json",),
    "parse_units.py": ("divisions.json",),
    "parse_weather.py": ("weather.json",),
    "parse_world.py": ("world.json",),
}


PRODUCERS_BY_OUTPUT = {}
for _producer, _outputs in OUTPUTS_BY_PRODUCER.items():
    for _output in _outputs:
        PRODUCERS_BY_OUTPUT.setdefault(_output, []).append(_producer)

_DUPLICATE_OUTPUT_OWNERS = {
    output: tuple(sorted(producers))
    for output, producers in PRODUCERS_BY_OUTPUT.items()
    if len(producers) != 1
}
if _DUPLICATE_OUTPUT_OWNERS:
    raise RuntimeError(
        "save-oracle outputs must have exactly one producer; duplicate owners: "
        f"{_DUPLICATE_OUTPUT_OWNERS}"
    )


REQUIRED_OUTPUTS = tuple(sorted({
    output
    for outputs in OUTPUTS_BY_PRODUCER.values()
    for output in outputs
}))
