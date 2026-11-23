use super::*;

#[test]
fn wing_replacement_setting_preserves_all_byte_values_and_absent_key() {
    for (suffix, expected) in [
        ("", None),
        ("reinforcement_setting=0", Some(0)),
        ("reinforcement_setting=255", Some(255)),
    ] {
        let source = format!(
            "strategic_air={{AAA={{air_wing_pool={{air_base={{id=2 type=54}} air_wings={{id={{id=7 type=69}} tag=AAA count=0 mission={{type=0}} {suffix}}}}}}}}}"
        );
        let document = content_reader_parser::parser::parse(source.as_bytes()).unwrap();
        let rows = parse_air_wings(&document.items).unwrap();
        assert_eq!(
            rows.values().next().unwrap().reinforcement_setting,
            expected
        );
    }
}

#[test]
fn air_transfer_progress_keeps_saved_presence_and_signed_precision() {
    for (extra, progress, target) in [
        ("", None, None),
        ("transfer_progress=0 transferring_to=0", Some(0), Some(0)),
        (
            "transfer_progress=-1.00001 transferring_to=7",
            Some(-100_001),
            Some(7),
        ),
    ] {
        let text = [
            b"strategic_air={AAA={air_wing_pool={air_base={id=1 type=65} air_wings={id={id=2 type=69} count=3 tag=AAA mission={type=0} ".as_slice(),
            extra.as_bytes(), b"}}}}".as_slice(),
        ].concat();
        let doc = content_reader_parser::parser::parse(&text).unwrap();
        let wings = parse_air_wings(&doc.items).unwrap();
        let wing = &wings[&((2 << 32) | 69)];
        assert_eq!(wing.transfer_progress, progress);
        assert_eq!(wing.transferring_to, target);
    }
}

#[test]
fn saved_unit_withdrawal_is_shared_and_omission_means_false() {
    for (suffix, expected) in [("", false), ("withdraw=yes", true), ("withdraw=no", false)] {
        let division = content_reader_parser::parser::parse(
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
        let gun = content_reader_parser::parser::parse(
            format!("id={{id=7 type=74}} logical_country=AAA strength=0 {suffix}").as_bytes(),
        )
        .unwrap();
        assert_eq!(
            parse_railway_gun(b"AAA", &gun.items).unwrap().withdrawal,
            expected
        );
        let force = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(
            format!("id={{id=7 type=74}} logical_country=AAA strength=0 {suffix}").as_bytes(),
        )
        .unwrap();
        assert!(parse_railway_gun(b"AAA", &document.items).is_err());
    }
}

#[test]
fn division_priority_retains_the_writer_enum_and_default_omission() {
    let parse = |suffix: &str| {
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(&text)?;
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
        let document = content_reader_parser::parser::parse(&text)?;
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
        let document = content_reader_parser::parser::parse(
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
    let parse = |fields: &str| {
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(
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
        let document = content_reader_parser::parser::parse(&text)?;
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
    let document = content_reader_parser::parser::parse(
        b"id={id=7 type=61} name=\"Synthetic command name\" leader={id=2147483648 type=4713}",
    )
    .unwrap();
    let fleet = parse_fleet(b"MOD", &document.items).unwrap();
    assert_eq!(fleet.name.as_deref(), Some(&b"Synthetic command name"[..]));
    assert_eq!(fleet.leader, Some((2_147_483_648_u64 << 32) | 4713));
    let document = content_reader_parser::parser::parse(b"id={id=7 type=61}").unwrap();
    let fleet = parse_fleet(b"MOD", &document.items).unwrap();
    assert_eq!(fleet.name, None);
    assert_eq!(fleet.leader, None);
}

#[test]
fn manpower_template_origin_retains_exile_other_and_omission() {
    for (field, expected) in [
        ("", None),
        ("origin_type=exile", Some(b"exile".as_slice())),
        ("origin_type=subject", Some(b"subject".as_slice())),
    ] {
        let doc = content_reader_parser::parser::parse(format!("division_templates={{division_template={{id={{id=7 type=52}} country=MOD template_counter=0 ingame_set_template_counter=no {field}}}}}").as_bytes()).unwrap();
        let saved = parse_division_templates(&doc.items).unwrap();
        assert_eq!(
            saved[&((7_u64 << 32) | 52)].origin_type.as_deref(),
            expected
        );
    }
}

#[test]
fn division_experience_is_independent_of_strength_and_exercising() {
    let document = content_reader_parser::parser::parse(b"id={id=7 type=51} logical_country=AAA strength=0 organisation=0 experience=92233720368547.75807 old_division_template_id={id=2147483648 type=55}").unwrap();
    let division = parse_division(b"AAA", &document.items).unwrap();
    assert_eq!(division.presentation.experience.unwrap().raw, i64::MAX);
    assert_eq!(
        division.presentation.pending_template,
        Some((2_147_483_648_u64 << 32) | 55)
    );
}

#[test]
fn experience_division_history_save_preserves_order_repeats_defaults_and_optional_fields() {
    let entry = "army_names=\"A\" target_country=AAA date=1936.1.1.1 unique=16 medal_count=yes inherit=yes location={9 2 9} unit_medals=mod_medal orders=-3 custom_lockey=mod_history multiplier=-0.00001";
    let text = "unit_medals={amount=3 history={history_queue={ENTRY}}} army_history={history_queue={ENTRY} history_queue={ENTRY}}".replace("ENTRY", entry);
    let document = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
    let history = division_history(&document.items).unwrap();
    assert_eq!(history.amount, Some(3));
    assert_eq!(history.career.len(), 2);
    assert_eq!(history.awards.len(), 1);
    assert_eq!(history.career[0], history.career[1]);
    assert_eq!(history.career[0].provinces, [9, 2, 9]);
    assert_eq!(history.career[0].multiplier.as_ref().unwrap().raw, -1);
    let empty = content_reader_parser::parser::parse(b"unit_medals={}").unwrap();
    let empty = division_history(&empty.items).unwrap();
    assert_eq!(empty.amount, None);
    assert!(empty.career.is_empty() && empty.awards.is_empty());
}

#[test]
fn division_history_reads_boolean_flags_in_both_saved_lists() {
    for (medal_count, medal_value) in [("no", false), ("yes", true)] {
        for (inherit, inherit_value) in [("no", false), ("yes", true)] {
            let entry = format!(
                "army_names=synthetic target_country=AAA date=1936.1.1.1 unique=16 \
                 medal_count={medal_count} inherit={inherit} location={{9 2 9}}"
            );
            let text = format!(
                "id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 \
                 army_history={{unit_medals={{amount=3 history={{history_queue={{{entry}}}}}}} \
                 army_history={{history_queue={{{entry}}} history_queue={{{entry}}}}}}}"
            );
            let document = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
            let history = parse_division(b"AAA", &document.items)
                .unwrap()
                .presentation
                .history
                .unwrap();
            assert_eq!(history.amount, Some(3));
            assert_eq!(history.career.len(), 2);
            assert_eq!(history.awards.len(), 1);
            assert_eq!(history.career[0], history.career[1]);
            for entry in history.career.iter().chain(&history.awards) {
                assert_eq!(entry.medal_count, medal_value);
                assert_eq!(entry.inherit, inherit_value);
                assert_eq!(entry.provinces, [9, 2, 9]);
            }
        }
    }
}

#[test]
fn division_history_requires_each_saved_boolean_flag() {
    for key in ["medal_count", "inherit"] {
        for awards in [false, true] {
            let other = if key == "medal_count" {
                "inherit=yes"
            } else {
                "medal_count=no"
            };
            let mut cases = vec![
                (String::new(), format!("save lacks {key}")),
                (
                    format!("{key}=yes {key}=no"),
                    format!("duplicate save field {key}"),
                ),
                (format!("{key}={{}}"), "expected a save scalar".into()),
            ];
            for value in ["0", "1", "2", "255", "true", "YES", "invalid"] {
                cases.push((
                    format!("{key}={value}"),
                    "save boolean must be yes or no".into(),
                ));
            }
            for (flags, expected) in cases {
                let entry = format!(
                    "history_queue={{army_names=synthetic target_country=AAA \
                     date=1936.1.1.1 unique=1 {other} {flags} location={{1}}}}"
                );
                let holder = if awards {
                    format!("unit_medals={{history={{{entry}}}}}")
                } else {
                    format!("army_history={{{entry}}}")
                };
                let text = format!(
                    "id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 \
                     army_history={{{holder}}}"
                );
                let document = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
                let error = parse_division(b"AAA", &document.items).unwrap_err();
                assert_eq!(error.to_string(), expected, "{text}");
            }
        }
    }
}

#[test]
fn division_history_unwraps_the_saved_holder_and_keeps_independent_officer_totals() {
    let doc = content_reader_parser::parser::parse(br#"id={id=7 type=51} logical_country=AAA strength=0 organisation=0 killed=123 held_officer={experience=45.12345} army_history={unit_medals={amount=2 history={history_queue={army_names=award target_country=AAA date=1936.1.1.1 unique=1 medal_count=yes inherit=no location={2}}}} army_history={history_queue={army_names=unit target_country=AAA date=1936.1.1.1 unique=1 medal_count=no inherit=no location={1}} history_queue={army_names=unit target_country=AAA date=1936.1.1.1 unique=1 medal_count=no inherit=no location={1}}}}"#).unwrap();
    let division = parse_division(b"AAA", &doc.items).unwrap();
    assert_eq!(division.presentation.career_counter, Some(123));
    assert_eq!(
        division.presentation.officer_experience.unwrap().raw,
        4_512_345
    );
    let history = division.presentation.history.unwrap();
    assert_eq!(history.amount, Some(2));
    assert_eq!(history.career.len(), 2);
    assert_eq!(history.career[0], history.career[1]);
    assert_eq!(history.awards.len(), 1);
    assert_eq!(history.awards[0].name.as_deref(), Some(b"award".as_slice()));
    assert_eq!(history.awards[0].provinces, [2]);
}

#[test]
fn division_history_distinguishes_an_omitted_holder_from_empty_lists() {
    for (suffix, present) in [("", false), ("army_history={unit_medals={}}", true)] {
        let text =
            format!("id={{id=7 type=51}} logical_country=AAA strength=0 organisation=0 {suffix}");
        let doc = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
        let division = parse_division(b"AAA", &doc.items).unwrap();
        assert_eq!(division.presentation.history.is_some(), present);
        if let Some(history) = division.presentation.history {
            assert_eq!(history.amount, None);
            assert!(history.career.is_empty() && history.awards.is_empty());
        }
    }
}

#[test]
fn template_equipment_restrictions_and_model_choices_preserve_saved_state() {
    let document = content_reader_parser::parser::parse(
        br#"division_templates={division_template={
        id={id=7 type=52} country=AAA original_tag=BBB foreign_template_tag=CCC
        template_counter=-1 ingame_set_template_counter=no priority=-9
        is_locked=yes obsolete=yes obsolete_change_date=1937.2.3.4
        force_allow_recruiting=yes is_army_hq=yes is_fake_intel_division=yes
        allow_new_equipment=no allow_foreign_equipment=yes
        role=synthetic_role equipment_niche=-2 division_cap=-11 parent={id=4294967295 type=52}
        origin_type=exile override_model="synthetic_model"
        override_model_equipment_category_filter=18446744073709551615
        forbidden_equipment_types={token=7 token=7 token=4294967295}
        not_allowed_equipment={equipment_variant_index={id=9 type=70}}
        regiments={synthetic={x=2 y=3}}
    }}"#,
    )
    .unwrap();
    let rows = parse_division_templates(&document.items).unwrap();
    let row = &rows[&((7_u64 << 32) | 52)];
    let s = &row.settings;
    assert_eq!(s.priority, Some(-9));
    assert_eq!(s.role.as_deref(), Some(b"synthetic_role".as_slice()));
    assert!(s.locked && s.obsolete && s.is_army_hq && s.force_allow_recruiting && s.fake_intel);
    assert_eq!(
        s.obsolete_change_date,
        Some(
            super::super::GameDate::parse(b"1937.2.3.4")
                .unwrap()
                .engine_hour()
                .unwrap()
        )
    );
    assert_eq!(
        (s.allow_new_equipment, s.allow_foreign_equipment),
        (Some(false), Some(true))
    );
    assert_eq!((s.division_cap, s.equipment_niche), (Some(-11), Some(-2)));
    assert_eq!(s.parent, Some((u64::from(u32::MAX) << 32) | 52));
    assert_eq!(s.forbidden_equipment, [7, 7, u32::MAX]);
    assert_eq!(row.excluded_variants, [(9_u64 << 32) | 70]);
    assert_eq!(row.model_category, Some(u64::MAX));
    assert_eq!(row.model_entity, b"synthetic_model");
    assert_eq!(row.composition[&(0, 2, 3)], b"synthetic");
}

#[test]
fn critical_wrapper_presence_preserves_even_an_empty_emitted_array() {
    for (suffix, expected) in [
        ("", false),
        ("critical_damage={}", true),
        ("critical_damage={synthetic=0 synthetic=2}", true),
    ] {
        let parsed = content_reader_parser::parser::parse(
            format!("id={{id=7 type=51}} strength=1 organisation=1 {suffix}").as_bytes(),
        )
        .unwrap();
        assert_eq!(
            parse_ship(9, &parsed.items).unwrap().has_criticals,
            expected
        );
    }
    let parsed = content_reader_parser::parser::parse(
        b"id={id=7 type=51} strength=1 organisation=1 critical_damage=no",
    )
    .unwrap();
    assert!(parse_ship(9, &parsed.items).is_err());
}

#[test]
fn air_mission_training_stop_retains_true_false_and_native_omission() {
    for (field, expected) in [
        ("", false),
        ("stop_training_at_max_xp=yes", true),
        ("stop_training_at_max_xp=no", false),
    ] {
        let text = "strategic_air={AAA={air_wing_pool={air_base={id=2 type=68} air_wings={id={id=7 type=69} name=wing tag=AAA count=5 mission={type=2048 FIELD}}}}}".replace("FIELD", field);
        let parsed = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
        let wings = parse_air_wings(&parsed.items).unwrap();
        assert_eq!(wings[&((7 << 32) | 69)].stop_training_at_max_xp, expected);
    }
}
