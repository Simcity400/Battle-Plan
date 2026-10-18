use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn air_superiority_inherited_state_is_independent_between_revisions() {
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 100, Some(1))])],
        )
        .unwrap(),
    );
    let make = |revision: &str| {
        crate::Source::from_replay(
            crate::SourceKey {
                source_id: revision.into(),
                revision: revision.into(),
            },
            "memory.battleplan".into(),
            Arc::clone(&replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let previous = make("1");
    previous.append_state::<Vec<i32>>().lock().unwrap().push(23);
    let next = make("2");
    next.inherit_append_states(&previous);
    next.state::<Vec<i32>>().lock().unwrap().push(47);
    assert_eq!(*previous.append_state::<Vec<i32>>().lock().unwrap(), [23]);
    assert_eq!(*next.state::<Vec<i32>>().lock().unwrap(), [23, 47]);

    previous
        .state::<Vec<String>>()
        .lock()
        .unwrap()
        .push("revision local".into());
    let other = make("3");
    other.inherit_append_states(&previous);
    assert!(other.state::<Vec<String>>().lock().unwrap().is_empty());
}

#[test]
fn the_catalogue_names_countries_when_content_is_known_and_shows_tags_otherwise() {
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 9001,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 9001,
            ..Default::default()
        }]));
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "source-1".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let localisation: installed_content_localisation::localisation::Localisation =
        [("GER".to_owned(), "German Reich".to_owned())]
            .into_iter()
            .collect();
    let named = source.named_metadata(Ok(Arc::new(localisation)));
    assert_eq!(named.countries[0].name, "German Reich");
    source.name_countries(None);
    let tagged = source.metadata();
    assert_eq!(tagged.countries[0].name, "GER");
    assert!(
        tagged
            .warnings
            .iter()
            .any(|w| w.contains("until the game folders"))
    );
}
