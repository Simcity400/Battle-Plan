import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import verify_x12_speed_pause_activation as verifier


def valid_history():
    return [
        {
            "hour_index": 100,
            "speed": 2,
            "paused": False,
            "transition_sequence": 0,
            "source": "hourly_hook_ready_baseline",
        },
        {
            "hour_index": 100,
            "speed": 3,
            "paused": False,
            "transition_sequence": 1,
            "tick_progress_f32_bits": 0x3E000000,
            "source": "speed_setter",
        },
        {
            "hour_index": 100,
            "speed": 3,
            "paused": True,
            "transition_sequence": 2,
            "tick_progress_f32_bits": 0x3E000000,
            "source": "pause_toggle",
        },
        {
            "hour_index": 100,
            "speed": 4,
            "paused": True,
            "transition_sequence": 3,
            "tick_progress_f32_bits": 0x3E000000,
            "source": "speed_setter",
        },
        {
            "hour_index": 100,
            "speed": 4,
            "paused": False,
            "transition_sequence": 4,
            "tick_progress_f32_bits": 0x3E000000,
            "source": "pause_toggle",
        },
    ]


class X12ActivationVerifierTests(unittest.TestCase):
    def test_exact_history_and_required_pattern_pass(self):
        history = verifier.validate_history(valid_history(), "fixture")
        pattern = verifier.required_action_pattern(history)
        self.assertEqual(pattern["same_hour_speed_pair_sequences"], [1, 3])
        self.assertEqual(pattern["pause_speed_resume_sequences"], [2, 3, 4])

    def test_source_that_does_not_match_delta_fails(self):
        history = valid_history()
        history[3]["source"] = "pause_forced"
        with self.assertRaisesRegex(verifier.ProofError, "source does not match"):
            verifier.validate_history(history, "fixture")

    def test_machine_pattern_requires_pause_bracket(self):
        history = valid_history()
        history.pop()
        normalized = verifier.validate_history(history, "fixture")
        with self.assertRaisesRegex(verifier.ProofError, "pause -> speed mutation -> resume"):
            verifier.required_action_pattern(normalized)


if __name__ == "__main__":
    unittest.main()
