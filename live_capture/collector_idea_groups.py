"""Static idea-group contract for live politics classification."""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
from pathlib import Path
import sys
from typing import Any, Dict, Iterable, Optional, Tuple

from .collector_profile_check import process_image_path


MAX_IDEA_GROUP_FILTER_ROWS = 8192
MAX_IDEA_GROUP_FILTER_PAYLOAD_BYTES = 512 * 1024
MAX_IDEA_GROUP_TOKEN_LENGTH = 160
LAW_GROUPS = ("economy", "trade_laws", "mobilization_laws")


@dataclass(frozen=True)
class IdeaGroupFilterPreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_idea_group_filter_preflight(
    pid: int,
    *,
    vanilla_root: Optional[Path] = None,
) -> IdeaGroupFilterPreflight:
    diagnostics: Dict[str, Any] = {
        "enabled": True,
        "source": "game_defs.common_ideas.vanilla",
        "vanilla_root_source": "explicit" if vanilla_root is not None else "process_image",
        "row_count": 0,
    }
    try:
        root = Path(vanilla_root) if vanilla_root is not None else _vanilla_root_from_pid(pid)
        groups = idea_groups_from_static_defs(root)
        encoded = encode_idea_group_filter(groups)
    except Exception as exc:
        diagnostics["status"] = "unavailable"
        diagnostics["error"] = _safe_error_message(exc)
        return IdeaGroupFilterPreflight(
            status="unavailable",
            row_count=0,
            encoded=None,
            diagnostics=diagnostics,
        )
    row_count = _encoded_row_count(encoded)
    diagnostics.update({
        "status": "resolved" if encoded else "empty",
        "row_count": row_count,
        "payload_bytes": len(encoded.encode("utf-8")),
        "sha256": hashlib.sha256(encoded.encode("utf-8")).hexdigest()
        if encoded else None,
    })
    return IdeaGroupFilterPreflight(
        status="resolved" if encoded else "empty",
        row_count=row_count,
        encoded=encoded if encoded else None,
        diagnostics=diagnostics,
    )


def idea_groups_from_static_defs(vanilla_root: Path) -> Dict[str, str]:
    parse_catalogs = _import_parse_catalogs()
    no_modset = Path(__file__).with_name("__vanilla_only_no_modset__.json")
    layers = parse_catalogs.R.build_layers(
        vanilla=str(Path(vanilla_root)),
        modset_json=no_modset,
    )
    built = parse_catalogs.parse_catalogs(layers)
    ideas = ((built.get("catalogs") or {}).get("ideas") or {})
    if not isinstance(ideas, dict):
        raise RuntimeError("parse_catalogs returned no ideas catalog")
    groups: Dict[str, str] = {}
    for token, rec in ideas.items():
        if not isinstance(token, str) or not isinstance(rec, dict):
            continue
        group = rec.get("category")
        if isinstance(group, str) and _valid_script_token(token) and _valid_script_token(group):
            groups[token] = group
    return groups


def encode_idea_group_filter(groups: Any) -> str:
    normalized = _normalize_idea_group_rows(groups)
    encoded = ";".join(f"{token}={group}" for token, group in normalized)
    if len(encoded.encode("utf-8")) > MAX_IDEA_GROUP_FILTER_PAYLOAD_BYTES:
        raise ValueError("idea_group_filter payload too large")
    return encoded


def idea_group_filter_metadata(encoded: Optional[str]) -> Dict[str, Any]:
    if not encoded:
        return {
            "idea_group_filter_supplied": False,
            "idea_group_filter_rows": 0,
            "idea_group_filter_sha256": None,
        }
    return {
        "idea_group_filter_supplied": True,
        "idea_group_filter_rows": _encoded_row_count(encoded),
        "idea_group_filter_sha256": hashlib.sha256(encoded.encode("utf-8")).hexdigest(),
    }


def _normalize_idea_group_rows(groups: Any) -> list[Tuple[str, str]]:
    if groups is None:
        return []
    if isinstance(groups, str):
        raw_rows: Iterable[Any] = _iter_encoded_rows(groups)
    elif isinstance(groups, dict):
        raw_rows = groups.items()
    else:
        raw_rows = groups
    normalized: Dict[str, str] = {}
    for item in raw_rows:
        try:
            token_raw, group_raw = item
        except Exception as exc:
            raise ValueError("idea_group_filter row must be pair-like") from exc
        if not isinstance(token_raw, str) or not isinstance(group_raw, str):
            raise ValueError("idea_group_filter row must be str pair")
        token = token_raw.strip()
        group = group_raw.strip()
        if not _valid_script_token(token) or not _valid_script_token(group):
            raise ValueError("idea_group_filter token/group not script token")
        previous = normalized.get(token)
        if previous is not None and previous != group:
            raise ValueError("idea_group_filter duplicate token with different group")
        normalized[token] = group
        if len(normalized) > MAX_IDEA_GROUP_FILTER_ROWS:
            raise ValueError("idea_group_filter too many rows")
    return sorted(normalized.items())


def _iter_encoded_rows(value: str) -> Iterable[Tuple[str, str]]:
    if not value:
        return []
    rows = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("idea_group_filter contains empty row")
        if "=" not in raw:
            raise ValueError("idea_group_filter row missing '='")
        token, group = raw.split("=", 1)
        rows.append((token, group))
    return rows


def _valid_script_token(value: str) -> bool:
    if not (0 < len(value) <= MAX_IDEA_GROUP_TOKEN_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _encoded_row_count(encoded: str) -> int:
    if not encoded:
        return 0
    return len([row for row in encoded.split(";") if row])


def _vanilla_root_from_pid(pid: int) -> Path:
    return Path(process_image_path(int(pid))).parent


def _import_parse_catalogs() -> Any:
    game_defs = Path(__file__).resolve().parents[1] / "game_defs"
    game_defs_s = str(game_defs)
    if game_defs_s not in sys.path:
        sys.path.insert(0, game_defs_s)
    import parse_catalogs  # type: ignore

    return parse_catalogs


def _safe_error_message(exc: Exception) -> str:
    return str(exc).replace("\r", " ").replace("\n", " ")[:240]


__all__ = [
    "IdeaGroupFilterPreflight",
    "LAW_GROUPS",
    "encode_idea_group_filter",
    "idea_group_filter_metadata",
    "idea_groups_from_static_defs",
    "resolve_idea_group_filter_preflight",
]
