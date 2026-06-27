#!/usr/bin/env python3
"""Shared Clausewitz (Paradox script) text-parsing primitives for the map pipeline.

Both build_map_assets.py and build_strategic_regions.py parse the same
`keyword = { ... }` brace-block syntax out of the game's map data files. The comment
stripper, brace-matcher, and the id/name/comment regexes were duplicated
verbatim across both; they live here now as the single source of truth.

Pure stdlib (`re`). No I/O, no behaviour beyond text parsing — importing this
module has no side effects, so it is safe for either builder to import without
re-running the other's build.
"""
from __future__ import annotations

import re

# Strip line comments but keep braces/numbers. `#` to end-of-line.
COMMENT_RE = re.compile(r"#.*")
# `id = 123`  (word-boundary so `grid=` etc. never matches).
ID_RE = re.compile(r"\bid\s*=\s*(\d+)")
# `name = "..."`
NAME_RE = re.compile(r'name\s*=\s*"([^"]*)"')


def strip_comments(text):
    """Remove `# ...` line comments, leaving braces/numbers intact."""
    return COMMENT_RE.sub("", text)


def extract_brace_block(text, brace_idx):
    """Given the index of the opening '{', return the substring strictly inside
    the matched braces. On an unbalanced block, return the remainder of the text.

    `brace_idx` must point at a '{' (both builders only ever call it that way).
    """
    depth = 0
    inner_start = brace_idx + 1
    i = brace_idx
    n = len(text)
    while i < n:
        c = text[i]
        if c == "{":
            depth += 1
            if depth == 1:
                inner_start = i + 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return text[inner_start:i]
        i += 1
    return text[brace_idx + 1:]  # unbalanced; return the rest


def find_block(text, keyword, *, word_boundary=False):
    """Return (inner_text, brace_index) for the first `keyword = { ... }`, or
    (None, -1) if not found.

    `word_boundary=True` anchors the keyword on a `\\b` word boundary (so e.g.
    `provinces` does not match inside `xprovinces`); pass it to reproduce the
    strategic-regions builder's stricter match. The default (no boundary)
    matches the map-assets builder's historical behaviour.
    """
    pat = (r"\b" + keyword if word_boundary else keyword) + r"\s*=\s*\{"
    m = re.search(pat, text)
    if not m:
        return None, -1
    brace_idx = m.end() - 1
    return extract_brace_block(text, brace_idx), brace_idx
