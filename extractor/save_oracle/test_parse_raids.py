import unittest
from pathlib import Path

import parse_raids


FIXTURE = """HOI4txt
date=1944.2.11.18
raids={
 countries={
  { priority=1 dummy={ id={ id=1 type=84 } } }
  { tag=GER priority=1 dummy={ id={ id=2 type=84 } }
    raid_instance={
     { id={ id=509 type=84 } prep_time=2160 phase=ENDED outcome=FAILURE
       raid_source={ type=air_base province=6570 tag=GER }
       end_date=1940.5.2.6 victim_country=FRA }
     { id={ id=513 type=84 } phase=ENDED outcome=CANCELED
       raid_source={ type=air_base province=6570 tag=GER }
       end_date=1940.7.26.24 victim_country=ENG }
    }
  }
 }
}
"""


class ParseRaidsTest(unittest.TestCase):
    def test_preserves_parent_child_order_identity_and_dummy(self):
        statuses, raids, meta = parse_raids.parse_raids_text(FIXTURE)
        self.assertEqual(meta["date"], "1944.2.11.18")
        self.assertEqual(len(statuses), 2)
        self.assertEqual(statuses[0]["classification"], "missing_tag_dummy")
        self.assertEqual((statuses[0]["dummy_id"], statuses[0]["dummy_id_type"]), (1, 84))
        self.assertEqual(statuses[1]["classification"], "country")
        self.assertEqual([row["id"] for row in raids], [509, 513])
        self.assertEqual([row["raid_index"] for row in raids], [0, 1])
        self.assertEqual(raids[0]["owner_tag"], "GER")
        self.assertEqual(raids[0]["raid_source"]["province"], 6570)
        self.assertEqual(raids[1]["victim_country"], "ENG")

    def test_build_uses_distinct_contract_and_reports_phase_identity(self):
        outputs, _ = parse_raids.build(FIXTURE, Path("fixture.hoi4"), False)
        self.assertEqual(list(outputs), ["raid_instances.json"])
        stats = outputs["raid_instances.json"]["meta"]["stats"]
        self.assertEqual(stats["country_statuses"], 2)
        self.assertEqual(stats["raid_instances"], 2)
        self.assertEqual(stats["phase_counts"], {"ENDED": 2})
        self.assertEqual(stats["raid_instances_with_id"], 2)
        self.assertEqual(stats["unique_raid_ids"], 2)
        self.assertEqual(stats["duplicate_raid_ids"], [])


if __name__ == "__main__":
    unittest.main()
