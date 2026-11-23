use super::*;

fn weather(text: &[u8]) -> io::Result<Pack> {
    parse(&content_reader_parser::parser::parse(text)?.items)
}

#[test]
fn weather_keeps_zero_slot_repeated_entries_and_exact_snow_words() {
    let saved = weather(
        br#"weather={provinces={
            "0"={province=0 temperature=0}
            "7"={province=8 snow=0.00001 temperature=-90071992547.40993 mud=yes water=unread custom_modifiers={dry=-1 Wet=12 dry=3}}
            "0"={temperature=2}
        } regions={
            "0"={region=0 temperature=0 next_weather_change=unread}
            "4"={region=4 rain_light=yes rain_heavy=no snow=yes active_modifiers={ 1 0 -1 }}
        } seed=unread}"#,
    )
    .unwrap();
    let provinces = saved.provinces.unwrap();
    assert_eq!(
        provinces.iter().map(|row| row.slot).collect::<Vec<_>>(),
        [0, 7, 0]
    );
    assert_eq!(provinces[1].province, Some(8));
    assert_eq!(provinces[1].snow.as_ref().unwrap().raw, 1);
    assert_eq!(provinces[0].snow, None);
    assert_eq!(provinces[0].temperature.as_ref().unwrap().raw, 0);
    assert_eq!(
        provinces[1].temperature.as_ref().unwrap().raw,
        -9_007_199_254_740_993
    );
    assert_eq!(provinces[1].mud, Some(true));
    assert_eq!(provinces[0].mud, None);
    assert_eq!(
        provinces[1].custom_modifiers,
        Some(vec![
            ("dry".to_owned(), -1),
            ("Wet".to_owned(), 12),
            ("dry".to_owned(), 3)
        ])
    );
    assert_eq!(provinces[0].custom_modifiers, None);
    assert_eq!(provinces[2].province, None);
    let regions = saved.regions.unwrap();
    assert_eq!(regions[0].slot, 0);
    assert_eq!(regions[1].rain_light, Some(true));
    assert_eq!(regions[1].rain_heavy, Some(false));
    assert_eq!(regions[1].snow, Some(true));
    assert_eq!(regions[1].blizzard, None);
    assert_eq!(regions[0].active_modifiers, None);
    assert_eq!(regions[1].active_modifiers, Some(vec![1, 0, -1]));
}

#[test]
fn weather_distinguishes_missing_root_missing_rosters_and_empty_rosters() {
    assert_eq!(weather(b"").unwrap(), Pack::default());
    let present = weather(b"weather={}").unwrap();
    assert!(present.present);
    assert_eq!(present.provinces, None);
    assert_eq!(present.regions, None);
    let empty = weather(b"weather={provinces={0={custom_modifiers={}}} regions={}}").unwrap();
    assert_eq!(
        empty.provinces.as_ref().unwrap()[0].custom_modifiers,
        Some(Vec::new())
    );
    let empty = weather(b"weather={provinces={} regions={}}").unwrap();
    assert_eq!(empty.provinces, Some(Vec::new()));
    assert_eq!(empty.regions, Some(Vec::new()));
}

#[test]
fn weather_refuses_malformed_or_ambiguous_compared_fields_without_rounding() {
    for text in [
        "weather={} weather={}",
        "weather={provinces={} provinces={}}",
        "weather={provinces={{province=0}}}",
        "weather={provinces={-1={province=0}}}",
        "weather={provinces={0={snow=0.000001}}}",
        "weather={provinces={0={snow=0 snow=1}}}",
        "weather={provinces={0={temperature=0.000001}}}",
        "weather={provinces={0={temperature=0 temperature=1}}}",
        "weather={provinces={0={mud=1}}}",
        "weather={provinces={0={mud=yes mud=no}}}",
        "weather={regions={0={rain_light=1}}}",
        "weather={regions={0={active_modifiers={a=1}}}}",
        "weather={regions={0={active_modifiers={yes}}}}",
        "weather={regions={0={active_modifiers=1}}}",
        "weather={regions={0={active_modifiers={} active_modifiers={}}}}",
        "weather={provinces={0={custom_modifiers={dry=yes}}}}",
        "weather={provinces={0={custom_modifiers={dry=1.5}}}}",
        "weather={provinces={0={custom_modifiers={7}}}}",
        "weather={provinces={0={custom_modifiers={dry={}}}}}",
        "weather={provinces={0={custom_modifiers=dry}}}",
        "weather={provinces={0={custom_modifiers={} custom_modifiers={}}}}",
    ] {
        assert!(weather(text.as_bytes()).is_err(), "{text}");
    }
}
