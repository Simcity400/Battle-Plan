use super::*;
use native::Blob;

#[test]
fn first_goal_zero_country_uses_dated_reserved_names_and_refuses_missing_input() {
    use replay::{frame::TableView, rows::TableRows};
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[
        (
            "common/wargoals/mod.txt",
            "wargoal_types={synthetic_goal={war_name=SYNTHETIC_NAME}}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n ONE:0 \"One\"\n ONE_ADJ:0 \"First\"\n TWO:0 \"Two\"\n TWO_ADJ:0 \"Second\"\n THREE:0 \"Three\"\n THREE_ADJ:0 \"Third\"\n ZERO:0 \"Reserved first\"\n ZERO_ADJ:0 \"first reserved\"\n LATER:0 \"Reserved later\"\n LATER_ADJ:0 \"later reserved\"\n SYNTHETIC_NAME:0 \"$COUNTRY$ / $COUNTRY_ADJ$\"\n",
        ),
    ]);
    let mut first = recorded(100);
    let TableView::Complete(TableRows::WarOwnedRelations(owned)) =
        &mut *first.tables[TableId::WarOwnedRelations.index()]
    else {
        panic!()
    };
    owned[0].first_goal_count = 1;
    first.tables[TableId::WarRelationGoals.index()] =
        TableView::Complete(TableRows::WarRelationGoals(vec![
            rows::WarRelationGoalRow {
                owner: 1,
                resolved: 1,
                type_token: 42,
                country_count: 1,
                first_country: 0,
                ..Default::default()
            },
        ]))
        .into();
    first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
            token: 42,
            literal: b"synthetic_goal".to_vec(),
        }]))
        .into();
    let mut frames = Vec::new();
    for (hour, tag) in [(100, Some("ZERO")), (101, Some("LATER")), (102, None)] {
        let mut frame = first.clone();
        frame.hour = hour;
        frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
                hour,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::ReservedCountryNaming.index()] = match tag {
            Some(tag) => TableView::Complete(TableRows::ReservedCountryNaming(vec![
                rows::ReservedCountryNamingRow {
                    tag: tag.as_bytes().to_vec(),
                    built: 7,
                    ..Default::default()
                },
            ])),
            None => TableView::Unavailable {
                reason: "synthetic reserved naming read failed".into(),
            },
        }
        .into();
        frames.push(frame);
    }
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, Some(&cat), &AtomicBool::new(false)).unwrap();
    let observations = &reply.wars[0].observations;
    assert_eq!(observations.len(), 3);
    assert_eq!(observations[0].name.text, "Reserved first / first reserved");
    assert_eq!(observations[1].name.text, "Reserved later / later reserved");
    assert!(observations[0].name.known && observations[1].name.known);
    assert!(!observations[2].name.known);
    assert!(
        observations[2]
            .name
            .reason
            .as_ref()
            .unwrap()
            .contains("ReservedCountryNaming")
    );
}

#[test]
fn selected_native_viewpoint_uses_dated_tags_and_refuses_their_replacement() {
    use replay::{frame::TableView, rows::TableRows};
    let frames = (100..=102)
        .map(|hour| {
            let mut frame = recorded(hour);
            if hour > 100 {
                let TableView::Complete(TableRows::Countries(countries)) =
                    &mut *frame.tables[TableId::Countries.index()]
                else {
                    panic!()
                };
                countries[0].tag = b"renamed".to_vec();
                if hour == 102 {
                    countries[0].id = 99;
                    countries.sort_by_key(|row| row.id);
                }
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan_selected(
        &replay,
        1,
        Some("ONE"),
        Some(&HashSet::from([(1, 1)])),
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .viewpoints
            .iter()
            .map(|run| (run.tag.as_str(), run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [("ONE", 100, 100), ("renamed", 101, 101)]
    );
    assert!(reply.wars.iter().any(|war| {
        war.relations
            .iter()
            .any(|run| run.first_hour <= 101 && 101 <= run.last_hour)
    }));
    assert_eq!(
        reply.unobserved,
        [HourSpan {
            first_hour: 102,
            last_hour: 102
        }]
    );
}

#[test]
fn obsolete_war_content_versions_are_replaced_without_evicting_other_views() {
    let replay = Arc::new(
        Replay::from_records(
            b"analytics-test".to_vec(),
            &[crate::test_fixtures::records(1, vec![recorded(100)])],
        )
        .unwrap(),
    );
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "wars".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[]);
    let obsolete = Arc::new(scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap());
    let released = Arc::downgrade(&obsolete);
    let state = source.state::<Vec<Cache>>();
    state.lock().unwrap().extend([
        Cache {
            branch: 1,
            view: None,
            identities: None,
            catalogue: Some(Arc::new(cat)),
            reply: obsolete,
        },
        Cache {
            branch: 1,
            view: Some("TWO".into()),
            identities: None,
            catalogue: None,
            reply: Arc::new(scan(&replay, 1, Some("TWO"), None, &AtomicBool::new(false)).unwrap()),
        },
    ]);
    let params = WarsParams {
        branch_id: "1".into(),
        viewpoint_tag: None,
    };
    let context = LoadContext { content: None };
    Wars::shared(&source, &params, None, &context, &AtomicBool::new(false)).unwrap();
    let cache = state.lock().unwrap();
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.iter().filter(|entry| entry.view.is_none()).count(), 1);
    assert!(
        cache
            .iter()
            .any(|entry| entry.view.as_deref() == Some("TWO"))
    );
    assert!(released.upgrade().is_none());
    // A hit becomes the most recent; a new view evicts the least recently requested.
    let reply = Arc::clone(&cache[0].reply);
    drop(cache);
    state
        .lock()
        .unwrap()
        .extend((0..CACHED_REPLIES).map(|view| Cache {
            branch: 1,
            view: Some(format!("V{view}")),
            identities: None,
            catalogue: None,
            reply: Arc::clone(&reply),
        }));
    for view in ["TWO", "THREE"] {
        let params = WarsParams {
            branch_id: "1".into(),
            viewpoint_tag: Some(view.into()),
        };
        Wars::shared(&source, &params, None, &context, &AtomicBool::new(false)).unwrap();
    }
    let views: Vec<_> = state
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.view.clone())
        .collect();
    assert_eq!(
        views,
        [
            Some("THREE".to_owned()),
            Some("TWO".to_owned()),
            None,
            Some("V0".to_owned())
        ]
    );
}

#[test]
fn viewpoint_runs_do_not_index_a_missing_country_tag() {
    let frame = recorded(100);
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![frame.clone()])],
    )
    .unwrap();
    let mut reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    reply.viewpoints.clear();
    let mut inputs = Hour::read(&frame, None).unwrap();
    append_viewpoint(&mut reply, &inputs, 100);
    append_viewpoint(&mut reply, &inputs, 101);
    assert_eq!(reply.viewpoints[0].last_hour, 101);
    inputs.tags.remove(&inputs.view);
    append_viewpoint(&mut reply, &inputs, 102);
    assert_eq!(reply.viewpoints.len(), 1);
    assert_eq!(reply.viewpoints[0].last_hour, 101);
}

#[test]
fn label_catalogue_keeps_historical_names_after_fighting_ends() {
    use replay::{frame::TableView, rows::TableRows};
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[(
        "localisation/mod_l_english.yml",
        "\u{feff}l_english:\n ONE:0 \"Short\"\n TWO:0 \"Second\"\n THREE:0 \"Third\"\n LATER:0 \"A much longer later country name\"\n",
    )]);
    let first = recorded(100);
    let mut later = recorded(101);
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *later.tables[TableId::CountryAppearance.index()]
    else {
        panic!()
    };
    rows[0].cosmetic_tag = b"LATER".to_vec();
    later.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![])).into();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![first, later])],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, Some(&cat), &AtomicBool::new(false)).unwrap();
    assert!(reply.country_names.contains(&"Short".to_owned()));
    assert!(
        reply
            .country_names
            .contains(&"A much longer later country name".to_owned())
    );
    assert!(reply.country_names.contains(&"Second".to_owned()));
    assert_eq!(
        reply.country_names.iter().collect::<HashSet<_>>().len(),
        reply.country_names.len()
    );
}

#[test]
fn label_catalogue_resolves_only_changed_naming_inputs_and_their_subjects() {
    use replay::{frame::TableView, rows::TableRows};
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[(
        "localisation/mod_l_english.yml",
        "\u{feff}l_english:\n ONE:0 \"First\"\n ONE_ADJ:0 \"First adjective\"\n TWO:0 \"$OVERLORDADJ$ subject\"\n THREE:0 \"Third\"\n THREE_mod_group:0 \"New third\"\n LATER:0 \"Later\"\n LATER_ADJ:0 \"Later adjective\"\n",
    )]);
    let mut first = recorded(100);
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *first.tables[TableId::CountryAppearance.index()]
    else {
        panic!()
    };
    rows[1].overlord = 1;
    let mut labels = labels::CountryLabels::default();
    labels.observe(&first, Some(&cat));
    assert_eq!(labels.resolutions, 3);
    labels.observe(&first, Some(&cat));
    assert_eq!(labels.resolutions, 3);
    let inputs = Hour::read(&first, None).unwrap();
    labels.include(&inputs, &inputs.observe(&first, Some(&cat)));

    let mut daily = first.clone();
    for day in 1..=10 {
        let TableView::Complete(TableRows::Politics(rows)) =
            &mut *daily.tables[TableId::Politics.index()]
        else {
            panic!()
        };
        rows[0].capital = day;
        daily.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![rows::PoliticalPartyRow {
                country: 3,
                id: 7,
                token: b"mod_group".to_vec(),
                popularity: i64::from(day),
                ..Default::default()
            }]))
            .into();
        labels.observe(&daily, Some(&cat));
        assert_eq!(labels.resolutions, 3);
    }
    // Hours agreeing on every naming operand keep none of their views, so a
    // decoder can apply the next change in place.
    let parties = &mut daily.tables[TableId::PoliticalParties.index()];
    let held = std::mem::replace(parties, first.tables[TableId::Clock.index()].clone());
    *parties = held.try_unwrap().expect("a naming view is held").into();
    let TableView::Complete(TableRows::Politics(rows)) =
        &mut *daily.tables[TableId::Politics.index()]
    else {
        panic!()
    };
    rows[2].ruling_party = Some(7);
    labels.observe(&daily, Some(&cat));
    assert_eq!(labels.resolutions, 4);
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *daily.tables[TableId::CountryAppearance.index()]
    else {
        panic!()
    };
    rows[0].cosmetic_tag = b"LATER".to_vec();
    labels.observe(&daily, Some(&cat));
    assert_eq!(labels.resolutions, 6);
    let names = labels.finish();
    for name in [
        "First",
        "Later",
        "First adjective subject",
        "Later adjective subject",
        "Third",
        "New third",
    ] {
        assert!(names.contains(&name.to_owned()), "{name}");
    }
}

fn recorded(hour: i32) -> DecodedFrame {
    use replay::{
        frame::TableView,
        rows::{
            CountryAppearanceRow, CountryLinkRow, CountryRow, LocalPlayerRow, PoliticsRow,
            TableRows, WarNameRuleRow, WarOverviewCountryRow, WarOwnedRelationRow, WarRelationRow,
        },
    };
    let mut frame = crate::test_fixtures::source(1, hour, Some(1));
    for table in [
        TableId::Factions,
        TableId::FactionMembers,
        TableId::WarRelationGoals,
        TableId::Capitulation,
        TableId::LexicalTokens,
        TableId::PoliticalParties,
        TableId::Autonomy,
        TableId::States,
    ] {
        frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
    }
    frame.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
            hour,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![rows::CampaignRow {
            id: b"analytics-test".to_vec(),
        }]))
        .into();
    let tags = ["ONE", "TWO", "THREE"];
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        tags.iter()
            .enumerate()
            .map(|(i, tag)| CountryRow {
                id: i as u32 + 1,
                canonical_slot: i as u32 + 1,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=3)
            .map(|i| CountryLinkRow {
                id: i,
                canonical_slot: i,
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            (1..=3)
                .map(|country| CountryAppearanceRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        (1..=3)
            .map(|country| PoliticsRow {
                country,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 1,
            observer_country: 0,
        }]))
        .into();
    let relations: Vec<_> = [2, 3]
        .into_iter()
        .map(|second_country| WarRelationRow {
            first_country: 1,
            second_country,
            start_hour: 90,
            first_was_instigator: 1,
            first_casualties: i64::MAX,
            second_casualties: 7,
            first_unknown_casualties: -1,
            second_unknown_casualties: 3,
            ..Default::default()
        })
        .collect();
    frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(relations)).into();
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(vec![
            WarOwnedRelationRow {
                owner: 1,
                ordinal: 0,
                first_country: 1,
                second_country: 2,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 1,
                ordinal: 1,
                first_country: 1,
                second_country: 3,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 2,
                ordinal: 0,
                first_country: 1,
                second_country: 2,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 3,
                ordinal: 0,
                first_country: 1,
                second_country: 3,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(
            (1..=3)
                .map(|country| WarOverviewCountryRow {
                    score_available: 1,
                    country,
                    major: 1,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(vec![WarNameRuleRow {
            viewpoint: 1,
            index: 0,
            prefix: b"NATIVE".to_vec(),
        }]))
        .into();
    frame
}

#[test]
fn content_mod_override_native_key_fallback_and_missing_first_goal() {
    use replay::{frame::TableView, rows::TableRows};
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[
        (
            "common/wargoals/mod.txt",
            "wargoal_types={star_contest={war_name=MOD_NAME}}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n ONE:0 \"One\"\n ONE_ADJ:0 \"First\"\n TWO:0 \"Two\"\n TWO_ADJ:0 \"Second\"\n THREE:0 \"Three\"\n THREE_ADJ:0 \"Third\"\n NATIVE:0 \"$ORDER$$FIRST$ versus $SECOND$\"\n MOD_NAME:0 \"$FIRST_COUNTRY$ claims $SECOND_COUNTRY$ $STATE$\"\n MOD_NAME_ONE_TWO:0 \"Pair\"\n MOD_NAME_ONE_TWO_TWO:0 \"Defender suffix\"\n",
        ),
    ]);
    let mut frame = recorded(100);
    let hour = Hour::read(&frame, None).unwrap();
    let name = naming::name(
        &frame,
        &hour,
        pair(1, 2),
        Some(&cat),
        &naming::Inputs::read(&frame),
    );
    assert_eq!(name.text, "First versus Second");
    assert!(name.known);
    let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
        &mut *frame.tables[TableId::WarOwnedRelations.index()]
    else {
        panic!()
    };
    rows[0].first_goal_count = 1;
    frame.tables[TableId::WarRelationGoals.index()] =
        TableView::Complete(TableRows::WarRelationGoals(vec![
            rows::WarRelationGoalRow {
                owner: 1,
                resolved: 1,
                type_token: 42,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
            token: 42,
            literal: b"star_contest".to_vec(),
        }]))
        .into();
    let hour = Hour::read(&frame, None).unwrap();
    let name = naming::name(
        &frame,
        &hour,
        pair(1, 2),
        Some(&cat),
        &naming::Inputs::read(&frame),
    );
    assert_eq!(name.text, "Defender suffix");
    assert!(name.known);
    // The defended country also takes the same defender-tag suffix branch.
    let mut scores = frame.complete::<tables::WarNameRules>().unwrap().to_vec();
    for row in &mut scores {
        row.viewpoint = 2;
    }
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(scores)).into();
    frame.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![rows::LocalPlayerRow {
            country: 2,
            observer_country: 0,
        }]))
        .into();
    let hour = Hour::read(&frame, None).unwrap();
    assert_eq!(
        naming::name(
            &frame,
            &hour,
            pair(1, 2),
            Some(&cat),
            &naming::Inputs::read(&frame)
        )
        .text,
        "Defender suffix"
    );
    frame.tables[TableId::WarRelationGoals.index()] = TableView::Unavailable {
        reason: "missed goal".into(),
    }
    .into();
    let hour = Hour::read(&frame, None).unwrap();
    let name = naming::name(
        &frame,
        &hour,
        pair(1, 2),
        Some(&cat),
        &naming::Inputs::read(&frame),
    );
    assert!(!name.known);
    assert_eq!(name.text, "Not recorded");
}

#[test]
fn branch_scan_preserves_exact_losses_capitulation_identity_and_gap_bounds() {
    use replay::{frame::TableView, rows::TableRows};
    let mut frames: Vec<_> = [100, 101, 103, 104].into_iter().map(recorded).collect();
    for frame in &mut frames {
        frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![rows::CapitulationRow {
                country: 2,
                capitulated: 1,
                capitulated_hour: 101,
                ..Default::default()
            }]))
            .into();
    }
    // Never attribute a capitulation first seen through a newly reused raw id.
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frames[3].tables[TableId::Countries.index()]
    else {
        panic!()
    };
    rows[1].tag = b"NEW".to_vec();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.wars.len(), 2);
    assert_eq!(
        reply.unobserved,
        vec![HourSpan {
            first_hour: 102,
            last_hour: 102
        }]
    );
    let war = &reply.wars[0];
    assert_eq!(
        (war.first_hour, war.last_hour, war.started_by, war.ended_by),
        (100, 101, true, true)
    );
    assert_eq!(
        war.casualties[0].2,
        (2_i128 * i128::from(i64::MAX)).to_string()
    );
    assert_eq!(war.casualties[0].3, "-2");
    assert_eq!(war.casualties[0].4, "14");
    assert_eq!(
        war.capitulations,
        vec![WarCapitulation {
            tag: "TWO".into(),
            hour: 101
        }]
    );
    assert!(reply.wars[1].capitulations.is_empty());
    assert!(reply.warnings.iter().any(|s| s.contains("identity")));
}

#[test]
fn whole_branch_product_at_five_thousand_countries() {
    use replay::{frame::TableView, rows::TableRows};
    let count = 5_000;
    let mut frame = recorded(100);
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|country| rows::CountryRow {
                id: country,
                canonical_slot: country,
                tag: format!("COUNTRY_{country}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| rows::CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (2..=count)
            .map(|second_country| rows::WarRelationRow {
                first_country: 1,
                second_country,
                start_hour: 90,
                first_was_instigator: 1,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    let mut owned = Vec::new();
    for second_country in 2..=count {
        for owner in [1, second_country] {
            owned.push(rows::WarOwnedRelationRow {
                owner,
                ordinal: if owner == 1 { second_country - 2 } else { 0 },
                first_country: 1,
                second_country,
                ..Default::default()
            });
        }
    }
    owned.sort_by_key(|row| (row.owner, row.ordinal));
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(owned)).into();
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(
            (1..=count)
                .map(|country| rows::WarOverviewCountryRow {
                    score_available: 1,
                    country,
                    major: 1,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    let mut next = frame.clone();
    next.hour = 101;
    next.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
            hour: next.hour,
            ..Default::default()
        }]))
        .into();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![frame, next])],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert!(reply.unobserved.is_empty());
    assert_eq!(reply.wars.len(), 1);
    assert_eq!(reply.wars[0].participants.len(), count as usize);
    assert_eq!(reply.wars[0].observations.len(), 1);
    assert_eq!(reply.wars[0].last_hour, 101);
}

#[test]
fn fallback_goal_name_preserves_plain_state_text_and_first_country_operands() {
    use replay::{frame::TableView, rows::TableRows};
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[
        (
            "common/wargoals/mod.txt",
            "wargoal_types={star_contest={war_name=MOD}}",
        ),
        (
            "common/defines/war.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 8 } }",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n ONE:0 \"One\"\n ONE_ADJ:0 \"First\"\n TWO:0 \"Two\"\n TWO_ADJ:0 \"Second\"\n THREE:0 \"Three\"\n THREE_ADJ:0 \"Third\"\n MOD:0 \"$ORDER$$STATE$ $COUNTRY$ $COUNTRY_ADJ$\"\n",
        ),
    ]);
    let mut frame = recorded(100);
    let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
        &mut *frame.tables[TableId::WarOwnedRelations.index()]
    else {
        panic!()
    };
    rows[0].first_goal_count = 1;
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
            token: 42,
            literal: b"star_contest".to_vec(),
        }]))
        .into();
    frame.tables[TableId::WarRelationGoals.index()] =
        TableView::Complete(TableRows::WarRelationGoals(vec![
            rows::WarRelationGoalRow {
                owner: 1,
                resolved: 1,
                type_token: 42,
                state_count: 1,
                first_state: 9,
                country_count: 1,
                first_country: 3,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![rows::StateRow {
            id: 9,
            custom_name: b"[ROOT.GetName]".to_vec(),
            ..Default::default()
        }]))
        .into();
    let hour = Hour::read(&frame, None).unwrap();
    let name = naming::name(
        &frame,
        &hour,
        pair(1, 2),
        Some(&cat),
        &naming::Inputs::read(&frame),
    );
    assert!(name.known, "{:?}", name.reason);
    assert_eq!(name.key.as_deref(), Some("MOD"));
    assert_eq!(name.text, "[ROOT.GetName] Three Third");
}

fn world(edges: &[(u32, u32, i32)], owned: &[(u32, &[u32])]) -> World {
    let mut w = World {
        owned: owned.iter().map(|(s, v)| (*s, v.to_vec())).collect(),
        relations: HashMap::new(),
        adjacent: HashMap::new(),
        factions: HashMap::new(),
        scores: HashMap::new(),
    };
    for &(a, b, start) in edges {
        w.relations.insert(
            pair(a, b),
            rows::WarRelationRow {
                first_country: a,
                second_country: b,
                start_hour: start,
                first_was_instigator: 1,
                ..Default::default()
            },
        );
        w.adjacent.entry(a).or_default().push(b);
        w.adjacent.entry(b).or_default().push(a);
        for country in [a, b] {
            w.scores
                .entry(country)
                .or_insert(rows::WarOverviewCountryRow {
                    score_available: 1,
                    country,
                    divisions: 1,
                    ..Default::default()
                });
        }
    }
    w
}
#[test]
fn native_candidate_faction_ties_swap_order_and_asymmetric_pruning() {
    // Retired 2--5 appears in its owned vector but is not an active edge.
    let mut w = world(
        &[
            (1, 2, 0),
            (2, 3, 0),
            (2, 4, 0),
            (3, 4, 0),
            (1, 6, 0),
            (3, 6, 0),
        ],
        &[(1, &[2, 6]), (2, &[1, 3, 4, 5]), (6, &[1, 3])],
    );
    w.factions.extend([(1, 9), (3, 9)]);
    let blobs = w.blobs(1);
    // 3 and 4 are at war: keep the member of seed 1's faction, remove
    // index 2 by swapping the last (5) into it. Side two's 6 is removed
    // because it lacks an active cross edge toward nonseed 5.
    assert_eq!(
        blobs[0],
        Blob {
            first: vec![1, 3, 5],
            second: vec![2]
        }
    );
    w.factions.remove(&3);
    assert_eq!(w.blobs(1)[0].first, vec![1, 5]);
    w.factions.insert(3, 9);
    w.factions.insert(4, 9);
    assert_eq!(w.blobs(1)[0].first, vec![1, 5]);
}
#[test]
fn dedup_is_viewpoint_local_and_keeps_first_seed_order() {
    let w = world(
        &[(1, 2, 0), (1, 4, 0), (3, 2, 0), (3, 4, 0)],
        &[(1, &[4, 2]), (2, &[3, 1]), (3, &[2, 4]), (4, &[1, 3])],
    );
    assert_eq!(
        w.blobs(1),
        vec![Blob {
            first: vec![1, 3],
            second: vec![4, 2]
        }]
    );
    assert_eq!(
        w.blobs(2),
        vec![Blob {
            first: vec![2, 4],
            second: vec![3, 1]
        }]
    );
}
#[test]
fn representative_is_not_always_oldest_and_major_pins_and_ties_are_native() {
    let mut w = world(&[(1, 2, -10), (1, 4, -10), (3, 2, 0), (3, 4, 1)], &[]);
    let blob = Blob {
        first: vec![1, 3],
        second: vec![4, 2],
    };
    // Equal start hours pick array order (1--4) before (1--2).
    w.scores.get_mut(&1).unwrap().major = 1;
    w.scores.get_mut(&4).unwrap().major = 1;
    assert_eq!(w.representative(&blob).unwrap(), pair(1, 4));
    w.scores.get_mut(&1).unwrap().major = 0;
    w.scores.get_mut(&4).unwrap().major = 0;
    w.scores.get_mut(&3).unwrap().divisions = 50;
    w.scores.get_mut(&2).unwrap().divisions = 30;
    assert_eq!(w.representative(&blob).unwrap(), pair(3, 2));
    w.scores.get_mut(&1).unwrap().major = 1;
    assert_eq!(w.representative(&blob).unwrap(), pair(1, 2));
    w.scores.remove(&1);
    assert!(w.representative(&blob).is_err());
}
fn observation(hour: i32, relations: &[(u32, u32, i32)]) -> Observed {
    Observed {
        relations: relations
            .iter()
            .map(|(a, b, start)| RelationRun {
                first: a.to_string(),
                second: b.to_string(),
                start: *start,
                identity: WarRelationKey::new((*a, *a), (*b, *b), *start),
            })
            .collect(),
        loss_pairs: Vec::new(),
        losses: std::array::from_fn(|_| "0".into()),
        data: WarObservation {
            first_hour: hour,
            last_hour: hour,
            viewpoint_tag: "1".into(),
            viewpoint_side: vec!["1".into()],
            opposing_side: vec!["2".into()],
            attackers_are_viewpoint: Some(true),
            representative: Some(["1".into(), "2".into()]),
            name: naming::unknown("test missing content"),
        },
    }
}
#[test]
fn continuity_split_merge_gaps_redeclarations_and_observation_bounds() {
    let mut wars = Vec::new();
    let open = continue_wars(
        &mut wars,
        &[],
        vec![observation(1, &[(1, 2, 0), (1, 3, 0)])],
        None,
    );
    let split = continue_wars(
        &mut wars,
        &open,
        vec![observation(2, &[(1, 2, 0)]), observation(2, &[(1, 3, 0)])],
        Some(1),
    );
    assert_eq!(
        split.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![0, 1]
    );
    let merge = continue_wars(
        &mut wars,
        &split,
        vec![observation(3, &[(1, 2, 0), (1, 3, 0)])],
        Some(2),
    );
    assert_eq!(merge[0].0, 0);
    assert!(!wars[1].ended_by);
    assert!(wars[0].started_by);
    let restarted = continue_wars(
        &mut wars,
        &merge,
        vec![observation(4, &[(1, 2, 4)])],
        Some(3),
    );
    assert_eq!(restarted[0].0, 2);
    assert!(!wars[0].ended_by && !wars[2].started_by);
    let after_gap = continue_wars(&mut wars, &[], vec![observation(6, &[(1, 2, 4)])], None);
    assert_eq!(after_gap[0].0, 3);
    assert!(wars[2].ended_by && wars[3].started_by);
    assert_eq!(wars[0].observations.len(), 1);
    assert_eq!(wars[0].observations[0].last_hour, 3);
}
#[test]
fn five_thousand_countries_have_no_population_ceiling_or_render_work() {
    for count in [1_500, 5_000] {
        let edges: Vec<_> = (2..=count).map(|s| (1, s, 0)).collect();
        let enemy: Vec<_> = (2..=count).collect();
        let mut w = world(&edges, &[(1, enemy.as_slice())]);
        for s in 2..=count {
            w.owned.insert(s, vec![1]);
        }
        w.scores.get_mut(&1).unwrap().major = 1;
        for score in w.scores.values_mut() {
            score.major = 1;
        }
        let blobs = w.blobs(1);
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].second.len(), count as usize - 1);
        assert_eq!(w.inside(&blobs[0]).len(), count as usize - 1);
        assert_eq!(w.representative(&blobs[0]).unwrap(), pair(1, 2));
    }
}

#[test]
fn indexed_grouping_preserves_membership_properties_with_retired_repeats_and_factions() {
    let pairs = [(1, 2), (1, 3), (1, 4), (2, 3), (2, 4), (3, 4)];
    let mut combinations = 0;
    for bits in 0..64 {
        let edges: Vec<_> = pairs
            .iter()
            .enumerate()
            .filter(|(i, _)| bits & (1 << i) != 0)
            .map(|(_, &(a, b))| (a, b, 0))
            .collect();
        for factions in 0..16 {
            let mut w = world(&edges, &[]);
            for country in 1..=4 {
                let mut owned: Vec<_> = (1..=4).filter(|x| *x != country).collect();
                if bits & 1 != 0 {
                    owned.reverse();
                }
                if bits & 2 != 0 {
                    owned.push(owned[0]);
                }
                w.owned.insert(country, owned);
                if factions & (1 << (country - 1)) != 0 {
                    w.factions.insert(country, 7);
                }
            }
            for view in 1..=4 {
                combinations += 1;
                let groups = w.blobs(view);
                let active_seeds: BTreeSet<_> = w.owned[&view]
                    .iter()
                    .copied()
                    .filter(|&enemy| w.active(view, enemy))
                    .collect();
                assert_eq!(groups.is_empty(), active_seeds.is_empty());
                assert!(groups.len() <= active_seeds.len());
                let mut previous_seed = None;
                for (index, group) in groups.iter().enumerate() {
                    assert_eq!(group.first.first(), Some(&view));
                    let enemy = *group.second.first().unwrap();
                    assert!(active_seeds.contains(&enemy));
                    let seed_position = w.owned[&view].iter().position(|&x| x == enemy).unwrap();
                    assert!(previous_seed.is_none_or(|position| position < seed_position));
                    previous_seed = Some(seed_position);
                    for (side, owner, seed) in
                        [(&group.first, enemy, view), (&group.second, view, enemy)]
                    {
                        for &country in side.iter().skip(1) {
                            assert_ne!(country, seed);
                            assert!(w.owned[&owner].contains(&country));
                            assert!(
                                side.iter().filter(|&&x| x == country).count()
                                    <= w.owned[&owner].iter().filter(|&&x| x == country).count()
                            );
                        }
                        assert!(
                            side.iter()
                                .all(|&a| { side.iter().all(|&b| !w.active(a, b)) })
                        );
                    }
                    assert!(group.first.iter().all(|x| !group.second.contains(x)));
                    assert!(
                        group
                            .first
                            .iter()
                            .skip(1)
                            .all(|&a| { group.second.iter().skip(1).all(|&b| w.active(a, b)) })
                    );
                    for previous in &groups[..index] {
                        assert!(
                            previous.first.len() != group.first.len()
                                || previous.second.len() != group.second.len()
                                || previous.first.iter().any(|x| !group.first.contains(x))
                                || previous.second.iter().any(|x| !group.second.contains(x))
                        );
                    }
                }
            }
        }
    }
    assert_eq!(combinations, 4_096);
}

#[test]
fn retired_occurrences_and_repeated_members_keep_the_first_seed_arrays() {
    let w = world(
        &[(1, 2, 0), (2, 3, 0)],
        &[(1, &[2, 4, 2]), (2, &[1, 3, 3, 4]), (3, &[2])],
    );
    assert_eq!(
        w.blobs(1),
        vec![Blob {
            first: vec![1, 3, 3, 4],
            second: vec![2],
        }]
    );
}

#[test]
fn representative_preserves_wrapping_strength_and_refuses_nonboolean_sorting() {
    let mut w = world(&[(1, 2, -1), (3, 2, 0)], &[]);
    let blob = Blob {
        first: vec![1, 3],
        second: vec![2],
    };
    w.scores.get_mut(&1).unwrap().divisions = i32::MAX;
    w.scores.get_mut(&1).unwrap().factories = 1;
    assert_eq!(w.representative(&blob).unwrap(), pair(3, 2));
    w.scores.get_mut(&3).unwrap().major = 2;
    assert!(w.representative(&blob).unwrap_err().contains("nonboolean"));
}

#[test]
fn casualty_only_hours_reuse_groups_and_observations_with_exact_dated_losses() {
    use replay::{frame::TableView, rows::TableRows};
    let first = recorded(100);
    let mut next = recorded(101);
    let mut rows = first.complete::<tables::WarRelations>().unwrap().to_vec();
    rows[0].first_casualties -= 12;
    next.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(rows.clone())).into();
    let mut inputs = Hour::read(&first, None).unwrap();
    let before = inputs.observe(&first, None);
    assert!(inputs.refresh_losses(&next));
    assert_eq!(
        inputs.losses(&before[0].loss_pairs)[0],
        (i128::from(i64::MAX) * 2 - 12).to_string()
    );
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![first, next])],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.wars[0].observations.len(), 1);
    assert_eq!(reply.wars[0].casualties.len(), 2);
    assert_eq!(reply.wars[0].casualties[1].0, 101);
    rows[0].start_hour += 1;
    let mut redeclared = recorded(102);
    redeclared.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(rows)).into();
    assert!(!inputs.refresh_losses(&redeclared));
}

#[test]
fn overview_viewpoint_must_match_local_player_even_for_a_selected_foreign_view() {
    use replay::{frame::TableView, rows::TableRows};
    let mut frame = recorded(100);
    let mut scores = frame.complete::<tables::WarNameRules>().unwrap().to_vec();
    scores[0].viewpoint = 2;
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(scores)).into();
    frame.tables[TableId::WarOverviewCountries.index()] = TableView::Unavailable {
        reason: "bad score input".into(),
    }
    .into();
    for view in [None, Some("TWO")] {
        assert!(
            Hour::read(&frame, view)
                .err()
                .unwrap()
                .contains("war overview viewpoint differs from LocalPlayer")
        );
    }
}

#[test]
fn loss_only_scan_settles_participants_before_viewpoint_changes_and_gaps() {
    use replay::{frame::TableView, rows::TableRows};
    let mut frames = Vec::new();
    for hour in [100, 101, 102, 103, 104, 106, 107] {
        let mut frame = recorded(hour);
        let mut losses = frame.complete::<tables::WarRelations>().unwrap().to_vec();
        losses[0].first_casualties = hour as i64;
        frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(losses)).into();
        if hour >= 103 {
            frame.tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![rows::LocalPlayerRow {
                    country: 2,
                    observer_country: 0,
                }]))
                .into();
            let mut rules = frame.complete::<tables::WarNameRules>().unwrap().to_vec();
            rules[0].viewpoint = if hour == 107 { 3 } else { 2 };
            frame.tables[TableId::WarNameRules.index()] =
                TableView::Complete(TableRows::WarNameRules(rules)).into();
        }
        frames.push(frame);
    }
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply
            .wars
            .iter()
            .map(|w| (w.first_hour, w.last_hour))
            .collect::<Vec<_>>(),
        [(100, 102), (103, 104), (106, 106)]
    );
    for war in &reply.wars {
        assert!(
            war.participants
                .iter()
                .all(|p| p.last_hour == war.last_hour)
        );
        assert_eq!(war.observations.len(), 1);
    }
    assert_eq!(
        reply.unobserved,
        [
            HourSpan {
                first_hour: 105,
                last_hour: 105
            },
            HourSpan {
                first_hour: 107,
                last_hour: 107
            }
        ]
    );
}

#[test]
fn multi_year_overlapping_groups_keep_casualties_out_of_membership_reply_size() {
    let mut wars = Vec::new();
    let mut open = Vec::new();
    let hours = 24 * 365 * 4;
    for hour in 0..hours {
        let current = (0..4)
            .map(|group| {
                let mut observed = observation(hour, &[(1, group + 2, 0)]);
                observed.data.viewpoint_side = (0..250).map(|i| format!("C{i}")).collect();
                observed.data.opposing_side = (200 + group * 10..450 + group * 10)
                    .map(|i| format!("C{i}"))
                    .collect();
                observed.losses = [
                    (hour as i64 * 1234567).to_string(),
                    "0".into(),
                    (hour as i64 * 7654321).to_string(),
                    "-7".into(),
                ];
                observed
            })
            .collect();
        open = continue_wars(&mut wars, &open, current, hour.checked_sub(1));
    }
    assert_eq!(wars.len(), 4);
    assert!(
        wars.iter()
            .all(|w| w.observations.len() == 1 && w.casualties.len() == hours as usize)
    );
    let reply = WarsReply {
        branch_id: "1".into(),
        available: true,
        viewpoints: vec![],
        country_names: vec![],
        wars,
        unobserved: vec![],
        capitulations_unobserved: vec![],
        warnings: vec![],
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    println!("wars reply four years / four overlapping groups / 500 side entries: {bytes} bytes");
    assert!(bytes < MAX_PRODUCT_REPLY_BYTES / 3, "reply bytes {bytes}");
}

fn overlapping_branch(hours: i32, redeclare: bool) -> Replay {
    use replay::{
        frame::{FrameEncoder, TableView},
        rows::TableRows,
        spool::Record,
    };
    let mut base = recorded(100);
    base.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=10)
            .map(|id| rows::CountryRow {
                id,
                canonical_slot: id,
                tag: format!("C{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    base.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=10)
            .map(|id| rows::CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ))
    .into();
    let mut relations = Vec::new();
    let mut owned = Vec::new();
    let mut ordinals = [0; 11];
    for enemy in 2..=5 {
        for ally in [1, enemy + 4, enemy + 5] {
            relations.push(rows::WarRelationRow {
                first_country: ally,
                second_country: enemy,
                start_hour: 90,
                first_was_instigator: 1,
                ..Default::default()
            });
            for owner in [ally, enemy] {
                owned.push(rows::WarOwnedRelationRow {
                    owner,
                    ordinal: ordinals[owner as usize],
                    first_country: ally,
                    second_country: enemy,
                    ..Default::default()
                });
                ordinals[owner as usize] += 1;
            }
        }
    }
    relations.sort_by_key(|r| (r.first_country, r.second_country));
    owned.sort_by_key(|r| (r.owner, r.ordinal));
    base.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(owned)).into();
    base.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(
            (1..=10)
                .map(|country| rows::WarOverviewCountryRow {
                    country,
                    major: 1,
                    score_available: 1,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    let mut records = crate::test_fixtures::records(1, vec![base.clone()]);
    records.pop();
    let mut encoder = FrameEncoder::new();
    for hour in 100..100 + hours {
        let mut frame = base.clone();
        frame.hour = hour;
        frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
                hour,
                ..Default::default()
            }]))
            .into();
        for row in &mut relations {
            row.first_casualties = i64::from(hour) * 123;
            row.second_casualties = i64::from(hour) * 321;
            if redeclare {
                row.start_hour = hour;
            }
        }
        frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(relations.clone())).into();
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(b"analytics-test".to_vec(), &[records]).unwrap()
}

#[test]
fn three_year_branch_reuses_four_overlapping_groups_when_losses_change_hourly() {
    let hours = 24 * 365 * 3;
    let replay = overlapping_branch(hours, false);
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert!(reply.unobserved.is_empty());
    assert_eq!(reply.wars.len(), 4);
    for war in &reply.wars {
        assert_eq!(war.observations.len(), 1);
        assert_eq!(war.casualties.len(), hours as usize);
        assert_eq!(war.last_hour, 99 + hours);
        assert!(
            war.participants
                .iter()
                .all(|p| p.last_hour == war.last_hour)
        );
    }
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    println!("wars actual three-year branch / four overlapping groups: {bytes} bytes");
    assert!(bytes < MAX_PRODUCT_REPLY_BYTES / 3, "reply bytes {bytes}");
}

#[test]
fn branch_scan_keeps_more_than_half_a_million_runs_when_the_reply_fits() {
    let hours = 125_001;
    let replay = overlapping_branch(hours, false);
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert!(reply.unobserved.is_empty());
    assert_eq!(reply.wars.len(), 4);
    for war in &reply.wars {
        assert_eq!(war.observations.len(), 1);
        assert_eq!(war.casualties.len(), hours as usize);
        assert_eq!(war.last_hour, 99 + hours);
        assert_eq!(
            war.casualties.last().unwrap().2,
            ((99 + hours) * 369).to_string()
        );
        assert!(
            war.participants
                .iter()
                .all(|p| p.last_hour == war.last_hour)
        );
    }
    let bytes = crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    println!("wars reply beyond 500,000 runs: {bytes} bytes");
}

#[test]
fn branch_scan_past_the_former_timeline_ceiling_refuses_only_the_reply_bytes() {
    // Four fresh groups each hour exceed the old 100,000-timeline ceiling.
    let replay = overlapping_branch(25_001, true);
    let error = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "Wars reply exceeds the shared product reply budget ({} MiB)",
            MAX_PRODUCT_REPLY_BYTES / (1024 * 1024)
        )
    );
}

#[test]
fn near_star_with_retired_occurrence_and_second_front_keeps_both_groups_at_scale() {
    fn shape(count: u32) -> World {
        let mut edges: Vec<_> = (2..count - 2).map(|b| (1, b, 0)).collect();
        edges.push((1, count - 1, 1));
        edges.push((count, count - 1, 1));
        let mut w = world(&edges, &[]);
        w.owned = w.adjacent.clone();
        // This retired-only occurrence defeats closed_bipartite.
        w.owned.get_mut(&1).unwrap().push(count - 2);
        w.owned.insert(count - 2, vec![1]);
        w
    }
    for count in [1_500, 5_000] {
        let w = shape(count);
        let blobs = w.blobs(1);
        assert_eq!(blobs.len(), 2);
        let sides: Vec<(HashSet<u32>, HashSet<u32>)> = blobs
            .iter()
            .map(|blob| {
                (
                    blob.first.iter().copied().collect(),
                    blob.second.iter().copied().collect(),
                )
            })
            .collect();
        for (a, b) in w.relations.keys() {
            assert!(sides.iter().any(|(first, second)| {
                (first.contains(a) && second.contains(b))
                    || (first.contains(b) && second.contains(a))
            }));
        }
    }
}

#[test]
fn equivalent_seed_reuse_cannot_hide_the_native_leading_seed_exemption() {
    let w = world(
        &[(1, 2, 0), (1, 3, 0)],
        &[(1, &[2, 3]), (2, &[1, 4]), (3, &[1, 4]), (4, &[2, 3])],
    );
    // Both enemies retain a retired-only occurrence toward 4. Native pruning
    // removes the non-leading enemy, leaving a distinct blob for each seed.
    assert_eq!(
        w.blobs(1),
        [
            Blob {
                first: vec![1, 4],
                second: vec![2]
            },
            Blob {
                first: vec![1, 4],
                second: vec![3]
            }
        ]
    );
}

#[test]
fn availability_distinguishes_absent_overview_tables_from_failed_hour_inputs() {
    use replay::frame::TableView;

    let reply_for = |frames| {
        let replay = Replay::from_records(
            b"analytics-test".to_vec(),
            &[crate::test_fixtures::records(1, frames)],
        )
        .unwrap();
        scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap()
    };
    let mut absent = recorded(100);
    for table in [
        TableId::WarOwnedRelations,
        TableId::WarRelationGoals,
        TableId::WarOverviewCountries,
        TableId::WarNameRules,
    ] {
        absent.tables[table.index()] = TableView::Unavailable {
            reason: "outside recording".into(),
        }
        .into();
    }
    let reply = reply_for(vec![absent.clone()]);
    assert!(!reply.available);
    assert!(reply.wars.is_empty());

    let mut failed = recorded(100);
    failed.tables[TableId::LocalPlayer.index()] = TableView::Unavailable {
        reason: "viewpoint capture failed".into(),
    }
    .into();
    let reply = reply_for(vec![failed]);
    assert!(reply.available);
    assert!(reply.wars.is_empty());
    assert!(reply.viewpoints.is_empty());
    assert_eq!(
        reply.unobserved,
        vec![HourSpan {
            first_hour: 100,
            last_hour: 100
        }]
    );

    let initial = absent.clone();
    absent.hour = 101;
    absent.tables[TableId::Clock.index()] =
        TableView::Complete(rows::TableRows::Clock(vec![rows::ClockRow {
            hour: 101,
            ..Default::default()
        }]))
        .into();
    absent.tables[TableId::WarOwnedRelations.index()] = TableView::Truncated {
        rows: rows::TableRows::empty(TableId::WarOwnedRelations),
        missing: 1,
    }
    .into();
    assert!(reply_for(vec![initial, absent]).available);
}

#[test]
fn peaceful_viewpoint_changes_and_missing_hours_keep_dated_runs() {
    use replay::{
        frame::TableView,
        rows::{LocalPlayerRow, TableRows},
    };

    let frames = [(100, 1), (101, 2), (103, 2), (104, 2)]
        .into_iter()
        .map(|(hour, viewpoint)| {
            let mut frame = recorded(hour);
            for table in [TableId::WarRelations, TableId::WarOwnedRelations] {
                frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
            }
            frame.tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                    country: viewpoint,
                    observer_country: 0,
                }]))
                .into();
            let mut rules = frame.complete::<tables::WarNameRules>().unwrap().to_vec();
            rules[0].viewpoint = viewpoint;
            frame.tables[TableId::WarNameRules.index()] =
                TableView::Complete(TableRows::WarNameRules(rules)).into();
            frame
        })
        .collect();
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let reply = scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap();
    assert!(reply.wars.is_empty());
    assert_eq!(
        reply.viewpoints,
        vec![
            MembershipRun {
                tag: "ONE".into(),
                first_hour: 100,
                last_hour: 100
            },
            MembershipRun {
                tag: "TWO".into(),
                first_hour: 101,
                last_hour: 101
            },
            MembershipRun {
                tag: "TWO".into(),
                first_hour: 103,
                last_hour: 104
            },
        ]
    );
    assert_eq!(
        reply.unobserved,
        vec![HourSpan {
            first_hour: 102,
            last_hour: 102
        }]
    );
}
