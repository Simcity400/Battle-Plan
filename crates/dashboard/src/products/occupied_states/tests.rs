use super::*;

fn value() -> OccupiedValues {
    OccupiedValues {
        name: Some("Custom state".into()),
        name_missing: None,
        owner: Some("Mod owner".into()),
        occupier: Some("Dynamic tag".into()),
        occupied: Some("Old tag".into()),
        resistance_raw: "0".into(),
        compliance_raw: "10000000".into(),
        resistance_speed_raw: "-7".into(),
        compliance_speed_raw: "9".into(),
        garrison_strength_raw: None,
        required_manpower: Some(0),
        garrison_required_raw: Some("0".into()),
        garrison_alert: false,
        garrison_red: None,
        assigned_manpower: None,
        garrison_missing: None,
        law: None,
        law_missing: Some(GROUP_LAW_MISSING.into()),
    }
}

#[test]
fn occupied_is_activation_not_legal_ownership() {
    let mut row = StateResistanceRow {
        country: 12,
        active: 1,
        ..Default::default()
    };
    assert!(occupied(&row));
    row.active = 0;
    assert!(!occupied(&row));
    row.active = 1;
    row.country = 0;
    assert!(!occupied(&row));
    row.country = u32::MAX;
    assert!(!occupied(&row));
}

#[test]
fn law_precedence_stops_at_the_missing_group_pointer() {
    assert_eq!(
        law_for(Some(b"mod_law"), true, true, Some((true, b"default"))),
        (Some("mod_law".into()), None)
    );
    // A pointer to a law with an empty token is still an explicit override.
    assert_eq!(
        law_for(Some(b""), true, true, None),
        (Some("".into()), None)
    );
    for default in [
        None,
        Some((false, b"".as_slice())),
        Some((true, b"default".as_slice())),
    ] {
        let (law, reason) = law_for(None, true, true, default);
        assert!(law.is_none());
        assert!(reason.unwrap().contains("takes precedence"));
    }
    assert!(
        law_for(None, true, false, None)
            .1
            .unwrap()
            .contains("incomplete")
    );
    assert!(
        law_for(None, false, true, None)
            .1
            .unwrap()
            .contains("unresolved")
    );
    assert!(
        law_for(Some(&[0xff]), true, true, None)
            .1
            .unwrap()
            .contains("UTF-8")
    );
}

#[test]
fn equal_values_compress_without_bridging_missing_hours() {
    let mut runs = Vec::new();
    append(&mut runs, 10, value());
    append(&mut runs, 11, value());
    append(&mut runs, 13, value());
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (10, 11));
    assert_eq!(runs[1].first_hour, 13);
    let mut changed = value();
    changed.required_manpower = None;
    changed.garrison_required_raw = None;
    append(&mut runs, 14, changed);
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].value.required_manpower, Some(0));
    assert_eq!(runs[2].value.required_manpower, None);
}

#[test]
fn raw_links_resolve_to_opaque_mod_tags_at_scale() {
    let links: Vec<_> = (1..=5000)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id + 7000,
        })
        .collect();
    let countries: Vec<_> = links
        .iter()
        .map(|link| CountryRow {
            id: link.id,
            canonical_slot: link.canonical_slot,
            tag: format!("Mod country {}", link.id).into_bytes(),
            ..Default::default()
        })
        .collect();
    let mut refused = Refusals::default();
    let mapped = tags(&countries, &links, &mut refused);
    assert_eq!(mapped.len(), 5000);
    assert_eq!(mapped.get(&12000), Some(&"Mod country 5000"));
    assert!(refused.lines("countries").is_empty());

    let mut invalid = countries[..3].to_vec();
    invalid[0].tag.clear();
    invalid[1].tag = b"Bad\0tag".to_vec();
    invalid[2].tag = vec![0xff];
    let mut refused = Refusals::default();
    assert!(tags(&invalid, &links, &mut refused).is_empty());
    let warnings = refused.lines("countries");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("3") && warnings[0].contains("country 1"));
}

#[test]
fn mod_added_law_uses_overridden_localisation_and_sprite() {
    let (mut cat, folder) = crate::catalogue::tests::folder_catalogue(&[
        ("base/common/occupation_laws/a.txt", "renamed={icon=1}"),
        (
            "base/localisation/english/a_l_english.yml",
            "\u{feff}l_english:\n renamed:0 \"Original name\"\n",
        ),
        (
            "base/interface/mod.gfx",
            "spriteTypes={spriteType={name=\"overridden_sprite\" texturefile=\"gfx/original.dds\" noOfFrames=1}}",
        ),
        (
            "mod/common/occupation_laws/z.txt",
            "mod_law={icon=\"overridden_sprite:3\"}",
        ),
        (
            "mod/localisation/english/a_l_english.yml",
            "\u{feff}l_english:\n renamed:0 \"Modded name\"\n mod_law:0 \"Extra law\"\n literal:0 \"Literal §§Y name\"\n",
        ),
        (
            "mod/interface/mod.gfx",
            "spriteTypes={spriteType={name=\"overridden_sprite\" texturefile=\"gfx/mod_missing.dds\" noOfFrames=3}}",
        ),
    ]);
    let base = folder.0.join("base");
    let added = folder.0.join("mod");
    let layers = installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&added, &[])]).unwrap();
    cat.localisation = Arc::new(installed_content::localisation::read(&layers, "english").unwrap());
    cat.sprites = installed_content::gfx::read(&layers).unwrap();
    assert_eq!(
        cat.sprites
            .find("overridden_sprite")
            .unwrap()
            .unwrap()
            .texture
            .as_deref(),
        Some("gfx/mod_missing.dds")
    );
    cat.content = Arc::new(installed_content::ResolvedContent {
        layers,
        stats: Default::default(),
    });
    let laws = installed_content::occupation_laws::read(&cat.content.layers).unwrap();
    assert_eq!(laws.definitions["mod_law"].sprite, "overridden_sprite");
    let info = law_content(&cat, &laws, "mod_law");
    assert_eq!(info.name.as_deref(), Some("Extra law"));
    assert!(info.icon.is_none());
    assert!(info.missing.is_some());
    assert!(
        cat.refusal_lines()
            .iter()
            .any(|line| line.contains("mod_law") && line.contains("overridden_sprite"))
    );
    assert_eq!(
        cat.localised("renamed", View::Plain).as_deref(),
        Some("Modded name")
    );
    assert_eq!(
        cat.localised("removed", View::Plain).as_deref(),
        Some("removed")
    );
    let state = StateRow {
        id: 6000,
        custom_name: b"Custom mod state".to_vec(),
        owner_name: b"Owner state".to_vec(),
        ..Default::default()
    };
    assert_eq!(
        name_of(&state, Some(&cat), &BTreeMap::new()).unwrap(),
        "Custom mod state"
    );
    let state = StateRow {
        id: 6001,
        ..Default::default()
    };
    assert_eq!(
        name_of(
            &state,
            Some(&cat),
            &BTreeMap::from([(6001, Ok("literal".into()))])
        )
        .unwrap(),
        "Literal §Y name"
    );
}

#[test]
fn law_icons_follow_vertical_native_frames_and_refuse_custom_rendering() {
    let (cat, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/occupation_laws/a.txt",
            "extra={icon=\"vertical:2\"} custom={icon=shader}",
        ),
        (
            "interface/a.gfx",
            "spriteTypes={spriteType={name=vertical texturefile=\"gfx/strip.tga\" noOfFrames=2 framedirectiony=yes} spriteType={name=shader texturefile=\"gfx/strip.tga\" effectfile=\"gfx/FX/mod.lua\"}}",
        ),
    ]);
    let mut tga = vec![0; 18];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 2;
    tga[16] = 32;
    tga[17] = 0x28;
    tga.extend_from_slice(&[0, 0, 255, 255, 255, 0, 0, 255]);
    std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
    std::fs::write(folder.0.join("gfx/strip.tga"), tga).unwrap();
    let laws = installed_content::occupation_laws::read(&cat.content.layers).unwrap();
    let vertical = law_content(&cat, &laws, "extra");
    let blue = catalogue::png_data_url(&installed_content::image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![0, 0, 255, 255],
    })
    .unwrap();
    assert_eq!(vertical.icon.as_deref(), Some(blue.as_str()));
    assert!(vertical.missing.is_none());
    let custom = law_content(&cat, &laws, "custom");
    assert!(custom.icon.is_none());
    assert!(custom.missing.unwrap().contains("custom effect"));
    assert!(
        cat.refusal_lines()
            .iter()
            .any(|line| line.contains("1") && line.contains("custom"))
    );
}

fn frame(hour: i32, read: bool) -> DecodedFrame {
    use replay::rows::{OccupationSettingsRow, StateGarrisonRow, StateOccupationLawRow, TableRows};
    let mut frame = crate::test_fixtures::source(1, hour, Some(0));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 2000,
            owner: 9001,
            controller: 11,
            custom_name: b"Annexed coreless state".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
            state: 2000,
            country: 9001,
            active: 1,
            level: 9007199254740993,
            compliance: 0,
            speed: -1,
            compliance_speed: 2,
        }]));
    *frame.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
            controller: 9001,
            occupied: 11,
            state: 2000,
            required_manpower: 0,
            ..Default::default()
        }]));
    *frame.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![
            StateOccupationLawRow {
                controller: 9001,
                occupied: 11,
                state: 2000,
                name: b"extra_mod_law".to_vec(),
            },
        ]));
    *frame.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![OccupationSettingsRow {
            country: 11,
            law: b"lower_precedence_default".to_vec(),
            default_law_present: 1,
            ..Default::default()
        }]));
    if read {
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
    }
    frame
}

#[test]
fn branch_scan_dates_equal_reads_and_requires_a_new_read_after_a_gap() {
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(
            1,
            vec![
                frame(100, true),
                frame(101, false),
                frame(102, true),
                frame(104, false),
                frame(105, true),
                frame(106, false),
            ],
        )],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.states.len(), 1);
    let runs = &reply.states[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 102));
    assert_eq!((runs[1].first_hour, runs[1].last_hour), (105, 106));
    assert_eq!(
        reply
            .observed
            .iter()
            .map(|span| span.read_hour)
            .collect::<Vec<_>>(),
        [100, 102, 105]
    );
    assert_eq!(runs[0].value.required_manpower, Some(0));
    assert_eq!(runs[0].value.law.as_deref(), Some("extra_mod_law"));
    assert_eq!(runs[0].value.resistance_raw, "9007199254740993");
}

#[test]
fn garrison_strength_keeps_the_native_early_zero_but_never_uses_strength_ratio() {
    use replay::rows::{StateGarrisonRow, TableRows};
    let mut missing = frame(100, true);
    let mut garrison = StateGarrisonRow {
        controller: 9001,
        occupied: 11,
        state: 2000,
        strength_ratio: 100000,
        ..Default::default()
    };
    *missing.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![garrison.clone()]));
    let mut known = frame(101, true);
    garrison.serialized_flag = 1;
    garrison.garrison_required = -1;
    *known.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![garrison]));
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![missing, known])],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    let runs = &reply.states[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].value.garrison_strength_raw, None);
    assert_eq!(runs[1].value.garrison_strength_raw.as_deref(), Some("0"));
}

#[test]
fn partial_resistance_keeps_known_rows_and_does_not_claim_an_empty_inventory() {
    use replay::rows::TableRows;
    // A keyframe cannot carry a partial table; start with a complete baseline.
    let first = frame(100, true);
    let mut partial = frame(101, true);
    let rows = partial
        .complete::<tables::StateResistance>()
        .unwrap()
        .to_vec();
    *partial.tables[TableId::StateResistance.index()] = TableView::Truncated {
        rows: TableRows::StateResistance(rows),
        missing: 1,
    };
    let mut empty = frame(102, true);
    *empty.tables[TableId::StateResistance.index()] = TableView::Truncated {
        rows: TableRows::StateResistance(Vec::new()),
        missing: 1,
    };
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(
            1,
            vec![first, partial, empty],
        )],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.states.len(), 1);
    assert_eq!(reply.states[0].runs[0].last_hour, 101);
    assert!(
        reply
            .observed
            .iter()
            .any(|span| span.first_hour == 101 && !span.complete)
    );
    assert!(
        reply
            .observed
            .iter()
            .any(|span| span.first_hour == 102 && !span.complete)
    );
}

#[test]
fn a_branch_with_five_thousand_states_and_countries_is_not_capped() {
    use replay::rows::TableRows;
    let mut first = frame(100, true);
    *first.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=5000)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id + 7000,
            })
            .collect(),
    ));
    *first.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id + 7000,
                tag: format!("Mod country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *first.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=5000)
            .map(|id| StateRow {
                id,
                owner: id,
                controller: id,
                ..Default::default()
            })
            .collect(),
    ));
    *first.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(
            (1..=5000)
                .map(|id| StateResistanceRow {
                    state: id,
                    country: id,
                    active: 1,
                    level: i64::from(id),
                    ..Default::default()
                })
                .collect(),
        ));
    let mut second = first.clone();
    second.hour = 101;
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![first, second])],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.states.len(), 5000);
    assert!(
        reply
            .states
            .iter()
            .all(|state| state.runs.len() == 1 && state.runs[0].last_hour == 101)
    );
    assert_eq!(
        reply.states[4999].runs[0].value.occupier.as_deref(),
        Some("Mod country 5000")
    );
}

#[test]
fn controller_and_link_changes_between_reads_do_not_redate_occupation_values() {
    use replay::rows::{StateGarrisonRow, StateOccupationLawRow, TableRows};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let mut first = frame(100, true);
    let mut countries = vec![
        CountryRow {
            id: 9001,
            canonical_slot: 7,
            tag: b"Read occupier".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 12,
            canonical_slot: 8,
            tag: b"New owner".to_vec(),
            ..Default::default()
        },
    ];
    countries.sort_unstable_by_key(|row| row.id);
    *first.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let mut between = first.clone();
    between.hour = 101;
    between.metrics.table_ns[TableId::StateResistance.index()] = 0;
    *between.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 2000,
            owner: 12,
            controller: 12,
            custom_name: b"Renamed between reads".to_vec(),
            ..Default::default()
        }]));
    *between.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 12,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 9001,
                canonical_slot: 7,
            },
        ]));
    let mut next = between.clone();
    next.hour = 102;
    next.metrics.table_ns[TableId::StateResistance.index()] = 1;
    *next.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
            controller: 12,
            occupied: 9001,
            state: 2000,
            garrison_required: 123999,
            ..Default::default()
        }]));
    *next.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![
            StateOccupationLawRow {
                controller: 12,
                occupied: 9001,
                state: 2000,
                name: b"new_mod_law".to_vec(),
            },
        ]));
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![first, between, next])],
    )
    .unwrap();
    let reply = scan(&replay, 1, Some(&cat), &AtomicBool::new(false)).unwrap();
    let runs = &reply.states[0].runs;
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1].value.owner.as_deref(), Some("New owner"));
    assert_eq!(runs[1].value.name.as_deref(), Some("Renamed between reads"));
    assert_eq!(runs[1].value.occupier.as_deref(), Some("Read occupier"));
    assert_eq!(runs[1].value.law.as_deref(), Some("extra_mod_law"));
    assert_eq!(runs[1].value.garrison_required_raw.as_deref(), Some("0"));
    assert!(
        !runs[1]
            .value
            .garrison_missing
            .as_deref()
            .unwrap_or("")
            .contains("No matching")
    );
    assert_eq!(runs[2].value.occupier.as_deref(), Some("New owner"));
    assert_eq!(runs[2].value.law.as_deref(), Some("new_mod_law"));
    assert_eq!(
        runs[2].value.garrison_required_raw.as_deref(),
        Some("123999")
    );
    assert_eq!(
        reply
            .observed
            .iter()
            .map(|span| span.read_hour)
            .collect::<Vec<_>>(),
        [100, 102]
    );
}

#[test]
fn native_zero_strength_requires_complete_resolved_manpower_or_the_failure_flag() {
    use replay::rows::{StateAssignedManpowerRow, StateGarrisonRow, TableRows};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/defines/mod.lua",
        "NDefines = { NInterface = { GARRISON_STRENGTH_TO_SHOW_RED = 0.75 } }",
    )]);
    let row = |amount| StateAssignedManpowerRow {
        controller: 11,
        occupied: 9001,
        state: 2000,
        amount,
        ..Default::default()
    };
    let inventories = [
        TableView::Complete(TableRows::StateAssignedManpower(vec![])),
        TableView::Complete(TableRows::StateAssignedManpower(vec![row(1)])),
        TableView::Truncated {
            rows: TableRows::StateAssignedManpower(vec![]),
            missing: 1,
        },
        TableView::Complete(TableRows::StateAssignedManpower(vec![
            StateAssignedManpowerRow {
                controller: 999999,
                ..row(0)
            },
        ])),
        TableView::Complete(TableRows::StateAssignedManpower(vec![
            row(i32::MAX),
            StateAssignedManpowerRow {
                ordinal: 1,
                ..row(i32::MIN)
            },
            StateAssignedManpowerRow {
                ordinal: 2,
                ..row(1)
            },
        ])),
    ];
    let mut scan = super::scan::Scan::new(1, Some(&cat));
    for (index, assigned) in inventories.into_iter().enumerate() {
        let hour = 100 + index as i32;
        let mut read = frame(hour, true);
        *read.tables[TableId::StateGarrisons.index()] =
            TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
                controller: 11,
                occupied: 9001,
                state: 2000,
                garrison_required: 100000,
                strength_ratio: 100000,
                ..Default::default()
            }]));
        *read.tables[TableId::StateAssignedManpower.index()] = assigned;
        scan.push(&read, hour).unwrap();
    }
    let reply = scan.finish();
    let runs = &reply.states[0].runs;
    assert_eq!(runs.len(), 4); // Equal unknown reads compress, with distinct read dates.
    let at = |hour| {
        &runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap()
            .value
    };
    for hour in [100, 104] {
        assert_eq!(at(hour).assigned_manpower, Some(0));
        assert_eq!(at(hour).garrison_strength_raw.as_deref(), Some("0"));
        assert_eq!(at(hour).garrison_red, Some(true));
    }
    for hour in [101, 102, 103] {
        assert_eq!(at(hour).garrison_strength_raw, None);
    }
    assert_eq!(at(101).assigned_manpower, Some(1));
    for hour in [102, 103] {
        assert_eq!(at(hour).assigned_manpower, None);
    }
}

#[test]
fn a_mod_threshold_uses_native_fixed5_and_missing_garrisons_are_not_zero() {
    use replay::rows::{StateGarrisonRow, TableRows};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/defines/mod.lua",
        "NDefines = { NInterface = { GARRISON_STRENGTH_TO_SHOW_RED = 0.0000001 } }",
    )]);
    let mut known = frame(100, true);
    *known.tables[TableId::StateAssignedManpower.index()] =
        TableView::Complete(TableRows::StateAssignedManpower(vec![]));
    *known.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
            controller: 11,
            occupied: 9001,
            state: 2000,
            garrison_required: 100000,
            ..Default::default()
        }]));
    let mut missing = known.clone();
    missing.hour = 101;
    *missing.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![]));
    let mut scan = super::scan::Scan::new(1, Some(&cat));
    for frame in [known, missing] {
        scan.push(&frame, frame.hour).unwrap();
    }
    let reply = scan.finish();
    assert_eq!(reply.states[0].runs[0].value.garrison_red, Some(false));
    assert_eq!(reply.states[0].runs[1].value.garrison_strength_raw, None);
    assert_eq!(reply.states[0].runs[1].value.assigned_manpower, None);
}

#[test]
fn non_utf8_overrides_and_unresolved_links_are_named_and_counted_by_reason() {
    use replay::rows::{StateOccupationLawRow, TableRows};
    let mut read = frame(100, true);
    *read.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 2000,
            owner: 11,
            controller: 11,
            ..Default::default()
        },
        StateRow {
            id: 2001,
            owner: 11,
            controller: 11,
            ..Default::default()
        },
    ]));
    *read.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(vec![
            StateResistanceRow {
                state: 2000,
                country: 11,
                active: 1,
                ..Default::default()
            },
            StateResistanceRow {
                state: 2001,
                country: 11,
                active: 1,
                ..Default::default()
            },
        ]));
    *read.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![
            StateOccupationLawRow {
                controller: 11,
                occupied: 11,
                state: 2000,
                name: vec![0xff],
            },
            StateOccupationLawRow {
                controller: 11,
                occupied: 11,
                state: 2001,
                name: vec![0xfe],
            },
        ]));
    let mut scan = super::scan::Scan::new(1, None);
    scan.push(&read, 100).unwrap();
    let reply = scan.finish();
    assert!(
        reply
            .warnings
            .iter()
            .any(|line| line.contains("2 recorded occupation facts")
                && line.contains("not UTF-8")
                && line.contains("2000")
                && line.contains("2001"))
    );
}

#[test]
#[ignore = "long-campaign projection benchmark; run explicitly with --ignored --nocapture"]
fn long_campaign_scan_at_design_scale() {
    use replay::rows::TableRows;
    use std::time::Instant;
    let mut frame = frame(0, true);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=1500)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("Mod country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=1500)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=5000)
            .map(|id| StateRow {
                id,
                owner: (id - 1) % 1500 + 1,
                controller: (id - 1) % 1500 + 1,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(
            (1..=5000)
                .map(|state| StateResistanceRow {
                    state,
                    country: (state - 1) % 1500 + 1,
                    active: 1,
                    level: i64::from(state),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![]));
    *frame.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![]));
    *frame.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![]));
    *frame.tables[TableId::StateAssignedManpower.index()] =
        TableView::Complete(TableRows::StateAssignedManpower(vec![]));
    let hours = 10 * 365 * 24;
    for changed_hourly in [false, true] {
        let mut scan = super::scan::Scan::new(1, None);
        let started = Instant::now();
        for hour in 0..hours {
            frame.hour = hour;
            frame.metrics.table_ns[TableId::StateResistance.index()] = u64::from(hour % 24 == 0);
            if changed_hourly {
                // Force an hourly catalogue identity change without changing any
                // owner/name or daily occupation value. Mirrors unrelated balances.
                let rows = frame.complete::<tables::Countries>().unwrap().to_vec();
                *frame.tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(rows));
            }
            scan.push(&frame, hour).unwrap();
        }
        let reply = scan.finish();
        let seconds = started.elapsed().as_secs_f64();
        eprintln!(
            "occupied-state projection: 1500 countries, 5000 states, {hours} hours (10 years), hourly catalogue changes={changed_hourly}: {seconds:.3}s ({:.3}ms/hour); excludes decoding/content I/O",
            seconds * 1000.0 / f64::from(hours)
        );
        assert_eq!(reply.states.len(), 5000);
        assert_eq!(reply.observed.len(), 3650);
        assert!(
            reply
                .states
                .iter()
                .all(|state| state.runs.len() == 1 && state.runs[0].last_hour == hours - 1)
        );
    }
}
