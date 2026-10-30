//! The division log's columnar path ([`super::super::columnar`]) against its row path on the
//! same hours, written as the native recorder writes them, walked whole and from a restart at
//! every hour, then on into a later run of the branch: every prepared section equal.

use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    series::tests::{Fixture, assert_payloads_equal},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{rows::TableSchema, scope::CaptureMode};

/// Writes `frame`'s `T` as the native recorder writes it.
fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

/// Writes every table of `input` as the native recorder writes it.
fn write_hour(walk: &mut Synthetic, input: &DecodedFrame) {
    write::<tables::Campaign>(walk, input);
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::Divisions>(walk, input);
    write::<tables::DivisionStatus>(walk, input);
    write::<tables::DivisionTemplates>(walk, input);
    write::<tables::DivisionEquipmentDeletions>(walk, input);
    write::<tables::DestroyedDivisions>(walk, input);
    write::<tables::LandEventCoverage>(walk, input);
    write::<tables::DivisionTemplateIcons>(walk, input);
}

/// Division `index`'s handle in [`frame`].
fn handle(index: u32) -> u64 {
    u64::from(index) << 32 | 4713
}

/// `frame` without divisions `gone`.
fn without(mut frame: DecodedFrame, gone: &[u32]) -> DecodedFrame {
    let gone: Vec<u64> = gone.iter().map(|&index| handle(index)).collect();
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    {
        rows.retain(|row| !gone.contains(&row.id));
    }
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *frame.tables[TableId::DivisionStatus.index()]
    {
        rows.retain(|row| !gone.contains(&row.division));
    }
    frame
}

/// `frame` with division `index` in province `province`.
fn moved(mut frame: DecodedFrame, index: u32, province: u32) -> DecodedFrame {
    if let TableView::Complete(TableRows::Divisions(rows))
    | TableView::Truncated {
        rows: TableRows::Divisions(rows),
        ..
    } = &mut *frame.tables[TableId::Divisions.index()]
    {
        for row in rows.iter_mut().filter(|row| row.id == handle(index)) {
            row.province = province;
        }
    }
    frame
}

/// `frame` with `table`'s last row missing from a partial read.
fn truncated(mut frame: DecodedFrame, table: TableId) -> DecodedFrame {
    let view = &mut frame.tables[table.index()];
    let TableView::Complete(rows) = &**view else {
        panic!("{table:?} is not complete");
    };
    let mut rows = rows.clone();
    match &mut rows {
        TableRows::Divisions(rows) => drop(rows.pop()),
        TableRows::DivisionStatus(rows) => drop(rows.pop()),
        _ => panic!("{table:?} is not truncated here"),
    }
    *view = TableView::Truncated { rows, missing: 1 }.into();
    frame
}

fn unavailable(frame: DecodedFrame, table: TableId) -> DecodedFrame {
    refused(frame, table, "synthetic refusal")
}

fn refused(mut frame: DecodedFrame, table: TableId, reason: &str) -> DecodedFrame {
    frame.tables[table.index()] = TableView::Unavailable {
        reason: reason.into(),
    }
    .into();
    frame
}

/// A removal witness of division `index`.
fn deletion(
    index: u32,
    disposition: Disposition,
    cause: Option<Cause>,
) -> DivisionEquipmentDeletionRow {
    DivisionEquipmentDeletionRow {
        division_handle: handle(index),
        country: index,
        expeditionary_owner: u32::MAX,
        disposition,
        cause,
    }
}

fn witnessed(mut frame: DecodedFrame, rows: Vec<DivisionEquipmentDeletionRow>) -> DecodedFrame {
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] =
        TableView::Complete(TableRows::DivisionEquipmentDeletions(rows));
    frame
}

/// Census, creations, a renamed template, a covered loss, a partial census reading a new and
/// a moved division, unreadable links with a merge and a loss, partial names, unreadable
/// templates, icons refused for one reason then another with a loss between, a reappearance,
/// a division hidden as an intelligence copy and shown again, and a loss presented from a
/// census built hours before, whose names' refusals are dated then, with a witness held to
/// the run's end.
fn hours() -> Vec<DecodedFrame> {
    let mut renamed = frame(102, 5);
    if let TableView::Complete(TableRows::DivisionTemplates(rows)) =
        &mut *renamed.tables[TableId::DivisionTemplates.index()]
    {
        rows[1].name = b"Renamed template".to_vec();
    }
    let mut loss = without(frame(103, 5), &[1]);
    witness(&mut loss, Disposition::Loss, Some(Cause::DivisionDestroyed));
    destroyed(&mut loss);
    let merged = witnessed(
        unavailable(without(frame(106, 6), &[1, 2, 3]), TableId::CountryLinks),
        vec![
            deletion(2, Disposition::Merged, None),
            deletion(3, Disposition::Loss, Some(Cause::DivisionDestroyed)),
        ],
    );
    let mut hidden = witnessed(
        refused(
            without(frame(110, 8), &[2, 3]),
            TableId::DivisionTemplateIcons,
            "icons refused again",
        ),
        vec![deletion(6, Disposition::Loss, Some(Cause::Unknown))],
    );
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *hidden.tables[TableId::Divisions.index()]
    {
        rows[1].reveal_skip = 1;
    }
    let held = || {
        witnessed(
            refused(
                without(frame(111, 8), &[2, 3]),
                TableId::DivisionTemplateIcons,
                "icons refused all along",
            ),
            vec![deletion(7, Disposition::Returned, None)],
        )
    };
    let mut quiet = held();
    quiet.hour = 112;
    let mut lost = witnessed(
        refused(
            without(frame(113, 8), &[2, 3, 4]),
            TableId::DivisionTemplateIcons,
            "icons refused all along",
        ),
        vec![
            deletion(4, Disposition::Loss, Some(Cause::DivisionDestroyed)),
            deletion(7, Disposition::Returned, None),
        ],
    );
    lost.hour = 113;
    vec![
        frame(100, 3),
        frame(101, 5),
        renamed,
        loss,
        moved(
            truncated(without(frame(104, 7), &[1]), TableId::Divisions),
            3,
            99,
        ),
        moved(without(frame(105, 6), &[1]), 3, 99),
        merged,
        truncated(without(frame(107, 7), &[1, 2, 3]), TableId::DivisionStatus),
        unavailable(
            without(frame(108, 8), &[1, 2, 3]),
            TableId::DivisionTemplates,
        ),
        refused(
            without(frame(109, 8), &[2, 3]),
            TableId::DivisionTemplateIcons,
            "icons refused",
        ),
        hidden,
        held(),
        quiet,
        lost,
    ]
}

/// A later run of the branch from `first`: a creation, a loss presented from the census
/// before it, a division gone with no witness, and the witness held across it.
fn later(first: i32) -> Vec<DecodedFrame> {
    let rows = || {
        vec![
            deletion(4, Disposition::Loss, Some(Cause::DivisionDestroyed)),
            deletion(7, Disposition::Returned, None),
        ]
    };
    vec![
        witnessed(without(frame(first, 9), &[2, 3, 4, 5]), rows()),
        witnessed(without(frame(first + 1, 9), &[2, 3, 4, 5]), rows()),
        witnessed(without(frame(first + 2, 10), &[2, 3, 4, 5]), rows()),
    ]
}

/// Every division log section `scan` publishes, one block per hour of `source`, and the
/// events it kept, with the presentations their names are drawn from.
fn published(
    source: &Source,
    mut observe: impl FnMut(&mut Scan, usize) -> io::Result<()>,
) -> (
    dashboard_base::prepared::series::tests::PublishedPayloads,
    serde_json::Value,
) {
    let (flag, context) = (AtomicBool::new(false), LoadContext { content: None });
    let fixture = Fixture::new();
    let mut scan = Scan::new(source.replay_ref().format);
    for ordinal in 0..super::super::windows(source.replay_ref(), 1).len() {
        observe(&mut scan, ordinal).unwrap();
        let mut writer = fixture.writer();
        scan.publish_block(source, &context, &mut writer, 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
    }
    (
        fixture.payloads(),
        serde_json::to_value(&scan.events).unwrap(),
    )
}

/// `runs` of branch 1 prepared by the row path and by segment walks restarting before every
/// hour of the first run: every section equal.
fn parity(runs: &[Vec<DecodedFrame>]) {
    use super::super::columnar::{Hour, Projector};
    let format = replay_data::format::FormatVersion::CURRENT;
    // A walk's projector and the hours it projected.
    let projector = || -> (Projector, Vec<Hour>) {
        (
            Projector::new(format, CaptureMode::Full).unwrap(),
            Vec::new(),
        )
    };
    // Split k restarts before the first run's hour k: a walk of its own from the state the
    // hour before left, or, where the projection cannot start, the walk before going on.
    // Split 0 walks every hour in one. Each split's last walk goes on into the later runs.
    let mut splits: Vec<Vec<(Projector, Vec<Hour>)>> =
        (0..runs[0].len()).map(|_| vec![projector()]).collect();
    let mut frames = Vec::new();
    for (run, inputs) in (0..).zip(runs) {
        let mut walk = Synthetic::new(&TABLES, 1, inputs[0].hour).unwrap();
        for walks in &mut splits {
            let (walker, _) = walks.last_mut().unwrap();
            if run == 0 {
                walker
                    .start(Start::Run { continues: false }, 1, inputs[0].hour)
                    .unwrap();
            } else {
                walker.cross(inputs[0].hour).unwrap();
            }
        }
        for (ordinal, input) in (0..).zip(inputs) {
            write_hour(&mut walk, input);
            let frame = walk
                .end(|state, manifest, frame, _| {
                    for (split, walks) in (0..).zip(&mut splits) {
                        let opens = ordinal == 0 || run == 0 && ordinal == split;
                        let starts = ordinal == 0 || opens && walks.len() > 1;
                        let (walker, hours) = walks.last_mut().unwrap();
                        walker
                            .hour(
                                &segments::Hour {
                                    branch: 1,
                                    hour: frame.hour,
                                    run,
                                    ordinal,
                                    starts,
                                    opens,
                                    state,
                                    manifest,
                                },
                                hours,
                            )
                            .unwrap();
                    }
                    frame.clone()
                })
                .unwrap();
            if run == 0
                && let Some(walks) = splits.get_mut(ordinal as usize + 1)
            {
                let mut next = projector();
                let restart = Start::Restart(walk.state());
                if next.0.starts_at(restart) {
                    next.0.start(restart, 1, frame.hour + 1).unwrap();
                    walks.push(next);
                }
            }
            frames.push(frame);
        }
    }

    let inputs: Vec<DecodedFrame> = runs.iter().flatten().cloned().collect();
    let flag = AtomicBool::new(false);
    let mut replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![inputs[0].clone()],
        )],
    )
    .unwrap();
    for input in &inputs[1..] {
        replay = replay
            .append(&[dashboard_base::test_fixtures::continuation_records(
                &replay,
                1,
                vec![input.clone()],
            )])
            .unwrap();
    }
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "columnar-divisions".into(),
            revision: "1".into(),
        },
        "columnar-divisions.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    assert_eq!(
        super::super::windows(source.replay_ref(), 1).len(),
        inputs.len()
    );

    // The row path over the same hours, a row block starting where split `split` restarts:
    // there every table reads as changed, as at a segment's first hour.
    let row = |split: usize| {
        let mut tracker = FrameChangeTracker::new();
        published(&source, |scan, ordinal| {
            if split > 0 && ordinal == split {
                tracker.invalidate();
            }
            let frame = &frames[ordinal];
            scan.prepare_changed(frame, &tracker.observe(frame, format))
        })
    };
    let (whole, _) = row(0);
    let names: std::collections::BTreeSet<_> =
        whole.keys().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "division-log-1",
            "division-log-count-keys-1-catalogue",
            "division-log-counts-1",
            "division-log-manifest-1",
        ]
        .into()
    );
    for (split, walks) in splits.into_iter().enumerate() {
        let mut projected = Vec::new();
        for (mut walker, mut hours) in walks {
            walker.end(&mut hours).unwrap();
            projected.extend(hours);
        }
        assert_eq!(projected.len(), inputs.len(), "restart at hour {split}");
        let mut projected = projected.into_iter();
        let (actual, kept) = published(&source, |scan, _| {
            scan.observe_projected(projected.next().expect("a projected hour"))
        });
        let (expected, events) = row(split);
        assert_payloads_equal(&actual, &expected, &format!("restart at hour {split}"));
        assert_eq!(kept, events, "restart at hour {split}");
    }
}

#[test]
fn columnar_division_log_publishes_what_the_row_path_does_from_any_restart() {
    let first = hours();
    let last = first.last().unwrap().hour;
    // A later session resuming the branch at the next hour, and after a gap.
    parity(&[first.clone(), later(last + 1)]);
    parity(&[first, later(last + 9)]);
}
