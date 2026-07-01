"""Tolerant tokenizer + brace parser for Paradox Clausewitz script (.gfx / .gui).

the game interface files are Clausewitz-script: nested `key = { ... }` blocks, `#`
line comments, quoted and unquoted tokens, repeated keys, mixed-case keywords,
numbers with `%` / `%%` suffixes, negatives and decimals.

This module is deliberately permissive: it never throws on unexpected tokens, it
preserves source order, and it keeps raw token text where the meaning is unclear
(e.g. `100%%`) so downstream consumers can decide. See game_assets/README.md.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any

# A token is either a delimiter ('{', '}', '='), a quoted string, or a bare run
# of non-whitespace/non-delimiter characters. Comments (# .. EOL) are dropped by
# the lexer, but only when the '#' is OUTSIDE a quoted string.
_DELIMS = set("{}=")


def tokenize(text: str) -> list[tuple[str, str]]:
    """Return a list of (kind, value) tokens.

    kind in {"lbrace","rbrace","eq","str","bare"}. Quote-aware: a '#' inside a
    quoted string does not start a comment, and '{' '}' inside quotes are not
    delimiters.
    """
    tokens: list[tuple[str, str]] = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        # Whitespace
        if c in " \t\r\n":
            i += 1
            continue
        # Comment to end of line (outside quotes; we are between tokens here)
        if c == "#":
            j = text.find("\n", i)
            i = n if j == -1 else j + 1
            continue
        # Quoted string
        if c == '"':
            j = i + 1
            buf = []
            while j < n:
                cj = text[j]
                if cj == "\\" and j + 1 < n:
                    # keep escaped char literally (rare in the game, defensive)
                    buf.append(text[j + 1])
                    j += 2
                    continue
                if cj == '"':
                    j += 1
                    break
                buf.append(cj)
                j += 1
            tokens.append(("str", "".join(buf)))
            i = j
            continue
        # Delimiters
        if c == "{":
            tokens.append(("lbrace", "{"))
            i += 1
            continue
        if c == "}":
            tokens.append(("rbrace", "}"))
            i += 1
            continue
        if c == "=":
            tokens.append(("eq", "="))
            i += 1
            continue
        # Bare token: read until whitespace / delimiter / comment / quote
        j = i
        while j < n and text[j] not in " \t\r\n" and text[j] not in _DELIMS \
                and text[j] != "#" and text[j] != '"':
            j += 1
        tokens.append(("bare", text[i:j]))
        i = j
    return tokens


@dataclass
class Node:
    """A parsed block. `entries` preserves source order and duplicate keys.

    Each entry is (key, value) where value is either a scalar str or a child
    Node. Top-level scalars in a list-like block use key="" (anonymous).
    """
    entries: list[tuple[str, Any]] = field(default_factory=list)

    # convenience accessors -------------------------------------------------
    def get(self, key: str, default=None):
        """First value for `key` (case-insensitive), scalar or Node."""
        kl = key.lower()
        for k, v in self.entries:
            if k.lower() == kl:
                return v
        return default

    def get_all(self, key: str):
        kl = key.lower()
        return [v for k, v in self.entries if k.lower() == kl]

    def children_named(self, type_name: str):
        """All child Nodes whose key (block type) matches case-insensitively."""
        tl = type_name.lower()
        return [(k, v) for k, v in self.entries
                if isinstance(v, Node) and k.lower() == tl]


def _parse_block(tokens: list[tuple[str, str]], pos: int) -> tuple[Node, int]:
    """Parse entries until a closing brace or EOF. Returns (Node, next_pos)."""
    node = Node()
    n = len(tokens)
    while pos < n:
        kind, val = tokens[pos]
        if kind == "rbrace":
            return node, pos + 1
        if kind == "lbrace":
            # anonymous nested block (rare); attach under "")
            child, pos = _parse_block(tokens, pos + 1)
            node.entries.append(("", child))
            continue
        if kind in ("str", "bare"):
            key = val
            # lookahead for '='
            if pos + 1 < n and tokens[pos + 1][0] == "eq":
                # key = value
                if pos + 2 < n and tokens[pos + 2][0] == "lbrace":
                    child, pos = _parse_block(tokens, pos + 3)
                    node.entries.append((key, child))
                    continue
                if pos + 2 < n and tokens[pos + 2][0] in ("str", "bare"):
                    node.entries.append((key, tokens[pos + 2][1]))
                    pos += 3
                    continue
                # '=' with nothing usable after; record bare key
                node.entries.append((key, ""))
                pos += 2
                continue
            # bare scalar in a list (e.g. inside { 1 2 3 } or { tagA tagB })
            node.entries.append(("", key))
            pos += 1
            continue
        # stray '=' or unexpected: skip defensively
        pos += 1
    return node, pos


def parse(text: str) -> Node:
    """Parse a full Clausewitz-script document into a root Node."""
    tokens = tokenize(text)
    root, _ = _parse_block(tokens, 0)
    return root


# --- value coercion helpers (used by the gui parser) -----------------------

_NUM_RE = re.compile(r"^-?\d+(\.\d+)?$")


def coerce_scalar(raw: str):
    """Return int/float for clean numbers, else a {'raw': ...} dict for things
    like '100%%' / '50%' / orientation strings, else the bare string.
    """
    if raw is None:
        return None
    if _NUM_RE.match(raw):
        if "." in raw:
            return float(raw)
        return int(raw)
    return raw


def parse_xy(node: Node):
    """Parse a {x= y=} (or {width= height=}) block into a dict, preserving raw
    percent suffixes."""
    if not isinstance(node, Node):
        return None
    out = {}
    for k, v in node.entries:
        if isinstance(v, Node):
            continue
        out[k.lower()] = coerce_scalar(v)
    return out
