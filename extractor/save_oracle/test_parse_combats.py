import unittest

import parse_combats


class ParseCombatsTest(unittest.TestCase):
    def test_persisted_naval_and_exact_terminal_history(self):
        text = """
date=1944.2.11.18
combat={
 land_combat={
  id={ id=10 type=4712 } location=100 day=2 duration=3 terrain=plains
  attacker={ losses=0 tactic=7 log={ progress=12.5 } }
  defender={ losses=0 tactic=8 log={ progress=22.5 } }
 }
 naval_combat={
  id={ id=20 type=4713 } location=200 day=4 duration=5 terrain=shallow_sea
  attacker={ unit={ id=30 type=71 } }
  defender={ unit={ id=40 type=71 } }
 }
 history={ history={
  { location=300 attacker=GER defender=ENG end_date=1944.2.10.3 type=2 }
  { location=301 attacker=--- defender=D04 end_date=1944.2.10.4 type=1 }
 } }
}
"""
        combats, kinds, history, meta = parse_combats.parse_combats_text(text)

        self.assertEqual(meta["date"], "1944.2.11.18")
        self.assertEqual(kinds, {"land": 1, "naval": 1})
        self.assertEqual([row["kind"] for row in combats], ["land", "naval"])
        self.assertEqual(
            history,
            [
                {
                    "location": 300,
                    "attacker": "GER",
                    "defender": "ENG",
                    "end_date": "1944.2.10.3",
                    "type_raw": 2,
                },
                {
                    "location": 301,
                    "attacker": "---",
                    "defender": "D04",
                    "end_date": "1944.2.10.4",
                    "type_raw": 1,
                },
            ],
        )


if __name__ == "__main__":
    unittest.main()
