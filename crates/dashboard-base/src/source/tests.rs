use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

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

/// A request under folders the user has just left finishing while one under the new folders
/// runs: each reply reports, and is kept by, its own folders' read set, and the old one's
/// diagnostics never rebind what the new one reads.
#[test]
fn a_reply_reports_its_own_folders_read_set_whatever_is_bound_meanwhile() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    let uncovered = |lines: &[String]| lines.iter().any(|line| line.contains("gfx/custom/icon.dds"));
    // Each request holds its folders' content while it runs.
    let held_old = source.content(&old.options).unwrap();
    let held_new = source.content(&new.options).unwrap();
    let _ = held_new.layers.read_uncovered_path(b"gfx/custom/icon.dds");
    let mut old_lines = Vec::new();
    source.append_reply_warnings(&mut old_lines, Some(&old));
    assert!(!uncovered(&old_lines));
    assert!(!source.reply_reads_uncovered(Some(&old)));
    assert!(Arc::ptr_eq(&source.bound_content(&new.options).unwrap(), &held_new));
    // A read under the old folders meanwhile reuses their held content, and the new request's
    // reads still share its own.
    assert!(Arc::ptr_eq(&source.content(&old.options).unwrap(), &held_old));
    assert!(Arc::ptr_eq(&source.content(&new.options).unwrap(), &held_new));
    source.content(&old.options).unwrap();
    let mut new_lines = Vec::new();
    source.append_reply_warnings(&mut new_lines, Some(&new));
    assert!(uncovered(&new_lines));
    assert!(source.reply_reads_uncovered(Some(&new)));
    drop((held_old, held_new));
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

fn two_block_source() -> (Arc<Replay>, impl Fn(&Arc<Replay>) -> Arc<crate::Source>) {
    let next_hour = 100 + replay_frame::KEYFRAME_INTERVAL as i32;
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![source(1, 100, Some(1)), source(1, next_hour, Some(2))],
            )],
        )
        .unwrap(),
    );
    assert_eq!(replay.index.len(), 2);
    let open = |replay: &Arc<Replay>| {
        Arc::new(
            crate::Source::from_replay(
                crate::SourceKey {
                    source_id: "1".into(),
                    revision: "1".into(),
                },
                "memory.battleplan".into(),
                Arc::clone(replay),
                &AtomicBool::new(false),
            )
            .unwrap(),
        )
    };
    (replay, open)
}

fn ahead(source: &Arc<crate::Source>) -> (super::Ahead, Arc<AtomicBool>) {
    let stop = Arc::new(AtomicBool::new(false));
    let ahead = super::Ahead {
        source: Arc::downgrade(source),
        next: 0,
        stop: Arc::clone(&stop),
    };
    (ahead, stop)
}

/// Each block handed out, by its source's recording and position.
fn handed(queue: &super::Queue) -> Vec<(usize, usize)> {
    std::iter::from_fn(|| super::next_block(queue))
        .map(|(replay, position, _)| (Arc::as_ptr(&replay) as usize, position))
        .collect()
}

#[test]
fn verifying_ahead_hands_out_the_newest_sources_blocks_first_and_never_the_source() {
    let (older_replay, open) = two_block_source();
    let newer_replay = Arc::new(Replay::open(older_replay.bytes().to_vec()).unwrap());
    let (older, newer) = (open(&older_replay), open(&newer_replay));
    let queue = Mutex::new((vec![ahead(&older).0, ahead(&newer).0], true));
    let (first, _, _) = super::next_block(&queue).unwrap();
    assert!(Arc::ptr_eq(&first, &newer_replay));
    assert_eq!(Arc::strong_count(&newer), 1, "the source is not held");
    let (older_at, newer_at) = (
        Arc::as_ptr(&older_replay) as usize,
        Arc::as_ptr(&newer_replay) as usize,
    );
    let rest = [(newer_at, 1), (older_at, 0), (older_at, 1)];
    assert_eq!(handed(&queue), rest);
    let queue = queue.into_inner().unwrap();
    assert!(queue.0.is_empty());
    assert!(!queue.1, "the thread is marked gone");
    // Started once per source, however often it opens.
    let state = older.state::<super::VerifyAhead>();
    assert!(super::claim(&state));
    assert!(!super::claim(&state));
}

#[test]
fn verifying_ahead_skips_stopped_and_released_sources() {
    let (replay, open) = two_block_source();
    let released_replay = Arc::new(Replay::open(replay.bytes().to_vec()).unwrap());
    let stopped_replay = Arc::new(Replay::open(replay.bytes().to_vec()).unwrap());
    let kept = open(&replay);
    let released = open(&released_replay);
    let stopped = open(&stopped_replay);
    let (released_ahead, _) = ahead(&released);
    let (stopped_ahead, stop) = ahead(&stopped);
    drop(released);
    stop.store(true, Ordering::Release);
    let queue = Mutex::new((vec![ahead(&kept).0, released_ahead, stopped_ahead], true));
    let at = Arc::as_ptr(&replay) as usize;
    assert_eq!(handed(&queue), [(at, 0), (at, 1)]);
    assert!(queue.lock().unwrap().0.is_empty());
}

#[test]
fn verifying_ahead_starts_one_thread_and_keeps_queued_sources_when_it_cannot() {
    let (replay, open) = two_block_source();
    let (first, second) = (open(&replay), open(&replay));
    let queue = Mutex::new((Vec::new(), false));
    assert!(super::schedule(&queue, Some(ahead(&first).0)));
    let started = super::schedule(&queue, Some(ahead(&second).0));
    assert!(!started, "one thread checks");
    // The thread could not start: the sources stay queued for the next open to start it.
    queue.lock().unwrap().1 = false;
    assert_eq!(queue.lock().unwrap().0.len(), 2);
    assert!(super::schedule(&queue, None));
    assert!(!super::schedule(&queue, None));
    // Nothing queued: nothing to start.
    assert!(!super::schedule(&Mutex::new((Vec::new(), false)), None));
}
