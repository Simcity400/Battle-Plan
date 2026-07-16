#!/usr/bin/env python3
"""the game save -> campaign identity oracle.

This is an offline fixture/oracle tool. It never supplies runtime capture data.
It extracts the engine-persisted top-level ``game_unique_id`` and
``game_unique_seed`` pair with duplicate-aware, signed-i32 validation and emits
the exact version-1 digest used by ``recorder_host::campaign_identity``.

``multiplayer_random_seed`` is retained only as a diagnostic witness; it is
never part of the campaign identity digest.
"""

from __future__ import annotations

import hashlib
import re
from pathlib import Path

import deep_common as dc
from deep_common import EQUALS, LBRACE, RBRACE, atom_value, _skip_block_body


SCHEMA = "hoi4.save_campaign_identity_oracle.v1"
CID_PREIMAGE_PREFIX = b"hoi4-campaign-id-v1\0"
IDENTITY_KEYS = ("game_unique_id", "game_unique_seed")
DIAGNOSTIC_KEYS = ("multiplayer_random_seed",)
ALL_KEYS = IDENTITY_KEYS + DIAGNOSTIC_KEYS
UUID_RE = re.compile(
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-"
    r"[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
)
I32_MIN = -(1 << 31)
I32_MAX = (1 << 31) - 1


def canonical_uuid(raw: str | None) -> str | None:
    if raw is None or UUID_RE.fullmatch(raw) is None:
        return None
    return raw.lower()


def canonical_i32(raw: str | None) -> str | None:
    if raw is None or re.fullmatch(r"-?(?:0|[1-9][0-9]*)", raw) is None:
        return None
    if raw == "-0":
        return None
    value = int(raw, 10)
    if value < I32_MIN or value > I32_MAX:
        return None
    canonical = str(value)
    return canonical if canonical == raw else None


def campaign_id_v1(canonical_id: str, canonical_seed: str) -> str:
    preimage = (
        CID_PREIMAGE_PREFIX
        + canonical_id.encode("ascii")
        + b"\0"
        + canonical_seed.encode("ascii")
    )
    return hashlib.sha256(preimage).hexdigest()


def parse_identity(text: str) -> dict:
    """Parse only top-level identity scalars from melted the game text.

    Every occurrence is retained. Duplicate or block-valued identity keys are
    errors; the oracle never applies last-wins semantics to identity authority.
    """

    values = {key: [] for key in ALL_KEYS}
    block_values = {key: 0 for key in ALL_KEYS}
    save_dates: list[str] = []
    tz = dc.Tokenizer(text)

    while True:
        token = tz.next()
        if token is None:
            break
        if token is RBRACE:
            continue
        if token is LBRACE:
            _skip_block_body(tz)
            continue
        key = atom_value(token)
        if key is None or tz.peek() is not EQUALS:
            continue
        tz.next()
        value = tz.next()
        if value is LBRACE:
            if key in block_values:
                block_values[key] += 1
            _skip_block_body(tz)
            continue
        scalar = atom_value(value)
        if scalar is None:
            continue
        if key in values:
            values[key].append(scalar)
        elif key == "date":
            save_dates.append(scalar)

    errors: list[str] = []
    warnings: list[str] = []
    for key in IDENTITY_KEYS:
        count = len(values[key]) + block_values[key]
        if count == 0:
            errors.append(f"missing required top-level {key}")
        elif count != 1:
            errors.append(f"top-level {key} occurs {count} times; expected exactly one")
        elif block_values[key]:
            errors.append(f"top-level {key} is a block; expected a scalar")

    raw_id = values["game_unique_id"][0] if len(values["game_unique_id"]) == 1 else None
    raw_seed = (
        values["game_unique_seed"][0]
        if len(values["game_unique_seed"]) == 1
        else None
    )
    canonical_id = canonical_uuid(raw_id)
    canonical_seed = canonical_i32(raw_seed)
    if raw_id is not None and canonical_id is None:
        errors.append("game_unique_id is not UUID-shaped ASCII")
    if raw_seed is not None and canonical_seed is None:
        errors.append("game_unique_seed is not a canonical signed i32")

    diagnostic_raw = (
        values["multiplayer_random_seed"][0]
        if len(values["multiplayer_random_seed"]) == 1
        else None
    )
    diagnostic_seed = canonical_i32(diagnostic_raw)
    diagnostic_count = (
        len(values["multiplayer_random_seed"])
        + block_values["multiplayer_random_seed"]
    )
    if diagnostic_count > 1:
        warnings.append(
            "multiplayer_random_seed occurs multiple times; diagnostic witness unavailable"
        )
        diagnostic_seed = None
    elif block_values["multiplayer_random_seed"]:
        warnings.append(
            "multiplayer_random_seed is block-valued; diagnostic witness unavailable"
        )
        diagnostic_seed = None
    elif diagnostic_raw is not None and diagnostic_seed is None:
        warnings.append(
            "multiplayer_random_seed is not a canonical signed i32; diagnostic witness unavailable"
        )

    pair_valid = canonical_id is not None and canonical_seed is not None and not errors
    return {
        "schema": SCHEMA,
        "save_date": save_dates[0] if len(save_dates) == 1 else None,
        "identity": {
            "game_unique_id_raw": raw_id,
            "game_unique_seed_raw": raw_seed,
            "game_unique_id": canonical_id if pair_valid else None,
            "game_unique_seed": canonical_seed if pair_valid else None,
            "campaign_id_v1": (
                campaign_id_v1(canonical_id, canonical_seed) if pair_valid else None
            ),
            "pair_valid": pair_valid,
        },
        "diagnostic_witnesses": {
            "multiplayer_random_seed_raw": diagnostic_raw,
            "multiplayer_random_seed": diagnostic_seed,
        },
        "occurrences": {
            key: len(values[key]) + block_values[key] for key in ALL_KEYS
        },
        "raw_values": values,
        "errors": errors,
        "warnings": warnings,
    }


def _sha256_file(path: Path) -> str | None:
    try:
        hasher = hashlib.sha256()
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1 << 20), b""):
                hasher.update(chunk)
        return hasher.hexdigest()
    except OSError:
        return None


def build(text: str, source: Path, was_melted: bool):
    result = parse_identity(text)
    result["source"] = {
        "filename": source.name,
        "path_redacted": dc.ps._redact(source),
        "sha256": _sha256_file(source),
        "was_melted": was_melted,
    }
    identity = result["identity"]
    report = "\n".join(
        [
            "=== CAMPAIGN IDENTITY SAVE ORACLE ===",
            f"source       : {source.name} (melted={was_melted})",
            f"save date    : {result['save_date']}",
            f"pair valid   : {identity['pair_valid']}",
            f"unique id    : {identity['game_unique_id']}",
            f"unique seed  : {identity['game_unique_seed']}",
            f"campaign id  : {identity['campaign_id_v1']}",
            f"errors       : {result['errors']}",
            f"warnings     : {result['warnings']}",
        ]
    )
    return {"campaign_identity_oracle.json": result}, report


if __name__ == "__main__":
    raise SystemExit(
        dc.run_cli(
            "campaign_identity",
            "the game save -> duplicate-safe campaign identity oracle.",
            build,
            ["campaign_identity_oracle.json"],
        )
    )
