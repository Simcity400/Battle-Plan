import unittest

import parse_combats


SYN_COMBATS = """
date=1944.2.11.18
combat={
 land_combat={
  id={ id=10 type=4712 } location=100 day=2 duration=3 terrain=plains
  attacker={ losses=0 tactic=7 log={ progress=12.5 } }
  defender={ losses=0 tactic=8 log={ progress=22.5 } }
 }
 naval_combat={
  id={ id=20 type=4713 } location=200 day=4 duration=5 terrain=shallow_sea
  attacker={ unit={ id=30 type=71 } losses=1.25 tactic=9 log={ progress=7.5 } }
  defender={ unit={ id=40 type=71 } losses=2.5 tactic=10 log={ progress=8.5 } }
 }
 air_combat={
  id={ id=50 type=4714 } location=400 day=6 duration=7 terrain=air
  attacker={
   unit={ id=60 type=72 }
   air_damage_str=3.25 air_damage_org=4.5
   ground_damage_str=5.75 ground_damage_org=6.0
   tactic=11 log={ progress=9.25 }
  }
  defender={ unit={ id=70 type=72 } tactic=12 log={ progress=10.5 } }
 }
 history={ history={
  { location=300 attacker=GER defender=ENG end_date=1944.2.10.3 type=2 }
  { location=301 attacker=--- defender=D04 end_date=1944.2.10.4 type=1 }
 } }
}
"""


class ParseCombatsTest(unittest.TestCase):
    def test_persisted_naval_and_exact_terminal_history(self):
        combats, kinds, history, meta = parse_combats.parse_combats_text(SYN_COMBATS)

        self.assertEqual(meta["date"], "1944.2.11.18")
        self.assertEqual(kinds, {"land": 1, "naval": 1, "air": 1})
        self.assertEqual(
            [row["kind"] for row in combats], ["land", "naval", "air"]
        )
        naval = combats[1]
        self.assertEqual(naval["combat_id"], [20, 4713])
        self.assertEqual(naval["attacker"]["units"], [[30, 71]])
        self.assertEqual(naval["attacker"]["unit_count"], 1)
        self.assertEqual(naval["attacker"]["losses"], 1.25)
        self.assertEqual(naval["attacker"]["progress"], 7.5)
        air = combats[2]
        self.assertEqual(air["combat_id"], [50, 4714])
        self.assertEqual(air["attacker"]["units"], [[60, 72]])
        self.assertEqual(air["attacker"]["air_damage_str"], 3.25)
        self.assertEqual(air["attacker"]["air_damage_org"], 4.5)
        self.assertEqual(air["attacker"]["ground_damage_str"], 5.75)
        self.assertEqual(air["attacker"]["ground_damage_org"], 6.0)
        self.assertEqual(air["attacker"]["progress"], 9.25)
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
