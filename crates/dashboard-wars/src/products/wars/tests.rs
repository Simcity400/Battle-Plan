use super::test_fixtures::{recorded, scaled_recorded};
use super::*;
use native::Blob;

#[test]
fn first_goal_zero_country_uses_dated_reserved_names_and_refuses_missing_input() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        &[dashboard_base::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let cat = Arc::new(cat);
    let reply = scan(&replay, 1, None, Some(&cat), &AtomicBool::new(false)).unwrap();
    assert_prepared_parity(&replay, Some(&cat), &[None, Some("TWO")]);
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
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        &[dashboard_base::test_fixtures::records(1, frames)],
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
            &[dashboard_base::test_fixtures::records(
                1,
                vec![recorded(100)],
            )],
        )
        .unwrap(),
    );
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
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
        &[dashboard_base::test_fixtures::records(
            1,
            vec![frame.clone()],
        )],
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
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        &[dashboard_base::test_fixtures::records(
            1,
            vec![first, later],
        )],
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
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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

#[test]
fn content_mod_override_native_key_fallback_and_missing_first_goal() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
fn alliance_name_variant_reads_the_viewpoint_predicate_only_when_its_key_exists() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    let content = |keys: &str| {
        let file = format!(
            "\u{feff}l_english:\n ONE:0 \"One\"\n ONE_ADJ:0 \"First\"\n TWO:0 \"Two\"\n TWO_ADJ:0 \"Second\"\n THREE:0 \"Three\"\n THREE_ADJ:0 \"Third\"\n NATIVE:0 \"$ORDER$$FIRST$ versus $SECOND$\"\n{keys}"
        );
        catalogue::tests::folder_catalogue(&[("localisation/mod_l_english.yml", file.as_str())])
    };
    let (prefix_only, _prefix_folder) = content("");
    let (pair_only, _pair_folder) = content(" NATIVE_ONE_TWO:0 \"Pair\"\n");
    let (alliance, _alliance_folder) =
        content(" NATIVE_ONE_TWO:0 \"Pair\"\n NATIVE_ONE_TWO_TWO:0 \"Allied pair\"\n");
    fn named(frame: &DecodedFrame, view: Option<&str>, cat: &Catalogue) -> WarName {
        let hour = Hour::read(frame, view).unwrap();
        naming::name(
            frame,
            &hour,
            pair(1, 2),
            Some(cat),
            &naming::Inputs::read(frame),
        )
    }
    fn plain(name: WarName, text: &str, key: &str) {
        assert!(name.known, "{:?}", name.reason);
        assert_eq!((name.text.as_str(), name.key.as_deref()), (text, Some(key)));
    }
    fn refused(name: WarName, reason: &str) {
        assert!(!name.known);
        assert_eq!(name.text, "Not recorded");
        assert_eq!(name.reason.as_deref(), Some(reason));
    }
    fn score(frame: &mut DecodedFrame, country: u32, available: u8, subject: u8) {
        let TableView::Complete(TableRows::WarOverviewCountries(scores)) =
            &mut *frame.tables[TableId::WarOverviewCountries.index()]
        else {
            panic!()
        };
        let row = scores.iter_mut().find(|r| r.country == country).unwrap();
        row.score_available = available;
        row.view_subject = subject;
    }
    fn factions(frame: &mut DecodedFrame, members: &[u32]) {
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(
                members
                    .iter()
                    .enumerate()
                    .map(|(ordinal, &country)| rows::FactionMemberRow {
                        country,
                        faction: 7,
                        ordinal: ordinal as u32,
                    })
                    .collect(),
            ))
            .into();
    }
    const UNCAPTURED: &str = "Not recorded: cached subject naming predicate";
    const FOREIGN: &str = "Not recorded: cached subject naming predicate for the selected viewpoint (only the recording client's is captured)";

    // ONE records. THREE fights the instigator ONE, so only TWO's cached
    // subject flag toward THREE could ally them, and only THREE captures it.
    let mut frame = recorded(100);
    plain(
        named(&frame, Some("THREE"), &prefix_only),
        "First versus Second",
        "NATIVE",
    );
    plain(
        named(&frame, Some("THREE"), &pair_only),
        "Pair",
        "NATIVE_ONE_TWO",
    );
    refused(named(&frame, Some("THREE"), &alliance), FOREIGN);

    // THREE records, but without score capture TWO's flag toward it is missing.
    frame.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![rows::LocalPlayerRow {
            country: 3,
            observer_country: 0,
        }]))
        .into();
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(vec![rows::WarNameRuleRow {
            viewpoint: 3,
            index: 0,
            prefix: b"NATIVE".to_vec(),
        }]))
        .into();
    score(&mut frame, 2, 0, 1);
    plain(
        named(&frame, None, &prefix_only),
        "First versus Second",
        "NATIVE",
    );
    plain(named(&frame, None, &pair_only), "Pair", "NATIVE_ONE_TWO");
    refused(named(&frame, None, &alliance), UNCAPTURED);

    // A captured flag selects the alliance key exactly as before.
    score(&mut frame, 2, 1, 0);
    plain(named(&frame, None, &alliance), "Pair", "NATIVE_ONE_TWO");
    score(&mut frame, 2, 1, 1);
    plain(
        named(&frame, None, &alliance),
        "Allied pair",
        "NATIVE_ONE_TWO_TWO",
    );
    // Either endpoint's own viewpoint is allied without any cached flag.
    score(&mut frame, 2, 0, 0);
    for view in ["ONE", "TWO"] {
        plain(
            named(&frame, Some(view), &alliance),
            "Allied pair",
            "NATIVE_ONE_TWO_TWO",
        );
    }

    // At peace with THREE, the instigator is tested first: its faction allies
    // without reading TWO's missing flag, and its own missing flag refuses the
    // name even though TWO shares THREE's faction.
    let TableView::Complete(TableRows::WarRelations(relations)) =
        &mut *frame.tables[TableId::WarRelations.index()]
    else {
        panic!()
    };
    relations.retain(|r| r.second_country != 3);
    factions(&mut frame, &[1, 3]);
    plain(
        named(&frame, None, &alliance),
        "Allied pair",
        "NATIVE_ONE_TWO_TWO",
    );
    factions(&mut frame, &[2, 3]);
    score(&mut frame, 1, 0, 0);
    refused(named(&frame, None, &alliance), UNCAPTURED);
    score(&mut frame, 1, 1, 0);
    plain(
        named(&frame, None, &alliance),
        "Allied pair",
        "NATIVE_ONE_TWO_TWO",
    );
}

#[test]
fn branch_scan_preserves_exact_losses_capitulation_identity_and_gap_bounds() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        &[dashboard_base::test_fixtures::records(1, frames)],
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
fn fallback_goal_name_preserves_plain_state_text_and_first_country_operands() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        representative: None,
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
/// Losses for groups that never share a war.
fn unmerged(_: &[(Pair, bool)]) -> [String; 4] {
    unreachable!("no war holds two groups")
}
/// A group named for its first relation, with its members and loss pairs.
fn group(hour: i32, viewpoint: &[&str], opposing: &[&str], relations: &[(u32, u32, i32)]) -> Observed {
    let mut observed = observation(hour, relations);
    observed.representative = observed.relations.first().map(|r| r.identity.clone());
    observed.loss_pairs = relations.iter().map(|&(a, b, _)| (pair(a, b), true)).collect();
    observed.data.viewpoint_side = viewpoint.iter().map(|t| t.to_string()).collect();
    observed.data.opposing_side = opposing.iter().map(|t| t.to_string()).collect();
    observed
}
#[test]
fn a_declaration_listed_beside_the_war_it_is_named_for_joins_that_war() {
    let pairs_losses = |pairs: &[(Pair, bool)]| -> [String; 4] {
        [pairs.len().to_string(), "0".into(), "0".into(), "0".into()]
    };
    let mut wars = Vec::new();
    // 1 at war with 2 and 3 (named for 1--2).
    let mut open = continue_wars(
        &mut wars,
        &[],
        vec![group(1, &["1"], &["2", "3"], &[(1, 2, 0), (1, 3, 0)])],
        None,
        &pairs_losses,
    );
    // 1 declares on 4: the game lists 4 in a second group, still named for 1--2,
    // in either order.
    for (hour, order) in [(2, false), (3, true)] {
        let mut current = vec![
            group(hour, &["1"], &["2", "3"], &[(1, 2, 0), (1, 3, 0)]),
            group(hour, &["1"], &["4", "2", "3"], &[(1, 2, 0), (1, 4, 1), (1, 3, 0)]),
        ];
        if order {
            current.reverse();
        }
        open = continue_wars(&mut wars, &open, current, Some(hour - 1), &pairs_losses);
        assert_eq!(open.len(), 1);
        assert_eq!((open[0].id, open[0].groups), (0, 2));
        assert_eq!(wars.len(), 1, "no war starts for the listed declaration");
        // Each relation counts once though both groups hold 1--2 and 1--3.
        assert_eq!(wars[0].casualties.last().unwrap().2, "3");
        let observed = wars[0].observations.last().unwrap();
        let mut opposing = observed.opposing_side.clone();
        opposing.sort();
        assert_eq!(opposing, ["2", "3", "4"]);
    }
    assert!(wars[0].participants.iter().any(|p| p.tag == "4" && p.first_hour == 2));
    // A group named for a relation no war holds is a war of its own, though it
    // shares the viewpoint's other relations.
    let mut wars = Vec::new();
    let open = continue_wars(
        &mut wars,
        &[],
        vec![group(1, &["1"], &["2"], &[(1, 2, 0)])],
        None,
        &pairs_losses,
    );
    let next = continue_wars(
        &mut wars,
        &open,
        vec![
            group(2, &["1"], &["2"], &[(1, 2, 0)]),
            group(2, &["1"], &["5", "2"], &[(1, 5, 2), (1, 2, 0)]),
        ],
        Some(1),
        &pairs_losses,
    );
    assert_eq!(next.iter().map(|war| war.id).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(wars.len(), 2);
}
#[test]
fn continuity_split_merge_gaps_redeclarations_and_observation_bounds() {
    let mut wars = Vec::new();
    let open = continue_wars(
        &mut wars,
        &[],
        vec![observation(1, &[(1, 2, 0), (1, 3, 0)])],
        None,
        &unmerged,
    );
    let split = continue_wars(
        &mut wars,
        &open,
        vec![observation(2, &[(1, 2, 0)]), observation(2, &[(1, 3, 0)])],
        Some(1),
        &unmerged,
    );
    assert_eq!(split.iter().map(|war| war.id).collect::<Vec<_>>(), vec![0, 1]);
    let merge = continue_wars(
        &mut wars,
        &split,
        vec![observation(3, &[(1, 2, 0), (1, 3, 0)])],
        Some(2),
        &unmerged,
    );
    assert_eq!(merge[0].id, 0);
    assert!(!wars[1].ended_by);
    assert!(wars[0].started_by);
    let restarted = continue_wars(
        &mut wars,
        &merge,
        vec![observation(4, &[(1, 2, 4)])],
        Some(3),
        &unmerged,
    );
    assert_eq!(restarted[0].id, 2);
    assert!(!wars[0].ended_by && !wars[2].started_by);
    let after_gap = continue_wars(
        &mut wars,
        &[],
        vec![observation(6, &[(1, 2, 4)])],
        None,
        &unmerged,
    );
    assert_eq!(after_gap[0].id, 3);
    assert!(wars[2].ended_by && wars[3].started_by);
    assert_eq!(wars[0].observations.len(), 1);
    assert_eq!(wars[0].observations[0].last_hour, 3);
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
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        &[dashboard_base::test_fixtures::records(1, vec![first, next])],
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
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
        &[dashboard_base::test_fixtures::records(1, frames)],
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
fn overlapping_groups_keep_casualties_out_of_membership_reply_size() {
    let groups: Vec<_> = (0..4)
        .map(|group| {
            let mut observed = observation(0, &[(1, group + 2, 0)]);
            observed.data.viewpoint_side = (0..250).map(|i| format!("C{i}")).collect();
            observed.data.opposing_side = (200 + group * 10..450 + group * 10)
                .map(|i| format!("C{i}"))
                .collect();
            observed
        })
        .collect();
    let membership_bytes: usize = groups
        .iter()
        .map(|group| {
            dashboard_core::transport::json_len(&group.data, MAX_PRODUCT_REPLY_BYTES).unwrap()
        })
        .sum();
    // Repeat just far enough that copying the side lists into every hourly
    // observation would exceed the asserted reply bound.
    let hours = i32::try_from(MAX_PRODUCT_REPLY_BYTES / 3 / membership_bytes + 1).unwrap();
    let mut wars = Vec::new();
    let mut open = Vec::new();
    for hour in 0..hours {
        let mut current = groups.clone();
        for observed in &mut current {
            observed.data.first_hour = hour;
            observed.data.last_hour = hour;
            observed.losses = [
                (hour as i64 * 1234567).to_string(),
                "0".into(),
                (hour as i64 * 7654321).to_string(),
                "-7".into(),
            ];
        }
        open = continue_wars(&mut wars, &open, current, hour.checked_sub(1), &unmerged);
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
    println!(
        "wars reply {hours} hours / four overlapping groups / 500 side entries: {bytes} bytes"
    );
    assert!(bytes < MAX_PRODUCT_REPLY_BYTES / 3, "reply bytes {bytes}");
}

fn overlapping_frames(hours: i32, redeclare: bool) -> impl Iterator<Item = DecodedFrame> {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
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
    (100..100 + hours).map(move |hour| {
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
        frame
    })
}

fn overlapping_branch(hours: i32) -> Replay {
    use replay::spool::Record;
    use replay_frame::frame::FrameEncoder;
    let mut frames = overlapping_frames(hours, false).peekable();
    let mut records =
        dashboard_base::test_fixtures::records(1, vec![frames.peek().unwrap().clone()]);
    records.pop();
    let mut encoder = FrameEncoder::new();
    for frame in frames {
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(b"analytics-test".to_vec(), &[records]).unwrap()
}

// Drives the branch walk without frame encoding, which dominated these regressions' runtime.
fn walk_overlapping(hours: i32, redeclare: bool) -> io::Result<WarsReply> {
    let mut walk = Walk::new(1, 100);
    for frame in overlapping_frames(hours, redeclare) {
        walk.observe(&frame, None, None, None);
    }
    walk.finish(99 + hours, None)
}

#[test]
fn cross_block_branch_reuses_four_overlapping_groups_when_losses_change_hourly() {
    let hours = i32::try_from(replay_frame::KEYFRAME_INTERVAL).unwrap() + 1;
    let replay = overlapping_branch(hours);
    assert!(replay.index.len() > 1);
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
    println!("wars cross-block branch / four overlapping groups: {bytes} bytes");
    assert!(bytes < MAX_PRODUCT_REPLY_BYTES / 3, "reply bytes {bytes}");
}

// regression: 6d4ca1eea6 Measure complete war replies instead of limiting timeline and run counts
#[test]
fn branch_walk_keeps_more_than_half_a_million_runs_when_the_reply_fits() {
    let hours = 125_001;
    let reply = walk_overlapping(hours, false).unwrap();
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
}

// regression: 6d4ca1eea6 Measure complete war replies instead of limiting timeline and run counts
#[test]
fn branch_walk_past_the_former_timeline_ceiling_refuses_only_the_reply_bytes() {
    let error = walk_overlapping(25_001, true).unwrap_err();
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
    for count in [16] {
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
    use replay_frame::frame::TableView;

    let reply_for = |frames| {
        let replay = Replay::from_records(
            b"analytics-test".to_vec(),
            &[dashboard_base::test_fixtures::records(1, frames)],
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
    use replay_data::{rows::LocalPlayerRow, rows::TableRows};
    use replay_frame::frame::TableView;

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
        &[dashboard_base::test_fixtures::records(1, frames)],
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

/// The actual prepared observer/assembler, compared with the retained scan for every reached hour.
fn assert_prepared_parity(replay: &Replay, cat: Option<&Arc<Catalogue>>, views: &[Option<&str>]) {
    use dashboard_base::prepared::series;
    let fixture = series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "wars-prepared".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::open(replay.bytes().to_vec()).unwrap()),
        &flag,
    )
    .unwrap();
    let mut observer = prepared::Observer::default();
    let mut hours = BTreeSet::new();
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(TABLES);
        let frames = replay.read_block_selected(block, TABLES).unwrap();
        for chunk in frames.chunks(2) {
            let mut writer = fixture.writer();
            for frame in chunk {
                let decoded = decoder.advance(frame).unwrap();
                hours.insert(decoded.hour);
                observer.observe(decoded).unwrap();
            }
            observer.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            observer = prepared::Observer::restore(
                series::read_catalogue(&fixture.store, &fixture.key, "wars", 1)
                    .unwrap()
                    .unwrap(),
            );
        }
    }
    for &view in views {
        let expected = scan(replay, 1, view, cat.map(|cat| &**cat), &flag).unwrap();
        let actual = prepared::projection(
            &fixture.store,
            &fixture.key,
            &source,
            &WarsParams {
                branch_id: "1".into(),
                viewpoint_tag: view.map(str::to_owned),
            },
            None,
            &hours,
            cat,
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "view {view:?}"
        );
    }
}

#[test]
fn prepared_wars_preserve_viewpoints_future_capitulations_truncation_gaps_and_native_identity_changes()
 {
    use replay_data::rows::TableRows;
    let mut frames: Vec<_> = (100..=108)
        .filter(|hour| *hour != 104)
        .map(recorded)
        .collect();
    for frame in &mut frames {
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            for row in rows {
                row.first_casualties -= i64::from(frame.hour);
                row.second_casualties = i64::from(frame.hour) * 7;
            }
        }
        frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![rows::CapitulationRow {
                country: 2,
                capitulated: 1,
                capitulated_hour: 102,
                ..Default::default()
            }]))
            .into();
        if frame.hour == 103 {
            let rows = frame
                .complete::<tables::WarOverviewCountries>()
                .unwrap()
                .to_vec();
            frame.tables[TableId::WarOverviewCountries.index()] = TableView::Truncated {
                rows: TableRows::WarOverviewCountries(rows),
                missing: 1,
            }
            .into();
        }
        if frame.hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod-rename".to_vec();
        }
        if frame.hour == 108
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id = 99;
            rows.sort_by_key(|row| row.id);
        }
    }
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames)],
    )
    .unwrap();
    assert_prepared_parity(
        &replay,
        None,
        &[None, Some("ONE"), Some("TWO"), Some("mod-rename")],
    );
}

#[test]
fn prepared_wars_counter_only_hours_publish_one_global_input_change_and_reached_values() {
    use dashboard_base::prepared::series::{self, Reader};
    let frames: Vec<_> = (100..200)
        .map(|hour| {
            let mut frame = recorded(hour);
            if let TableView::Complete(rows::TableRows::WarRelations(rows)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            {
                for row in rows {
                    row.first_casualties -= i64::from(hour);
                }
            }
            // Unrelated hourly country balances must not create naming/topology events.
            if let TableView::Complete(rows::TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                for row in rows {
                    row.political_power_raw = Some(i64::from(hour) * 100000);
                }
            }
            frame
        })
        .collect();
    let fixture = series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut observer = prepared::Observer::default();
    for chunk in frames.chunks(50) {
        let mut writer = fixture.writer();
        for frame in chunk {
            observer.observe(frame).unwrap();
        }
        observer.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        observer = prepared::Observer::restore(
            series::read_catalogue(&fixture.store, &fixture.key, "wars", 1)
                .unwrap()
                .unwrap(),
        );
    }
    let mut reader =
        Reader::new(&fixture.store, &fixture.key, "wars-series-1".into(), &flag).unwrap();
    assert_eq!(reader.event_count_through(0, i32::MAX).unwrap(), 1);
    // The input row is raw bytes, not a JSON array of byte numbers: the
    // changed tables, then the compressed packet, which decodes whole.
    let (hour, row) = reader.event_raw(0, 0).unwrap();
    assert_eq!(hour, 100);
    assert!(serde_json::from_slice::<serde_json::Value>(&row).is_err());
    let (changes, packet) = walks::decode_input(&row).unwrap();
    // Every input but the clock, whose rows no reply reads.
    assert_eq!(changes.len(), TABLES.len() - 1);
    assert!(!changes.contains(&(TableId::Clock as u16)));
    let packet = dashboard_base::prepared::series::decompress(packet).unwrap();
    let mut input = replay_data::codec::Decoder::new(&packet);
    assert_eq!(
        DecodedFrame::decode_checkpoint(&mut input).unwrap().hour,
        100
    );
    input.finish().unwrap();
    assert_eq!(reader.runs(0, 100, 199).unwrap().len(), 1);
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "wars-series-1")
            .unwrap(),
        Some(2)
    );
    assert_prepared_parity(
        &Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(1, frames)],
        )
        .unwrap(),
        None,
        &[None, Some("TWO")],
    );
}

fn set_recorded_hour(frame: &mut DecodedFrame, hour: i32) {
    frame.hour = hour;
    if let TableView::Complete(rows::TableRows::Clock(rows)) =
        &mut *frame.tables[TableId::Clock.index()]
    {
        rows[0].hour = hour;
    }
}

#[test]
fn prepared_wars_native_projection_keeps_prefix_names_memberships_and_rejects_replacement_holders()
{
    use dashboard_base::prepared::series;
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[(
        "localisation/mod_l_english.yml",
        "\u{feff}l_english:\n mod-1:0 \"Original\"\n mod-1_ADJ:0 \"First\"\n mod-2:0 \"Opponent\"\n mod-2_ADJ:0 \"Second\"\n renamed:0 \"Renamed\"\n renamed_ADJ:0 \"Later\"\n NATIVE:0 \"$FIRST$ versus $SECOND$\"\n",
    )]);
    let cat = Arc::new(cat);
    let flag = AtomicBool::new(false);
    for count in [3, 12] {
        let mut frames: Vec<_> = (100..=102)
            .map(|hour| scaled_recorded(hour, count))
            .collect();
        for frame in &mut frames[1..] {
            let TableView::Complete(rows::TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                panic!()
            };
            countries
                .iter_mut()
                .find(|row| row.canonical_slot == 1)
                .unwrap()
                .tag = b"renamed".to_vec();
        }
        let TableView::Complete(rows::TableRows::Countries(countries)) =
            &mut *frames[2].tables[TableId::Countries.index()]
        else {
            panic!()
        };
        // Another native country takes the canonical slot and the same alias.
        let replacement = count + 1;
        countries
            .iter_mut()
            .find(|row| row.canonical_slot == 1)
            .unwrap()
            .id = replacement;
        countries.sort_by_key(|row| row.id);
        let replay = Arc::new(
            Replay::from_records(
                dashboard_base::test_fixtures::CAMPAIGN.into(),
                &[dashboard_base::test_fixtures::records(1, frames.clone())],
            )
            .unwrap(),
        );
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "native-wars".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let fixture = series::tests::Fixture::new();
        let mut observer = prepared::Observer::default();
        for chunk in frames.chunks(2) {
            let mut writer = fixture.writer();
            for frame in chunk {
                observer.observe(frame).unwrap();
            }
            observer.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            observer = prepared::Observer::restore(
                series::read_catalogue(&fixture.store, &fixture.key, "wars", 1)
                    .unwrap()
                    .unwrap(),
            );
        }
        let selected = HashSet::from([(1, 1)]);
        let params = WarsParams {
            branch_id: "1".into(),
            viewpoint_tag: Some("mod-1".into()),
        };
        let mut expected = scan_selected(
            &replay,
            1,
            Some("mod-1"),
            Some(&selected),
            Some(&cat),
            &flag,
        )
        .unwrap();
        for war in &mut expected.wars {
            war.casualties.clear();
        }
        let projection = prepared::projection(
            &fixture.store,
            &fixture.key,
            &source,
            &params,
            Some(&selected),
            &BTreeSet::new(),
            Some(&cat),
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(&projection).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert!(projection.available);
        assert_eq!(
            projection
                .viewpoints
                .iter()
                .map(|run| (&*run.tag, run.first_hour, run.last_hour))
                .collect::<Vec<_>>(),
            [("mod-1", 100, 100), ("renamed", 101, 101)]
        );
        assert_eq!(
            projection.unobserved,
            [HourSpan {
                first_hour: 102,
                last_hour: 102
            }]
        );
        assert!(
            projection
                .warnings
                .iter()
                .any(|warning| warning.contains("selected native war viewpoint holder"))
        );
        assert_eq!(projection.wars.len(), expected.wars.len());
        for (actual, expected) in projection.wars.iter().zip(&expected.wars) {
            let memberships = |war: &WarTimeline| {
                war.relations
                    .iter()
                    .map(|run| (run.key.clone(), run.first_hour, run.last_hour))
                    .collect::<Vec<_>>()
            };
            assert_eq!(memberships(actual), memberships(expected));
        }
        let prefix = &projection.wars[0];
        assert_eq!(prefix.first_hour, 100);
        assert_eq!(prefix.observations[0].name.text, "First versus Second");
        assert!(prefix.observations[0].name.known);
        assert_eq!(prefix.relations.len(), (count - 1) as usize);
        assert!(
            prefix
                .relations
                .iter()
                .all(|run| run.key.first == (1, 1) && run.key.start == 90 && run.first_hour == 100)
        );
        assert!(projection.wars.iter().any(|war| {
            war.observations
                .iter()
                .any(|observation| observation.name.text == "Later versus Second")
        }));
        let reached = prepared::projection(
            &fixture.store,
            &fixture.key,
            &source,
            &params,
            Some(&selected),
            &BTreeSet::from([101]),
            Some(&cat),
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reached.wars[0].id, prefix.id);
        assert_eq!(
            reached.wars[0].observations, prefix.observations,
            "requesting a later hour keeps earlier dated names and memberships"
        );
        assert_eq!(reached.wars[0].relations.len(), prefix.relations.len());
        assert!(
            reached.wars[0].casualties.is_empty(),
            "the prefix remains structural, without fetching unrequested counters"
        );
        assert!(reached.wars.iter().any(|war| !war.casualties.is_empty()));
        let unavailable = prepared::projection(
            &fixture.store,
            &fixture.key,
            &source,
            &params,
            Some(&HashSet::new()),
            &BTreeSet::new(),
            Some(&cat),
            &flag,
        )
        .unwrap()
        .unwrap();
        assert!(
            unavailable.viewpoints.is_empty() && unavailable.wars.is_empty(),
            "an empty native selection never falls back to LocalPlayer"
        );
    }
}

#[test]
fn prepared_wars_bulk_counters_preserve_hourly_changes_and_long_closed_runs_after_resume() {
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = scaled_recorded(hour, 17);
            let TableView::Complete(rows::TableRows::WarRelations(relations)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            else {
                panic!()
            };
            for row in relations {
                // A long closed run, an hourly-changing run, signed values and a
                // constant open counter all contribute to the same exact sums.
                row.first_casualties = if row.second_country == 2 {
                    if hour < 103 { 19 } else { -23 }
                } else {
                    i64::MAX - i64::from(hour)
                };
                row.second_casualties = i64::from(hour);
                row.first_unknown_casualties = -i64::from(hour);
                row.second_unknown_casualties = 11;
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames)],
    )
    .unwrap();
    // The real helper commits twice and reloads the catalogue after each commit.
    assert_prepared_parity(&replay, None, &[None, Some("mod-1"), Some("mod-2")]);
}

#[test]
fn prepared_wars_sparse_casualty_reads_keep_canonical_bounds_and_gaps() {
    use dashboard_base::prepared::series;
    let flag = AtomicBool::new(false);
    let frames: Vec<_> = [100, 101, 102, 103, 105, 106, 107]
        .into_iter()
        .map(|hour| {
            let mut frame = scaled_recorded(hour, 3);
            let TableView::Complete(rows::TableRows::WarRelations(relations)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            else {
                panic!()
            };
            for relation in relations {
                relation.first_casualties = 18 + i64::from(hour >= 105);
                relation.second_casualties = 11;
            }
            frame
        })
        .collect();
    let replay = Arc::new(
        Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(1, frames.clone())],
        )
        .unwrap(),
    );
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "wars-sparse".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay.clone(),
        &flag,
    )
    .unwrap();
    let fixture = series::tests::Fixture::new();
    let mut observer = prepared::Observer::default();
    for chunk in frames.chunks(2) {
        let mut writer = fixture.writer();
        for frame in chunk {
            observer.observe(frame).unwrap();
        }
        observer.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        observer = prepared::Observer::restore(
            series::read_catalogue(&fixture.store, &fixture.key, "wars", 1)
                .unwrap()
                .unwrap(),
        );
    }
    let hours = BTreeSet::from([101, 104, 106]);
    for view in [None, Some("mod-2")] {
        let mut expected = scan(&replay, 1, view, None, &flag).unwrap();
        for war in &mut expected.wars {
            war.casualties
                .retain(|run| hours.range(run.0..=run.1).next().is_some());
        }
        let actual = prepared::projection(
            &fixture.store,
            &fixture.key,
            &source,
            &WarsParams {
                branch_id: "1".into(),
                viewpoint_tag: view.map(str::to_owned),
            },
            None,
            &hours,
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn casualty_only_war_hours_reuse_counter_keys_and_publish_identical_bytes() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        series::tests::{Fixture, assert_payloads_equal},
    };
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut cached = prepared::Observer::default();
    let mut rebuilt = prepared::Observer::default();
    let mut tracker = FrameChangeTracker::new();
    let initial = test_fixtures::recorded(100);
    for hour in 100..106 {
        let mut frame = initial.clone();
        set_recorded_hour(&mut frame, if hour == 105 { 107 } else { hour });
        if let TableView::Complete(rows::TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows[0].first_casualties -= i64::from(hour - 100);
            rows[0].second_unknown_casualties += i64::from(hour - 100);
            if hour >= 103 {
                rows[0].start_hour = 101;
            }
        }
        let changes = tracker.observe(&frame, format);
        let builds = cached.counter_key_builds;
        cached.observe_changed(&frame, &changes).unwrap();
        rebuilt.counter_keys.clear();
        rebuilt.observe_changed(&frame, &changes).unwrap();
        if hour == 101 || hour == 102 || hour == 104 {
            assert_eq!(
                cached.counter_key_builds, builds,
                "casualty counters rebuilt lifetime keys"
            );
        }
        if hour == 103 {
            assert!(
                cached.counter_key_builds > builds,
                "a new lifetime retained old counter keys"
            );
        }
        assert_eq!(
            serde_json::to_vec(&cached).unwrap(),
            serde_json::to_vec(&rebuilt).unwrap()
        );
    }
    let fixtures = [Fixture::new(), Fixture::new()];
    for (state, fixture) in [&mut cached, &mut rebuilt].into_iter().zip(&fixtures) {
        let mut writer = fixture.writer();
        state
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
    }
    assert_payloads_equal(
        &fixtures[0].payloads(),
        &fixtures[1].payloads(),
        "cached vs rebuilt war counter keys",
    );
}

/// Fresh observers of one recording publish one catalogue: relation lifetimes
/// are numbered in recorded WarRelations order, never the joined map's hash order.
#[test]
fn prepared_wars_fresh_observers_number_relation_lifetimes_in_recorded_order() {
    use dashboard_base::prepared::series;
    let count = 80;
    let frames = [scaled_recorded(100, count), scaled_recorded(101, count)];
    let flag = AtomicBool::new(false);
    let published: Vec<_> = (0..2)
        .map(|_| {
            let fixture = series::tests::Fixture::new();
            let mut observer = prepared::Observer::default();
            for frame in &frames {
                observer.observe(frame).unwrap();
            }
            let mut writer = fixture.writer();
            observer.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let (indices, state) = series::read_catalogue::<prepared::Observer>(
                &fixture.store,
                &fixture.key,
                "wars",
                1,
            )
            .unwrap()
            .unwrap();
            (indices, serde_json::to_vec(&state).unwrap())
        })
        .collect();
    assert_eq!(published[0], published[1]);
    let recorded = frames[0].complete::<tables::WarRelations>().unwrap();
    assert_eq!(published[0].0.len(), recorded.len());
    for (position, row) in recorded.iter().enumerate() {
        // Synthetic countries share their slot, raw id and link id.
        let key = WarRelationKey::new(
            (row.first_country, row.first_country),
            (row.second_country, row.second_country),
            row.start_hour,
        );
        assert_eq!(
            published[0].0[&serde_json::to_string(&key).unwrap()],
            position as u64
        );
    }
}

/// A country listed on both native sides has two runs at one hour; they keep
/// native side order through the reply's participant sort.
#[test]
fn participant_runs_sharing_an_hour_and_tag_keep_native_side_order() {
    let tags: Vec<String> = (0..64).map(|index| format!("mod-{index:02}")).collect();
    let data = WarObservation {
        first_hour: 7,
        last_hour: 7,
        viewpoint_tag: tags[0].clone(),
        viewpoint_side: tags.clone(),
        opposing_side: tags.iter().rev().cloned().collect(),
        attackers_are_viewpoint: Some(true),
        representative: None,
        name: naming::unknown("Not recorded: synthetic representative"),
    };
    let replies: Vec<_> = (0..2)
        .map(|_| {
            let mut walk = Walk::new(1, 7);
            walk.reply.wars.push(WarTimeline {
                id: "0".into(),
                first_hour: 7,
                last_hour: 7,
                started_by: true,
                ended_by: true,
                observations: Vec::new(),
                casualties: Vec::new(),
                participants: Vec::new(),
                capitulations: Vec::new(),
                relations: Vec::new(),
                open_participants: HashMap::new(),
                open_relations: HashMap::new(),
            });
            add_observation(&mut walk.reply.wars[0], data.clone(), None);
            let reply = walk.finish(7, None).unwrap();
            reply.wars[0]
                .participants
                .iter()
                .map(|run| (run.tag.clone(), run.side))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(replies[0], replies[1]);
    let expected: Vec<_> = tags
        .iter()
        .flat_map(|tag| {
            [
                (tag.clone(), WarSide::Attacker),
                (tag.clone(), WarSide::Defender),
            ]
        })
        .collect();
    assert_eq!(replies[0], expected);
}

#[test]
fn restored_wars_close_a_relation_that_ends_at_the_first_hour_after_a_commit() {
    use dashboard_base::prepared::series;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    // The second war relation ends at hour 104, the first hour after a commit.
    let frames: Vec<_> = (100..108)
        .map(|hour| {
            let mut frame = recorded(hour);
            if hour >= 104
                && let TableView::Complete(TableRows::WarRelations(rows)) =
                    &mut *frame.tables[TableId::WarRelations.index()]
            {
                rows.truncate(1);
            }
            frame
        })
        .collect();
    let flag = AtomicBool::new(false);
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        replay_data::format::FormatVersion::CURRENT,
        &[&frames[..4], &frames[4..]],
        prepared::Observer::default,
        |observer, frame, changes, _| observer.observe_changed(frame, changes).unwrap(),
        |observer, writer, _| observer.flush(writer, 1, &flag).unwrap(),
        |fixture| {
            prepared::Observer::restore(
                series::read_catalogue(&fixture.store, &fixture.key, "wars", 1)
                    .unwrap()
                    .unwrap(),
            )
        },
    );
}

/// A multi-block branch whose topology changes every hour (war scores move),
/// where the war with THREE ends at 150 and a new one starts at 201, TWO
/// capitulates at 180 and is renamed at 300, casualty runs of seven hours
/// cross every publication, and hours 250-252 are missing. From
/// `counter_only` on, only casualties change.
fn walked_frames(hours: std::ops::Range<i32>, counter_only: i32) -> Vec<DecodedFrame> {
    use replay_data::rows::TableRows;
    hours
        .filter(|hour| !(250..=252).contains(hour))
        .map(|hour| {
            let mut frame = recorded(hour);
            if let TableView::Complete(TableRows::WarOverviewCountries(rows)) =
                &mut *frame.tables[TableId::WarOverviewCountries.index()]
            {
                for row in rows {
                    row.divisions = hour.min(counter_only);
                }
            }
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            {
                for row in rows.iter_mut() {
                    row.second_casualties = i64::from(hour / 7);
                    if row.second_country == 3 && hour > 200 {
                        row.start_hour = 201;
                    }
                }
                if (150..=200).contains(&hour) {
                    rows.retain(|row| row.second_country != 3);
                }
            }
            if (150..=200).contains(&hour)
                && let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
                    &mut *frame.tables[TableId::WarOwnedRelations.index()]
            {
                rows.retain(|row| row.owner != 3 && row.second_country != 3);
            }
            if hour >= 180 {
                frame.tables[TableId::Capitulation.index()] =
                    TableView::Complete(TableRows::Capitulation(vec![rows::CapitulationRow {
                        country: 2,
                        capitulated: 1,
                        capitulated_hour: 180,
                        ..Default::default()
                    }]))
                    .into();
            }
            if hour >= 300
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows[1].tag = b"mod-two".to_vec();
            }
            frame
        })
        .collect()
}

/// Prepare `frames` through the production observer, committing and reloading
/// its saved state every `block` frames, then calling `committed`.
fn prepare_walked(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    frames: &[DecodedFrame],
    block: usize,
    mut committed: impl FnMut(i32),
) {
    let flag = AtomicBool::new(false);
    let mut observer = prepared::Observer::default();
    for chunk in frames.chunks(block) {
        let mut writer = fixture.writer();
        for frame in chunk {
            observer.observe(frame).unwrap();
        }
        observer.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        observer = prepared::Observer::restore(
            dashboard_base::prepared::series::read_catalogue(
                &fixture.store,
                &fixture.key,
                "wars",
                1,
            )
            .unwrap()
            .unwrap(),
        );
        committed(chunk.last().unwrap().hour);
    }
}

fn walked_source(replay: &Arc<Replay>) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "wars-walks".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay.clone(),
        &AtomicBool::new(false),
    )
    .unwrap()
}

type Memberships = Vec<Vec<(WarRelationKey, i32, i32)>>;

fn memberships(reply: &WarsReply) -> Memberships {
    reply
        .wars
        .iter()
        .map(|war| {
            war.relations
                .iter()
                .map(|run| (run.key.clone(), run.first_hour, run.last_hour))
                .collect()
        })
        .collect()
}

/// The reply and its Rust-only relation memberships.
fn walked(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    source: &Source,
    view: Option<&str>,
    selected: Option<&HashSet<(u32, u32)>>,
    hours: &BTreeSet<i32>,
    cat: Option<&Arc<Catalogue>>,
) -> (serde_json::Value, Memberships) {
    let reply = prepared::projection(
        &fixture.store,
        &fixture.key,
        source,
        &WarsParams {
            branch_id: "1".into(),
            viewpoint_tag: view.map(str::to_owned),
        },
        selected,
        hours,
        cat,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let memberships = memberships(&reply);
    (serde_json::to_value(reply).unwrap(), memberships)
}

fn decoded_inputs() -> usize {
    walks::DECODED.with(std::cell::Cell::get)
}

/// Kept walks answer every requested hour set exactly as the walk from the
/// branch start, and a later query decodes at most one resume interval of
/// change inputs however many blocks precede it.
#[test]
fn prepared_wars_resume_points_match_the_whole_walk_and_bound_decoded_inputs() {
    let selected = HashSet::from([(1, 1)]);
    for end in [300, 420] {
        let frames = walked_frames(100..end, end);
        let replay = Arc::new(
            Replay::from_records(
                dashboard_base::test_fixtures::CAMPAIGN.into(),
                &[dashboard_base::test_fixtures::records(1, frames.clone())],
            )
            .unwrap(),
        );
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        prepare_walked(&fixture, &frames, 16, |_| {});
        let last = end - 1;
        let all: BTreeSet<i32> = frames.iter().map(|frame| frame.hour).collect();
        let sets = [
            BTreeSet::new(),
            BTreeSet::from([100]),
            BTreeSet::from([230]),
            BTreeSet::from([last]),
            BTreeSet::from([248, 249, 253, 254]),
            BTreeSet::from([120, 175, 230, last]),
            all.clone(),
        ];
        type View<'a> = (Option<&'a str>, Option<&'a HashSet<(u32, u32)>>);
        let views: [View; 3] = [
            (None, None),
            (Some("ONE"), None),
            (Some("ONE"), Some(&selected)),
        ];
        for (view, selection) in views {
            let kept = walked_source(&replay);
            // A structural query walks every input once and keeps the walk.
            let before = decoded_inputs();
            walked(&fixture, &kept, view, selection, &BTreeSet::new(), None);
            let inputs = (decoded_inputs() - before) as u64;
            let held = walks::held(&kept);
            assert_eq!(held.len(), 1);
            assert_eq!(held[0].inputs, inputs);
            assert!(inputs > 190, "one change input per recorded hour: {inputs}");
            assert!(held[0].points <= walks::WALK_POINTS, "{:?}", held[0]);
            assert!(
                held[0].point_bytes <= walks::WALK_POINT_BYTES,
                "{:?}",
                held[0]
            );
            assert!(
                held[0].interval > 1 && held[0].interval * 32 <= held[0].inputs,
                "{:?}",
                held[0]
            );
            let interval = held[0].interval as usize;
            for hours in &sets {
                let before = decoded_inputs();
                let warm = walked(&fixture, &kept, view, selection, hours, None);
                let decoded = decoded_inputs() - before;
                let cold = walked(
                    &fixture,
                    &walked_source(&replay),
                    view,
                    selection,
                    hours,
                    None,
                );
                assert_eq!(warm, cold, "{view:?} {selection:?} {hours:?}");
                if hours.is_empty() || hours.len() == 1 && hours.contains(&last) {
                    assert_eq!(decoded, 0, "the kept walk already holds the last input");
                } else if hours.len() == 1 {
                    assert!(decoded < interval, "{hours:?}: {decoded} of {interval}");
                } else if hours.len() <= 4 {
                    assert!(
                        decoded <= 4 * interval,
                        "{hours:?}: {decoded} of {interval}"
                    );
                }
            }
            // The whole walk from the branch start is the oracle.
            let (expected, hours) = match selection {
                None => (
                    scan(&replay, 1, view, None, &AtomicBool::new(false)).unwrap(),
                    all.clone(),
                ),
                Some(selection) => {
                    let mut reply = scan_selected(
                        &replay,
                        1,
                        view,
                        Some(selection),
                        None,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                    for war in &mut reply.wars {
                        war.casualties.clear();
                    }
                    (reply, BTreeSet::new())
                }
            };
            let (reply, relations) = walked(&fixture, &kept, view, selection, &hours, None);
            assert_eq!(reply, serde_json::to_value(&expected).unwrap(), "{view:?}");
            assert_eq!(relations, memberships(&expected), "{view:?}");
        }
    }
}

/// A walk kept across published appends, including counter-only ones that
/// only extend the last input's coverage, answers as a walk made fresh from
/// the reloaded store at every commit.
#[test]
fn prepared_wars_kept_walks_continue_across_appends_as_fresh_ones() {
    let frames = walked_frames(100..330, 290);
    let replay = Arc::new(
        Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(1, frames.clone())],
        )
        .unwrap(),
    );
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let kept = walked_source(&replay);
    let selected = HashSet::from([(1, 1)]);
    let mut commits = 0;
    prepare_walked(&fixture, &frames, 23, |hour| {
        commits += 1;
        for hours in [
            BTreeSet::new(),
            BTreeSet::from([hour]),
            BTreeSet::from([hour - 30, hour - 1, hour]),
        ] {
            for (view, selection) in [(None, None), (Some("ONE"), Some(&selected))] {
                let continued = walked(&fixture, &kept, view, selection, &hours, None);
                let fresh = walked(
                    &fixture,
                    &walked_source(&replay),
                    view,
                    selection,
                    &hours,
                    None,
                );
                assert_eq!(continued, fresh, "after {hour}: {view:?} {hours:?}");
            }
        }
    });
    assert!(commits >= 10);
    // After the rename at 300, counter-only commits publish no input but
    // extend the last one's coverage.
    let unread = AtomicBool::new(false);
    let mut reader = dashboard_base::prepared::series::Reader::new(
        &fixture.store,
        &fixture.key,
        "wars-series-1".into(),
        &unread,
    )
    .unwrap();
    let inputs = reader.event_count_through(0, i32::MAX).unwrap();
    assert_eq!(reader.event_date(0, inputs - 1).unwrap(), 300);
    assert_eq!(reader.event_date(0, inputs - 2).unwrap(), 290);
    let all = frames.iter().map(|frame| frame.hour).collect();
    assert_eq!(
        walked(&fixture, &kept, None, None, &all, None).0,
        serde_json::to_value(scan(&replay, 1, None, None, &AtomicBool::new(false)).unwrap())
            .unwrap()
    );
}

/// The source keeps a few walks, most recently used first, each within its
/// resume point bounds; a different content catalogue walks again.
#[test]
fn prepared_wars_keep_bounded_walks_per_viewpoint_and_catalogue() {
    let frames = walked_frames(100..300, 300);
    let replay = Arc::new(
        Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(1, frames.clone())],
        )
        .unwrap(),
    );
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    prepare_walked(&fixture, &frames, 16, |_| {});
    let source = walked_source(&replay);
    let structure = BTreeSet::new();
    let selected = HashSet::from([(1, 1)]);
    let before = decoded_inputs();
    walked(&fixture, &source, None, None, &structure, None);
    let inputs = decoded_inputs() - before;
    for (view, selection) in [
        (Some("ONE"), None),
        (Some("TWO"), None),
        (Some("THREE"), None),
        (Some("ONE"), Some(&selected)),
    ] {
        walked(&fixture, &source, view, selection, &structure, None);
    }
    let held = walks::held(&source);
    assert_eq!(held.len(), walks::CACHED_WALKS);
    for walk in &held {
        assert_eq!(walk.inputs as usize, inputs);
        assert!(walk.points <= walks::WALK_POINTS, "{walk:?}");
        assert!(walk.point_bytes <= walks::WALK_POINT_BYTES, "{walk:?}");
        assert!(walk.tail_bytes > 0, "{walk:?}");
    }
    // The least recently used walk (LocalPlayer) was dropped and walks again;
    // the most recent one answers without decoding.
    let before = decoded_inputs();
    walked(
        &fixture,
        &source,
        Some("ONE"),
        Some(&selected),
        &structure,
        None,
    );
    assert_eq!(decoded_inputs() - before, 0);
    walked(&fixture, &source, None, None, &structure, None);
    assert_eq!(decoded_inputs() - before, inputs);
    assert_eq!(walks::held(&source).len(), walks::CACHED_WALKS);

    // Walks are keyed by the catalogue they name, held by identity.
    let (first, _first_folder) = catalogue::tests::folder_catalogue(&[]);
    let (second, _second_folder) = catalogue::tests::folder_catalogue(&[]);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let named = walked_source(&replay);
    let before = decoded_inputs();
    let a = walked(&fixture, &named, None, None, &structure, Some(&first));
    assert_eq!(decoded_inputs() - before, inputs);
    let again = walked(&fixture, &named, None, None, &structure, Some(&first));
    assert_eq!(decoded_inputs() - before, inputs);
    let b = walked(&fixture, &named, None, None, &structure, Some(&second));
    assert_eq!(decoded_inputs() - before, 2 * inputs);
    assert_eq!(a, again);
    assert_eq!(a, b);
    assert_eq!(walks::held(&named).len(), 2);

    // A smaller point budget doubles the interval until the points fit.
    let mid = BTreeSet::from([230]);
    let plain = walked_source(&replay);
    let expected = walked(&fixture, &plain, None, None, &mid, None);
    let reference = walks::held(&plain).remove(0);
    let budget = reference.point_bytes / 4;
    walks::POINT_BYTES.with(|bytes| bytes.set(Some(budget)));
    let thin = walked_source(&replay);
    walked(&fixture, &thin, None, None, &structure, None);
    let thinned = walks::held(&thin);
    walks::POINT_BYTES.with(|bytes| bytes.set(None));
    assert!(
        thinned[0].point_bytes <= budget || thinned[0].points == 1,
        "{:?}",
        thinned[0]
    );
    assert!(thinned[0].interval > reference.interval, "{:?}", thinned[0]);
    let before = decoded_inputs();
    assert_eq!(walked(&fixture, &thin, None, None, &mid, None), expected);
    assert!(decoded_inputs() - before < thinned[0].interval as usize);
}

#[test]
fn resumable_walks_keep_each_open_wars_continuation_and_not_its_history() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    // Losses move every hour, so every hour adds a loss run to each war.
    let losses = |hour: i32| {
        let mut frame = recorded(hour);
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            for row in rows {
                row.second_casualties = i64::from(hour);
            }
        }
        frame
    };
    let mut walk = Walk::new(1, 100);
    for hour in 100..140 {
        walk.observe(&losses(hour), None, None, None);
    }
    let ids = |walk: &Walk| walk.open.iter().map(|war| war.id).collect::<Vec<_>>();
    let open = ids(&walk);
    assert!(!open.is_empty(), "the fixture's wars are open");
    let mut copy = walk.resumable();
    for &id in &open {
        assert!(
            walk.reply.wars[id].casualties.len() >= 40,
            "war {id}: a loss run every hour"
        );
        let kept = &copy.reply.wars[id];
        assert_eq!(kept.casualties.len(), 1, "war {id}: its last loss run");
        assert_eq!(kept.observations.len(), 1, "war {id}: its last observation");
        assert_eq!(kept.participants.len(), kept.open_participants.len());
        assert_eq!(kept.relations.len(), kept.open_relations.len());
    }
    // Continued alike, the copy holds what the walk holds of each open war.
    for hour in 140..150 {
        let frame = losses(hour);
        walk.observe(&frame, None, None, None);
        copy.observe(&frame, None, None, None);
    }
    assert_eq!(ids(&copy), ids(&walk));
    fn json(value: impl Serialize) -> serde_json::Value {
        serde_json::to_value(value).unwrap()
    }
    for OpenWar { id, .. } in &walk.open {
        let (full, kept) = (&walk.reply.wars[*id], &copy.reply.wars[*id]);
        assert_eq!(json(full.casualties.last()), json(kept.casualties.last()));
        assert_eq!(
            json(full.observations.last()),
            json(kept.observations.last())
        );
        assert_eq!(
            (full.first_hour, full.last_hour),
            (kept.first_hour, kept.last_hour)
        );
        for (key, &index) in &full.open_participants {
            assert_eq!(
                json(&full.participants[index]),
                json(&kept.participants[kept.open_participants[key]])
            );
        }
        for (key, &index) in &full.open_relations {
            let (a, b) = (
                &full.relations[index],
                &kept.relations[kept.open_relations[key]],
            );
            assert_eq!(
                (&a.key, a.first_hour, a.last_hour),
                (&b.key, b.first_hour, b.last_hour)
            );
        }
    }
}
