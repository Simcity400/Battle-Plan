use super::*;
use crate::products::unit_leaders::prepared::test_fixtures::{frame, set};
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use dashboard_core::test_fixtures::{CAMPAIGN, records};
use replay_data::{
    rows::CountryCounterInputRow, rows::Recorded, rows::ScriptVariableEntry,
    rows::ScriptVariablesRow,
};

#[test]
fn bound_content_keeps_variables_and_only_the_tag_resolvers_counter_field() {
    let content = selected(true);
    for table in [TableId::ScriptVariables, TableId::CountryCounterInputs] {
        assert!(content.tables.contains(&table), "{table:?}");
        assert!(!content.unread.has(table), "{table:?}");
    }
    assert!(!content.tables.contains(&TableId::DynamicModifiers));
    assert!(content.unread.has(TableId::DynamicModifiers));
    assert!(matches!(
        policy(TableId::DynamicModifiers),
        (Watch::Keys, Keep::Dropped)
    ));
    assert_eq!(
        content
            .watch
            .iter()
            .find(|watch| watch.table == TableId::ScriptVariables)
            .unwrap()
            .fields,
        TableFields::named(TableId::ScriptVariables, &["entries", "seed"])
            .unwrap()
            .fields
    );
    assert!(!selected(false).tables.contains(&TableId::ScriptVariables));
    assert_eq!(
        content
            .watch
            .iter()
            .find(|watch| watch.table == TableId::CountryCounterInputs)
            .unwrap()
            .fields,
        TableFields::named(TableId::CountryCounterInputs, &["owned_state_count"])
            .unwrap()
            .fields
    );

    let frames: Vec<_> = (100..=102)
        .map(|hour| {
            let mut frame = frame(hour, 1);
            set(
                &mut frame,
                TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                    country: 100_000,
                    owned_state_count: 1,
                    pride_of_the_fleet: hour as u64,
                    ..Default::default()
                }]),
            );
            set(
                &mut frame,
                TableRows::ScriptVariables(vec![ScriptVariablesRow {
                    scope_kind: 1,
                    scope_id: 100_000,
                    seed: Recorded::Value(Some(hour as u32)),
                    entries: vec![ScriptVariableEntry {
                        name: b"leader_gate".to_vec(),
                        value_raw: i64::from(hour) * 100_000,
                    }],
                }]),
            );
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut extract = start(replay.format, content);
    let mut tracker = FrameChangeTracker::new();
    replay
        .read_block_alone(
            &replay.index[0],
            &content.tables,
            |_, frame| extract.observe(&replay, frame, &tracker.observe(frame, replay.format)),
            || Ok(()),
        )
        .unwrap();
    let facts = extract.finish().unwrap().downcast::<BlockFacts>().unwrap();
    assert!(!facts.reduction.has(TableId::ScriptVariables));
    assert_eq!(facts.hours.len(), frames.len());
    for (position, hour) in facts.hours.iter().enumerate() {
        let kept = match hour {
            Hour::Whole { frame, .. } if position == 0 => frame,
            Hour::Kept { frame, .. } if position != 0 => frame,
            _ => panic!("changed variable hour was not kept"),
        };
        assert_eq!(
            kept.complete::<tables::ScriptVariables>().unwrap(),
            frames[position]
                .complete::<tables::ScriptVariables>()
                .unwrap()
        );
        if position != 0 {
            assert_eq!(
                kept.complete::<tables::CountryCounterInputs>().unwrap(),
                &[CountryCounterInputRow {
                    country: 100_000,
                    owned_state_count: 1,
                    ..Default::default()
                }]
            );
        }
    }
}

#[test]
fn tag_resolver_declarations_fit_the_bound_watch_and_kept_frame() {
    use dashboard_text::{
        scope::Query,
        triggers::inputs::{Key, Since},
    };
    let selection = selected(true);
    let reduced = reduction();
    for key in [
        Key::Scopes(Query::AliasEntry("AAA".into())),
        Key::Scopes(Query::ResolveTag("AAA".into())),
    ] {
        for read in key.dependence().expect("declared tag resolver") {
            assert!(
                selection.tables.contains(&read.table),
                "{key:?}: {:?}",
                read.table
            );
            let watched = selection
                .watch
                .iter()
                .find(|watch| watch.table == read.table)
                .unwrap();
            assert_eq!(
                read.fields & !watched.fields,
                0,
                "{key:?}: {:?}",
                read.table
            );
            assert!(
                !Since::span(1, 100, 100, None, &reduced).changed(&read),
                "{key:?}: {read:?}"
            );
        }
    }
}

#[test]
fn overlord_visibility_folds_fast_and_matches_whole_frame_payloads() {
    use dashboard_base::prepared::{prepass::noted, series::tests::Fixture};
    use dashboard_text::triggers::{Evaluator, inputs};
    use replay_data::rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LexicalTokenRow, ReservedCountryNamingRow,
    };
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/test.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
        (
            "common/characters/test.txt",
            "characters={mod_commander={corps_commander={visible={OVERLORD={tag=BBB}}}}}",
        ),
    ]);
    let templates =
        installed_content_core::characters::leader_templates(&catalogue.content.layers).unwrap();
    let template = &templates[0];
    // AAA is BBB's subject but at 102 and 103; its appearance's colour, which
    // no read reads, moves every hour.
    let subject = |hour: i32| !matches!(hour, 102 | 103);
    let frames: Vec<_> = (100..=107)
        .map(|hour| {
            let mut frame = frame(hour, 1);
            for rows in [
                TableRows::Countries(
                    [(100_000, 1, "AAA"), (100_001, 2, "BBB")]
                        .into_iter()
                        .map(|(id, canonical_slot, tag)| CountryRow {
                            id,
                            canonical_slot,
                            tag: tag.as_bytes().to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ),
                TableRows::CountryLinks(
                    [(100_000, 1), (100_001, 2)]
                        .into_iter()
                        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                        .collect(),
                ),
                TableRows::CountryAppearance(vec![
                    CountryAppearanceRow {
                        country: 100_000,
                        overlord: if subject(hour) { 100_001 } else { 0 },
                        red: hour as u32,
                        original_tag_reference: Recorded::Value(Some(100_000)),
                        ..Default::default()
                    },
                    CountryAppearanceRow {
                        country: 100_001,
                        original_tag_reference: Recorded::Value(Some(100_001)),
                        ..Default::default()
                    },
                ]),
                TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                    original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![], vec![]])),
                    ..Default::default()
                }]),
                TableRows::ScriptVariables(
                    [100_000, 100_001]
                        .map(|scope_id| ScriptVariablesRow {
                            scope_kind: 1,
                            scope_id,
                            seed: Recorded::Value(Some(7)),
                            ..Default::default()
                        })
                        .into(),
                ),
                TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 901,
                    literal: template.tag.as_bytes().to_vec(),
                }]),
                TableRows::empty(TableId::Provinces),
                TableRows::empty(TableId::States),
            ] {
                set(&mut frame, rows);
            }
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *frame.tables[TableId::UnitLeaders.index()]
            {
                rows[0].visibility_choice = 1;
            }
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *frame.tables[TableId::Characters.index()]
            {
                rows[0].template_present = 1;
                rows[0].template_tag = 901;
                rows[0].template_file = template.path.as_bytes().to_vec();
                rows[0].template_line = template.line.unwrap() as i32;
            }
            frame
        })
        .collect();
    let catalogue = Arc::new(catalogue);
    let content = Content {
        politics: Arc::new(
            dashboard_base::products::politics_content::PoliticsContent::load(catalogue.clone())
                .unwrap(),
        ),
        leaders: installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)
            .unwrap(),
        catalogue,
    };
    let scripted = content.catalogue.scripted_content().unwrap();
    for frame in &frames {
        let world = scripted.recorded_world(frame).unwrap();
        let row = &frame.complete::<tables::UnitLeaders>().unwrap()[0];
        let (value, reads) = inputs::capture(frame, || {
            Evaluator::new(&world, None).decide_leader_visibility(
                row,
                world.decoded_leader_visibility(row),
                true,
            )
        });
        let decided = value.decided();
        assert_eq!(
            decided == Ok(true),
            subject(frame.hour),
            "hour {}: {decided:?}",
            frame.hour
        );
        assert!(
            reads
                .keys()
                .any(|key| format!("{key:?}").contains("Keyword(Overlord)")),
            "{reads:?}"
        );
        assert!(
            reads.keys().all(|key| key.dependence().is_some()),
            "{reads:?}"
        );
    }

    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let selection = bound(true);
    let mut extract = (selection.start)(replay.format);
    let mut tracker = FrameChangeTracker::new();
    replay
        .read_block_alone(
            &replay.index[0],
            &selection.tables,
            |_, frame| extract.observe(&replay, frame, &tracker.observe(frame, replay.format)),
            || Ok(()),
        )
        .unwrap();
    let facts = extract.finish().unwrap().downcast::<BlockFacts>().unwrap();
    // The colour alone leaves hours quiet; the overlord's changes are kept.
    assert!(
        facts
            .hours
            .iter()
            .any(|hour| matches!(hour, Hour::Quiet { .. }))
    );
    assert!(
        facts
            .hours
            .iter()
            .any(|hour| matches!(hour, Hour::Kept { .. }))
    );
    let mut folded = Prepared::new(&replay, 1);
    let rejected = noted("leaders.reject.undeclared-input");
    let (mode, reached) = inputs::reusing(|| {
        fold(
            &mut folded,
            facts,
            &replay,
            &replay.index[0],
            Some(&content),
            &mut |_, result| result.map(|()| true),
            &mut || Ok(()),
        )
        .unwrap()
    });
    assert_eq!(mode, Folded::Fast);
    assert_eq!(reached, Some(107));
    assert_eq!(noted("leaders.reject.undeclared-input"), rejected);

    let mut hourly = Prepared::new(&replay, 1);
    observe_block(
        &mut hourly,
        &replay,
        &replay.index[0],
        None,
        Some(&content),
        &mut |_, result| result.map(|()| true),
        &mut || Ok(()),
    )
    .unwrap();
    let publish = |prepared: &mut Prepared| {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        prepared
            .flush(&mut writer, 1, Some(&content), &[], &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        fixture.payloads()
    };
    assert_eq!(publish(&mut folded), publish(&mut hourly));
}

#[test]
fn original_bucket_visibility_fast_fold_matches_whole_frame_payloads() {
    use dashboard_base::prepared::{prepass::noted, series::tests::Fixture};
    use dashboard_text::{
        scope::Query,
        triggers::{Evaluator, inputs},
    };
    use replay_data::rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LexicalTokenRow, ReservedCountryNamingRow,
    };
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/test.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
        (
            "common/characters/test.txt",
            "characters={mod_commander={corps_commander={visible={any_country_with_original_tag={original_tag_to_check=THIS count=2 always=yes}}}}}",
        ),
    ]);
    let templates =
        installed_content_core::characters::leader_templates(&catalogue.content.layers).unwrap();
    let template = &templates[0];
    let frames: Vec<_> = (100..=107)
        .map(|hour| {
            let mut frame = frame(hour, 1);
            for rows in [
                TableRows::Countries(
                    [(100_000, 1, "AAA"), (100_001, 2, "BBB")]
                        .into_iter()
                        .map(|(id, canonical_slot, tag)| CountryRow {
                            id,
                            canonical_slot,
                            tag: tag.as_bytes().to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ),
                TableRows::CountryLinks(
                    [(100_000, 1), (100_001, 2)]
                        .into_iter()
                        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                        .collect(),
                ),
                TableRows::CountryAppearance(vec![CountryAppearanceRow {
                    country: 100_000,
                    original_tag_reference: Recorded::Value(Some(100_000)),
                    ..Default::default()
                }]),
                TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                    original_tag_buckets: Recorded::Value(match hour {
                        102 | 103 => Some(vec![vec![], vec![], vec![]]),
                        104 | 105 => None,
                        _ => Some(vec![vec![], vec![100_001], vec![]]),
                    }),
                    ..Default::default()
                }]),
                TableRows::ScriptVariables(
                    [100_000, 100_001]
                        .map(|scope_id| ScriptVariablesRow {
                            scope_kind: 1,
                            scope_id,
                            seed: Recorded::Value(Some(7)),
                            ..Default::default()
                        })
                        .into(),
                ),
                TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 901,
                    literal: template.tag.as_bytes().to_vec(),
                }]),
                TableRows::empty(TableId::Provinces),
                TableRows::empty(TableId::States),
            ] {
                set(&mut frame, rows);
            }
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *frame.tables[TableId::UnitLeaders.index()]
            {
                rows[0].visibility_choice = 1;
            }
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *frame.tables[TableId::Characters.index()]
            {
                rows[0].template_present = 1;
                rows[0].template_tag = 901;
                rows[0].template_file = template.path.as_bytes().to_vec();
                rows[0].template_line = template.line.unwrap() as i32;
            }
            frame
        })
        .collect();
    let catalogue = Arc::new(catalogue);
    let content = Content {
        politics: Arc::new(
            dashboard_base::products::politics_content::PoliticsContent::load(catalogue.clone())
                .unwrap(),
        ),
        leaders: installed_content_core::unit_leader::Leaders::load(&catalogue.content.layers, &|_| None)
            .unwrap(),
        catalogue,
    };
    let scripted = content.catalogue.scripted_content().unwrap();
    for frame in &frames {
        let world = scripted.recorded_world(frame).unwrap();
        let row = &frame.complete::<tables::UnitLeaders>().unwrap()[0];
        let (value, reads) = inputs::capture(frame, || {
            Evaluator::new(&world, None).decide_leader_visibility(
                row,
                world.decoded_leader_visibility(row),
                true,
            )
        });
        let expected = match frame.hour {
            102 | 103 => Some(false),
            104 | 105 => None,
            _ => Some(true),
        };
        let decided = value.decided();
        assert_eq!(
            decided.as_ref().ok().copied(),
            expected,
            "hour {}: {decided:?}",
            frame.hour
        );
        if let Err(why) = decided {
            assert!(why.contains("original_tag_buckets"), "{why}");
        }
        assert!(
            reads
                .keys()
                .any(|key| *key == inputs::Key::Scopes(Query::OriginalBucket(100_000)))
        );
        assert!(
            reads.keys().all(|key| key.dependence().is_some()),
            "{reads:?}"
        );
    }

    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let selection = bound(true);
    let mut extract = (selection.start)(replay.format);
    let mut tracker = FrameChangeTracker::new();
    replay
        .read_block_alone(
            &replay.index[0],
            &selection.tables,
            |_, frame| extract.observe(&replay, frame, &tracker.observe(frame, replay.format)),
            || Ok(()),
        )
        .unwrap();
    let facts = extract.finish().unwrap().downcast::<BlockFacts>().unwrap();
    assert!(
        facts
            .hours
            .iter()
            .any(|hour| matches!(hour, Hour::Quiet { .. }))
    );
    assert!(
        facts
            .hours
            .iter()
            .any(|hour| matches!(hour, Hour::Kept { .. }))
    );
    let mut folded = Prepared::new(&replay, 1);
    let rejected = noted("leaders.reject.undeclared-input");
    let (mode, reached) = inputs::reusing(|| {
        fold(
            &mut folded,
            facts,
            &replay,
            &replay.index[0],
            Some(&content),
            &mut |_, result| result.map(|()| true),
            &mut || Ok(()),
        )
        .unwrap()
    });
    assert_eq!(mode, Folded::Fast);
    assert_eq!(reached, Some(107));
    assert_eq!(noted("leaders.reject.undeclared-input"), rejected);

    let mut hourly = Prepared::new(&replay, 1);
    observe_block(
        &mut hourly,
        &replay,
        &replay.index[0],
        None,
        Some(&content),
        &mut |_, result| result.map(|()| true),
        &mut || Ok(()),
    )
    .unwrap();
    let publish = |prepared: &mut Prepared| {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        prepared
            .flush(&mut writer, 1, Some(&content), &[], &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        fixture.payloads()
    };
    assert_eq!(publish(&mut folded), publish(&mut hourly));
}
