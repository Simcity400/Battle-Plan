"""Static advisor-token contract for DLL-appointed-advisor extraction."""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
from pathlib import Path
import sys
from typing import Any, Dict, Iterable, Optional, Tuple

from .collector_profile_check import process_image_path


MAX_ADVISOR_TOKEN_FILTER_TOKENS = 8192
MAX_ADVISOR_TOKEN_FILTER_PAYLOAD_BYTES = 512 * 1024
MAX_ADVISOR_TOKEN_LENGTH = 160


@dataclass(frozen=True)
class AdvisorTokenFilterPreflight:
    status: str
    token_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_advisor_token_filter_preflight(
    pid: int,
    *,
    vanilla_root: Optional[Path] = None,
) -> AdvisorTokenFilterPreflight:
    """Build the bounded advisor-token filter from static game definitions.

    The filter is definition-derived, not save-derived. For this first collector
    bridge it is intentionally vanilla-only: the live state still comes from the
    DLL active-idea vector, while this static contract only marks which active
    idea tokens are advisor ideas.
    """
    diagnostics: Dict[str, Any] = {
        "enabled": True,
        "source": "game_defs.common_characters.vanilla",
        "vanilla_root_source": "explicit" if vanilla_root is not None else "process_image",
        "token_count": 0,
    }
    try:
        root = Path(vanilla_root) if vanilla_root is not None else _vanilla_root_from_pid(pid)
        tokens = advisor_token_metadata_from_static_defs(root)
        encoded = encode_advisor_token_filter(tokens)
    except Exception as exc:
        diagnostics["status"] = "unavailable"
        diagnostics["error"] = _safe_error_message(exc)
        return AdvisorTokenFilterPreflight(
            status="unavailable",
            token_count=0,
            encoded=None,
            diagnostics=diagnostics,
        )
    token_count = _encoded_token_count(encoded)
    diagnostics.update({
        "status": "resolved" if encoded else "empty",
        "token_count": token_count,
        "payload_bytes": len(encoded.encode("utf-8")),
        "sha256": hashlib.sha256(encoded.encode("utf-8")).hexdigest()
        if encoded else None,
    })
    return AdvisorTokenFilterPreflight(
        status="resolved" if encoded else "empty",
        token_count=token_count,
        encoded=encoded if encoded else None,
        diagnostics=diagnostics,
    )


def advisor_token_metadata_from_static_defs(
    vanilla_root: Path,
) -> Dict[str, Dict[str, Optional[str]]]:
    """Return advisor idea-token metadata from current vanilla ``common/characters``."""
    parse_characters = _import_parse_characters()
    no_modset = Path(__file__).with_name("__vanilla_only_no_modset__.json")
    layers = parse_characters.R.build_layers(
        vanilla=str(Path(vanilla_root)),
        modset_json=no_modset,
    )
    built = parse_characters.parse_characters(layers)
    advisors = built.get("advisors_by_token")
    if not isinstance(advisors, dict):
        raise RuntimeError("parse_characters returned no advisors_by_token")
    out: Dict[str, Dict[str, Optional[str]]] = {}
    for token, rec in advisors.items():
        if not isinstance(token, str) or not isinstance(rec, dict):
            continue
        out[token] = {
            "slot": _optional_token(rec.get("slot")),
            "ledger": _optional_token(rec.get("ledger")),
            "character": _optional_token(rec.get("character")),
        }
    return out


def advisor_tokens_from_static_defs(vanilla_root: Path) -> set[str]:
    """Return advisor idea tokens from current vanilla ``common/characters``."""
    return set(advisor_token_metadata_from_static_defs(vanilla_root).keys())


def encode_advisor_token_filter(tokens: Any) -> str:
    """Encode a validated, sorted semicolon token list for collector_control.json."""
    normalized = _normalize_advisor_rows(tokens)
    encoded = ";".join(_encode_advisor_row(*row) for row in normalized)
    if len(encoded.encode("utf-8")) > MAX_ADVISOR_TOKEN_FILTER_PAYLOAD_BYTES:
        raise ValueError("advisor_token_filter payload too large")
    return encoded


def advisor_token_filter_metadata(encoded: Optional[str]) -> Dict[str, Any]:
    if not encoded:
        return {
            "advisor_token_filter_supplied": False,
            "advisor_token_filter_tokens": 0,
            "advisor_token_filter_sha256": None,
        }
    return {
        "advisor_token_filter_supplied": True,
        "advisor_token_filter_tokens": _encoded_token_count(encoded),
        "advisor_token_filter_sha256": hashlib.sha256(
            encoded.encode("utf-8")).hexdigest(),
    }


def _normalize_advisor_tokens(tokens: Any) -> list[str]:
    return [token for token, _slot, _ledger, _character in _normalize_advisor_rows(tokens)]


def _normalize_advisor_rows(tokens: Any) -> list[Tuple[str, Optional[str], Optional[str], Optional[str]]]:
    if tokens is None:
        return []
    if isinstance(tokens, str):
        raw_tokens: Iterable[Any] = _iter_encoded_rows(tokens)
    elif isinstance(tokens, dict):
        raw_tokens = tokens.items()
    else:
        raw_tokens = tokens
    normalized: Dict[str, Tuple[Optional[str], Optional[str], Optional[str]]] = {}
    for raw in raw_tokens:
        token, slot, ledger, character = _normalize_advisor_row(raw)
        previous = normalized.get(token)
        meta = (slot, ledger, character)
        if previous is not None and previous != meta:
            raise ValueError("advisor_token_filter duplicate token with different metadata")
        normalized[token] = meta
        if len(normalized) > MAX_ADVISOR_TOKEN_FILTER_TOKENS:
            raise ValueError("advisor_token_filter too many tokens")
    return [
        (token, slot, ledger, character)
        for token, (slot, ledger, character) in sorted(normalized.items())
    ]


def _normalize_advisor_row(
    raw: Any,
) -> Tuple[str, Optional[str], Optional[str], Optional[str]]:
    if isinstance(raw, str):
        token = raw.strip()
        slot = ledger = character = None
    elif isinstance(raw, tuple) and len(raw) == 2:
        token_raw, meta_raw = raw
        if not isinstance(token_raw, str):
            raise ValueError("advisor_token_filter token must be str")
        token = token_raw.strip()
        if isinstance(meta_raw, dict):
            slot = _optional_token(meta_raw.get("slot"))
            ledger = _optional_token(meta_raw.get("ledger"))
            character = _optional_token(meta_raw.get("character"))
        elif meta_raw is None:
            slot = ledger = character = None
        else:
            raise ValueError("advisor_token_filter metadata must be dict|null")
    else:
        raise ValueError("advisor_token_filter row must be token or pair")
    if not _valid_script_token(token):
        raise ValueError("advisor_token_filter token not script token")
    for value in (slot, ledger, character):
        if value is not None and not _valid_script_token(value):
            raise ValueError("advisor_token_filter metadata not script token")
    return token, slot, ledger, character


def _iter_encoded_rows(value: str) -> Iterable[Tuple[str, Optional[Dict[str, Optional[str]]]]]:
    if not value:
        return []
    rows = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("advisor_token_filter contains empty row")
        if "=" not in raw:
            rows.append((raw, None))
            continue
        token, meta_raw = raw.split("=", 1)
        parts = meta_raw.split("|")
        if len(parts) > 3:
            raise ValueError("advisor_token_filter metadata has too many fields")
        parts += [""] * (3 - len(parts))
        rows.append((
            token,
            {
                "slot": parts[0] or None,
                "ledger": parts[1] or None,
                "character": parts[2] or None,
            },
        ))
    return rows


def _optional_token(value: Any) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        return None
    value = value.strip()
    return value or None


def _encode_advisor_row(
    token: str,
    slot: Optional[str],
    ledger: Optional[str],
    character: Optional[str],
) -> str:
    if slot is None and ledger is None and character is None:
        return token
    return f"{token}={slot or ''}|{ledger or ''}|{character or ''}"


def _valid_script_token(value: str) -> bool:
    if not (0 < len(value) <= MAX_ADVISOR_TOKEN_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _encoded_token_count(encoded: str) -> int:
    if not encoded:
        return 0
    return len([token for token in encoded.split(";") if token])


def _vanilla_root_from_pid(pid: int) -> Path:
    return Path(process_image_path(int(pid))).parent


def _import_parse_characters() -> Any:
    game_defs = Path(__file__).resolve().parents[1] / "game_defs"
    game_defs_s = str(game_defs)
    if game_defs_s not in sys.path:
        sys.path.insert(0, game_defs_s)
    import parse_characters  # type: ignore

    return parse_characters


def _safe_error_message(exc: Exception) -> str:
    return str(exc).replace("\r", " ").replace("\n", " ")[:240]


__all__ = [
    "AdvisorTokenFilterPreflight",
    "advisor_token_metadata_from_static_defs",
    "advisor_token_filter_metadata",
    "advisor_tokens_from_static_defs",
    "encode_advisor_token_filter",
    "resolve_advisor_token_filter_preflight",
]
