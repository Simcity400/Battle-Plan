use dashboard_base::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::container::Replay;
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::atomic::AtomicBool;

#[test]
fn prepared_observer_roundtrip_preserves_reply_names_with_and_without_content() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    use crate::products::subjects::{Scan, SubjectParams, Subjects};
    use dashboard_base::products::Product;
    use dashboard_core::config::{ContentOptions, LoadContext};
    let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/autonomous_states/mod.txt",
            "autonomy_state={id=mod_level min_freedom_level=0.2}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n mod_level:0 \"\"\n",
        ),
        ("settings.txt", "language=\"l_english\""),
    ]);
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.clone(),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let frames = [
        subject_frame(100, &[(9001, 44, 40)], Some(&[(44, "mod_level")])),
        subject_frame(101, &[(9001, 44, 40)], Some(&[(44, "mod_level")])),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "subjects-content".into(),
            revision: "1".into(),
        },
        Default::default(),
        std::sync::Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    source.set_test_content(options.options.clone(), catalogue.content.clone());
    let mut scan = Scan::new(source.replay_ref(), 1);
    scan.observe(&frames[0]).unwrap();
    let mut scan: Scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
    scan.observe(&frames[1]).unwrap();
    for content in [None, Some(options)] {
        let context = LoadContext { content };
        let expected = Subjects::load(
            &source,
            &SubjectParams {
                branch_id: "1".into(),
            },
            &context,
            &AtomicBool::new(false),
        )
        .unwrap();
        let actual = scan
            .finish(source.replay_ref(), 1)
            .reply(&source, "1", &context)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(
            actual.levels[0].name,
            context.content.map(|_| String::new())
        );
    }
}

/// Countries GER (link 9001), FRA (44), CTL (55) and MOD (66), with the given
/// subject relations (overlord, subject, start hour) and each subject's current
/// autonomy state; `None` leaves the Autonomy table unavailable this hour.
fn subject_frame(
    hour: i32,
    relations: &[(u32, u32, i32)],
    levels: Option<&[(u32, &str)]>,
) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let mut rows: Vec<_> = relations
        .iter()
        .map(
            |&(overlord, subject, start_hour)| replay_data::rows::SubjectRow {
                overlord,
                subject,
                start_hour,
                ..Default::default()
            },
        )
        .collect();
    rows.sort_by_key(|row| (u64::from(row.overlord) << 32) | u64::from(row.subject));
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(rows));
    *frame.tables[TableId::Autonomy.index()] = match levels {
        Some(levels) => {
            let mut rows: Vec<_> = levels
                .iter()
                .map(|&(subject, state)| replay_data::rows::AutonomyRow {
                    subject,
                    state: state.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect();
            rows.sort_by_key(|row| row.subject);
            TableView::Complete(TableRows::Autonomy(rows))
        }
        None => TableView::Unavailable {
            reason: "fixture".into(),
        },
    };
    frame
}

#[test]
fn subjects_follow_relations_their_levels_and_break_at_gaps() {
    use crate::products::subjects::{LevelStep, SubjectRun, scan_branch};
    use dashboard_base::products::country_groups::HourSpan;
    let step = |first_hour, last_hour, level: Option<&str>| LevelStep {
        first_hour,
        last_hour,
        level: level.map(str::to_owned),
    };
    let mut frames = vec![
        // GER's puppet FRA, begun before the branch (the engine's start hour, 40).
        subject_frame(100, &[(9001, 44, 40)], Some(&[(44, "autonomy_puppet")])),
        subject_frame(
            101,
            &[(9001, 44, 40), (9001, 55, 101)],
            Some(&[(44, "autonomy_puppet"), (55, "autonomy_colony")]),
        ),
        // FRA's level changes; CTL's level is not recorded this hour.
        subject_frame(
            102,
            &[(9001, 44, 40), (9001, 55, 101)],
            Some(&[(44, "autonomy_integrated")]),
        ),
        // The Autonomy table is unavailable: every level is unknown.
        subject_frame(103, &[(9001, 44, 40), (9001, 55, 101)], None),
        // Hour 104 is missing. FRA is subjugated again (a new relation: another
        // start hour), and CTL now by MOD.
        subject_frame(
            105,
            &[(9001, 44, 105), (66, 55, 105)],
            Some(&[(44, "autonomy_puppet"), (55, "autonomy_puppet")]),
        ),
    ];
    let mut ending = subject_frame(
        106,
        &[(9001, 44, 105), (66, 55, 105)],
        Some(&[(44, "autonomy_puppet"), (55, "autonomy_puppet")]),
    );
    if let TableView::Complete(TableRows::Subjects(rows)) =
        &mut *ending.tables[TableId::Subjects.index()]
    {
        // A cancelled relation, and one recorded as ending this hour, no longer count.
        for row in rows.iter_mut() {
            if row.overlord == 9001 {
                row.cancel = 1;
            } else {
                row.end_hour = 106;
            }
        }
    }
    frames.push(ending);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let run =
        |overlord: &str, subject: &str, start_hour, first_hour, last_hour, steps| SubjectRun {
            overlord: overlord.into(),
            subject: subject.into(),
            start_hour,
            first_hour,
            last_hour,
            steps,
        };
    assert_eq!(
        scan.runs(),
        [
            run(
                "GER",
                "FRA",
                40,
                100,
                103,
                vec![
                    step(100, 101, Some("autonomy_puppet")),
                    step(102, 102, Some("autonomy_integrated")),
                    step(103, 103, None)
                ]
            ),
            run(
                "GER",
                "CTL",
                101,
                101,
                103,
                vec![
                    step(101, 101, Some("autonomy_colony")),
                    step(102, 103, None)
                ]
            ),
            run(
                "GER",
                "FRA",
                105,
                105,
                105,
                vec![step(105, 105, Some("autonomy_puppet"))]
            ),
            run(
                "MOD",
                "CTL",
                105,
                105,
                105,
                vec![step(105, 105, Some("autonomy_puppet"))]
            ),
        ]
    );
    assert_eq!(
        scan.unobserved(),
        [HourSpan {
            first_hour: 104,
            last_hour: 104
        }]
    );
    assert!(
        scan.warnings()
            .iter()
            .any(|warning| warning.contains("level was not recorded"))
    );
}

#[test]
fn subjects_link_tags_follow_identity_changes_and_unavailable_reads() {
    use crate::products::subjects::{LevelStep, SubjectRun, scan_branch};
    use dashboard_base::products::country_groups::HourSpan;
    let frames = (100..=108)
        .map(|hour| {
            let mut frame = subject_frame(hour, &[(9001, 45, 40)], Some(&[(45, "mod_level")]));
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                for row in rows {
                    row.political_power_raw = Some(i64::from(hour));
                    if row.id == 44 && hour >= 102 {
                        row.tag = if hour >= 107 { b"REC" } else { b"REN" }.to_vec();
                    }
                }
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.push(CountryLinkRow {
                    id: 45,
                    canonical_slot: if (103..=106).contains(&hour) { 9 } else { 8 },
                });
                rows.sort_by_key(|row| row.id);
            }
            // Each unavailable identity table interrupts the relation separately.
            if hour == 104 || hour == 106 {
                let table = if hour == 104 {
                    TableId::Countries
                } else {
                    TableId::CountryLinks
                };
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "fixture".into(),
                };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let run = |subject: &str, first_hour, last_hour| SubjectRun {
        overlord: "GER".into(),
        subject: subject.into(),
        start_hour: 40,
        first_hour,
        last_hour,
        steps: vec![LevelStep {
            first_hour,
            last_hour,
            level: Some("mod_level".into()),
        }],
    };
    assert_eq!(
        scan.runs(),
        [
            run("FRA", 100, 101),
            run("REN", 102, 102),
            run("CTL", 103, 103),
            run("CTL", 105, 105),
            run("REC", 107, 108),
        ]
    );
    assert_eq!(
        scan.unobserved(),
        [
            HourSpan {
                first_hour: 104,
                last_hour: 104,
            },
            HourSpan {
                first_hour: 106,
                last_hour: 106,
            },
        ]
    );
    assert_eq!(scan.warnings().len(), 1);
    assert!(scan.warnings()[0].contains("incomplete at 2 hours"));
}

#[test]
fn subjects_leave_unknown_countries_out_and_mark_the_hour() {
    use crate::products::subjects::scan_branch;
    use dashboard_base::products::country_groups::HourSpan;
    // Two link ids naming FRA give it two levels in one hour: its level is not known.
    // CTL's level is recorded under an id that is not text: refused, and counted.
    let mut unknown = subject_frame(
        100,
        &[(9001, 44, 1), (9001, 55, 1), (9001, 777, 1)],
        Some(&[(44, "a"), (45, "b"), (55, "x")]),
    );
    if let TableView::Complete(TableRows::Autonomy(rows)) =
        &mut *unknown.tables[TableId::Autonomy.index()]
    {
        rows.iter_mut()
            .filter(|row| row.subject == 55)
            .for_each(|row| row.state = vec![0xff, 0xfe]);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *unknown.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 45,
            canonical_slot: 8,
        });
        rows.sort_by_key(|row| row.id);
    }
    // A subject table that is not complete is not read at all.
    let mut missing = subject_frame(101, &[(9001, 44, 1)], Some(&[]));
    *missing.tables[TableId::Subjects.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let frames = vec![
        unknown,
        missing,
        subject_frame(102, &[(9001, 44, 1)], Some(&[])),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scan.runs().len(), 3);
    assert_eq!(
        (scan.runs()[0].first_hour, scan.runs()[0].last_hour),
        (100, 100)
    );
    assert_eq!(scan.runs()[0].steps[0].level, None);
    assert_eq!(
        scan.unobserved(),
        [HourSpan {
            first_hour: 100,
            last_hour: 101
        }]
    );
    let warnings = scan.warnings().join(" ");
    assert!(warnings.contains("no country identity"), "{warnings}");
    assert!(warnings.contains("two levels"), "{warnings}");
    assert!(
        warnings.contains("not text for 1 subject-hours"),
        "{warnings}"
    );
    assert!(warnings.contains("incomplete at 1 hours"), "{warnings}");
}

#[test]
fn subject_levels_follow_the_content_order_and_keep_unknown_ids_last() {
    use crate::products::subjects::levels;
    use installed_content_core::{autonomy::AutonomyState, autonomy::AutonomyStates};
    let state = |id: &str, min_freedom_level| AutonomyState {
        id: id.into(),
        min_freedom_level,
    };
    let states = AutonomyStates {
        states: vec![
            state("loose", Some(80_000)),
            state("tight", Some(20_000)),
            state("unordered", None),
            state("unused", Some(0)),
        ],
    };
    let named = ["loose", "tight", "unordered", "modded"]
        .into_iter()
        .collect();
    let result = levels(&named, Some(&states), None);
    let order: Vec<_> = result
        .iter()
        .map(|level| (level.token.as_str(), level.ordered))
        .collect();
    assert_eq!(
        order,
        [
            ("tight", true),
            ("loose", true),
            ("modded", false),
            ("unordered", false)
        ]
    );
    // Without content, every level is listed by id and none is ordered.
    assert!(
        levels(&named, None, None)
            .iter()
            .all(|level| !level.ordered && level.name.is_none())
    );
}

/// A mod over vanilla: the mod replaces the autonomy folder (so a vanilla level it
/// leaves out is gone), redefines a vanilla level's freedom, adds its own and one
/// whose level is scripted. Levels, their order and their names follow the stack.
#[test]
fn subject_levels_follow_a_modded_content_stack() {
    use crate::products::subjects::levels;
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_subject_levels_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let (base, modded) = (root.join("vanilla"), root.join("mod"));
    for (dir, path, text) in [
        (
            &base,
            "common/autonomous_states/puppet.txt",
            "autonomy_state = { id = autonomy_puppet min_freedom_level = 0.4 }",
        ),
        (
            &base,
            "common/autonomous_states/dominion.txt",
            "autonomy_state = { id = autonomy_dominion min_freedom_level = 0.8 }",
        ),
        (
            &modded,
            "common/autonomous_states/puppet.txt",
            "autonomy_state = { id = autonomy_puppet min_freedom_level = 0.9 }",
        ),
        (
            &modded,
            "common/autonomous_states/mod_levels.txt",
            "autonomy_state = { id = autonomy_mod_march min_freedom_level = 0.1 }\n\
             autonomy_state = { id = autonomy_mod_scripted min_freedom_level = @freedom }",
        ),
    ] {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers = installed_content_core::ResolvedLayers::from_folders(&[
        (&base, &[][..]),
        (&modded, &["common/autonomous_states"][..]),
    ])
    .unwrap();
    let (states, warnings) = installed_content_core::autonomy::read(&layers).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    // The scripted level is refused by name, not assumed.
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].1.contains("autonomy_mod_scripted"));
    let named = [
        "autonomy_puppet",
        "autonomy_dominion",
        "autonomy_mod_march",
        "autonomy_mod_scripted",
    ]
    .into_iter()
    .collect();
    let order: Vec<_> = levels(&named, Some(&states), None)
        .into_iter()
        .map(|level| (level.token, level.ordered))
        .collect();
    assert_eq!(
        order,
        [
            ("autonomy_mod_march".to_owned(), true),
            ("autonomy_puppet".to_owned(), true),
            // Replaced away by the mod: named by the recording but not by content.
            ("autonomy_dominion".to_owned(), false),
            ("autonomy_mod_scripted".to_owned(), false),
        ]
    );
}

/// Tags are opaque: a dynamic country and a tag with a space or a colon are
/// overlords and subjects like any other.
#[test]
fn subjects_keep_dynamic_and_unusual_tags_as_recorded() {
    use crate::products::subjects::scan_branch;
    let frame = |hour| {
        let mut frame = subject_frame(
            hour,
            &[(77, 88, 90), (77, 99, 90)],
            Some(&[(88, "autonomy_mod_march"), (99, "autonomy_mod_march")]),
        );
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for (id, slot, tag) in [
                (77, 11, &b"D01"[..]),
                (88, 12, &b"MY TAG"[..]),
                (99, 13, &b"A:B"[..]),
            ] {
                rows.push(CountryRow {
                    id,
                    canonical_slot: slot,
                    tag: tag.to_vec(),
                    ..CountryRow::default()
                });
            }
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.extend(
                [(77, 11), (88, 12), (99, 13)]
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }),
            );
            rows.sort_by_key(|row| row.id);
        }
        frame
    };
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100), frame(101)])]).unwrap();
    let scan = scan_branch(&replay, 1, &AtomicBool::new(false)).unwrap();
    let pairs: Vec<_> = scan
        .runs()
        .iter()
        .map(|run| {
            (
                run.overlord.as_str(),
                run.subject.as_str(),
                run.first_hour,
                run.last_hour,
                run.steps[0].level.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [
            ("D01", "A:B", 100, 101, Some("autonomy_mod_march")),
            ("D01", "MY TAG", 100, 101, Some("autonomy_mod_march")),
        ]
    );
    assert!(scan.warnings().is_empty(), "{:?}", scan.warnings());
}

#[test]
fn subject_deferred_spans_and_refusal_rates_match_oracle_after_commits_resume_append_and_expiry() {
    use super::Scan;
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        series::{Accumulator, LateIndex},
    };
    let flag = AtomicBool::new(false);
    for levels in [None, Some(&[(44, "level")][..])] {
        let mut initial = subject_frame(100, &[(9001, 44, 40)], levels);
        if let TableView::Complete(TableRows::Subjects(rows)) =
            &mut *initial.tables[TableId::Subjects.index()]
        {
            rows[0].end_hour = 109;
        }
        let mut frames: Vec<_> = (100..=110)
            .map(|hour| {
                let mut frame = initial.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        let mut resumed = initial.clone();
        resumed.hour = 113;
        frames.push(resumed.clone());
        resumed.hour = 114;
        frames.push(resumed);
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
        let source = dashboard_base::source::Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "subjects-deferred".into(),
                revision: "1".into(),
            },
            Default::default(),
            std::sync::Arc::new(replay),
            &flag,
        )
        .unwrap();
        let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut scan = Scan::new(source.replay_ref(), 1);
        let mut events = Accumulator::default();
        let mut late = LateIndex::default();
        for (index, batch) in [&frames[..6], &frames[6..11], &frames[11..]]
            .into_iter()
            .enumerate()
        {
            let mut tracker = FrameChangeTracker::new();
            for frame in batch {
                scan.prepare_changes_changed(
                    frame,
                    &tracker.observe(frame, source.replay_ref().format),
                    &mut events,
                    &mut late,
                )
                .unwrap();
            }
            scan.seal();
            let mut writer = if index == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            events
                .flush(&mut writer, "subject-changes-1", 0, &flag)
                .unwrap();
            writer
                .write_at(
                    "subjects-1",
                    0,
                    zstd::stream::encode_all(serde_json::to_vec(&scan).unwrap().as_slice(), 3)
                        .unwrap()
                        .as_slice(),
                )
                .unwrap();
            writer.commit().unwrap();
            scan = serde_json::from_reader(
                zstd::stream::Decoder::new(
                    fixture
                        .store
                        .read(&fixture.key, "subjects-1", 0)
                        .unwrap()
                        .unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            events = serde_json::from_slice(&serde_json::to_vec(&events).unwrap()).unwrap();
        }
        let context = dashboard_core::config::LoadContext { content: None };
        let actual = scan
            .finish(source.replay_ref(), 1)
            .reply(&source, "1", &context)
            .unwrap();
        let expected = super::scan_branch(source.replay_ref(), 1, &flag)
            .unwrap()
            .reply(&source, "1", &context)
            .unwrap();
        dashboard_base::prepared::observer_tests::assert_reply_parity(&actual, &expected).unwrap();
        assert_eq!(
            actual.runs[0].last_hour, 108,
            "a stored native end seals the open relation at the old frontier"
        );
    }
}

#[test]
fn subject_holder_change_reuses_unchanged_native_relation_level_and_link_tables() {
    use super::Scan;
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        series::{Accumulator, LateIndex},
    };
    let mut first = dashboard_base::test_fixtures::world_frame(100, true);
    *first.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![replay_data::rows::SubjectRow {
            overlord: 11,
            subject: 31,
            start_hour: 90,
            ..Default::default()
        }]));
    *first.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![replay_data::rows::AutonomyRow {
            subject: 31,
            state: b"level".to_vec(),
            ..Default::default()
        }]));
    let mut renamed = first.clone();
    renamed.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows.iter_mut().find(|row| row.id == 31).unwrap().tag = b"RENAMED".to_vec();
    }
    let mut scan = Scan::default();
    let mut events = Accumulator::default();
    let mut late = LateIndex::default();
    let mut tracker = FrameChangeTracker::new();
    let format = replay_data::format::FormatVersion::CURRENT;
    scan.prepare_changes_changed(
        &first,
        &tracker.observe(&first, format),
        &mut events,
        &mut late,
    )
    .unwrap();
    let before = scan.work;
    scan.prepare_changes_changed(
        &renamed,
        &tracker.observe(&renamed, format),
        &mut events,
        &mut late,
    )
    .unwrap();
    assert_eq!(
        scan.work.row_visits - before.row_visits,
        renamed
            .complete::<replay_data::rows::tables::Countries>()
            .unwrap()
            .len()
    );
    let mut oracle = Scan::default();
    oracle.observe(&first).unwrap();
    oracle.observe(&renamed).unwrap();
    scan.seal();
    assert_eq!(
        serde_json::to_value(&scan.runs).unwrap(),
        serde_json::to_value(&oracle.runs).unwrap()
    );
    assert_eq!(scan.change_warnings(), oracle.change_warnings());
}

/// A span of unchanged hours adds each diagnostic rate as that many hourly
/// additions do: the same sum, and an overflow exactly where one addition at
/// a time overflows (a panic where overflow is checked, a wrap otherwise).
#[test]
fn unchanged_spans_add_diagnostic_rates_as_hourly_additions_do() {
    for (value, rate, hours) in [
        (0, 0, 5),
        (7, 3, 1),
        (7, 3, 1_000),
        (u32::MAX - 10, 2, 5),
        (u32::MAX - 10, 2, 6),
        (u32::MAX, 0, 1_000_000),
        (1, u32::MAX, 2),
    ] {
        let hourly = std::panic::catch_unwind(|| (0..hours).fold(value, |value, _| value + rate));
        let spanned = std::panic::catch_unwind(|| super::repeated(value, rate, hours));
        match (hourly, spanned) {
            (Ok(hourly), Ok(spanned)) => {
                assert_eq!(spanned, hourly, "{value} + {hours} x {rate}");
            }
            (Err(_), Err(_)) => {}
            _ => panic!("{value} + {hours} x {rate}: only one way overflowed"),
        }
    }
}
