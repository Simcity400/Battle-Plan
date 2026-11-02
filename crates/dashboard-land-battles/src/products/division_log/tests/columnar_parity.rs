//! The division log's columnar path ([`super::super::columnar`]) against its row path on the
//! same hours, written as the native recorder writes them: walked whole, from a restart at
//! every hour, and through segments its walk goes on through, written out and read back
//! between them, then on into later runs of the branch: every prepared section equal.

use super::super::columnar::{Hour, Projector};
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    series::tests::{Fixture, assert_payloads_equal},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{format::FormatVersion, rows::TableSchema, scope::CaptureMode};

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

/// Branch 1 of a replay holding each of `inputs` as a block of its own.
fn source(inputs: &[DecodedFrame]) -> Source {
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
    source
}

/// Branch 1 of a replay holding `inputs` in as few blocks as it records them in.
fn one_block(inputs: &[DecodedFrame]) -> Source {
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, inputs.to_vec())],
    )
    .unwrap();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "columnar-divisions-block".into(),
            revision: "1".into(),
        },
        "columnar-divisions-block.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        super::super::windows(source.replay_ref(), 1).len() < inputs.len(),
        "a block holds several hours"
    );
    source
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

/// Every division log section the native emit publishes, keeping `hours` of `source` and saved
/// and taken up again after every hour.
fn emitted(
    source: &Source,
    hours: Vec<Hour>,
) -> dashboard_base::prepared::series::tests::PublishedPayloads {
    use crate::products::division_log::emit::Emit;
    let (flag, context) = (AtomicBool::new(false), LoadContext { content: None });
    let fixture = Fixture::new();
    let replay = source.replay_ref();
    let names = || crate::products::division_log::names_content(source, &context);
    let mut emit = Emit::new(replay, 1, names());
    for hour in hours {
        let mut writer = fixture.writer();
        emit.observe(source, &mut writer, hour, &flag).unwrap();
        writer.commit().unwrap();
        emit = Emit::load(replay, 1, replay.format, names(), &emit.save().unwrap()).unwrap();
    }
    fixture.payloads()
}

/// The totals the emit publishes over `hours` of one replay block of `source`, flushed, saved
/// and taken up again after every hour: through each, as the row path counts them.
fn totals_through_every_flush(source: &Source, hours: Vec<Hour>) {
    use crate::products::division_log::{DivisionLogTotals, DivisionLogTotalsParams, emit::Emit};
    use crate::products::land_battles::LandBattleCountryFilter;
    let (flag, context) = (AtomicBool::new(false), LoadContext { content: None });
    let fixture = Fixture::new();
    let replay = source.replay_ref();
    let names = || crate::products::division_log::names_content(source, &context);
    let first = hours.first().expect("an hour").hour;
    let mut emit = Emit::new(replay, 1, names());
    for hour in hours {
        let at = hour.hour;
        let mut writer = fixture.writer();
        emit.observe(source, &mut writer, hour, &flag).unwrap();
        emit.flush(source, &mut writer, None, &flag).unwrap();
        writer.commit().unwrap();
        emit = Emit::load(replay, 1, replay.format, names(), &emit.save().unwrap()).unwrap();
        let params = DivisionLogTotalsParams {
            branch_id: "1".into(),
            first_hour: first,
            last_hour: at,
            countries: LandBattleCountryFilter {
                tags: None,
                membership: None,
                unobserved: Vec::new(),
            },
            templates: None,
        };
        let published =
            super::super::prepared::totals(&params, &fixture.store, &fixture.key, 1, &flag)
                .unwrap();
        let counted = DivisionLogTotals::load(source, &params, &context, &flag).unwrap();
        assert_eq!(
            serde_json::to_value(published).unwrap(),
            serde_json::to_value(counted).unwrap(),
            "totals through hour {at}"
        );
    }
}

/// `runs` of branch 1 prepared by the row path and by segment walks restarting before every
/// hour of the first run: every section equal.
fn parity(runs: &[Vec<DecodedFrame>]) {
    let format = FormatVersion::CURRENT;
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
        let mut world_held: Option<dashboard_world::World> = None;
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
                .end(|state, manifest, frame, walk_starts| {
                    let world = dashboard_world::World::at(
                        &mut world_held,
                        walk_starts,
                        &crate::products::division_log::columnar::needs().unwrap(),
                        (
                            replay_data::format::FormatVersion::CURRENT,
                            replay_data::scope::CaptureMode::Full,
                        ),
                        state,
                        manifest,
                    )
                    .unwrap();
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
                                    world,
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
    let source = source(&inputs);

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
        if split == 0 {
            let copy: Vec<Hour> =
                serde_json::from_value(serde_json::to_value(&projected).unwrap()).unwrap();
            let (expected, _) = row(0);
            assert_payloads_equal(&emitted(&source, copy), &expected, "the native emit");
            let again: Vec<Hour> =
                serde_json::from_value(serde_json::to_value(&projected).unwrap()).unwrap();
            totals_through_every_flush(&one_block(&inputs), again);
        }
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

/// Template `index`'s handle.
fn template_handle(index: u32) -> u64 {
    u64::from(index) << 32 | 4714
}

/// One division of a [`Campaign`].
struct Unit {
    owner: u32,
    province: u32,
    template: u32,
    name: Vec<u8>,
    hidden: bool,
}

/// A campaign hour by hour: its countries, templates and divisions, and the hour's removal
/// witnesses and deaths.
struct Campaign {
    tags: Vec<Vec<u8>>,
    templates: BTreeMap<u32, Vec<u8>>,
    divisions: BTreeMap<u32, Unit>,
    witnesses: Vec<DivisionEquipmentDeletionRow>,
    deaths: Vec<DestroyedDivisionsRow>,
    covered: bool,
    next: u32,
    killed: u64,
    /// A division away on a temporary transfer, and the step it comes back at.
    away: Option<(u32, Unit, u32)>,
}

/// The witness of division `index` away on a temporary transfer.
fn transfer(index: u32, unit: &Unit) -> DivisionEquipmentDeletionRow {
    DivisionEquipmentDeletionRow {
        division_handle: handle(index),
        country: unit.owner,
        expeditionary_owner: u32::MAX,
        disposition: Disposition::VolunteerTransferred,
        cause: None,
    }
}

impl Campaign {
    /// `countries` countries with a template each, and `divisions` divisions among them.
    fn new(countries: u32, divisions: u32) -> Self {
        Self {
            tags: (1..=countries)
                .map(|id| format!("C{id:04}").into_bytes())
                .collect(),
            templates: (1..=countries)
                .map(|index| (index, format!("Template {index}").into_bytes()))
                .collect(),
            divisions: (1..=divisions)
                .map(|index| {
                    let owner = (index - 1) % countries + 1;
                    let unit = Unit {
                        owner,
                        province: index,
                        template: owner,
                        name: format!("Unit {index}").into_bytes(),
                        hidden: false,
                    };
                    (index, unit)
                })
                .collect(),
            witnesses: Vec::new(),
            deaths: Vec::new(),
            covered: true,
            next: divisions + 1,
            killed: 0,
            away: None,
        }
    }

    fn countries(&self) -> u32 {
        self.tags.len() as u32
    }

    /// The hour's frame, as the row decoder reads it.
    fn frame(&self, hour: i32) -> DecodedFrame {
        let countries = self.countries();
        let mut witnesses = self.witnesses.clone();
        witnesses.sort_by_key(|row| row.division_handle);
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(0));
        let mut set = |table: TableId, rows: TableRows| {
            *frame.tables[table.index()] = TableView::Complete(rows);
        };
        set(
            TableId::Countries,
            TableRows::Countries(
                (1..)
                    .zip(&self.tags)
                    .map(|(id, tag)| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: tag.clone(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            TableId::CountryLinks,
            TableRows::CountryLinks(
                (1..=countries)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ),
        );
        set(
            TableId::Divisions,
            TableRows::Divisions(
                self.divisions
                    .iter()
                    .map(|(&index, unit)| DivisionRow {
                        id: handle(index),
                        roster_owner: unit.owner,
                        logical_owner: unit.owner,
                        expeditionary_owner: u32::MAX,
                        province: unit.province,
                        template: template_handle(unit.template),
                        reveal_skip: u8::from(unit.hidden),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            TableId::DivisionStatus,
            TableRows::DivisionStatus(
                self.divisions
                    .iter()
                    .map(|(&index, unit)| DivisionStatusRow {
                        division: handle(index),
                        name: Recorded::Value(Some(unit.name.clone())),
                        naming: Recorded::Value(Some(NameOperands {
                            custom: unit.name.clone(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            TableId::DivisionTemplates,
            TableRows::DivisionTemplates(
                self.templates
                    .iter()
                    .map(|(&index, name)| DivisionTemplateRow {
                        template: template_handle(index),
                        owner: (index - 1) % countries + 1,
                        name: name.clone(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        // Each template's current display name equals its dictionary row.
        set(
            TableId::DivisionTemplateIcons,
            TableRows::DivisionTemplateIcons(
                self.templates
                    .keys()
                    .map(|&index| replay_data::rows::DivisionTemplateIconRow {
                        template: template_handle(index),
                        country: ((index - 1) % countries + 1) as i32,
                        priority_unit: index,
                        priority_unit_name: format!("infantry {index}").into_bytes(),
                        custom_icon: (index % 3) as i32 - 1,
                        display_name: Recorded::Value(Some(Default::default())),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            TableId::DivisionEquipmentDeletions,
            TableRows::DivisionEquipmentDeletions(witnesses),
        );
        set(
            TableId::DestroyedDivisions,
            TableRows::DestroyedDivisions(self.deaths.clone()),
        );
        set(
            TableId::LandEventCoverage,
            TableRows::LandEventCoverage(vec![LandEventCoverageRow {
                from_hour: hour - 1,
                to_hour: hour,
                complete: self.covered,
                lost_events: 73,
                ..Default::default()
            }]),
        );
        frame
    }

    /// Among the divisions shown, the one `salt` picks.
    fn pick(&self, salt: u32) -> Option<u32> {
        let shown: Vec<u32> = self
            .divisions
            .iter()
            .filter(|(_, unit)| !unit.hidden)
            .map(|(&index, _)| index)
            .collect();
        (!shown.is_empty()).then(|| shown[(salt as usize * 7 + 3) % shown.len()])
    }

    /// A death of division `index` read at `hour`, at the province it was last in.
    fn die(&mut self, index: u32, unit: &Unit, hour: i32) {
        self.killed += 1;
        self.deaths.push(DestroyedDivisionsRow {
            id: self.killed,
            division: handle(index),
            owner: unit.owner,
            killer_country: unit.owner % self.countries() + 1,
            killer_division: Some(handle(index + 1)),
            province: unit.province,
            hour,
            ..Default::default()
        });
    }

    /// Division `index` removed at step `step`, read at `hour`, with the witness the step
    /// chooses: a loss with one death, an overrun with two, a merge, a return, or a temporary
    /// transfer whose witness stays until the division comes back.
    fn remove(&mut self, step: u32, index: u32, hour: i32) {
        let Some(unit) = self.divisions.remove(&index) else {
            return;
        };
        let witness = |disposition, cause| DivisionEquipmentDeletionRow {
            division_handle: handle(index),
            country: unit.owner,
            expeditionary_owner: u32::MAX,
            disposition,
            cause,
        };
        match step / 4 % 5 {
            0 => {
                self.witnesses
                    .push(witness(Disposition::Loss, Some(Cause::DivisionDestroyed)));
                self.die(index, &unit, hour);
            }
            1 => {
                self.witnesses
                    .push(witness(Disposition::Loss, Some(Cause::DivisionOverrun)));
                self.die(index, &unit, hour);
                self.die(index, &unit, hour);
            }
            2 => self.witnesses.push(witness(Disposition::Merged, None)),
            3 => self.witnesses.push(witness(Disposition::Returned, None)),
            _ if self.away.is_none() => {
                self.witnesses.push(transfer(index, &unit));
                self.away = Some((index, unit, step + 3));
            }
            _ => self.witnesses.push(witness(Disposition::Merged, None)),
        }
    }

    /// What step `step`, read at `hour`, changes: divisions move, are renamed, created,
    /// hidden as intelligence copies and shown again, and lost with a witness or without one;
    /// templates are renamed and added, and countries take other tags.
    fn churn(&mut self, step: u32, hour: i32) {
        for salt in 0..3 {
            if let Some(index) = self.pick(step * 3 + salt) {
                self.divisions.get_mut(&index).expect("a division").province += 1;
            }
        }
        if step % 3 == 1
            && let Some(index) = self.pick(step + 11)
        {
            self.divisions.get_mut(&index).expect("a division").name =
                format!("Unit {index} renamed at {step}").into_bytes();
        }
        if step % 5 == 2 || step == 26 {
            let at = step as usize % self.templates.len();
            let index = *self.templates.keys().nth(at).expect("a template");
            self.templates.insert(
                index,
                format!("Template {index} renamed at {step}").into_bytes(),
            );
        }
        if step % 7 == 3 {
            for owner in [
                step % self.countries() + 1,
                (step + 5) % self.countries() + 1,
            ] {
                let index = self.next;
                self.next += 1;
                let unit = Unit {
                    owner,
                    province: index,
                    template: owner,
                    name: format!("Unit {index}").into_bytes(),
                    hidden: false,
                };
                self.divisions.insert(index, unit);
            }
        }
        if step == 21 {
            let template = self.templates.keys().last().expect("a template") + 1;
            self.templates
                .insert(template, format!("Template {template}").into_bytes());
            let index = self.next;
            self.next += 1;
            let unit = Unit {
                owner: 1,
                province: index,
                template,
                name: format!("Unit {index}").into_bytes(),
                hidden: false,
            };
            self.divisions.insert(index, unit);
        }
        if step % 11 == 0 && step > 0 {
            let at = step as usize % self.tags.len();
            self.tags[at] = format!("R{step:03}").into_bytes();
        }
        if step % 9 == 5
            && let Some(index) = self.pick(step + 5)
        {
            self.divisions.get_mut(&index).expect("a division").hidden = true;
        }
        if step % 9 == 8 {
            for unit in self.divisions.values_mut() {
                unit.hidden = false;
            }
        }
        if step == 14
            && let Some(index) = self.pick(step)
        {
            self.divisions.remove(&index);
        }
        if (step % 4 == 0 && step > 0 || step == 26)
            && let Some(index) = self.pick(step + 1)
        {
            self.remove(step, index, hour);
        }
    }

    /// Step `step` of the churn, read at `hour`: every eighth step quiet, and some tables read
    /// in part or not at all.
    fn step(&mut self, step: u32, hour: i32) -> DecodedFrame {
        self.witnesses.clear();
        self.deaths.clear();
        self.covered = step % 13 != 4;
        if let Some((index, unit, back)) = self.away.take() {
            if step >= back {
                self.divisions.insert(index, unit);
            } else {
                self.witnesses.push(transfer(index, &unit));
                self.away = Some((index, unit, back));
            }
        }
        if step % 8 != 7 {
            self.churn(step, hour);
        }
        let frame = self.frame(hour);
        match step {
            10 => truncated(frame, TableId::Divisions),
            12 => truncated(frame, TableId::DivisionStatus),
            16 => refused(frame, TableId::DivisionTemplateIcons, "icons refused"),
            17 => refused(frame, TableId::DivisionTemplateIcons, "icons refused again"),
            19 => unavailable(frame, TableId::CountryLinks),
            22 => unavailable(frame, TableId::DivisionTemplates),
            26 => unavailable(frame, TableId::Divisions),
            29 => unavailable(frame, TableId::DivisionEquipmentDeletions),
            30 => unavailable(frame, TableId::Countries),
            _ => frame,
        }
    }
}

/// A churned campaign's runs of branch 1: 36 hours from 100, ten more resuming at the next
/// hour, and eight after a gap.
fn churned() -> Vec<Vec<DecodedFrame>> {
    let mut campaign = Campaign::new(12, 40);
    let (mut step, mut hour) = (0, 100);
    let mut runs = Vec::new();
    for (length, gap) in [(36, 0), (10, 0), (8, 5)] {
        hour += gap;
        let mut run = Vec::new();
        for _ in 0..length {
            run.push(campaign.step(step, hour));
            (step, hour) = (step + 1, hour + 1);
        }
        runs.push(run);
    }
    runs
}

/// How a walk goes on where a segment opens.
#[derive(Clone, Copy)]
enum Opening {
    /// Its operator goes on.
    Chained,
    /// Its operator is parked, written out and read back.
    Parked,
    /// A new operator starts from the restart, where one can.
    Restarted,
}

/// Segments of the churned campaign's first run: one after a quiet hour, whose walk names its
/// templates at its end, restarts the projection cannot take, and a parked census whose naming
/// moved after its build.
const OPENINGS: [(u32, Opening); 8] = [
    (5, Opening::Chained),
    (8, Opening::Restarted),
    (11, Opening::Restarted),
    (16, Opening::Restarted),
    (20, Opening::Parked),
    (24, Opening::Restarted),
    (27, Opening::Parked),
    (32, Opening::Chained),
];

/// A new operator taking up what `operator` carries, as the continuation store reads a spilled
/// carry back; it writes out again what it read.
fn spilled(operator: &Projector) -> Projector {
    assert!(operator.carried().is_some_and(|bytes| bytes > 0));
    let mut bytes = Vec::new();
    assert!(operator.save(&mut bytes).unwrap());
    let mut next = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    next.load(&bytes).unwrap();
    let mut again = Vec::new();
    assert!(next.save(&mut again).unwrap());
    assert!(again == bytes, "a carry read back writes what it read");
    next
}

/// What `runs` of branch 1 project through segments opening in the first run where `openings`
/// say and into each later run, parked there when `parked`, the operator written out and read
/// back after every hour when `spill`: every projected hour, the row decoder's frames, and the
/// ordinals in the branch of the hours segments open at.
fn walked(
    runs: &[Vec<DecodedFrame>],
    openings: &[(u32, Opening)],
    parked: bool,
    spill: bool,
) -> (Vec<Hour>, Vec<DecodedFrame>, Vec<usize>) {
    let projector = || Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut operator = projector();
    let mut hours: Vec<Hour> = Vec::new();
    let (mut frames, mut opened) = (Vec::new(), Vec::new());
    for (run, inputs) in (0..).zip(runs) {
        let mut walk = Synthetic::new(&TABLES, 1, inputs[0].hour).unwrap();
        let mut world_held: Option<dashboard_world::World> = None;
        if run == 0 {
            operator
                .start(Start::Run { continues: false }, 1, inputs[0].hour)
                .unwrap();
        } else {
            operator.end(&mut hours).unwrap();
            if parked {
                operator = spilled(&operator);
            }
            operator.cross(inputs[0].hour).unwrap();
        }
        for (ordinal, input) in (0..).zip(inputs) {
            let opening = openings
                .iter()
                .find(|&&(at, _)| run == 0 && at > 0 && at == ordinal)
                .map(|&(_, opening)| opening);
            let mut starts = ordinal == 0;
            if let Some(opening) = opening {
                operator.end(&mut hours).unwrap();
                match opening {
                    Opening::Chained => {}
                    Opening::Parked => operator = spilled(&operator),
                    Opening::Restarted => {
                        let restart = Start::Restart(walk.state());
                        let mut next = projector();
                        if next.starts_at(restart) {
                            next.start(restart, 1, input.hour).unwrap();
                            (operator, starts) = (next, true);
                        }
                    }
                }
            }
            let opens = ordinal == 0 || opening.is_some();
            if opens {
                opened.push(frames.len());
            }
            write_hour(&mut walk, input);
            let frame = walk
                .end(|state, manifest, frame, walk_starts| {
                    let world = dashboard_world::World::at(
                        &mut world_held,
                        walk_starts,
                        &crate::products::division_log::columnar::needs().unwrap(),
                        (
                            replay_data::format::FormatVersion::CURRENT,
                            replay_data::scope::CaptureMode::Full,
                        ),
                        state,
                        manifest,
                    )
                    .unwrap();
                    operator
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
                                world,
                            },
                            &mut hours,
                        )
                        .unwrap();
                    frame.clone()
                })
                .unwrap();
            frames.push(frame);
            if spill {
                operator = spilled(&operator);
            }
        }
    }
    operator.end(&mut hours).unwrap();
    (hours, frames, opened)
}

/// `runs` walked as [`walked`] walks them publish every section the row path does, a row block
/// opening wherever a segment does: the projected hours, as they serialize.
fn check(
    runs: &[Vec<DecodedFrame>],
    openings: &[(u32, Opening)],
    parked: bool,
    spill: bool,
) -> serde_json::Value {
    let (hours, frames, opened) = walked(runs, openings, parked, spill);
    let inputs: Vec<DecodedFrame> = runs.iter().flatten().cloned().collect();
    assert_eq!(hours.len(), inputs.len());
    let projected = serde_json::to_value(&hours).unwrap();
    let source = source(&inputs);
    let mut hours = hours.into_iter();
    let (actual, kept) = published(&source, |scan, _| {
        scan.observe_projected(hours.next().expect("a projected hour"))
    });
    let mut tracker = FrameChangeTracker::new();
    let (expected, events) = published(&source, |scan, ordinal| {
        if opened.contains(&ordinal) {
            tracker.invalidate();
        }
        let frame = &frames[ordinal];
        scan.prepare_changed(frame, &tracker.observe(frame, FormatVersion::CURRENT))
    });
    assert_payloads_equal(&actual, &expected, "segments");
    assert_eq!(kept, events);
    projected
}

#[test]
fn columnar_division_log_publishes_what_the_row_path_does_through_its_segments() {
    let runs = churned();
    check(&runs, &[], false, false);
    check(&runs, &OPENINGS, true, false);
    // A segment opening every hour reads every table as changed there, as a row block's first
    // hour does.
    let every: Vec<(u32, Opening)> = (1..runs[0].len() as u32)
        .map(|at| {
            let opening = if at % 2 == 0 {
                Opening::Parked
            } else {
                Opening::Chained
            };
            (at, opening)
        })
        .collect();
    check(&runs, &every, true, false);
}

#[test]
fn a_division_log_walk_written_out_after_any_hour_goes_on_as_it_would_have() {
    let runs = churned();
    let resident = check(&runs, &OPENINGS, true, false);
    let spilled = check(&runs, &OPENINGS, true, true);
    assert_eq!(spilled, resident);
}

#[test]
fn fifteen_hundred_countries_read_only_what_each_hour_changed() {
    let mut campaign = Campaign::new(1_500, 3_000);
    let mut frames = vec![campaign.frame(100)];
    for index in (1..=10).map(|n| n * 97) {
        campaign.divisions.get_mut(&index).unwrap().province += 1;
    }
    frames.push(campaign.frame(101));
    campaign.divisions.get_mut(&7).unwrap().name = b"Unit 7 renamed".to_vec();
    frames.push(campaign.frame(102));
    campaign.templates.insert(5, b"Template 5 renamed".to_vec());
    frames.push(campaign.frame(103));
    campaign.tags[8] = b"RENAMED".to_vec();
    frames.push(campaign.frame(104));
    let created = Unit {
        owner: 1_500,
        province: 1,
        template: 1_500,
        name: b"Unit created".to_vec(),
        hidden: false,
    };
    campaign.divisions.insert(campaign.next, created);
    let lost = campaign.divisions.remove(&11).unwrap();
    campaign.witnesses.push(DivisionEquipmentDeletionRow {
        division_handle: handle(11),
        country: lost.owner,
        expeditionary_owner: u32::MAX,
        disposition: Disposition::Loss,
        cause: Some(Cause::DivisionDestroyed),
    });
    campaign.die(11, &lost, 105);
    frames.push(campaign.frame(105));

    let mut walk = Synthetic::new(&TABLES, 1, 100).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    projector
        .start(Start::Run { continues: false }, 1, 100)
        .unwrap();
    let mut hours: Vec<Hour> = Vec::new();
    let mut reads = Vec::new();
    for (ordinal, input) in (0..).zip(&frames) {
        write_hour(&mut walk, input);
        walk.end(|state, manifest, frame, walk_starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                walk_starts,
                &crate::products::division_log::columnar::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
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
                        starts: ordinal == 0,
                        opens: ordinal == 0,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        reads.push(projector.reads());
    }
    // The first hour names each division as first seen and as the census reads it; each hour
    // after reads only the names it wrote. The naming rows are the walk's frame's.
    assert_eq!(reads[0], 6_000);
    let read: Vec<usize> = reads.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert_eq!(read, [0, 1, 0, 0, 2]);
    check(&[frames], &[(3, Opening::Parked)], false, false);
}
