use super::*;
use std::collections::BTreeMap;

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

fn law_inputs<'a>(state: Option<&'a [u8]>, group: GroupLaw<'a>) -> LawInputs<'a> {
    LawInputs {
        state,
        key_resolved: true,
        overrides_complete: true,
        group,
        default: Some((true, b"default")),
        fallback: Ok(b"mod_fallback"),
    }
}

#[test]
fn native_law_precedence_and_default_validation() {
    for state in [Some(b"mod_law".as_slice()), Some(b"".as_slice())] {
        let (law, missing) = law_for(
            law_inputs(state, GroupLaw::Unknown(GROUP_LAW_MISSING.into())),
            |_| panic!("state override bypasses validation"),
        );
        assert_eq!(
            law.as_deref(),
            Some(std::str::from_utf8(state.unwrap()).unwrap())
        );
        assert!(missing.is_none());
    }
    let input = law_inputs(None, GroupLaw::Law(Some(b"group")));
    assert_eq!(
        law_for(input, |_| panic!("group override bypasses validation"))
            .0
            .as_deref(),
        Some("group")
    );
    let input = law_inputs(None, GroupLaw::Missing);
    assert_eq!(
        law_for(input, |_| panic!(
            "missing occupation data bypasses validation"
        ))
        .0
        .as_deref(),
        Some("mod_fallback")
    );
    for (accepted, expected) in [(true, "default"), (false, "mod_fallback")] {
        assert_eq!(
            law_for(law_inputs(None, GroupLaw::Law(None)), |token| {
                assert_eq!(token, "default");
                Ok(accepted)
            })
            .0
            .as_deref(),
            Some(expected)
        );
    }
    let unknown = law_for(law_inputs(None, GroupLaw::Law(None)), |_| {
        Err("missing trigger input".into())
    });
    assert_eq!(unknown, (None, Some("missing trigger input".into())));
    let mut input = law_inputs(None, GroupLaw::Law(None));
    input.default = Some((false, b""));
    assert_eq!(
        law_for(input, |token| {
            assert_eq!(token, "mod_fallback");
            Ok(true)
        })
        .0
        .as_deref(),
        Some("mod_fallback")
    );
    let unknown = law_for(
        law_inputs(None, GroupLaw::Unknown(GROUP_LAW_MISSING.into())),
        |_| panic!(),
    );
    assert!(unknown.1.unwrap().contains("takes precedence"));
    let mut input = law_inputs(None, GroupLaw::Law(None));
    input.overrides_complete = false;
    assert!(
        law_for(input, |_| panic!())
            .1
            .unwrap()
            .contains("incomplete")
    );
    let mut input = law_inputs(Some(b"override"), GroupLaw::Law(None));
    input.key_resolved = false;
    assert!(
        law_for(input, |_| panic!())
            .1
            .unwrap()
            .contains("unresolved")
    );
    assert!(
        law_for(law_inputs(Some(&[0xff]), GroupLaw::Law(None)), |_| panic!())
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
    let mapped = country_index(&countries, &links, &mut refused);
    assert_eq!(mapped.names.len(), 5000);
    assert_eq!(
        mapped.names.get(&(12000, 5000)).map(String::as_str),
        Some("Mod country 5000")
    );
    assert!(refused.lines("countries").is_empty());

    let mut invalid = countries[..3].to_vec();
    invalid[0].tag.clear();
    invalid[1].tag = b"Bad\0tag".to_vec();
    invalid[2].tag = vec![0xff];
    let mut refused = Refusals::default();
    assert!(
        country_index(&invalid, &links, &mut refused)
            .names
            .is_empty()
    );
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
fn law_icons_follow_vertical_native_frames_and_name_missing_effects() {
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
    let why = custom.missing.unwrap();
    assert!(why.contains("gfx/FX/mod") && why.contains("Up"), "{why}");
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
            group_laws: replay::rows::Recorded::Value(None),
            fallback_law: replay::rows::Recorded::Value(None),
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
    let shared = scan_history(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    let reply = &shared.states;
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
    let occupier = reply.states[0].runs[0].value.occupier.as_deref().unwrap();
    let means = &shared.trend.series[occupier];
    assert_eq!(
        means.iter().map(|run| run.read_hour).collect::<Vec<_>>(),
        [100, 102, 105]
    );
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
    let reply = scan_history(&replay, 1, None, &AtomicBool::new(false))
        .unwrap()
        .states;
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
    let reply = scan_history(&replay, 1, None, &AtomicBool::new(false))
        .unwrap()
        .states;
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
    use replay::rows::{Recorded, ScriptVariableEntry, ScriptVariablesRow, TableRows};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/occupation_laws/mod.txt",
            "synthetic_default={visible={is_in_array={array=arr value=1}}} synthetic_fallback={} synthetic_group_law={}",
        ),
    ]);
    cat.scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
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
                border_land_doctrine_level: replay::rows::Recorded::Value(None),
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
    *first.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(
            (1..=5000)
                .map(|id| replay::rows::OccupationSettingsRow {
                    country: id,
                    default_law_present: 1,
                    law: b"synthetic_default".to_vec(),
                    group_laws: replay::rows::Recorded::Value(Some(vec![
                        replay::rows::OccupationGroupLaw {
                            occupied: id,
                            scope_country: Some(id),
                            law: (id % 2 == 0).then(|| b"synthetic_group_law".to_vec()),
                        },
                    ])),
                    fallback_law: replay::rows::Recorded::Value(Some(
                        b"synthetic_fallback".to_vec(),
                    )),
                    ..Default::default()
                })
                .collect(),
        ));
    *first.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![]));
    *first.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            (1..=5000)
                .map(|id| ScriptVariablesRow {
                    scope_kind: 1,
                    scope_id: id,
                    seed: Recorded::Value(None),
                    entries: vec![
                        ScriptVariableEntry {
                            name: b"arr^0".to_vec(),
                            value_raw: 100_000,
                        },
                        ScriptVariableEntry {
                            name: b"arr^num".to_vec(),
                            value_raw: 100_000,
                        },
                    ],
                })
                .collect(),
        ));
    let mut second = first.clone();
    let TableView::Complete(TableRows::ScriptVariables(stores)) =
        &mut *second.tables[TableId::ScriptVariables.index()]
    else {
        unreachable!()
    };
    for store in stores {
        store.entries[0].value_raw = 0;
    }
    second.hour = 101;
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![first, second])],
    )
    .unwrap();
    let reply = scan_history(&replay, 1, Some(&cat), &AtomicBool::new(false))
        .unwrap()
        .states;
    assert_eq!(reply.states.len(), 5000);
    for state in &reply.states {
        let default = state.state % 2 == 1;
        assert_eq!(
            state.runs[0].value.law.as_deref(),
            Some(if default {
                "synthetic_default"
            } else {
                "synthetic_group_law"
            })
        );
        assert_eq!(state.runs.len(), if default { 2 } else { 1 });
        if default {
            assert_eq!(
                state.runs[1].value.law.as_deref(),
                Some("synthetic_fallback")
            );
        }
    }
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
    let reply = scan_history(&replay, 1, Some(&cat), &AtomicBool::new(false))
        .unwrap()
        .states;
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
    let reply = scan.finish().states;
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
fn positive_strength_uses_recorded_native_weights_and_mod_content_at_its_daily_read() {
    use replay::rows::{
        GarrisonEquipment, GarrisonEquipmentInputs, LexicalTokenRow, Recorded,
        StateAssignedManpowerRow, StateGarrisonRow, TableRows,
    };
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NResistance={GARRISON_STR_POW_MANPOWER=2, GARRISON_STR_POW_EQUIPMENT=1}, NInterface={GARRISON_STRENGTH_TO_SHOW_RED=0.75}}",
        ),
        (
            "common/units/equipment/mod.txt",
            "equipments={mod_weapon={is_archetype=yes build_cost_ic=2.00003}}",
        ),
    ]);
    let mut frames = Vec::new();
    for (hour, allocated, token) in [(100, 50_000, 700), (101, 100_000, 700), (102, 50_000, 701)] {
        let mut read = frame(hour, true);
        *read.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 700,
                    literal: b"mod_weapon".to_vec(),
                },
                LexicalTokenRow {
                    token: 701,
                    literal: b"missing_mod_weapon".to_vec(),
                },
            ]));
        *read.tables[TableId::StateGarrisons.index()] =
            TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
                controller: 11,
                occupied: 9001,
                state: 2000,
                required_manpower: 100,
                strength_ratio: 1,
                garrison_required: 123_999,
                equipment: Recorded::Value(GarrisonEquipmentInputs {
                    rows: vec![GarrisonEquipment {
                        equipment_token: token,
                        archetype_token: token,
                        archetype_active: 1,
                        required_raw: 100_000,
                        required_lookup_raw: 100_000,
                        allocated_raw: allocated,
                        weight_raw: 200_003,
                    }],
                    failure: None,
                }),
                ..Default::default()
            }]));
        *read.tables[TableId::StateAssignedManpower.index()] =
            TableView::Complete(TableRows::StateAssignedManpower(vec![
                StateAssignedManpowerRow {
                    controller: 11,
                    occupied: 9001,
                    state: 2000,
                    source_country: 11,
                    amount: 100,
                    ..Default::default()
                },
            ]));
        frames.push(read);
    }
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan_history(&replay, 1, Some(&cat), &AtomicBool::new(false))
        .unwrap()
        .states;
    let runs = &reply.states[0].runs;
    assert_eq!(
        runs[0].value.garrison_strength_raw.as_deref(),
        Some("49999")
    );
    assert_eq!(runs[0].value.garrison_red, Some(true));
    assert_eq!(
        runs[1].value.garrison_strength_raw.as_deref(),
        Some("100000")
    );
    assert_eq!(runs[1].value.garrison_red, Some(false));
    assert_eq!(runs[2].value.garrison_strength_raw, None);
    assert!(
        runs[2]
            .value
            .garrison_missing
            .as_ref()
            .unwrap()
            .contains("missing_mod_weapon")
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("missing_mod_weapon"))
    );
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
    let reply = scan.finish().states;
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
    let reply = scan.finish().states;
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
        let reply = scan.finish().states;
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

#[test]
fn occupation_default_validation_uses_native_root_from_and_prev_links() {
    use crate::triggers::{ContentFacts, World};
    let mut frame = frame(100, true);
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(replay::rows::TableRows::Countries(vec![
            CountryRow {
                id: 2,
                canonical_slot: 2,
                tag: b"TGT".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 9001,
                canonical_slot: 7,
                tag: b"OCC".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(replay::rows::TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 9001,
                canonical_slot: 7,
            },
        ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(replay::rows::TableRows::CountryCounterInputs(
            [2, 9001]
                .map(|country| replay::rows::CountryCounterInputRow {
                    country,
                    owned_state_count: 1,
                    ..Default::default()
                })
                .into(),
        ));
    let dlcs = std::collections::BTreeSet::new();
    let triggers = installed_content::trigger_keys::TriggerNames::default();
    let aliases = Arc::new(installed_content::tag_aliases::TagAliases::default());
    let world = World::read(
        &frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    );
    let laws = installed_content::occupation_laws::parse(
        b"graph={visible={PREV={tag=OCC} FROM={ROOT={tag=OCC} FROM={tag=TGT}}} available={FROM={tag=TGT}}} hidden={visible={always=no} available={unreached_operand=yes}} self={available={always=no}}",
        &installed_content::key_names::KeyNames::default(),
    ).unwrap();
    assert_eq!(occupation_law_valid(&world, &laws[0], 7, 2), Ok(true));
    assert_eq!(occupation_law_valid(&world, &laws[1], 7, 2), Ok(false));
    assert_eq!(occupation_law_valid(&world, &laws[2], 7, 7), Ok(true));
    assert_eq!(occupation_law_valid(&world, &laws[2], 7, 2), Ok(false));
}

#[test]
fn occupation_group_law_and_null_default_are_joined_at_the_supplying_read() {
    use replay::rows::{OccupationGroupLaw, OccupationSettingsRow, Recorded, TableRows};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/occupation_laws/mod.txt",
            "mod_default={available={always=no}} group_policy={} native_fallback={}",
        ),
    ]);
    let mut first = frame(100, true);
    *first.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![]));
    let mut settings = OccupationSettingsRow {
        country: 11,
        default_law_present: 1,
        law: b"mod_default".to_vec(),
        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
            occupied: 9001,
            scope_country: Some(9001),
            law: None,
        }])),
        fallback_law: Recorded::Value(Some(b"native_fallback".to_vec())),
        ..Default::default()
    };
    *first.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![settings.clone()]));
    let mut held = first.clone();
    held.hour = 101;
    held.metrics.table_ns[TableId::StateResistance.index()] = 0;
    settings.group_laws = Recorded::Value(Some(vec![OccupationGroupLaw {
        occupied: 11,
        scope_country: Some(11),
        law: Some(b"group_policy".to_vec()),
    }]));
    *held.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![settings.clone()]));
    let mut next = held.clone();
    next.hour = 102;
    next.metrics.table_ns[TableId::StateResistance.index()] = 1;
    let mut absent = next.clone();
    absent.hour = 103;
    settings.group_laws = Recorded::Value(Some(vec![]));
    *absent.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![settings]));
    let mut scan = super::scan::Scan::new(1, Some(&cat));
    for frame in [first, held, next, absent] {
        scan.push(&frame, frame.hour).unwrap();
    }
    let reply = scan.finish().states;
    let runs = &reply.states[0].runs;
    assert_eq!(runs.len(), 3);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 101));
    assert_eq!(runs[0].value.law.as_deref(), Some("mod_default"));
    assert_eq!(runs[1].value.law.as_deref(), Some("group_policy"));
    assert_eq!(runs[2].value.law.as_deref(), Some("native_fallback"));
}

#[test]
fn occupation_default_scope_uses_the_retained_country_pointer_independently_of_membership() {
    use replay::rows::{OccupationGroupLaw, OccupationSettingsRow, Recorded, TableRows};
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/occupation_laws/mod.txt",
            "mod_default={available={always=no}} group_policy={} native_fallback={}",
        ),
    ]);
    cat.scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let mut first = frame(100, true);
    *first.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![]));
    let mut countries = first.complete::<tables::Countries>().unwrap().to_vec();
    countries.push(CountryRow {
        id: 2,
        canonical_slot: 8,
        tag: b"Target".to_vec(),
        ..Default::default()
    });
    countries.sort_unstable_by_key(|row| row.id);
    *first.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let mut links = first.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 2,
        canonical_slot: 8,
    });
    links.sort_unstable_by_key(|row| row.id);
    *first.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    let cases = [
        (Some(9001), None, Some("mod_default")),
        (Some(2), None, Some("native_fallback")),
        (None, None, None),
        (None, Some(b"group_policy".to_vec()), Some("group_policy")),
    ];
    let mut scan = super::scan::Scan::new(1, Some(&cat));
    for (index, (scope_country, law, _)) in cases.iter().enumerate() {
        let mut read = first.clone();
        read.hour += index as i32;
        *read.tables[TableId::OccupationSettings.index()] =
            TableView::Complete(TableRows::OccupationSettings(vec![OccupationSettingsRow {
                country: 11,
                default_law_present: 1,
                law: b"mod_default".to_vec(),
                group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
                    occupied: 9001,
                    scope_country: *scope_country,
                    law: law.clone(),
                }])),
                fallback_law: Recorded::Value(Some(b"native_fallback".to_vec())),
                ..Default::default()
            }]));
        scan.push(&read, read.hour).unwrap();
    }
    let reply = scan.finish().states;
    let runs = &reply.states[0].runs;
    assert_eq!(runs.len(), cases.len());
    for (run, (_, _, expected)) in runs.iter().zip(cases) {
        assert_eq!(run.value.law.as_deref(), expected);
        if expected.is_none() {
            assert!(run.value.law_missing.as_deref().unwrap().contains("+0x18"));
        }
    }
}

#[test]
fn occupation_political_default_does_not_require_unrelated_map_assets() {
    use replay::rows::{
        CompletedFocusRow, CountryAppearanceRow, LexicalTokenRow, OccupationGroupLaw,
        OccupationSettingsRow, PoliticalPartyRow, Recorded, ScriptFlagRow, TableRows,
    };
    for visible in [
        "has_government=mod_party",
        "original_tag=MDA",
        "FROM={original_tag=MDA}",
        "has_country_flag=mod_law_flag",
        "FROM={has_completed_focus=mod_focus}",
    ] {
        let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/synthetic.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
            ),
            (
                "common/occupation_laws/mod.txt",
                &format!("mod_default={{visible={{{visible}}}}} native_fallback={{}}"),
            ),
            (
                "common/ideologies/mod.txt",
                "ideologies={mod_party={types={subtype={}}}}",
            ),
        ]);
        assert!(cat.cached_states().is_err());
        let mut read = frame(100, true);
        *read.tables[TableId::StateOccupationLaws.index()] =
            TableView::Complete(TableRows::StateOccupationLaws(vec![]));
        *read.tables[TableId::OccupationSettings.index()] =
            TableView::Complete(TableRows::OccupationSettings(vec![OccupationSettingsRow {
                country: 11,
                default_law_present: 1,
                law: b"mod_default".to_vec(),
                group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
                    occupied: 9001,
                    scope_country: Some(9001),
                    law: None,
                }])),
                fallback_law: Recorded::Value(Some(b"native_fallback".to_vec())),
                ..Default::default()
            }]));
        *read.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 9001,
                id: 7,
                token: b"mod_party".to_vec(),
                is_ruling: 1,
                ..Default::default()
            }]));
        *read.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 9001,
                name_tag: b"MDA".to_vec(),
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            }]));
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *read.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        countries[0].tag = b"MDA".to_vec();
        *read.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                replay::rows::CountryCounterInputRow {
                    country: 9001,
                    owned_state_count: 1,
                    ..Default::default()
                },
            ]));
        *read.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 901,
                literal: b"mod_law_flag".to_vec(),
            }]));
        *read.tables[TableId::ScriptFlags.index()] =
            TableView::Complete(TableRows::ScriptFlags(vec![ScriptFlagRow {
                scope_kind: 1,
                scope_id: 9001,
                token: 901,
                value: 1,
                ..Default::default()
            }]));
        *read.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
                country: 11,
                token: b"mod_focus".to_vec(),
                ..Default::default()
            }]));
        *read.tables[TableId::Provinces.index()] = TableView::Unavailable {
            reason: "unrelated map gap".into(),
        };
        let mut scan = super::scan::Scan::new(1, Some(&cat));
        scan.push(&read, 100).unwrap();
        assert_eq!(
            scan.finish().states.states[0].runs[0].value.law.as_deref(),
            Some("mod_default"),
            "{visible}"
        );
    }
}

#[test]
fn occupation_default_continent_reads_the_retained_cache_through_from_capital() {
    use replay::rows::{
        LexicalTokenRow, OccupationGroupLaw, OccupationSettingsRow, PoliticsRow, Recorded,
        TableRows,
    };
    let (cat, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/occupation_laws/mod.txt",
            "mod_default={visible={FROM={capital_scope={is_on_continent=second_mod_continent}}}} native_fallback={}",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=mod_continents.txt",
        ),
        (
            "map/mod_continents.txt",
            "continents={first_mod_continent second_mod_continent}",
        ),
    ]);
    assert!(cat.cached_states().is_err());
    let mut read = frame(100, true);
    *read.tables[TableId::StateOccupationLaws.index()] =
        TableView::Complete(TableRows::StateOccupationLaws(vec![]));
    *read.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(vec![OccupationSettingsRow {
            country: 11,
            default_law_present: 1,
            law: b"mod_default".to_vec(),
            group_laws: Recorded::Value(Some(vec![OccupationGroupLaw {
                occupied: 9001,
                scope_country: Some(9001),
                law: None,
            }])),
            fallback_law: Recorded::Value(Some(b"native_fallback".to_vec())),
            ..Default::default()
        }]));
    *read.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 9001,
            capital: 2000,
            ..Default::default()
        }]));
    *read.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 901,
            literal: b"second_mod_continent".to_vec(),
        }]));
    *read.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated province gap".into(),
    };
    for (continent, expected) in [
        (Recorded::Value(Some(901)), Some("mod_default")),
        (Recorded::Value(Some(0)), Some("native_fallback")),
        (Recorded::Value(None), None),
        (Recorded::NotRecorded, None),
    ] {
        let TableView::Complete(TableRows::States(states)) =
            &mut *read.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        states[0].continent = continent;
        let mut scan = super::scan::Scan::new(1, Some(&cat));
        scan.push(&read, 100).unwrap();
        let reply = scan.finish().states;
        let value = &reply.states[0].runs[0].value;
        assert_eq!(value.law.as_deref(), expected);
        if expected.is_none() {
            assert!(value.law_missing.as_deref().unwrap().contains("continent"));
        }
    }
    let (duplicates, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/occupation_laws/mod.txt",
            "mod_default={visible={FROM={capital_scope={is_on_continent=second_mod_continent}}}} native_fallback={}",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=mod_continents.txt",
        ),
        (
            "map/mod_continents.txt",
            "continents={second_mod_continent SECOND_MOD_CONTINENT}",
        ),
    ]);
    let TableView::Complete(TableRows::States(states)) = &mut *read.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    states[0].continent = Recorded::Value(Some(901));
    let mut scan = super::scan::Scan::new(1, Some(&duplicates));
    scan.push(&read, 100).unwrap();
    let reply = scan.finish().states;
    let value = &reply.states[0].runs[0].value;
    assert!(value.law.is_none());
    assert!(
        value
            .law_missing
            .as_deref()
            .unwrap()
            .contains("duplicate continent")
    );
}
