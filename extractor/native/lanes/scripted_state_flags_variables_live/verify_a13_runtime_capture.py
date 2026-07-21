#!/usr/bin/env python3
"""Fail-closed offline grader for an A13 exact-a729 runtime capture card.

This grader does not attach to the game and does not manufacture a capture. It
admits only the pointer-free observation emitted by the production collector
or a harness using the same contract.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCOPES = {"global": 1, "country": 2, "state": 3}
MAX_OWNER = {"global": 0, "country": 65535, "state": 65535}
FIXED_SCALE = 100_000
REQUIRED_AUTHORITY = {
    "variable_scope_law": {
        "job_id": "20260720T124554Z-w5-a13-scoped-variable-write-lifecycle-13b1fecb",
        "manifest_sha256": "30def87e38ce6507223de50d2233cd1dbd2253c28078f89041dcabe793415119",
    },
    "country_runtime_container_law": {
        "job_id": "20260720T124517Z-w5-a13-country-runtime-container-law-v2-b0587312",
        "manifest_sha256": "40a6547f654f016f9ae88ce448f42abe8d54daf6aed49528b757d019c1c6a279",
    },
    "source_assertion_xrefs": {
        "job_id": "20260720T124539Z-w5-a13-source-assertion-xrefs-defc1eb3",
        "artifact_sha256": "272139e3533710b0e9f722d8c9e99537b93ed741ae22fb6f2852ee6e885b3a2c",
    },
    "flag_runtime_seeds": {
        "job_id": "20260720T124728Z-w5-a13-flag-runtime-seeds-950c7df3",
        "artifact_sha256": "20eff5036625dfc5e2c346c0f9610c91ad842c5264ffa149ec3925952101f1b6",
    },
}


class Rejection(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Rejection(message)


def exact_object(value: Any, required: set[str], optional: set[str], ctx: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{ctx}: expected object")
    keys = set(value)
    require(required <= keys, f"{ctx}: missing {sorted(required - keys)}")
    require(keys <= required | optional, f"{ctx}: unknown {sorted(keys - required - optional)}")
    return value


def portable_key(value: Any, ctx: str) -> str:
    require(isinstance(value, str), f"{ctx}: key is not text")
    encoded = value.encode("utf-8")
    require(0 < len(encoded) <= 512, f"{ctx}: key length is invalid")
    require(all(32 <= byte <= 126 for byte in encoded), f"{ctx}: key is not portable ASCII")
    return value


def exact_int(value: Any, lo: int, hi: int, ctx: str) -> int:
    require(isinstance(value, int) and not isinstance(value, bool), f"{ctx}: expected integer")
    require(lo <= value <= hi, f"{ctx}: integer out of range")
    return value


def grade_rows(
    scope: dict[str, Any], ctx: str
) -> tuple[tuple[Any, ...], str, dict[str, int], dict[str, dict[str, int]], dict[str, dict[str, int]]]:
    kind = scope["scope_kind"]
    owner_id = exact_int(scope["owner_id"], 0, MAX_OWNER[kind], f"{ctx}.owner_id")
    require((kind == "global") == (owner_id == 0), f"{ctx}: global/owner identity mismatch")
    portable_owner = scope["portable_owner_key"]
    if kind == "global":
        require(portable_owner == "world", f"{ctx}: global portable owner must be world")
    elif kind == "state":
        require(portable_owner == str(owner_id), f"{ctx}: state portable owner mismatch")
    else:
        require(
            isinstance(portable_owner, str)
            and len(portable_owner) == 3
            and all(char.isupper() or char.isdigit() for char in portable_owner),
            f"{ctx}: country portable owner must be a canonical tag",
        )

    variables = exact_object(scope["variables"], {"status", "rows"}, set(), f"{ctx}.variables")
    flags = exact_object(scope["flags"], {"status", "rows"}, set(), f"{ctx}.flags")
    for family_name, family in (("variables", variables), ("flags", flags)):
        require(family["status"] in {"measured", "measured_empty"}, f"{ctx}.{family_name}: unavailable")
        require(isinstance(family["rows"], list), f"{ctx}.{family_name}.rows: expected list")
        require(
            (family["status"] == "measured_empty") == (len(family["rows"]) == 0),
            f"{ctx}.{family_name}: status/row shape mismatch",
        )

    variable_keys: list[str] = []
    variable_values: dict[str, int] = {}
    for index, value in enumerate(variables["rows"]):
        row = exact_object(value, {"key", "value_raw"}, set(), f"{ctx}.variables.rows[{index}]")
        key = portable_key(row["key"], f"{ctx}.variables.rows[{index}]")
        variable_keys.append(key)
        variable_values[key] = exact_int(
            row["value_raw"],
            -(1 << 63),
            (1 << 63) - 1,
            f"{ctx}.variables.rows[{index}].value_raw",
        )
    require(variable_keys == sorted(variable_keys), f"{ctx}.variables: non-canonical order")
    require(len(set(variable_keys)) == len(variable_keys), f"{ctx}.variables: duplicate key")

    flag_keys: list[tuple[int, str]] = []
    named_flags: dict[str, dict[str, int]] = {}
    token_flags: dict[str, dict[str, int]] = {}
    for index, value in enumerate(flags["rows"]):
        row = exact_object(
            value,
            {"token_id", "key", "date_hour_raw", "value_raw"},
            set(),
            f"{ctx}.flags.rows[{index}]",
        )
        token = exact_int(row["token_id"], 1, (1 << 32) - 1, f"{ctx}.flags.rows[{index}].token_id")
        key = "" if row["key"] is None else portable_key(row["key"], f"{ctx}.flags.rows[{index}].key")
        date = exact_int(
            row["date_hour_raw"],
            -(1 << 31),
            (1 << 31) - 1,
            f"{ctx}.flags.rows[{index}].date_hour_raw",
        )
        raw = exact_int(
            row["value_raw"], 0, (1 << 32) - 1, f"{ctx}.flags.rows[{index}].value_raw"
        )
        flag_keys.append((token, key))
        state = {"value_raw": raw, "date_hour_raw": date}
        if key:
            require(key not in named_flags, f"{ctx}.flags: duplicate named key")
            named_flags[key] = state
        else:
            token_flags[str(token)] = state
    require(flag_keys == sorted(flag_keys), f"{ctx}.flags: non-canonical order")
    require(len({token for token, _ in flag_keys}) == len(flag_keys), f"{ctx}.flags: duplicate token")
    return (
        (SCOPES[kind], owner_id),
        portable_owner,
        variable_values,
        named_flags,
        token_flags,
    )


def grade_projection(snapshot: Any, expected: dict[tuple[str, str], dict[str, Any]]) -> None:
    obj = exact_object(
        snapshot,
        {"global", "countries", "states"},
        set(),
        "portable_projection",
    )

    def pair(value: Any, ctx: str, wanted: dict[str, Any]) -> None:
        row = exact_object(value, {"variables_raw_fixed5", "variables"}, {"flags", "token_only_flags"}, ctx)
        raw = row["variables_raw_fixed5"]
        projected = row["variables"]
        require(isinstance(raw, dict) and isinstance(projected, dict), f"{ctx}: variable maps required")
        require(set(raw) == set(projected), f"{ctx}: raw/projected key roster mismatch")
        for key in raw:
            portable_key(key, f"{ctx}.variables_raw_fixed5")
            exact = exact_int(raw[key], -(1 << 63), (1 << 63) - 1, f"{ctx}.{key}.raw")
            number = projected[key]
            require(
                isinstance(number, (int, float))
                and not isinstance(number, bool)
                and math.isfinite(float(number))
                and float(number) == exact / FIXED_SCALE,
                f"{ctx}.{key}: raw/projected parity failure",
            )
        require(raw == wanted["variables_raw_fixed5"], f"{ctx}: collector/fold variable join mismatch")
        require(row.get("flags", {}) == wanted["flags"], f"{ctx}: collector/fold named-flag join mismatch")
        require(
            row.get("token_only_flags", {}) == wanted["token_only_flags"],
            f"{ctx}: collector/fold token-flag join mismatch",
        )

    require(("global", "world") in expected, "portable_projection: global source missing")
    pair(obj["global"], "portable_projection.global", expected[("global", "world")])
    for family in ("countries", "states"):
        require(isinstance(obj[family], dict), f"portable_projection.{family}: expected object")
        expected_keys = {
            owner for (expected_family, owner) in expected if expected_family == family
        }
        require(set(obj[family]) == expected_keys, f"portable_projection.{family}: owner roster mismatch")
        for owner, value in obj[family].items():
            pair(
                value,
                f"portable_projection.{family}.{owner}",
                expected[(family, owner)],
            )


def grade(card: Any) -> dict[str, Any]:
    root = exact_object(
        card,
        {
            "schema",
            "executable_sha256",
            "authority",
            "frame",
            "scopes",
            "unsupported_scopes",
            "portable_projection",
        },
        set(),
        "capture",
    )
    require(root["schema"] == "a13_runtime_capture_v1", "capture: wrong schema")
    require(root["executable_sha256"] == EXE_SHA256, "capture: wrong executable")
    require(root["authority"] == REQUIRED_AUTHORITY, "capture: static authority mismatch")

    frame = exact_object(
        root["frame"],
        {"game_hour_raw", "observed_game_hour_before", "observed_game_hour_after", "source_sequence"},
        set(),
        "frame",
    )
    hour = exact_int(frame["game_hour_raw"], -(1 << 31), (1 << 31) - 1, "frame.game_hour_raw")
    require(
        frame["observed_game_hour_before"] == hour == frame["observed_game_hour_after"],
        "frame: observation crossed an engine-hour boundary",
    )
    exact_int(frame["source_sequence"], 1, (1 << 64) - 1, "frame.source_sequence")
    require(root["unsupported_scopes"] == [], "capture: arbitrary engine scopes must be unavailable")
    require(isinstance(root["scopes"], list), "capture.scopes: expected list")

    roster: list[tuple[Any, ...]] = []
    projection_expected: dict[tuple[str, str], dict[str, Any]] = {}
    variables = flags = 0
    for index, value in enumerate(root["scopes"]):
        scope = exact_object(
            value,
            {"scope_kind", "owner_id", "portable_owner_key", "variables", "flags"},
            set(),
            f"scopes[{index}]",
        )
        require(scope["scope_kind"] in SCOPES, f"scopes[{index}]: unsupported scope")
        identity, portable_owner, variable_rows, named_flags, token_flags = grade_rows(
            scope, f"scopes[{index}]"
        )
        roster.append(identity)
        family = {
            "global": "global",
            "country": "countries",
            "state": "states",
        }[scope["scope_kind"]]
        projection_key = (family, portable_owner)
        require(projection_key not in projection_expected, f"scopes[{index}]: duplicate portable owner")
        projection_expected[projection_key] = {
            "variables_raw_fixed5": variable_rows,
            "flags": named_flags,
            "token_only_flags": token_flags,
        }
        variables += len(variable_rows)
        flags += len(named_flags) + len(token_flags)
    require(roster == sorted(roster), "capture.scopes: non-canonical roster order")
    require(len(set(roster)) == len(roster), "capture.scopes: duplicate owner identity")
    kinds = {identity[0] for identity in roster}
    require(kinds == set(SCOPES.values()), "capture.scopes: global/country/state coverage incomplete")
    grade_projection(root["portable_projection"], projection_expected)
    return {
        "ok": True,
        "schema": root["schema"],
        "scope_count": len(roster),
        "variable_rows": variables,
        "flag_rows": flags,
        "game_hour_raw": hour,
        "human_input_required": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    try:
        report = grade(json.loads(args.capture.read_text(encoding="utf-8")))
    except (OSError, json.JSONDecodeError, Rejection) as error:
        print(json.dumps({"ok": False, "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
