import unittest
from pathlib import Path

import parse_diplo


FIXTURE = """HOI4txt
date=1944.2.11.18
countries={
 GER={ diplomacy={ active_relations={
  SOV={ war_relation={
   first=GER second=SOV start_date=1941.6.12.2
   first_casualties=6722065 second_casualties=8060350
   threat=17.9599 first_was_instigator=yes
   hostility_reason_instigator=war hostility_reason_defender=war
  } }
 } } }
 SOV={ diplomacy={ active_relations={
  GER={ war_relation={
   first=GER second=SOV start_date=1941.6.12.2
   first_casualties=6722065 second_casualties=8060350
   threat=17.9599 first_was_instigator=yes
   hostility_reason_instigator=war hostility_reason_defender=war
  } }
 } } }
}
"""


class ParseDiploB5Test(unittest.TestCase):
    def test_preserves_exact_b5_rows_and_fixed5_threat(self):
        outputs, _ = parse_diplo.build(FIXTURE, Path("fixture.hoi4"), False)
        out = outputs["diplomacy.json"]

        self.assertEqual(out["meta"]["stats"]["wars"], 1)
        self.assertEqual(out["meta"]["stats"]["war_relation_rows"], 2)
        self.assertEqual(
            [row["owner"] for row in out["war_relation_rows"]],
            ["GER", "SOV"],
        )
        for row in out["war_relation_rows"]:
            self.assertEqual(row["threat_text"], "17.9599")
            self.assertEqual(row["threat_fixed5_raw"], 1_795_990)
            self.assertTrue(row["first_was_instigator"])
            self.assertEqual(row["hostility_reason_instigator"], "war")
            self.assertEqual(row["hostility_reason_defender"], "war")

    def test_preserves_explicit_false_instigator(self):
        text = FIXTURE.replace("first_was_instigator=yes", "first_was_instigator=no")
        outputs, _ = parse_diplo.build(text, Path("fixture.hoi4"), False)
        self.assertFalse(outputs["diplomacy.json"]["war_relation_rows"][0]["first_was_instigator"])


if __name__ == "__main__":
    unittest.main()
