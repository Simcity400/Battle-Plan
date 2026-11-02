//! The family's columnar preparation held to its row scan ([`super::row`]) on the same hours,
//! written as the native recorder writes them: the reply's runs, levels, unobserved hours and
//! warnings, and what the event feed reads, across segment starts, gaps, cuts and a resume.

use super::{
    Feed, SubjectReply,
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    query, row,
    scan::Input,
    tests::subject_frame,
};
use dashboard_base::{
    Source,
    products::LoadContext,
    test_fixtures::{CAMPAIGN, records},
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input as Written, Sample, Synthetic},
    container::Replay,
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{AutonomyRow, CountryLinkRow, CountryRow, RowCodec, SubjectRow, TableId, TableRows, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const REFUSAL: &str = "fixture";

fn put<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Written::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Written::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    let mut last: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let gap = last.is_some_and(|last| last + 1 != frame.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        put(
            walk,
            TableId::Countries,
            frame.complete::<tables::Countries>().ok(),
        );
        put(
            walk,
            TableId::CountryLinks,
            frame.complete::<tables::CountryLinks>().ok(),
        );
        put(walk, TableId::Subjects, frame.complete::<tables::Subjects>().ok());
        put(walk, TableId::Autonomy, frame.complete::<tables::Autonomy>().ok());
        let cold = starts.contains(&frame.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT).unwrap();
        }
        walk.end(|state, manifest, _, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
                (FormatVersion::CURRENT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
        last = Some(frame.hour);
    }
    out
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new(first);
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            &cache,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn source_of(frames: &[DecodedFrame], name: &str) -> Source {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: name.into(),
            revision: "1".into(),
        },
        Default::default(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// The reply and the event feed's view the publication `built` gives.
fn published(built: &Built, source: &Source, context: &LoadContext) -> (SubjectReply, Feed) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let reply = query::reply(
        &mut snapshot,
        &directory,
        source,
        1,
        "1",
        context,
        true,
        &AtomicBool::new(false),
    )
    .unwrap();
    let feed = query::feed(&mut snapshot, &directory).unwrap();
    (reply, feed)
}

/// Every placement of segment starts, cuts and a resume over `frames` publishes the reply and
/// the feed the row scan reads of them.
fn holds_to_the_row_scan(frames: &[DecodedFrame], label: &str) {
    let budget = Budget::new(256 << 20);
    let source = source_of(frames, label);
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let oracle = row::scan_branch(source.replay_ref(), 1, &cancelled).unwrap();
    let expected = serde_json::to_value(oracle.reply(&source, "1", &context).unwrap()).unwrap();
    let feed = oracle.feed();
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let (first, last) = (hours[0], *hours.last().unwrap());
    let middle = hours[hours.len() / 2];
    let after = hours[(hours.len() / 2 + 1).min(hours.len() - 1)];
    for (mut starts, mut cuts, resume) in [
        (vec![], vec![last], None),
        (vec![middle], vec![middle, last], Some(middle)),
        (vec![after], vec![first, middle, last], Some(first)),
        (hours.clone(), hours.clone(), Some(middle)),
    ] {
        cuts.dedup();
        // A preparation taken up again walks its first hour as a segment's first.
        if let Some(resume) = resume
            && let Some(&next) = hours.iter().find(|&&hour| hour > resume)
        {
            starts.push(next);
        }
        let placement = format!("{label}: starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let built = build(projected(frames, &starts), &cuts, resume, &budget);
        let (reply, published) = published(&built, &source, &context);
        assert_eq!(
            serde_json::to_value(reply).unwrap(),
            expected,
            "{placement}: the reply"
        );
        assert_eq!(published, feed, "{placement}: what the event feed reads");
    }
}

/// `frame` with its Subjects rows changed by `change`.
fn relations(mut frame: DecodedFrame, change: impl FnOnce(&mut Vec<SubjectRow>)) -> DecodedFrame {
    if let TableView::Complete(TableRows::Subjects(rows)) =
        &mut *frame.tables[TableId::Subjects.index()]
    {
        change(rows);
    }
    frame
}

/// Relations beginning before and during the branch, levels changing and unknown, an hour
/// missing, a relation renewed under another start, one cancelled and one ending.
#[test]
fn relations_levels_gaps_and_ends_publish_what_the_row_scan_reads() {
    let mut frames = vec![
        subject_frame(100, &[(9001, 44, 40)], Some(&[(44, "autonomy_puppet")])),
        subject_frame(
            101,
            &[(9001, 44, 40), (9001, 55, 101)],
            Some(&[(44, "autonomy_puppet"), (55, "autonomy_colony")]),
        ),
        subject_frame(
            102,
            &[(9001, 44, 40), (9001, 55, 101)],
            Some(&[(44, "autonomy_integrated")]),
        ),
        subject_frame(103, &[(9001, 44, 40), (9001, 55, 101)], None),
        subject_frame(
            105,
            &[(9001, 44, 105), (66, 55, 105)],
            Some(&[(44, "autonomy_puppet"), (55, "autonomy_puppet")]),
        ),
    ];
    frames.push(relations(
        subject_frame(
            106,
            &[(9001, 44, 105), (66, 55, 105)],
            Some(&[(44, "autonomy_puppet"), (55, "autonomy_puppet")]),
        ),
        |rows| {
            for row in rows {
                if row.overlord == 9001 {
                    row.cancel = 1;
                } else {
                    row.end_hour = 106;
                }
            }
        },
    ));
    holds_to_the_row_scan(&frames, "relations");
}

/// A relation recorded with an end inside the branch ends there with no field changing, across
/// a gap after it, with and without levels.
#[test]
fn a_recorded_end_ends_the_relation_where_no_field_changes() {
    for levels in [None, Some(&[(44, "level")][..])] {
        let initial = relations(subject_frame(100, &[(9001, 44, 40)], levels), |rows| {
            rows[0].end_hour = 109;
        });
        let frames: Vec<_> = (100..=110)
            .chain(113..=114)
            .map(|hour| {
                let mut frame = initial.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        holds_to_the_row_scan(&frames, &format!("end, levels {levels:?}"));
    }
}

/// Renamed countries, a link moving to another slot, and each identity table unavailable in
/// turn.
#[test]
fn identity_changes_and_unavailable_tables_publish_what_the_row_scan_reads() {
    let frames: Vec<_> = (100..=108)
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
            if hour == 104 || hour == 106 {
                let table = if hour == 104 {
                    TableId::Countries
                } else {
                    TableId::CountryLinks
                };
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: REFUSAL.into(),
                };
            }
            frame
        })
        .collect();
    holds_to_the_row_scan(&frames, "identities");
}

/// An unresolved country, a subject given two levels and one whose level is not text, and an
/// unavailable Subjects table between complete ones.
#[test]
fn unknown_countries_and_unreadable_levels_publish_what_the_row_scan_reads() {
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
    let alias = |frame: &mut DecodedFrame| {
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 45,
                canonical_slot: 8,
            });
            rows.sort_by_key(|row| row.id);
        }
    };
    alias(&mut unknown);
    let mut missing = subject_frame(101, &[(9001, 44, 1)], Some(&[]));
    *missing.tables[TableId::Subjects.index()] = TableView::Unavailable {
        reason: REFUSAL.into(),
    };
    let mut frames = vec![unknown.clone(), missing];
    for hour in 102..=105 {
        let mut frame = unknown.clone();
        frame.hour = hour;
        frames.push(frame);
    }
    frames.push(subject_frame(106, &[(9001, 44, 1)], Some(&[])));
    holds_to_the_row_scan(&frames, "unknown");
}

/// Levels alone changing under steady relations, the Autonomy table unavailable for an hour,
/// and relations starting during the branch: dated exactly, witnessed after a rename, and read
/// after their date.
#[test]
fn level_changes_and_relation_starts_publish_what_the_row_scan_reads() {
    let frames: Vec<_> = (100..=112)
        .map(|hour| {
            let mut relations = vec![(9001, 44, 40)];
            if hour >= 104 {
                // Begun at 103, read at 104: a start read after its date.
                relations.push((9001, 55, 103));
            }
            if hour >= 106 {
                // Begun at 105 and read at 106, as CTL is renamed: witnessed, not dated.
                relations.push((66, 55, 105));
            }
            if hour >= 109 {
                relations.push((66, 44, 109));
            }
            let level = match hour {
                ..=101 => "autonomy_puppet",
                102..=103 => "autonomy_dominion",
                _ => "autonomy_integrated",
            };
            let levels = [(44, level), (55, "autonomy_colony")];
            let mut frame = subject_frame(
                hour,
                &relations,
                (hour != 107).then_some(&levels[..]),
            );
            if let TableView::Complete(TableRows::Subjects(rows)) =
                &mut *frame.tables[TableId::Subjects.index()]
            {
                for row in rows {
                    row.autonomy_ratio = i64::from(hour);
                }
            }
            if hour >= 106
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                for row in rows.iter_mut().filter(|row| row.id == 55) {
                    row.tag = b"CTX".to_vec();
                }
            }
            frame
        })
        .collect();
    holds_to_the_row_scan(&frames, "starts");
}

/// The reply names its levels by the content's localisation where it is read, as the row reply
/// does, and by their ids without it.
#[test]
fn replies_name_levels_as_the_row_reply_with_and_without_content() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    use dashboard_core::config::ContentOptions;
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
    let source = source_of(&frames, "subjects-content");
    source.set_test_content(options.options.clone(), catalogue.content.clone());
    let budget = Budget::new(64 << 20);
    let built = build(projected(&frames, &[]), &[101], None, &budget);
    for content in [None, Some(options)] {
        let context = LoadContext { content };
        let (actual, _) = published(&built, &source, &context);
        let expected = row::scan_branch(source.replay_ref(), 1, &AtomicBool::new(false))
            .unwrap()
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

/// Of 1,500 countries, an hour hands the emit nothing when only balances, autonomy ratios and
/// scores move, and only the table that changed when one does.
#[test]
fn quiet_hours_carry_nothing_and_a_change_only_its_table() {
    let frame = |hour: i32| {
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(i64::from(hour)));
        let ids = 1..=1_500_u32;
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            ids.clone()
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("C{id:04}").into_bytes(),
                    political_power_raw: Some(i64::from(hour)),
                    ..CountryRow::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            ids.map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
        ));
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            (2..=40)
                .map(|subject| SubjectRow {
                    overlord: 1,
                    subject,
                    start_hour: 50,
                    autonomy_ratio: i64::from(hour),
                    ..SubjectRow::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            (2..=40)
                .map(|subject| AutonomyRow {
                    subject,
                    score: i64::from(hour),
                    state: if subject == 2 && hour >= 110 {
                        b"autonomy_dominion".to_vec()
                    } else {
                        b"autonomy_puppet".to_vec()
                    },
                    ..AutonomyRow::default()
                })
                .collect(),
        ));
        frame
    };
    let frames: Vec<_> = (100..=119).map(frame).collect();
    let hours = projected(&frames, &[]);
    for hour in &hours {
        let inputs = hour.inputs.as_deref();
        match hour.hour {
            100 => assert!(inputs.is_some(), "the first hour reads every table"),
            110 => {
                let inputs = inputs.expect("the level change");
                assert!(matches!(inputs.countries, Input::Same));
                assert!(matches!(inputs.links, Input::Same));
                assert!(matches!(inputs.relations, Input::Same));
                assert!(matches!(inputs.levels, Input::Read(_)));
            }
            at => assert!(inputs.is_none(), "hour {at} carries nothing"),
        }
    }
    holds_to_the_row_scan(&frames, "quiet");
}

/// A branch beginning with every table unavailable, and the hours after a resume unavailable
/// too: neither the scan's first hour nor the first it takes up again carries anything over.
#[test]
fn hours_without_any_table_after_a_start_or_a_resume_are_unobserved() {
    let frames: Vec<_> = (100..=108)
        .map(|hour| {
            let mut frame = subject_frame(hour, &[(9001, 44, 40)], Some(&[(44, "level")]));
            if matches!(hour, 100 | 105 | 106) {
                for table in TABLES_UNREAD {
                    *frame.tables[table.index()] = TableView::Unavailable {
                        reason: REFUSAL.into(),
                    };
                }
            }
            frame
        })
        .collect();
    // The placements resume at 104, before 105, and start the walk at 100.
    holds_to_the_row_scan(&frames, "unread");
}

const TABLES_UNREAD: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Subjects,
    TableId::Autonomy,
];

/// A subject renamed at an hour whose Subjects table is unavailable keeps its level under its
/// new tag once the table is read again.
#[test]
fn a_rename_while_subjects_are_unread_keeps_the_level_under_the_new_tag() {
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = subject_frame(hour, &[(9001, 44, 40)], Some(&[(44, "puppet")]));
            if hour >= 101
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                for row in rows.iter_mut().filter(|row| row.id == 44) {
                    row.tag = b"REN".to_vec();
                }
            }
            if hour == 101 {
                *frame.tables[TableId::Subjects.index()] = TableView::Unavailable {
                    reason: REFUSAL.into(),
                };
            }
            frame
        })
        .collect();
    holds_to_the_row_scan(&frames, "rename");
}
