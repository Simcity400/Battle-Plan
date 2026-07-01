from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import slot_integration_anchor_gate as gate


class SlotIntegrationAnchorGateTests(unittest.TestCase):
    def test_domain_maps_reject_44_and_45(self) -> None:
        collector = """
        const COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE: u16 = 42;
        const COMPACT_DOMAIN_MARKET_TRADE_LIVE: u16 = 43;
        const COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE: u16 = 44;
        const COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE: u16 = 45;
        """
        recorder = """
        const COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE: u16 = 45;
        fn compact_domain_id(domain: &str) -> Option<u16> {
            match domain {
                "supply_logistics_attrition_live" => Some(44),
                "focus_research_decisions_depth_live" => {
                    Some(COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE)
                }
                _ => None,
            }
        }
        fn compact_domain_name(domain_id: u16) -> &'static str {
            match domain_id {
                44 => "supply_logistics_attrition_live",
                COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE => {
                    "focus_research_decisions_depth_live"
                }
                _ => "unknown",
            }
        }
        """
        collector_ids = gate.collector_domain_ids(collector)
        recorder_ids = gate.recorder_domain_id_map(recorder)
        recorder_names = gate.recorder_domain_name_map(recorder)

        self.assertEqual(collector_ids["COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE"], 44)
        self.assertEqual(collector_ids["COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE"], 45)
        self.assertEqual(recorder_ids["supply_logistics_attrition_live"], 44)
        self.assertEqual(recorder_ids["focus_research_decisions_depth_live"], 45)
        self.assertEqual(recorder_names[44], "supply_logistics_attrition_live")
        self.assertEqual(recorder_names[45], "focus_research_decisions_depth_live")
        self.assertEqual(gate.ASSIGNED_DOMAIN_ID, 46)
        self.assertEqual(gate.DOMAIN_NAME, "air_variant_archetypes_live")
        self.assertNotIn(gate.ASSIGNED_DOMAIN_ID, gate.REJECTED_PROVISIONAL_DOMAIN_IDS)


if __name__ == "__main__":
    unittest.main()
