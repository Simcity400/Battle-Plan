"""Tests for tools/check-agent-context.py: python tools/check_agent_context_test.py"""

import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_agent_context", os.path.join(ROOT, "tools", "check-agent-context.py"))
cac = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cac)


class Budget(unittest.TestCase):

    def test_a_budget_above_or_below_the_measured_size_is_stale_either_way(self):
        self.assertEqual(len(cac.stale({"AGENTS.md": 10}, {"AGENTS.md": 12})), 1)
        self.assertEqual(len(cac.stale({"AGENTS.md": 10}, {"AGENTS.md": 8})), 1)

    def test_a_file_missing_from_the_budget_or_the_tree_is_stale(self):
        self.assertEqual(len(cac.stale({"AGENTS.md": 10}, {})), 1)
        self.assertEqual(len(cac.stale({}, {"AGENTS.md": 10})), 1)

    def test_only_a_larger_value_than_the_base_counts_as_growth(self):
        self.assertEqual(cac.grown({"AGENTS.md": 10}, {"AGENTS.md": 10}), [])
        self.assertEqual(cac.grown({"AGENTS.md": 9}, {"AGENTS.md": 10}), [])
        self.assertEqual(cac.grown({"AGENTS.md": 12}, {"AGENTS.md": 10}), ["AGENTS.md: 10 -> 12 (+2)"])

    def test_a_file_the_base_lacks_has_no_limit(self):
        self.assertEqual(cac.grown({"AGENTS.md": 12}, {}), [])

    def test_the_committed_budget_matches_the_tree(self):
        with open(os.path.join(ROOT, cac.BUDGET), encoding="utf-8") as budget:
            self.assertEqual(cac.stale(cac.measure(cac.ROOT), cac.read_budget(budget.read())), [])


if __name__ == "__main__":
    unittest.main()
