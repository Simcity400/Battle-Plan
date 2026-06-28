import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_profile_check import (  # noqa: E402
    country_payload_summary,
    validate_profile,
    validate_root_probe_against_profile,
)


class TestCollectorProfileCheck(unittest.TestCase):
    def test_profile_shape_validates(self):
        profile = _profile()

        self.assertEqual(validate_profile(profile), [])

    def test_profile_shape_rejects_bad_hash(self):
        profile = _profile()
        profile["exe_sha256"] = "not-a-hash"

        problems = validate_profile(profile)

        self.assertTrue(any("exe_sha256" in p for p in problems), problems)

    def test_root_probe_matches_profile_offsets_and_domains(self):
        profile = _profile()
        root_probe = {
            "profile": "vanilla_latest_2026_06_28",
            "image_base": "0x10000000",
            "daily_update": "0x101D4540",
            "get_long_string": "0x105269B0",
            "daily_update_sentinel_match": True,
            "long_string_sentinel_match": True,
            "domains": ["clock", "country_identity"],
        }

        self.assertEqual(validate_root_probe_against_profile(root_probe, profile), [])

    def test_root_probe_mismatch_reports_fail_closed_reasons(self):
        profile = _profile()
        root_probe = {
            "profile": "wrong",
            "image_base": "0x10000000",
            "daily_update": "0x101D4541",
            "get_long_string": "0x105269B0",
            "daily_update_sentinel_match": False,
            "long_string_sentinel_match": True,
            "domains": ["clock"],
        }

        problems = validate_root_probe_against_profile(root_probe, profile)

        self.assertIn("root_probe_profile_mismatch", problems)
        self.assertIn("daily_update_sentinel_not_verified", problems)
        self.assertIn("daily_update_address_mismatch", problems)
        self.assertIn("root_probe_missing_domains:country_identity", problems)

    def test_country_payload_summary_distinguishes_identity_from_scalars(self):
        root_probe = {
            "domains": ["clock", "country_identity"],
            "country_object_identity": {"status": "disabled_speed_safe"},
        }
        frame = {
            "countries": {
                "GER": {"present": True, "identity": {"tag_db_id": 1}},
                "ENG": {"present": True, "identity": {"tag_db_id": 2}},
            },
        }

        summary = country_payload_summary(frame, root_probe)

        self.assertEqual(summary["country_rows"], 2)
        self.assertEqual(summary["country_object_identity_status"], "disabled_speed_safe")
        self.assertFalse(summary["country_scalar_domain_present"])
        self.assertEqual(summary["country_scalar_rows"], 0)

    def test_country_payload_summary_counts_scalar_rows(self):
        root_probe = {
            "domains": ["clock", "country_identity", "country_scalars"],
            "country_object_identity": {"status": "control_bridge_verified"},
        }
        frame = {
            "countries": {
                "GER": {
                    "present": True,
                    "identity": {"tag_db_id": 1},
                    "scalars": {"stability": 0.8, "political_power": None},
                },
                "ENG": {
                    "present": True,
                    "identity": {"tag_db_id": 2},
                    "scalars": {"stability": None, "political_power": None},
                },
            },
        }

        summary = country_payload_summary(frame, root_probe)

        self.assertEqual(summary["country_rows"], 2)
        self.assertEqual(summary["country_object_identity_status"], "control_bridge_verified")
        self.assertTrue(summary["country_scalar_domain_present"])
        self.assertEqual(summary["country_scalar_rows"], 1)


def _profile():
    return {
        "label": "vanilla_latest_2026_06_28",
        "exe_sha256": "0" * 64,
        "hooks": {
            "daily_update": {
                "offset": "0x1D4540",
                "sentinel_bytes": "40 55",
            },
        },
        "roots": {
            "long_string": {
                "offset": "0x5269B0",
                "sentinel_bytes": "48 89",
            },
        },
        "domains_enabled": ["clock", "country_identity"],
    }


if __name__ == "__main__":
    unittest.main()
