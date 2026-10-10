use super::*;

#[test]
fn saved_unit_withdrawal_is_shared_and_omission_means_false() {
    for (suffix, expected) in [("", false), ("withdraw=yes", true), ("withdraw=no", false)] {
        let division = crate::parser::parse(
            format!("id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 {suffix}")
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            parse_division(b"AAA", &division.items)
                .unwrap()
                .presentation
                .withdrawal,
            expected
        );
        let gun = crate::parser::parse(
            format!("id={{id=7 type=74}} logical_country=AAA strength=0 {suffix}").as_bytes(),
        )
        .unwrap();
        assert_eq!(
            parse_railway_gun(b"AAA", &gun.items).unwrap().withdrawal,
            expected
        );
        let force = crate::parser::parse(
            format!("id={{id=7 type=51}} logical_country=AAA mission={{mission=0}} {suffix}")
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            parse_task_force(b"AAA", 1, &force.items)
                .unwrap()
                .withdrawal,
            expected
        );
    }
    for suffix in ["withdraw=bad", "withdraw={yes}", "withdraw=yes withdraw=no"] {
        let document = crate::parser::parse(
            format!("id={{id=7 type=74}} logical_country=AAA strength=0 {suffix}").as_bytes(),
        )
        .unwrap();
        assert!(parse_railway_gun(b"AAA", &document.items).is_err());
    }
}

#[test]
fn division_priority_retains_the_writer_enum_and_default_omission() {
    let parse = |suffix: &str| {
        let document = crate::parser::parse(
            format!("id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 {suffix}")
                .as_bytes(),
        )?;
        parse_division(b"AAA", &document.items)
    };
    for (suffix, expected) in [
        ("", None),
        ("move_priority=front_order", Some(0)),
        ("move_priority=player_order", Some(2)),
        ("move_priority=ai_player_order", Some(3)),
    ] {
        assert_eq!(
            parse(suffix).unwrap().presentation.movement_priority,
            expected
        );
    }
    for suffix in [
        "move_priority=normal",
        "move_priority=4",
        "move_priority=unknown",
        "move_priority={front_order}",
        "move_priority=front_order move_priority=player_order",
    ] {
        assert!(parse(suffix).is_err(), "{suffix}");
    }
}

#[test]
fn task_force_saved_progress_is_exact_and_retreat_is_boolean() {
    let parse = |suffix: &str| {
        let document = crate::parser::parse(
            format!("id={{id=7 type=51}} logical_country=AAA mission={{mission=0}} {suffix}")
                .as_bytes(),
        )?;
        parse_task_force(b"AAA", 1, &document.items)
    };
    let absent = parse("").unwrap();
    assert!(absent.movement_progress.is_none());
    assert!(!absent.retreat);
    let saved = parse("movement_progress=-90071992547.40993 retreat=yes").unwrap();
    assert_eq!(saved.movement_progress.unwrap().raw, -9_007_199_254_740_993);
    assert!(saved.retreat);
    for suffix in [
        "movement_progress=broken",
        "movement_progress=1 movement_progress=2",
        "retreat=broken",
        "retreat=yes retreat=no",
    ] {
        assert!(parse(suffix).is_err(), "{suffix}");
    }
}

#[test]
fn task_force_exile_preserves_saved_truth_and_refuses_malformed_fields() {
    let parse = |suffix: &str| {
        let document = crate::parser::parse(
            format!("id={{id=7 type=51}} logical_country=AAA mission={{mission=0}} {suffix}")
                .as_bytes(),
        )?;
        parse_task_force(b"AAA", 1, &document.items)
    };
    for (suffix, expected) in [("", false), ("exile=no", false), ("exile=yes", true)] {
        assert_eq!(parse(suffix).unwrap().exile, expected);
    }
    for suffix in [
        "exile=1",
        "exile=unknown",
        "exile={yes}",
        "exile=yes exile=no",
    ] {
        assert!(parse(suffix).is_err(), "{suffix}");
    }
}

#[test]
fn division_exile_keeps_saved_truth_and_rejects_malformed_fields() {
    let parse = |suffix: &str| {
        let document = crate::parser::parse(
            format!("id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 {suffix}")
                .as_bytes(),
        )?;
        parse_division(b"AAA", &document.items)
    };
    for (suffix, expected) in [("", false), ("exile=no", false), ("exile=yes", true)] {
        assert_eq!(parse(suffix).unwrap().presentation.exile, expected);
    }
    for suffix in [
        "exile=1",
        "exile=unknown",
        "exile={yes}",
        "exile=yes exile=no",
        "exile=yes exile=yes",
    ] {
        assert!(parse(suffix).is_err(), "{suffix}");
    }
}

#[test]
fn railway_gun_exile_keeps_saved_truth_and_rejects_malformed_fields() {
    let parse = |suffix: &str| {
        let document = crate::parser::parse(
            format!("id={{id=7 type=74}} logical_country=AAA strength=0 {suffix}").as_bytes(),
        )?;
        parse_railway_gun(b"AAA", &document.items)
    };
    for (suffix, expected) in [("", false), ("exile=no", false), ("exile=yes", true)] {
        assert_eq!(parse(suffix).unwrap().exile, expected);
    }
    for suffix in [
        "exile=1",
        "exile=unknown",
        "exile={yes}",
        "exile=yes exile=no",
    ] {
        assert!(parse(suffix).is_err(), "{suffix}");
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
fn task_force_names_keep_presence_and_unescaped_bytes() {
    let parse = |suffix: &[u8]| {
        let mut text = b"id={id=7 type=51} logical_country=AAA mission={mission=0} ".to_vec();
        text.extend_from_slice(suffix);
        let document = crate::parser::parse(&text)?;
        parse_task_force(b"AAA", 1, &document.items)
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

#[test]
fn template_name_fields_keep_omitted_empty_and_nonempty_presence_independently() {
    // The writer omits stored-empty strings, but
    // emits an empty scalar for a stored-nonempty string with a leading NUL.
    let parse = |fields: &str| {
        let document = crate::parser::parse(
            format!("division_templates={{division_template={{id={{id=3 type=52}} country=AAA template_counter=0 ingame_set_template_counter=no {fields}}}}}").as_bytes(),
        )?;
        parse_division_templates(&document.items)
    };
    let id = (3_u64 << 32) | 52;
    for (name, name_bytes, name_present) in [
        ("", b"".as_slice(), false),
        ("name=\"\"", b"".as_slice(), true),
        ("name=\"Literal\"", b"Literal".as_slice(), true),
    ] {
        for (key, key_bytes, key_present) in [
            ("", b"".as_slice(), false),
            ("localization_key=\"\"", b"".as_slice(), true),
            ("localization_key=\"KEY\"", b"KEY".as_slice(), true),
        ] {
            let rows = parse(&format!("{name} {key}")).unwrap();
            let row = &rows[&id];
            // Existing byte consumers retain exactly their old projection.
            assert_eq!(row.name, name_bytes);
            assert_eq!(row.name_key, key_bytes);
            assert_eq!(row.name_present, name_present);
            assert_eq!(row.name_key_present, key_present);
        }
    }
    for malformed in [
        "name={bad=x}",
        "name=\"\" name=\"\"",
        "localization_key={bad=x}",
        "localization_key=\"\" localization_key=\"\"",
    ] {
        assert!(parse(malformed).is_err(), "{malformed}");
    }
}
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
