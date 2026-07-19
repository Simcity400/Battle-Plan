"""Focused B1 market-stockpile save-oracle completeness tests."""

from pathlib import Path

import parse_market


SYNTHETIC_SAVE = r'''
date=1940.1.1.1
version="Operation Postern v1.19.2.0"
equipments={
    infantry_equipment_0={ id={ id=90 type=70 } }
}
equipment_market={ contracts={ } requests={ } }
countries={
    SAF={
        equipment_market={
            market_stockpile={
                equipments={
                    equipment={ id={ id=90 type=70 } amount=767 }
                }
            }
        }
    }
    BUL={
        equipment_market={ market_stockpile={ equipments={ } } }
    }
    ENG={ }
}
'''


def test_market_stockpile_preserves_measured_empty_country():
    files, _ = parse_market.build(SYNTHETIC_SAVE, Path("synthetic.hoi4"), True)
    market = files["market.json"]

    assert list(market["countries"]) == ["BUL", "SAF"]
    assert market["countries"]["BUL"]["market_stockpile"] == []
    assert market["countries"]["BUL"]["market_stockpile_complete"] is True
    assert market["countries"]["SAF"]["market_stockpile"] == [
        {
            "equip_id": 90,
            "equip_type": 70,
            "archetype": "infantry_equipment_0",
            "amount": 767.0,
        }
    ]
    stats = market["meta"]["stats"]
    assert stats["countries_with_market_data"] == 2
    assert stats["countries_with_market_stockpile"] == 2
    assert stats["empty_market_stockpiles"] == 1
    assert stats["total_market_stockpile_entries"] == 1
