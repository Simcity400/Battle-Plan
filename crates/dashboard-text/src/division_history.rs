use content_reader_localisation::localisation::Argument;
use replay_data::rows::DivisionHistoryRow;

pub fn event_key(entry: &DivisionHistoryRow) -> Result<&[u8], String> {
    const KEYS: [&[u8]; 25] = [
        b"CONQUERED_PROVINCE_HISTORY",
        b"UNIT_KILL_HISTORY",
        b"COMMANDER_PROMOTED_HISTORY",
        b"NEW_COMMANDER_HISTORY",
        b"INVASION_SUCCESS_HISTORY",
        b"PROVINCE_LIBERATED_HISTORY",
        b"CORE_LIBERATED_HISTORY",
        b"HIGH_VP_HISTORY",
        b"AIRFIELD_HISTORY",
        b"CAPITAL_HISTORY",
        b"PARADROP_HISTORY",
        b"UNIT_RENAMED_HISTORY",
        b"WAS_ON_OPERATION_HISTORY",
        b"WAS_ON_OPERATION_EXCELLED_HISTORY",
        b"RECEIVED_MANY_CASUALTIES_HISTORY",
        b"RAID_HISTORY",
        b"",
        b"",
        b"CAPTAIN_PROMOTED_HISTORY",
        b"NEW_CAPTAIN_HISTORY",
        b"SHIP_LAUNCHED_HISTORY",
        b"SHIP_RENAMED_HISTORY",
        b"SHIP_REFITTED_HISTORY",
        b"WAS_IN_NAVAL_COMBAT_HISTORY",
        b"WAS_IN_NAVAL_COMBAT_EXCELLED_HISTORY",
    ];
    if entry.kind == 16 {
        return Ok(&entry.custom_name);
    }
    KEYS.get(usize::try_from(entry.kind).unwrap_or(usize::MAX))
        .copied()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            format!(
                "division history event kind {} has no ordinary description",
                entry.kind
            )
        })
}

pub fn date(hour: i32) -> String {
    let elapsed = i64::from(hour) - i64::from(replay_data::temperature::EPOCH);
    let year = elapsed.div_euclid(8760);
    let mut day = elapsed.div_euclid(24).rem_euclid(365);
    let mut month = 1;
    for length in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    format!("{year}.{month}.{}", day + 1)
}

pub fn arguments<'a>(
    entry: &DivisionHistoryRow,
    subject: &'a str,
    country: &'a str,
    province: &'a str,
    date: &'a str,
) -> [(&'static str, Argument<'a>); 7] {
    [
        ("DATE", Argument::Text(date)),
        ("SUBJECT", Argument::Text(subject)),
        ("TARGET_COUNTRY", Argument::Text(country)),
        ("PROVINCE", Argument::Text(province)),
        ("VALUE", Argument::Fixed5(entry.multiplier)),
        ("AMOUNT", Argument::Int32(i32::from(entry.medal_count))),
        ("ORDERS", Argument::Int32(entry.orders)),
    ]
}

pub fn province_list(
    provinces: &[String],
    token: impl Fn(&[u8], &[(&str, Argument<'_>)]) -> Result<String, String>,
) -> Result<String, String> {
    match provinces {
        [] => Ok(String::new()),
        [one] => Ok(one.clone()),
        [first, second] => token(
            b"HISTORY_PROVINCE_LIST_TWO_ITEMS",
            &[
                ("FIRST", Argument::Text(first)),
                ("SECOND", Argument::Text(second)),
            ],
        ),
        _ => {
            let separator = token(b"HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_SEPARATOR", &[])?;
            let last = token(b"HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_LATTER_SEPARATOR", &[])?;
            let (final_item, preceding) = provinces.split_last().unwrap();
            Ok(format!(
                "{}{}{}",
                preceding.join(&separator),
                last,
                final_item
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_and_mod_event_keys_preserve_native_values() {
        assert_eq!(date(replay_data::temperature::EPOCH + 1936 * 8760), "1936.1.1");
        assert_eq!(date(replay_data::temperature::EPOCH + 1936 * 8760 + 59 * 24), "1936.3.1");
        let mut entry = DivisionHistoryRow {
            kind: 16,
            custom_name: b"mod_event".to_vec(),
            ..Default::default()
        };
        assert_eq!(event_key(&entry).unwrap(), b"mod_event");
        entry.kind = 1000;
        assert!(event_key(&entry).is_err());
        entry.kind = 17;
        assert!(event_key(&entry).is_err(), "an unused kind is refused by number");
        entry.kind = 11;
        assert_eq!(event_key(&entry).unwrap(), b"UNIT_RENAMED_HISTORY");
    }
    #[test]
    fn province_lists_use_the_recorded_two_item_and_final_separators() {
        let token = |key: &[u8], arguments: &[(&str, Argument<'_>)]| -> Result<String, String> {
            Ok(match key {
                b"HISTORY_PROVINCE_LIST_TWO_ITEMS" => match arguments {
                    [(_, Argument::Text(first)), (_, Argument::Text(second))] => format!("{second} plus {first}"),
                    _ => unreachable!(),
                },
                b"HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_SEPARATOR" => " / ".into(),
                _ => " finally ".into(),
            })
        };
        let values = ["A".to_string(), "B".into(), "C".into(), "D".into()];
        assert_eq!(province_list(&values[..0], token).unwrap(), "");
        assert_eq!(province_list(&values[..1], token).unwrap(), "A");
        assert_eq!(province_list(&values[..2], token).unwrap(), "B plus A");
        assert_eq!(province_list(&values, token).unwrap(), "A / B / C finally D");
    }
}
