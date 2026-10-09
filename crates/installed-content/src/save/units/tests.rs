use super::*;

#[test]
fn cached_division_demand_preserves_signed_zero_omission_and_rejects_malformed_present_values() {
    let parse = |suffix: &str| {
        let document = crate::parser::parse(
            format!("id={{id=7 type=29}} logical_country=AAA strength=0 organisation=0 {suffix}")
                .as_bytes(),
        )?;
        parse_division(b"AAA", &document.items)
    };
    assert_eq!(parse("").unwrap().presentation.fuel_requested, None);
    assert_eq!(
        parse("fuel_requested=-0.00001")
            .unwrap()
            .presentation
            .fuel_requested
            .unwrap()
            .raw,
        -1
    );
    assert_eq!(
        parse("fuel_requested=0")
            .unwrap()
            .presentation
            .fuel_requested
            .unwrap()
            .raw,
        0
    );
    for suffix in [
        "fuel_requested={bad=x}",
        "fuel_requested=1 fuel_requested=2",
        "fuel_requested=0.000001",
    ] {
        assert!(parse(suffix).is_err());
    }
}

#[test]
fn task_force_stored_name_retains_empty_non_utf8_and_refuses_duplicate_or_block() {
    let parse = |suffix: &[u8]| {
        let mut text = b"id={id=7 type=51} logical_country=AAA mission={mission=0} ".to_vec();
        text.extend_from_slice(suffix);
        let document = crate::parser::parse(&text)?;
        parse_task_force(b"AAA", 9, &document.items)
    };
    assert_eq!(parse(b"").unwrap().name, None);
    assert_eq!(parse(b"name=\"\"").unwrap().name, Some(Vec::new()));
    assert_eq!(
        parse(b"name=\"\xffF\"").unwrap().name,
        Some(vec![0xff, b'F'])
    );
    for suffix in [b"name=A name=B".as_slice(), b"name={bad=x}".as_slice()] {
        assert!(parse(suffix).is_err());
    }
}

#[test]
fn railway_gun_names_keep_presence_and_unescaped_bytes() {
    let parse = |suffix: &[u8]| {
        let mut text = b"id={id=7 type=74} logical_country=AAA strength=0 ".to_vec();
        text.extend_from_slice(suffix);
        let document = crate::parser::parse(&text)?;
        parse_railway_gun(b"AAA", &document.items)
    };
    assert_eq!(parse(b"").unwrap().name, None);
    assert_eq!(parse(b"name=\"\"").unwrap().name, Some(Vec::new()));
    assert_eq!(
        parse(b"name=\"\xff\\\\\\\"G\"").unwrap().name,
        Some(vec![0xff, b'\\', b'"', b'G'])
    );
    for text in [b"name=A name=B".as_slice(), b"name={bad=x}".as_slice()] {
        assert!(parse(text).is_err());
    }
}

#[test]
fn fleet_reads_stored_name_and_full_leader_reference_without_generation() {
    let document = crate::parser::parse(
        b"id={id=7 type=61} name=\"Synthetic command name\" leader={id=2147483648 type=4713}",
    )
    .unwrap();
    let fleet = parse_fleet(b"MOD", &document.items).unwrap();
    assert_eq!(fleet.name.as_deref(), Some(&b"Synthetic command name"[..]));
    assert_eq!(fleet.leader, Some((2_147_483_648_u64 << 32) | 4713));
    let document = crate::parser::parse(b"id={id=7 type=61}").unwrap();
    let fleet = parse_fleet(b"MOD", &document.items).unwrap();
    assert_eq!(fleet.name, None);
    assert_eq!(fleet.leader, None);
}

#[test]
fn template_fallback_key_is_saved_independently_of_explicit_name() {
    let parse = |fields: &str| {
        let document = crate::parser::parse(
            format!("division_templates={{division_template={{id={{id=3 type=52}} country=AAA template_counter=0 ingame_set_template_counter=no {fields}}}}}").as_bytes(),
        )?;
        parse_division_templates(&document.items)
    };
    let id = (3_u64 << 32) | 52;
    let rows = parse("name=Explicit localization_key=FALLBACK").unwrap();
    assert_eq!(rows[&id].name, b"Explicit");
    assert_eq!(rows[&id].name_key, b"FALLBACK");
    assert!(parse("").unwrap()[&id].name_key.is_empty());
    for text in [
        "localization_key={bad=x}",
        "localization_key=A localization_key=B",
    ] {
        assert!(parse(text).is_err());
    }
}
