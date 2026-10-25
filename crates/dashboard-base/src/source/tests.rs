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
    let named = source.named_metadata(Ok(Arc::new(localisation))).unwrap();
    assert_eq!(named.countries[0].name, "German Reich");
    source.name_countries(None).unwrap();
    let tagged = source.metadata().unwrap();
    assert_eq!(tagged.countries[0].name, "GER");
    assert!(
        tagged
            .warnings
            .iter()
            .any(|w| w.contains("until the game folders"))
    );
}

#[test]
fn warning_allocation_exhaustion_never_skips_localisation_or_saved_names() {
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
    let mut source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "naming-exhaustion".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    source.metadata.value.warnings = vec![
        "first retained diagnostic".into(),
        "second retained diagnostic".into(),
    ];
    let localisation: installed_content_localisation::localisation::Localisation =
        [("GER".to_owned(), "Synthetic localised name".to_owned())]
            .into_iter()
            .collect();
    for accepted in [0, 1] {
        let named = crate::metadata::with_storage_failure_after(accepted, || {
            source
                .named_metadata(Ok(Arc::new(localisation.clone())))
                .unwrap()
        });
        assert_eq!(named.countries[0].name, "Synthetic localised name");
        assert_eq!(named.warnings.len(), accepted);
        assert_eq!(named.refusals.warnings.count(), 2 - accepted);
        assert_eq!(named.warnings, source.metadata.warnings[..accepted]);
        let saved = crate::metadata::with_storage_failure_after(accepted, || {
            source
                .named_metadata_from(
                    Err("synthetic localisation unavailable".into()),
                    Some(&named),
                )
                .unwrap()
        });
        assert_eq!(saved.countries[0].name, "Synthetic localised name");
        assert_eq!(saved.refusals.warnings.count(), 3 - accepted);
        assert_eq!(saved.warnings, source.metadata.warnings[..accepted]);
    }
    assert_eq!(
        source.metadata.warnings,
        ["first retained diagnostic", "second retained diagnostic"]
    );
}

#[test]
fn failed_diagnostic_initialization_never_caches_an_incomplete_naming_result() {
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 100, Some(1))])],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "naming-failure".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let before = source.metadata().unwrap();
    let error =
        crate::metadata::with_warning_initialization_failure(|| source.name_countries(None))
            .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Country metadata naming failed:")
    );
    assert!(source.named.lock().unwrap().is_none());
    assert_eq!(source.metadata().unwrap(), before);
    source.name_countries(None).unwrap();
    assert!(source.named.lock().unwrap().is_some());
}
