#!/usr/bin/env python3
"""Pure-unit edge-case tests for the game_defs static parsers + shared
Clausewitz helpers.

Unlike ``test_game_defs.py`` (which parses a real the game install and is skipped
when ``BATTLE_PLAN_VANILLA`` is unset), these tests are hermetic: they feed crafted
Clausewitz snippets straight through ``paradox_parser`` + ``clausewitz_ops`` and
the per-parser block helpers, so they ALWAYS run. They pin the tricky
Clausewitz edge cases (comparison operators that embed ``=``, duplicate keys,
quoting, the no-fabrication invariant) that the integration tests cannot reach.
"""

from __future__ import annotations

import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_DEFS = _HERE.parent
_REPO = _DEFS.parent.parent
_GAME_ASSETS = _REPO / "replay_interface" / "game_assets"
for _p in (str(_GAME_ASSETS), str(_DEFS)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from paradox_parser import parse, Node
import clausewitz_ops as C
import parse_catalogs as catalogs


def _harvest(text: str):
    root = parse(text)
    reads, writes = set(), set()
    C.harvest_variables(root, reads, writes)
    return reads, writes


def test_harvest_recovers_var_for_all_operators():
    for op in (">", "<", ">=", "<=", "==", "!="):
        reads, _ = _harvest(f"check_variable = {{ my_var {op} 5 }}")
        assert "my_var" in reads, f"op {op!r}: var name lost, reads={reads}"
        assert not (reads & {">", "<", "!", "=", ">=", "<=", "!=", "=="}), \
            f"op {op!r}: fabricated operator token in reads={reads}"


def test_operator_triples_all_operators():
    """operator_triples must reconstruct (lhs, op, rhs) for the FULL operator set,
    including the ``>=`` / ``<=`` / ``!=`` forms the tokenizer splits on ``=``."""
    expected = {
        ">": (">", "5"), "<": ("<", "5"),
        ">=": (">=", "5"), "<=": ("<=", "5"),
        "==": ("==", "5"), "!=": ("!=", "5"),
    }
    for op, (want_op, want_rhs) in expected.items():
        node = parse(f"x = {{ my_var {op} 5 }}").get("x")
        triples = list(C.operator_triples(node))
        assert triples, f"op {op!r}: no triple produced, entries={node.entries}"
        lhs, got_op, rhs = triples[0]
        assert lhs == "my_var", f"op {op!r}: lhs={lhs!r}"
        assert got_op == want_op, f"op {op!r}: op reconstructed as {got_op!r}"
        assert rhs == want_rhs, f"op {op!r}: rhs={rhs!r}"


def test_harvest_glued_operator_form_still_works():
    reads, _ = _harvest("check_variable = { Root.foo>0 }")
    assert "Root.foo" in reads, reads
    reads, _ = _harvest("check_variable = { Root.bar>=2 }")
    assert "Root.bar" in reads, reads


def test_harvest_explicit_and_inline_forms_unaffected():
    reads, _ = _harvest("check_variable = { var = explicit_v value = 3 }")
    assert "explicit_v" in reads and ">" not in reads, reads
    reads, _ = _harvest("check_variable = { inline_v = 3 }")
    assert "inline_v" in reads, reads


def test_write_ops_recover_var_name():
    cases = {
        "set_variable = { var = w1 value = 3 }": "w1",
        "set_variable = { w2 = 3 }": "w2",
        "add_to_variable = { w3 = 1 }": "w3",
        "clear_variable = w4": "w4",
    }
    for text, want in cases.items():
        _, writes = _harvest(text)
        assert want in writes, f"{text!r}: writes={writes}"


def test_duplicate_keys_all_kept():
    """Clausewitz allows repeated keys (e.g. multiple add_core_of). The parser
    MUST keep every one, not collapse to the last (silent data loss)."""
    root = parse("x = { add_core_of = GER add_core_of = ITA add_core_of = JAP }")
    blk = root.get("x")
    cores = blk.get_all("add_core_of")
    assert cores == ["GER", "ITA", "JAP"], cores


def test_power_balance_effect_json_preserves_duplicate_sibling_commands():
    """The BOP catalog must not normalize repeated effect keys away."""
    block = parse("""
        range = {
            id = duplicate_gfx_range
            min = -1
            max = 0
            on_activate = {
                set_power_balance_gfx = { side = left gfx = GFX_left }
                set_power_balance_gfx = { side = right gfx = GFX_right }
                add_power_balance_modifier = one_modifier
            }
        }
    """).get("range")
    row = catalogs._bop_range(block, "common/bop/test.txt")
    gfx = row["on_activate"]["set_power_balance_gfx"]
    assert isinstance(gfx, list)
    assert [entry["side"] for entry in gfx] == ["left", "right"]
    assert row["on_activate"]["add_power_balance_modifier"] == "one_modifier"


def test_quoted_string_special_chars_preserved():
    root = parse('desc = "a = b { c } # d"')
    assert root.get("desc") == "a = b { c } # d"


def test_unterminated_quote_does_not_hang_or_crash():
    root = parse('name = "hello world')
    assert root.get("name") == "hello world"


def test_comment_at_eof_without_newline():
    root = parse("a = 1 # trailing comment, no newline at EOF")
    assert root.get("a") == "1"
    assert root.get("comment") is None


def test_empty_and_whitespace_only_yield_empty():
    assert parse("").entries == []
    assert parse("   \n\t  \n").entries == []


def test_empty_block_and_list_value():
    assert parse("x = {}").get("x").entries == []
    lst = parse("x = { a b c }").get("x")
    assert [v for k, v in lst.entries if k == ""] == ["a", "b", "c"]


def test_unbalanced_brace_at_eof_is_graceful():
    root = parse("x = { y = { z = 1 ")
    assert root.get("x").get("y").get("z") == "1"


def test_scripted_gui_visible_gate_with_compound_operator():
    import parse_scripted_gui as SG
    txt = """
    scripted_gui = {
      my_overlay = {
        context_type = player_context
        visible = { check_variable = { threat_level >= 50 } }
        effects = { btn_click = { add_to_variable = { total >= 1 } } }
      }
    }
    """
    body = parse(txt).get("scripted_gui").get("my_overlay")
    rec = SG.parse_gui_def("my_overlay", body, "f.txt", set())
    assert "threat_level" in rec["visible_vars"], rec["visible_vars"]
    assert not (set(rec["visible_vars"]) & {">", "<", "!"}), rec["visible_vars"]
    assert "total" in rec["writes"], rec["writes"]


if __name__ == "__main__":
    import pytest
    raise SystemExit(pytest.main([__file__, "-v"]))
