use super::*;
use crate::products::unit_leaders::prepared::test_fixtures::{frame, set};
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use dashboard_core::test_fixtures::{CAMPAIGN, records};
use replay::rows::{CountryCounterInputRow, Recorded, ScriptVariableEntry, ScriptVariablesRow};

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
    let reduced = reduction(false);
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
