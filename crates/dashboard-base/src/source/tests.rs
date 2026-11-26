use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn air_superiority_inherited_state_is_independent_between_revisions() {
    let replay = Arc::new(
        Replay::from_records(
            recorder_derive::writers(),
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
fn a_reply_reports_its_own_folders_read_set_whatever_is_bound_meanwhile() {
    crate::test_fixtures::game::enter();
    let root =
        std::env::temp_dir().join(format!("hoi4-source-own-content-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (source, context) =
        crate::test_fixtures::source_with_content(vec![source(1, 100, Some(1))], &root);
    let old = context.content.unwrap();
    let mut new = old.clone();
    new.options.cache_dir = root.join("other-cache");
    std::fs::create_dir_all(root.join("gfx/custom")).unwrap();
    std::fs::write(root.join("gfx/custom/icon.dds"), "synthetic").unwrap();
    let uncovered = |lines: &[String]| {
        lines
            .iter()
            .any(|line| line.contains("gfx/custom/icon.dds"))
    };
    let held_old = source.content(&old.options).unwrap();
    let held_new = source.content(&new.options).unwrap();
    let (_, mut own) = content_reader_layers::content::diagnostics::capture(|| {
        let _ = held_new.layers.read_uncovered_path(b"gfx/custom/icon.dds");
    });
    let mut old_lines = Vec::new();
    content_reader_layers::content::diagnostics::current().append_to(&mut old_lines);
    source.append_changed_warnings(&mut old_lines, Some(&old));
    assert!(!uncovered(&old_lines));
    assert!(!content_reader_layers::content::diagnostics::current().has_uncovered());
    assert!(Arc::ptr_eq(&source.bound_content(&new.options).unwrap(), &held_new));
    assert!(Arc::ptr_eq(&source.content(&old.options).unwrap(), &held_old));
    assert!(Arc::ptr_eq(&source.content(&new.options).unwrap(), &held_new));
    source.content(&old.options).unwrap();
    content_reader_layers::content::diagnostics::scope(&mut own, false, || {
        let mut new_lines = Vec::new();
        content_reader_layers::content::diagnostics::current().append_to(&mut new_lines);
        source.append_changed_warnings(&mut new_lines, Some(&new));
    assert!(uncovered(&new_lines));
    assert!(content_reader_layers::content::diagnostics::current().has_uncovered());
    });
    drop((held_old, held_new));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_recordings_own_presentation_is_mounted_once_per_content() {
    crate::test_fixtures::game::enter();
    let root =
        std::env::temp_dir().join(format!("hoi4-source-recorded-mount-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (source, context) =
        crate::test_fixtures::source_with_content(vec![source(1, 100, Some(1))], &root);
    let old = context.content.unwrap();
    let mut new = old.clone();
    new.options.cache_dir = root.join("other-cache");
    let first = source.recorded_presented(&old.options).unwrap();
    assert!(Arc::ptr_eq(&first, &source.recorded_presented(&old.options).unwrap()));
    let other = source.recorded_presented(&new.options).unwrap();
    assert!(!Arc::ptr_eq(&first.content, &other.content));
    assert!(Arc::ptr_eq(&other, &source.recorded_presented(&new.options).unwrap()));
    drop((first, other));
    let _ = std::fs::remove_dir_all(&root);
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
        Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
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
    let localisation: content_reader_localisation::localisation::Localisation =
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
        Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
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
    let localisation: content_reader_localisation::localisation::Localisation =
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
            recorder_derive::writers(),
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
