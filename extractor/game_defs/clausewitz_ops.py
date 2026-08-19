#!/usr/bin/env python3
"""Helpers for reading Clausewitz constructs the tolerant parser keeps as raw
anonymous-token streams: comparison OPERATORS and variable read/write ops.

The shared ``paradox_parser`` keeps source order and never throws, but it stores
a block like ``check_variable = { resources_disabled > 5 }`` as a Node whose
entries are anonymous tokens ``[('','resources_disabled'),('','>'),('','5')]``
(the operator is preserved, not dropped — verified). These helpers reconstruct
the operator triples and walk a block to harvest every variable name that is
read or written, covering the FULL the game variable-op set (Codex C5).
"""

from __future__ import annotations

import re

from paradox_parser import Node

_OPS = {">", "<", ">=", "<=", "=", "==", "!="}
_OP_KEY_TO_FULL = {">": ">=", "<": "<=", "!": "!=", "=": "=="}
_BARE_OPS = {">", "<", "=", "!", ">=", "<=", "==", "!="}

WRITE_OPS = {
    "set_variable", "add_to_variable", "subtract_from_variable",
    "multiply_variable", "divide_variable", "modulo_variable",
    "clamp_variable", "round_variable", "clear_variable",
    "set_temp_variable", "add_to_temp_variable", "subtract_from_temp_variable",
    "multiply_temp_variable", "divide_temp_variable", "clear_temp_variable",
    "set_variable_to_random", "export_to_variable", "round_temp_variable",
}
READ_OPS = {
    "check_variable", "is_variable_equal", "variable_op",
    "has_variable", "is_variable_set",
}


def operator_triples(node: Node):
    """Yield (lhs, op, rhs) for ``LHS op RHS`` patterns inside a block (e.g.
    ``var > 5`` / ``var >= 5``). Returns raw token strings so the operator is
    never lost (Codex C1).

    Two parse shapes are reconstructed:
      * single-char ops with no ``=`` (``>`` ``<``) arrive as three anonymous
        tokens ``[('','var'),('','>'),('','5')]``.
      * ops that embed ``=`` (``>=`` ``<=`` ``!=`` ``==``) are split by the
        tokenizer on the ``=`` delimiter; the parser then attaches the value to
        the operator glyph as a *keyed* artifact, e.g. ``my_var >= 5`` becomes
        ``[('','my_var'),('>','5')]`` and ``my_var == 5`` becomes
        ``[('my_var',''),('','5')]``. We recombine the swallowed ``=`` so the
        full operator (and the real LHS) survive.
    """
    if not isinstance(node, Node):
        return
    ents = [(k, v) for k, v in node.entries if isinstance(v, str)]
    i = 0
    n = len(ents)
    while i < n:
        k, v = ents[i]
        if k == "":
            if i + 2 < n and ents[i + 1][0] == "" and ents[i + 1][1] in _OPS \
                    and ents[i + 2][0] == "":
                yield (v, ents[i + 1][1], ents[i + 2][1])
                i += 3
                continue
            if i + 1 < n and ents[i + 1][0] in _OP_KEY_TO_FULL:
                yield (v, _OP_KEY_TO_FULL[ents[i + 1][0]], ents[i + 1][1])
                i += 2
                continue
        elif k not in _BARE_OPS and v == "" and i + 1 < n \
                and ents[i + 1][0] == "":
            yield (k, "==", ents[i + 1][1])
            i += 2
            continue
        i += 1


_GLUED = re.compile(r"^(.*?)(?:>=|<=|==|!=|>|<|=)\s*-?\d*\.?\d*$")


def _clean_var(name):
    if not isinstance(name, str):
        return None
    m = _GLUED.match(name)
    cleaned = m.group(1) if (m and m.group(1)) else name
    return cleaned.strip() or None


def _var_name_from_block(blk: Node):
    """A var op block is usually ``{ var = NAME value = N }`` OR an inline
    ``{ NAME = N }`` / ``{ NAME op N }``. Return the variable NAME (cleaned of a
    glued operator/value, Codex #5)."""
    if not isinstance(blk, Node):
        return None
    v = blk.get("var")
    if isinstance(v, str):
        return _clean_var(v)
    for k, val in blk.entries:
        if k and k not in _BARE_OPS and k.lower() not in ("value", "var", "tooltip"):
            return _clean_var(k)
    toks = [vv for kk, vv in blk.entries if kk == "" and isinstance(vv, str)]
    if toks:
        return _clean_var(toks[0])
    return None


FLAG_READ_OPS = {"has_country_flag", "has_global_flag", "has_state_flag",
                 "has_character_flag", "has_dynamic_flag"}


def harvest_variables(node: Node, reads: set, writes: set):
    """Recursively walk a block collecting variable (and flag) names by read/write
    op, INCLUDING flags nested inside scope blocks (Codex #5)."""
    if not isinstance(node, Node):
        return
    for key, val in node.entries:
        kl = key.lower() if key else ""
        if kl in WRITE_OPS:
            if isinstance(val, Node):
                nm = _var_name_from_block(val)
                if nm:
                    writes.add(nm)
            elif isinstance(val, str):
                writes.add(_clean_var(val))
        elif kl in READ_OPS:
            if isinstance(val, Node):
                nm = _var_name_from_block(val)
                if nm:
                    reads.add(nm)
            elif isinstance(val, str):
                nm = _clean_var(val)
                if nm:
                    reads.add(nm)
        elif kl in FLAG_READ_OPS and isinstance(val, str):
            reads.add("flag:" + val)
        if isinstance(val, Node):
            harvest_variables(val, reads, writes)
    writes.discard(None)
    reads.discard(None)
