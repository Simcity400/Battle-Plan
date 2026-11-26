use super::*;
use dashboard_core::test_fixtures::game;

fn localisation(language: &str) -> Localisation {
    let mut localisation = Localisation::default();
    localisation.language = language.into();
    localisation
}

#[test]
fn retained_task_force_name_preserves_empty_and_raw_text_with_language_guard() {
    let mut row = TaskForceCounterInputRow {
        task_force: (7_u64 << 32) | 61,
        ..Default::default()
    };
    for bytes in [b"Native task force".to_vec(), b"Name\0tail".to_vec()] {
        row.name = Recorded::Value(Some(bytes.clone()));
        assert_eq!(
            task_force_name(&row, &localisation("english"), "english"),
            Ok(String::from_utf8(bytes).unwrap())
        );
        assert!(
            task_force_name(&row, &localisation("french"), "english")
                .unwrap_err()
                .contains("stored in english")
        );
    }
    row.name = Recorded::Value(Some(Vec::new()));
    assert_eq!(
        task_force_name(&row, &localisation("french"), "english").as_deref(),
        Ok("")
    );
    row.name = Recorded::Value(Some(vec![0xff]));
    assert!(
        task_force_name(&row, &localisation("english"), "english")
            .unwrap_err()
            .contains("not UTF-8")
    );
}

#[test]
fn task_force_name_capture_failure_and_historical_absence_remain_distinct() {
    let mut row = TaskForceCounterInputRow {
        task_force: (7_u64 << 32) | 61,
        name: Recorded::NotRecorded,
        ..Default::default()
    };
    assert!(
        task_force_name(&row, &localisation("english"), "english")
            .unwrap_err()
            .contains("name of task force counter inputs not recorded")
    );
    row.name = Recorded::Value(None);
    assert!(
        task_force_name(&row, &localisation("english"), "english")
            .unwrap_err()
            .contains("guarded read or name arena refusal")
    );
}

#[test]
fn retained_army_name_is_authoritative_without_generator_or_content() {
    for bytes in [
        b"Allocated legacy name".to_vec(),
        Vec::new(),
        b"Cache\0suffix".to_vec(),
    ] {
        let status = DivisionStatusRow {
            division: 7,
            name: Recorded::Value(Some(bytes.clone())),
            naming: Recorded::Value(None),
            ..Default::default()
        };
        assert_eq!(
            army_name(&status, &localisation("english"), "english", || panic!(
                "cached name does not acquire content"
            )),
            Ok(String::from_utf8(bytes).unwrap())
        );
    }
    let status = DivisionStatusRow {
        name: Recorded::Value(None),
        naming: Recorded::Value(Some(NameOperands {
            custom: b"Generator substitute".to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(
        army_name(&status, &localisation("english"), "english", || panic!(
            "failed cache does not acquire content"
        ))
        .unwrap_err()
        .contains("name of division status unavailable")
    );
}

#[test]
fn only_equal_nonprogrammatic_bytes_certify_cross_language_literal_names() {
    let mut status = DivisionStatusRow {
        name: Recorded::Value(Some(b"Player literal".to_vec())),
        naming: Recorded::Value(Some(NameOperands {
            custom: b"Player literal".to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(
        army_name(&status, &localisation("french"), "english", || panic!(
            "literal needs no content"
        ))
        .as_deref(),
        Ok("Player literal")
    );
    for naming in [
        Recorded::Value(None),
        Recorded::NotRecorded,
        Recorded::Value(Some(NameOperands {
            custom: b"Different current literal".to_vec(),
            ..Default::default()
        })),
        Recorded::Value(Some(NameOperands {
            custom: b"Player literal".to_vec(),
            programmatic: 1,
            ..Default::default()
        })),
    ] {
        status.naming = naming;
        assert!(
            army_name(&status, &localisation("french"), "english", || panic!(
                "language refusal needs no content"
            ))
            .unwrap_err()
            .contains("stored in english")
        );
    }
    status.name = Recorded::Value(Some(Vec::new()));
    assert_eq!(
        army_name(&status, &localisation("french"), "english", || panic!(
            "empty needs no content"
        ))
        .as_deref(),
        Ok("")
    );
    status.name = Recorded::Value(Some(vec![0xff]));
    assert!(
        army_name(&status, &localisation("english"), "english", || panic!(
            "invalid text needs no content"
        ))
        .unwrap_err()
        .contains("not UTF-8")
    );
}

#[test]
fn only_historical_absence_uses_authored_name_selection() {
    game::enter();
    let names = content_reader_division_names::division_names::parse(
        br#"G={ordered={2={"Generated %d"}}}"#,
        &Default::default(),
    )
    .unwrap();
    let mut status = DivisionStatusRow {
        name: Recorded::NotRecorded,
        naming: Recorded::Value(Some(NameOperands {
            group: b"G".to_vec(),
            order: 2,
            ordered: 1,
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(
        army_name(&status, &localisation("french"), "english", || Ok(&names)).as_deref(),
        Ok("Generated 2")
    );
    if let Recorded::Value(Some(naming)) = &mut status.naming {
        naming.order = 3;
    }
    assert!(
        army_name(&status, &localisation("english"), "english", || Ok(&names))
            .unwrap_err()
            .contains("legacy name allocation inputs")
    );
    status.name = Recorded::Value(Some(b"Retained allocation".to_vec()));
    assert_eq!(
        army_name(&status, &localisation("english"), "english", || panic!(
            "retained legacy cache needs no allocator"
        ))
        .as_deref(),
        Ok("Retained allocation")
    );
}

#[test]
fn template_literal_and_known_empty_name_do_not_acquire_country_or_content() {
    use replay_data::rows::TemplateDisplayNameParts as Parts;
    assert_eq!(
        template_name(
            Parts {
                literal: b"Authored\0tail",
                fallback_key: None
            },
            -7,
            |_, _| panic!("nonempty literal must not acquire country")
        ),
        Ok("Authored\0tail".into())
    );
    assert_eq!(
        template_name(
            Parts {
                literal: b"",
                fallback_key: Some(b"")
            },
            0,
            |_, _| panic!("known empty name must not acquire country")
        ),
        Ok(String::new())
    );
    assert!(
        template_name(
            Parts {
                literal: &[255],
                fallback_key: None
            },
            0,
            |_, _| panic!("invalid literal still does not acquire fallback")
        )
        .unwrap_err()
        .contains("UTF-8")
    );
}

#[test]
fn template_fallback_lazily_preserves_genuine_signed_country_and_key() {
    use replay_data::rows::TemplateDisplayNameParts as Parts;
    for country in [i32::MIN, -1, 0, 1, i32::MAX] {
        assert_eq!(
            template_name(
                Parts {
                    literal: b"",
                    fallback_key: Some(b"KEY\0tail")
                },
                country,
                |raw, key| {
                    assert_eq!(raw, country);
                    assert_eq!(key, b"KEY\0tail");
                    Ok("country-localised".into())
                }
            ),
            Ok("country-localised".into())
        );
    }
    assert_eq!(
        template_name(
            Parts {
                literal: b"",
                fallback_key: Some(b"KEY")
            },
            0,
            |_, _| Err("authenticated slot zero missing".into())
        ),
        Err("authenticated slot zero missing".into())
    );
}

#[test]
fn template_noncanonical_demand_is_refused_without_country_read() {
    use replay_data::rows::TemplateDisplayNameParts as Parts;
    for parts in [
        Parts {
            literal: b"literal",
            fallback_key: Some(b"KEY"),
        },
        Parts {
            literal: b"",
            fallback_key: None,
        },
    ] {
        assert!(
            template_name(parts, 3, |_, _| panic!(
                "malformed demand never acquires country"
            ))
            .is_err()
        );
    }
}
