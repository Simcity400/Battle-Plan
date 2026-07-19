import unittest
from pathlib import Path

import parse_production


class ProductionEfficiencySaveOracleTests(unittest.TestCase):
    def test_preserves_exact_line_industrial_manufacturer_and_null(self):
        text = """
equipments={ infantry_equipment_1={ id={ id=7316 type=70 } } }
countries={
  SPR={ production={
    military_lines={
      id={ id=366 type=56 }
      equipment_variant_index={ id=7316 type=70 }
      industrial_manufacturer={ id=42 type=79 }
    }
    military_lines={
      id={ id=367 type=56 }
      equipment_variant_index={ id=7316 type=70 }
    }
  } }
}
"""
        outputs, _ = parse_production.build(text, Path("fixture.hoi4"), False)
        out = outputs["production.json"]
        assigned, unassigned = out["countries"]["SPR"]["equipment_lines"]
        self.assertEqual(assigned["industrial_manufacturer"], [42, 79])
        self.assertEqual(assigned["mio_id"], 42)
        self.assertIsNone(unassigned["industrial_manufacturer"])
        self.assertIsNone(unassigned["mio_id"])
        self.assertEqual(out["meta"]["stats"]["production_lines_with_mio"], 1)
        self.assertEqual(
            out["meta"]["stats"]["production_lines_with_invalid_mio_type"], 0
        )

    def test_preserves_ordered_factory_efficiencies(self):
        text = """
equipments={ infantry_equipment_1={ id={ id=7316 type=70 } } }
countries={
  SPR={ production={
    military_lines={
      id={ id=366 type=1 }
      equipment_variant_index={ id=7316 type=70 }
      active_factories=5
      factory_efficiencies={ 64.59667 63.64987 63.64987 63.07492 62.71997 }
    }
  } }
}
"""
        by_tag, _, _ = parse_production.parse_production_text(text)
        line = by_tag["SPR"]["lines"][0]
        self.assertEqual(
            line["factory_efficiencies"],
            [64.59667, 63.64987, 63.64987, 63.07492, 62.71997],
        )

    def test_preserves_proven_empty_factory_efficiency_vector(self):
        text = """
countries={
  SPR={ production={
    military_lines={ id={ id=1 type=1 } active_factories=0 factory_efficiencies={ } }
  } }
}
"""
        by_tag, _, _ = parse_production.parse_production_text(text)
        self.assertEqual(by_tag["SPR"]["lines"][0]["factory_efficiencies"], [])


if __name__ == "__main__":
    unittest.main()
