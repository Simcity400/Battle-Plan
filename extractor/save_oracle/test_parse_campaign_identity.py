import unittest

import parse_campaign_identity as pci


VALID = """HOI4txt
date=1944.2.11.18
game_unique_seed=1026046527
game_unique_id=\"a4b17fed-ed12-4361-9064-fb6576638110\"
multiplayer_random_seed=-1033122218
countries={ game_unique_seed=7 }
"""


class CampaignIdentityOracleTests(unittest.TestCase):
    def test_d04_pair_and_digest(self):
        parsed = pci.parse_identity(VALID)
        self.assertTrue(parsed["identity"]["pair_valid"])
        self.assertEqual(parsed["identity"]["game_unique_seed"], "1026046527")
        self.assertEqual(
            parsed["identity"]["game_unique_id"],
            "a4b17fed-ed12-4361-9064-fb6576638110",
        )
        self.assertEqual(
            parsed["identity"]["campaign_id_v1"],
            "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
        )
        self.assertEqual(
            parsed["diagnostic_witnesses"]["multiplayer_random_seed"],
            "-1033122218",
        )
        self.assertEqual(parsed["save_date"], "1944.2.11.18")

    def test_uuid_is_canonicalized_but_seed_must_already_be_canonical(self):
        text = VALID.replace(
            "a4b17fed-ed12-4361-9064-fb6576638110",
            "A4B17FED-ED12-4361-9064-FB6576638110",
        )
        parsed = pci.parse_identity(text)
        self.assertTrue(parsed["identity"]["pair_valid"])
        self.assertEqual(
            parsed["identity"]["game_unique_id"],
            "a4b17fed-ed12-4361-9064-fb6576638110",
        )
        self.assertFalse(
            pci.parse_identity(VALID.replace("1026046527", "01026046527"))[
                "identity"
            ]["pair_valid"]
        )

    def test_signed_i32_boundaries(self):
        self.assertEqual(pci.canonical_i32("-2147483648"), "-2147483648")
        self.assertEqual(pci.canonical_i32("2147483647"), "2147483647")
        for value in ("-2147483649", "2147483648", "-0", "+1", "01"):
            self.assertIsNone(pci.canonical_i32(value), value)

    def test_duplicate_required_key_fails_without_last_wins(self):
        parsed = pci.parse_identity(VALID + "\ngame_unique_seed=7\n")
        self.assertFalse(parsed["identity"]["pair_valid"])
        self.assertIsNone(parsed["identity"]["game_unique_seed"])
        self.assertEqual(parsed["occurrences"]["game_unique_seed"], 2)
        self.assertTrue(any("occurs 2 times" in error for error in parsed["errors"]))

    def test_nested_identity_decoy_is_ignored(self):
        parsed = pci.parse_identity(VALID)
        self.assertEqual(parsed["occurrences"]["game_unique_seed"], 1)
        self.assertTrue(parsed["identity"]["pair_valid"])

    def test_missing_or_block_valued_pair_fails(self):
        missing = pci.parse_identity("HOI4txt\ngame_unique_seed=1\n")
        self.assertFalse(missing["identity"]["pair_valid"])
        self.assertTrue(any("missing required" in error for error in missing["errors"]))

        blocked = pci.parse_identity(
            "HOI4txt\ngame_unique_seed={ 1 }\n"
            'game_unique_id="a4b17fed-ed12-4361-9064-fb6576638110"\n'
        )
        self.assertFalse(blocked["identity"]["pair_valid"])
        self.assertTrue(any("is a block" in error for error in blocked["errors"]))

    def test_diagnostic_witness_never_controls_pair_validity(self):
        parsed = pci.parse_identity(
            VALID.replace("multiplayer_random_seed=-1033122218", "multiplayer_random_seed=bad")
        )
        self.assertTrue(parsed["identity"]["pair_valid"])
        self.assertIsNone(
            parsed["diagnostic_witnesses"]["multiplayer_random_seed"]
        )
        self.assertTrue(parsed["warnings"])


if __name__ == "__main__":
    unittest.main()
