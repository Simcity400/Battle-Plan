"""Regression coverage for exact occupation garrison/history save carriers."""

from pathlib import Path
import sys
import unittest


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
MEMORY_OCCUPATION = REPO / "extractor" / "memory_reader" / "occupation"
for path in (HERE, MEMORY_OCCUPATION):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import parse_occupation  # noqa: E402
import build_garrison_oracle  # noqa: E402


SAVE = r'''
date=1942.8.16.19
version="1.19.2.0.a729"
player="GER"
countries={
 GER={
  occupation_status={
   division_template_id={ id=4183 type=4713 }
   default_law=foreign_civilian_oversight
   occupation={
    FRA={
     states={ 1 2 }
     resistance=12.5
     compliance=44.25
     occupation_law_list={ 1=military_governor_occupation }
     state_garrison_data={
      1={
       strength_ratio=0.9375
       garrison_required=0.55477
       army_manpower_need=13869
       manpower_pool={ value={ tag="GER" value=13869 } }
       equipment={
        equipment={ id={ id=90 type=70 } amount=1664.31 }
        allow_zero_entries=no
       }
       equipment_need={ infantry_equipment=1664.31 }
       garrison_reinforcement_requests={
        request={
         total_progress=7.57643
         need={ infantry_equipment=661.31 }
         date=1942.8.18.2
        }
       }
      }
      2={
       strength_ratio=1
       garrison_required=0
       army_manpower_need=0
       equipment={ allow_zero_entries=no }
      }
     }
    }
   }
   resistance_attack_log={
    state=1
    manpower={ value={ tag="GER" value=33 } }
    country="FRA"
    date=1942.8.15.1
    garrison=yes
    equipment={
     equipment={ id={ id=90 type=70 } amount=1 }
     allow_zero_entries=no
    }
   }
   resistance_attack_log={
    state=2
    country="FRA"
    date=1942.8.16.1
    garrison=no
    equipment={ allow_zero_entries=no }
    resistance_activity="sabotage_infrastructure"
   }
  }
 }
}
'''


class OccupationSaveCarrierTests(unittest.TestCase):
    def _assert_contract(self, occupation):
        ger = occupation["countries"]["GER"]
        template = ger["garrison_template_id"]
        if isinstance(template, tuple):
            self.assertEqual(template, (4183, 4713))
        else:
            self.assertEqual(template, 4183)
            self.assertEqual(ger["garrison_template_type"], 4713)
        group = ger["occupied"][0]
        self.assertEqual(group["occupation_law_by_state"][1],
                         "military_governor_occupation")

        first = group["state_garrison_data"][1]
        self.assertEqual(first["army_manpower_need"], 13869)
        self.assertEqual(first["equipment"][0]["equipment_id"], 90)
        self.assertTrue(first["equipment_meta"]["rows_complete"])
        self.assertEqual(first["reinforcement_requests"][0]["need"],
                         {"infantry_equipment": 661.31})
        self.assertEqual(first["reinforcement_requests"][0]["total_progress"],
                         7.57643)

        measured_empty = group["state_garrison_data"][2]
        self.assertEqual(measured_empty["equipment"], [])
        self.assertTrue(measured_empty["equipment_meta"]["rows_complete"])
        self.assertFalse(measured_empty["equipment_meta"]["allow_zero_entries"])

        history = ger["resistance_attack_log"]
        self.assertEqual([row["source_ordinal"] for row in history], [0, 1])
        self.assertTrue(history[0]["attack_blocked"])
        self.assertEqual(history[0]["lost_manpower"], 33)
        self.assertFalse(history[1]["attack_blocked"])
        self.assertEqual(history[1]["lost_manpower"], 0)
        self.assertEqual(history[1]["equipment_loss_rows"], [])
        self.assertEqual(history[1]["resistance_activity"],
                         "sabotage_infrastructure")
        self.assertTrue(ger["resistance_attack_log_complete"])

    def test_production_save_oracle_preserves_exact_rows_and_empty_collections(self):
        outputs, _report = parse_occupation.build(
            SAVE, Path("SYN_1942_melted.txt"), False
        )
        self._assert_contract(outputs["occupation.json"])

    def test_garrison_development_oracle_matches_production_contract(self):
        oracle = build_garrison_oracle.build_oracle(SAVE, "SYN_1942_melted.txt")
        self._assert_contract(oracle)
        self.assertTrue(oracle["resistance_attack_history_complete"])


if __name__ == "__main__":
    unittest.main()
