"""Small source-token helpers for rules that need quote/bare provenance.

The tolerant parser intentionally returns scalar values without token metadata.
For localisation coverage, a quoted ``name = "Literal"`` and a bare
``name = SOME_LOC_KEY`` are materially different, so this module scans the same
token stream and reports the original token kind for direct ``name`` fields.
"""

from __future__ import annotations

from paradox_parser import tokenize

_SCALAR_KINDS = {"str", "bare"}


def _matching_rbrace(tokens: list[tuple[str, str]], lbrace_idx: int) -> int:
    depth = 0
    for idx in range(lbrace_idx, len(tokens)):
        kind, _value = tokens[idx]
        if kind == "lbrace":
            depth += 1
        elif kind == "rbrace":
            depth -= 1
            if depth == 0:
                return idx
    return len(tokens)


def _top_level_blocks(tokens: list[tuple[str, str]], block_name: str):
    i = 0
    depth = 0
    wanted = block_name.lower()
    while i < len(tokens):
        kind, value = tokens[i]
        if kind == "lbrace":
            depth += 1
            i += 1
            continue
        if kind == "rbrace":
            depth = max(0, depth - 1)
            i += 1
            continue
        if (
            depth == 0
            and kind in _SCALAR_KINDS
            and value.lower() == wanted
            and i + 2 < len(tokens)
            and tokens[i + 1][0] == "eq"
            and tokens[i + 2][0] == "lbrace"
        ):
            end = _matching_rbrace(tokens, i + 2)
            yield i + 3, end
            i = end + 1
            continue
        i += 1


def _child_blocks(tokens: list[tuple[str, str]], start: int, end: int):
    i = start
    depth = 0
    while i < end:
        kind, value = tokens[i]
        if kind == "lbrace":
            depth += 1
            i += 1
            continue
        if kind == "rbrace":
            depth = max(0, depth - 1)
            i += 1
            continue
        if (
            depth == 0
            and kind in _SCALAR_KINDS
            and i + 2 < end
            and tokens[i + 1][0] == "eq"
            and tokens[i + 2][0] == "lbrace"
        ):
            child_end = _matching_rbrace(tokens, i + 2)
            yield value, i + 3, min(child_end, end)
            i = child_end + 1
            continue
        i += 1


def _direct_scalar_kind(tokens: list[tuple[str, str]], start: int, end: int, field: str) -> str | None:
    i = start
    depth = 0
    wanted = field.lower()
    while i < end:
        kind, value = tokens[i]
        if kind == "lbrace":
            depth += 1
            i += 1
            continue
        if kind == "rbrace":
            depth = max(0, depth - 1)
            i += 1
            continue
        if (
            depth == 0
            and kind in _SCALAR_KINDS
            and value.lower() == wanted
            and i + 2 < end
            and tokens[i + 1][0] == "eq"
            and tokens[i + 2][0] in _SCALAR_KINDS
        ):
            return tokens[i + 2][0]
        i += 1
    return None


def character_name_token_kinds(text: str) -> dict[str, str]:
    tokens = tokenize(text)
    out: dict[str, str] = {}
    for start, end in _top_level_blocks(tokens, "characters"):
        for cid, cstart, cend in _child_blocks(tokens, start, end):
            kind = _direct_scalar_kind(tokens, cstart, cend, "name")
            if kind:
                out[cid] = kind
    return out


def idea_name_token_kinds(text: str) -> dict[tuple[str, str], str]:
    tokens = tokenize(text)
    out: dict[tuple[str, str], str] = {}
    for start, end in _top_level_blocks(tokens, "ideas"):
        for category, cat_start, cat_end in _child_blocks(tokens, start, end):
            for idea_id, idea_start, idea_end in _child_blocks(tokens, cat_start, cat_end):
                kind = _direct_scalar_kind(tokens, idea_start, idea_end, "name")
                if kind:
                    out[(category, idea_id)] = kind
    return out


def technology_name_token_kinds(text: str) -> dict[str, str]:
    tokens = tokenize(text)
    out: dict[str, str] = {}
    for start, end in _top_level_blocks(tokens, "technologies"):
        for tech_id, tech_start, tech_end in _child_blocks(tokens, start, end):
            kind = _direct_scalar_kind(tokens, tech_start, tech_end, "name")
            if kind:
                out[tech_id] = kind
    return out
