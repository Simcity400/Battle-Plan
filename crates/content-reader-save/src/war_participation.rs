
use super::{Decimal, Item, block, field, integer, invalid, required_bytes, scalar};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ledger {
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    pub equipment_damage: i64,
    pub province_capture: i64,
    pub air_damage: i64,
    pub strategic_bombing: i64,
    pub sunk_ships: i64,
    pub sunk_convoys: i64,
    pub casualties: i64,
    pub lend_lease_sent: i64,
    pub lend_lease_received: i64,
    pub captured_provinces: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryHistory {
    pub owner: Vec<u8>,
    pub target: Vec<u8>,
    pub industry_sent: i64,
    pub fuel_sent: i64,
    pub industry_received: i64,
    pub fuel_received: i64,
}
pub fn parse_delivery_history(
    owner: &[u8],
    target: &[u8],
    items: &[Item],
) -> io::Result<DeliveryHistory> {
    let fixed =
        |key: &[u8]| -> io::Result<i64> { Ok(Decimal::parse(&required_bytes(items, key)?)?.raw) };
    Ok(DeliveryHistory {
        owner: owner.to_vec(),
        target: target.to_vec(),
        industry_sent: fixed(b"ic_given")?,
        fuel_sent: fixed(b"fuel_given")?,
        industry_received: fixed(b"ic_received")?,
        fuel_received: fixed(b"fuel_received")?,
    })
}

pub fn parse(items: &[Item]) -> io::Result<Ledger> {
    let fixed =
        |key: &[u8]| -> io::Result<i64> { Ok(Decimal::parse(&required_bytes(items, key)?)?.raw) };
    let captured_provinces = match field(items, b"captured_provinces")? {
        None => Vec::new(),
        Some(value) => {
            let entries = block(value)?;
            if entries.is_empty() {
                return Err(invalid(
                    "war participation emits captured provinces only for a nonempty set",
                ));
            }
            let mut captured = Vec::new();
            for entry in entries {
                if entry.key.is_some() {
                    return Err(invalid("war captured-province set has a keyed entry"));
                }
                let province = integer::<i32>(scalar(&entry.value)?)?;
                if captured
                    .last()
                    .is_some_and(|&previous| previous >= province)
                {
                    return Err(invalid(
                        "war captured-province set is not in signed key order",
                    ));
                }
                captured.push(province);
            }
            captured
        }
    };
    Ok(Ledger {
        first: required_bytes(items, b"first")?,
        second: required_bytes(items, b"second")?,
        equipment_damage: fixed(b"equipment_damage")?,
        province_capture: fixed(b"province_capture")?,
        air_damage: fixed(b"air_damage_str")?,
        strategic_bombing: fixed(b"strategic_air")?,
        sunk_ships: fixed(b"sunk_ship")?,
        sunk_convoys: fixed(b"convoy_attack")?,
        casualties: fixed(b"casualties")?,
        lend_lease_sent: fixed(b"lend_lease_sent")?,
        lend_lease_received: fixed(b"lend_lease_received")?,
        captured_provinces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_parser::parser;

    fn ledger(extra: &str) -> io::Result<Ledger> {
        parse(&parser::parse(
            format!(
                "first=MOD second=D01 equipment_damage=90071992547.40993 province_capture=-0.00001 \
             air_damage_str=0 strategic_air=0 sunk_ship=0 convoy_attack=0 casualties=4 \
             lend_lease_sent=0 lend_lease_received=-92233720368547.75808 {extra}"
            )
            .as_bytes(),
        )?.items)
    }

    #[test]
    fn participation_save_retains_exact_signed_credit_and_preexisting_captures() {
        let value = ledger("captured_provinces={ -2147483648 0 50000 2147483647 }").unwrap();
        assert_eq!(value.first, b"MOD");
        assert_eq!(value.second, b"D01");
        assert_eq!(value.equipment_damage, 9_007_199_254_740_993);
        assert_eq!(value.province_capture, -1);
        assert_eq!(value.lend_lease_received, i64::MIN);
        assert_eq!(value.captured_provinces, [i32::MIN, 0, 50000, i32::MAX]);
        assert!(ledger("").unwrap().captured_provinces.is_empty());
    }

    #[test]
    fn participation_save_does_not_turn_invalid_or_missing_credit_into_zero() {
        for extra in [
            "captured_provinces={}",
            "captured_provinces={ 2 1 }",
            "captured_provinces={ 1 1 }",
            "captured_provinces={ 1=yes }",
            "equipment_damage=0",
            "lend_lease_received=0",
        ] {
            assert!(ledger(extra).is_err(), "{extra}");
        }
        assert!(parse(&parser::parse(b"first=A second=B").unwrap().items).is_err());
    }

    #[test]
    fn participation_save_keeps_both_directions_separate_from_integer_losses() {
        let fields = |first: &str, second: &str, equipment: &str| {
            format!(
                "first={first} second={second} equipment_damage={equipment} province_capture=0 \
             air_damage_str=0 strategic_air=0 sunk_ship=0 convoy_attack=0 casualties=0.00003 \
             lend_lease_sent=0 lend_lease_received=-1"
            )
        };
        let source = format!(
            "first=MOD second=D01 cancel=yes first_casualties=77 second_casualties=88 \
             first_unknown_casualties=99 second_unknown_casualties=111 first_was_instigator=yes \
             war_score_first_vs_second={{ {} captured_provinces={{ 1 7 }} }} \
             war_score_second_vs_first={{ {} }}",
            fields("MOD", "D01", "1.00001"),
            fields("D01", "MOD", "2.00002")
        );
        let relation = super::super::parse_war_relation(
            b"MOD",
            b"D01",
            &parser::parse(source.as_bytes()).unwrap().items,
        )
        .unwrap();
        assert_eq!(relation.first_casualties, 77);
        assert_eq!(relation.second_unknown_casualties, 111);
        let first = relation.participation[0].as_ref().unwrap();
        let second = relation.participation[1].as_ref().unwrap();
        assert_eq!(
            (first.equipment_damage, second.equipment_damage),
            (100_001, 200_002)
        );
        assert_eq!((first.casualties, second.casualties), (3, 3));
        assert_eq!(first.captured_provinces, [1, 7]);
        assert!(second.captured_provinces.is_empty());
        assert_eq!(second.first, b"D01");
        assert_eq!(second.second, b"MOD");
    }

    #[test]
    fn participation_delivery_history_keeps_sub_binary_precision_and_signed_credit() {
        let document = parser::parse(
            b"ic_given=0.00003 fuel_given=-1 ic_received=90071992547.40993 fuel_received=0",
        )
        .unwrap();
        let history = parse_delivery_history(b"MOD", b"D01", &document.items).unwrap();
        assert_eq!(history.industry_sent, 3);
        assert_eq!(history.fuel_sent, -100_000);
        assert_eq!(history.industry_received, 9_007_199_254_740_993);
        assert_eq!(
            (history.owner, history.target),
            (b"MOD".to_vec(), b"D01".to_vec())
        );
    }
}
