from __future__ import annotations

import sys
from pathlib import Path

_DEFS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_DEFS))

import compile_fired_event_registry as registry
import resolver as R


def test_namespace_order_and_combined_id_are_first_registration_one_based(tmp_path):
    vanilla = tmp_path / "vanilla"
    events = vanilla / "events"
    events.mkdir(parents=True)
    (events / "a.txt").write_text(
        "add_namespace = alpha\n"
        "country_event = { id = alpha.7 desc = alpha.7.d option = { name = alpha.7.a } }\n",
        encoding="utf-8",
    )
    (events / "b.txt").write_text(
        "add_namespace = alpha\n"
        "add_namespace = beta\n"
        "news_event = { id = BETA.3 picture = GFX_beta desc = beta.3.d }\n",
        encoding="utf-8",
    )
    layers = R.build_layers(vanilla=str(vanilla), mods=[])
    result = registry.compile_registry(layers, registry.PINNED_EXE_SHA256)
    assert [(row["ordinal"], row["namespace"]) for row in result["namespaces"]] == [
        (1, "alpha"),
        (2, "beta"),
    ]
    assert [(row["combined_id"], row["event_id"]) for row in result["events"]] == [
        (100_007, "alpha.7"),
        (200_003, "BETA.3"),
    ]
    assert result["events"][0]["option_count"] == 1
    assert result["events"][1]["picture"] == "GFX_beta"
