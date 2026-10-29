"""Tests for tools/stack-frames.py: python tools/stack_frames_test.py"""

import importlib.util
import os
import unittest

SPEC = importlib.util.spec_from_file_location(
    'stack_frames', os.path.join(os.path.dirname(os.path.abspath(__file__)), 'stack-frames.py'))
sf = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sf)

KIB = 1024


class Deepest(unittest.TestCase):
    def test_a_call_holds_the_callers_frame_under_the_callee(self):
        graph = {1: {2: True}}
        self.assertEqual(sf.deepest(graph, {1: 50 * KIB, 2: 50 * KIB}, 1), (100 * KIB, [1, 2]))

    def test_a_tail_jump_replaces_the_callers_frame(self):
        graph = {1: {2: False}, 2: {3: False}}
        frames = {1: 50 * KIB, 2: 50 * KIB, 3: 40 * KIB}
        self.assertEqual(sf.deepest(graph, frames, 1), (50 * KIB, [1]))
        frames[3] = 60 * KIB
        self.assertEqual(sf.deepest(graph, frames, 1), (60 * KIB, [1, 2, 3]))

    def test_a_tail_jump_after_a_call_keeps_the_calling_frames(self):
        # 1 calls 2, which tail-jumps to 3: 3 runs on top of 1's frame, not 2's.
        graph = {1: {2: True}, 2: {3: False}}
        frames = {1: 10 * KIB, 2: 20 * KIB, 3: 30 * KIB}
        self.assertEqual(sf.deepest(graph, frames, 1), (40 * KIB, [1, 2, 3]))

    def test_a_recursive_chain_counts_each_function_once(self):
        graph = {1: {2: True}, 2: {1: True}}
        self.assertEqual(sf.deepest(graph, {1: 8, 2: 16}, 1), (24, [1, 2]))


if __name__ == '__main__':
    unittest.main()
