use super::*;
use crate::products::army_size::{columnar, test_fixtures};
use dashboard_base::{
    prepared::{
        frame_changes::FrameChangeTracker,
        series::tests::{Fixture, PublishedPayloads, assert_payloads_equal},
    },
    test_fixtures::{CAMPAIGN, records},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::rows::{
    CountryCounterInputRow, DivisionLandInputsRow, DivisionManpowerContributor, SentVolunteerRow,
    TableRows, TableSchema, VolunteerHostRow,
};
use replay_frame::frame::TableView;

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, carried: Option<TableId>) {
    let (read, sample) = if carried == Some(T::ID) {
        (false, Sample::Carried(frame.hour - 2))
    } else {
        (true, Sample::FromReadBit)
    };
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

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame, carried: Option<TableId>) {
    write::<tables::Campaign>(walk, frame, carried);
    write::<tables::Countries>(walk, frame, carried);
    write::<tables::CountryLinks>(walk, frame, carried);
    write::<tables::DivisionCensus>(walk, frame, carried);
    write::<tables::Divisions>(walk, frame, carried);
    write::<tables::DivisionTemplateIcons>(walk, frame, carried);
    write::<tables::DivisionLandInputs>(walk, frame, carried);
    write::<tables::LexicalTokens>(walk, frame, carried);
    write::<tables::SentVolunteers>(walk, frame, carried);
    write::<tables::VolunteerHosts>(walk, frame, carried);
    write::<tables::CountryCounterInputs>(walk, frame, carried);
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = test_fixtures::frame(hour);
    *frame.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(
            [1, 2]
                .into_iter()
                .map(|division| DivisionLandInputsRow {
                    division,
                    manpower: vec![
                        DivisionManpowerContributor {
                            country: 999,
                            amount: i32::MAX,
                        },
                        DivisionManpowerContributor {
                            country: 20,
                            amount: 7,
                        },
                    ],
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(
            [1, 1, 0, 99]
                .into_iter()
                .enumerate()
                .map(|(ordinal, division)| SentVolunteerRow {
                    country: 11,
                    ordinal: ordinal as u32,
                    division,
                })
                .collect(),
        ));
    *frame.tables[TableId::VolunteerHosts.index()] =
        TableView::Complete(TableRows::VolunteerHosts(vec![
            VolunteerHostRow {
                country: 10,
                position: 0,
                host: 20,
                deployed_divisions: -3,
                ..Default::default()
            },
            VolunteerHostRow {
                country: 10,
                position: 1,
                host: 20,
                deployed_divisions: 99,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            [10, 20, 30]
                .into_iter()
                .map(|country| CountryCounterInputRow {
                    country,
                    sent_expeditionaries: Some(
                        if country == 10 {
                            vec![1, 1, 0, 99]
                        } else {
                            Vec::new()
                        }
                        .into(),
                    )
                    .into(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

fn flush(prepared: &mut Prepared, fixture: &Fixture) {
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
}

/// The row path's payloads, with a row block starting at each of `starts`.
fn row_payloads(inputs: &[DecodedFrame], starts: &[usize], replay: &Replay) -> PublishedPayloads {
    let fixture = Fixture::new();
    let mut row = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, input) in inputs.iter().enumerate() {
        if starts.contains(&ordinal) {
            row.forget_runtime();
            tracker.invalidate();
        }
        row.observe(input, replay, &tracker.observe(input, replay.format))
            .unwrap();
        if input.hour % 3 == 0 {
            flush(&mut row, &fixture);
        }
    }
    flush(&mut row, &fixture);
    fixture.payloads()
}

fn emit(
    mut projector: Box<dyn Operator>,
    prepared: &mut Prepared,
    fixture: &Fixture,
) -> Vec<usize> {
    let hours = *projector
        .take()
        .unwrap()
        .downcast::<Vec<columnar::Hour>>()
        .unwrap();
    let mut handed = Vec::new();
    for hour in hours {
        let at = hour.hour;
        handed.push(hour.countries.len());
        prepared.observe_projected(hour).unwrap();
        if at % 3 == 0 {
            flush(prepared, fixture);
        }
    }
    handed
}

fn parity(
    label: &str,
    inputs: &[DecodedFrame],
    restart: Option<usize>,
    carried: Option<(i32, TableId)>,
) -> Vec<usize> {
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![inputs[0].clone()])]).unwrap();
    parity_on(label, inputs, restart, carried, &replay)
}

fn parity_on(
    label: &str,
    inputs: &[DecodedFrame],
    restart: Option<usize>,
    carried: Option<(i32, TableId)>,
    replay: &Replay,
) -> Vec<usize> {
    let mut walk = Synthetic::new(TABLES, 1, inputs[0].hour).unwrap();
    let fixtures = [Fixture::new(), Fixture::new()];
    let (mut row, mut projected) = (Prepared::default(), Prepared::default());
    let mut tracker = FrameChangeTracker::new();
    let mut projector: Box<dyn Operator> = Box::new(projected.projector(&replay).unwrap());
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].hour)
        .unwrap();
    let mut handed = Vec::new();
    for (ordinal, input) in inputs.iter().enumerate() {
        // A later run starts a row block, as a restart does.
        let run = ordinal > 0 && inputs[ordinal - 1].hour.checked_add(1) != Some(input.hour);
        if run {
            walk = Synthetic::new(TABLES, 1, input.hour).unwrap();
            handed.extend(emit(projector, &mut projected, &fixtures[1]));
            projector = Box::new(projected.projector(&replay).unwrap());
            assert!(projector.starts_at(Start::Run { continues: true }));
            projector
                .start(Start::Run { continues: true }, 1, input.hour)
                .unwrap();
        }
        let mut cold = false;
        if restart == Some(ordinal) {
            let mut next: Box<dyn Operator> = Box::new(projected.projector(&replay).unwrap());
            cold = next.starts_at(Start::Restart(walk.state()));
            assert_eq!(cold, projector.starts_at(Start::Restart(walk.state())));
            if ordinal > 0
                && !matches!(
                    &*inputs[ordinal - 1].tables[TableId::DivisionCensus.index()],
                    TableView::Complete(_)
                )
            {
                assert!(cold, "{label}: restart after a refused census");
            }
            if cold {
                handed.extend(emit(projector, &mut projected, &fixtures[1]));
                next.start(Start::Restart(walk.state()), 1, input.hour)
                    .unwrap();
                projector = next;
            }
        }
        write_hour(
            &mut walk,
            input,
            carried
                .filter(|(hour, _)| *hour == input.hour)
                .map(|(_, table)| table),
        );
        walk.end(|state, manifest, frame, starts| {
            // A segment starts where a row block does: its fold forgot the runtime state at the
            // block before's end, and every table reads as changed.
            if run || restart == Some(ordinal) {
                row.forget_runtime();
                tracker.invalidate();
            }
            row.observe(frame, &replay, &tracker.observe(frame, replay.format))
                .unwrap();
            if frame.hour % 3 == 0 {
                flush(&mut row, &fixtures[0]);
            }
            projector
                .hour(&segments::Hour {
                    branch: 1,
                    hour: frame.hour,
                    run: 0,
                    ordinal: ordinal as u32,
                    starts: starts || cold,
                    opens: starts || restart == Some(ordinal),
                    state,
                    manifest,
                })
                .unwrap();
        })
        .unwrap();
    }
    handed.extend(emit(projector, &mut projected, &fixtures[1]));
    flush(&mut row, &fixtures[0]);
    flush(&mut projected, &fixtures[1]);
    let expected = fixtures[0].payloads();
    let names: std::collections::BTreeSet<_> =
        expected.keys().map(|(name, _)| name.as_str()).collect();
    for section in [
        "army-1-catalogue",
        "army-parts-1",
        "army-coverage-1",
        "army-aid-1-catalogue",
        "army-aid-values-1",
        "army-aid-proof-1",
        "army-aid-occurrences-1",
    ] {
        assert!(names.contains(section), "{label}: {section}");
    }
    for index in 0..8 {
        assert!(
            names.contains(format!("army-values-1-{index}").as_str()),
            "{label}"
        );
        assert!(
            names.contains(format!("army-ranks-1-{index}").as_str()),
            "{label}"
        );
    }
    assert_payloads_equal(&fixtures[1].payloads(), &expected, label);
    handed
}

#[test]
fn partial_and_unavailable_inputs_restore_without_filling_unknowns() {
    for table in TABLES
        .iter()
        .copied()
        .filter(|table| *table != TableId::Campaign)
    {
        for partial in [false, true] {
            let mut refused = frame(102);
            let TableView::Complete(rows) = &*refused.tables[table.index()] else {
                unreachable!()
            };
            refused.tables[table.index()] = if partial {
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                }
            } else {
                TableView::Unavailable {
                    reason: "synthetic refusal".into(),
                }
            }
            .into();
            let inputs = [frame(100), frame(101), refused, frame(103), frame(104)];
            for restart in [None, Some(2), Some(3)] {
                parity(
                    &format!("{table:?}, partial={partial}, restart={restart:?}"),
                    &inputs,
                    restart,
                    None,
                );
            }
        }
    }
}

#[test]
fn owner_template_membership_tokens_and_identities_match_at_every_restart() {
    let mut inputs: Vec<_> = (100..=110).map(frame).collect();
    if let TableView::Complete(TableRows::DivisionLandInputs(rows)) =
        &mut *inputs[1].tables[TableId::DivisionLandInputs.index()]
    {
        rows[0].manpower[1].amount = 1;
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *inputs[2].tables[TableId::Divisions.index()]
    {
        rows[0].expeditionary_owner = u32::MAX;
        rows[0].logical_owner = 10;
        rows[0].roster_owner = 10;
        rows[0].template = 6;
        rows[1].hit_points = -1;
    }
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *inputs[2].tables[TableId::DivisionTemplateIcons.index()]
    {
        let mut added = rows[0].clone();
        added.template = 6;
        added.subunits[0].count = 7;
        rows.push(added);
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *inputs[3].tables[TableId::Divisions.index()]
    {
        rows.remove(0);
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *inputs[4].tables[TableId::Divisions.index()]
    {
        let mut added = rows[1].clone();
        added.id = 3;
        added.roster_owner = 30;
        rows.push(added);
    }
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *inputs[5].tables[TableId::LexicalTokens.index()]
    {
        rows[0].literal = b"renamed".to_vec();
    }
    if let TableView::Complete(TableRows::SentVolunteers(rows)) =
        &mut *inputs[5].tables[TableId::SentVolunteers.index()]
    {
        rows.remove(0);
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *inputs[6].tables[TableId::Countries.index()]
    {
        rows[2].id = 31;
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *inputs[7].tables[TableId::Divisions.index()]
    {
        rows[0].roster_owner = 999;
        rows[0].expeditionary_owner = u32::MAX;
    }
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *inputs[8].tables[TableId::DivisionCensus.index()]
    {
        rows.push(DivisionCensusRow {
            country: 999,
            grouped: 1,
            ungrouped: 0,
        });
    }
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *inputs[9].tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].count = -1;
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *inputs[9].tables[TableId::CountryCounterInputs.index()]
    {
        rows[0].sent_expeditionaries = None.into();
    }
    parity("whole script", &inputs, None, None);
    for restart in 1..inputs.len() {
        parity(
            &format!("script restart {restart}"),
            &inputs,
            Some(restart),
            None,
        );
    }
}

#[test]
fn carried_samples_do_not_invent_current_inventory_reads() {
    let inputs: Vec<_> = (100..=104).map(frame).collect();
    for table in [
        TableId::DivisionCensus,
        TableId::Divisions,
        TableId::DivisionTemplateIcons,
        TableId::DivisionLandInputs,
    ] {
        for restart in [None, Some(2), Some(3)] {
            parity(
                &format!("carried {table:?}, restart={restart:?}"),
                &inputs,
                restart,
                Some((102, table)),
            );
        }
    }
}

#[test]
fn a_second_projector_restarts_after_a_refused_census() {
    let mut inputs: Vec<_> = (100..=104).map(frame).collect();
    for input in &mut inputs[1..=2] {
        input.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
            reason: "synthetic census refusal".into(),
        }
        .into();
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *inputs[2].tables[TableId::Divisions.index()]
    {
        rows[0].expeditionary_owner = u32::MAX;
        rows[0].roster_owner = 30;
    }
    parity(
        "cold inventory after refused census",
        &inputs,
        Some(2),
        None,
    );
}

#[test]
fn a_census_that_changed_while_identities_were_refused_is_read_again_on_recovery() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100)])]).unwrap();
    let mut inputs: Vec<_> = (100..=103).map(frame).collect();
    for input in &mut inputs[2..] {
        if let TableView::Complete(TableRows::DivisionCensus(rows)) =
            &mut *input.tables[TableId::DivisionCensus.index()]
        {
            rows[1].grouped = 7;
        }
    }
    inputs[2].tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic identities refusal".into(),
    }
    .into();
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for input in &inputs {
        prepared
            .observe(input, &replay, &tracker.observe(input, replay.format))
            .unwrap();
    }
    let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &*inputs[3].tables[TableId::DivisionCensus.index()]
    else {
        panic!("complete census");
    };
    for row in rows {
        assert_eq!(
            prepared
                .state
                .census_rows
                .get(&row.country)
                .map(|&(_, count)| count),
            Some(i64::from(row.grouped) + i64::from(row.ungrouped)),
            "country {}",
            row.country
        );
    }
}

#[test]
fn census_changes_during_an_identity_refusal_match_at_every_restart() {
    let mut inputs: Vec<_> = (100..=106).map(frame).collect();
    for input in &mut inputs[2..=4] {
        if let TableView::Complete(TableRows::DivisionCensus(rows)) =
            &mut *input.tables[TableId::DivisionCensus.index()]
        {
            rows[1].grouped = 7;
        }
    }
    inputs[2].tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic identities refusal".into(),
    }
    .into();
    for restart in [None, Some(3), Some(4)] {
        parity(
            &format!("census after identities recover, restart={restart:?}"),
            &inputs,
            restart,
            None,
        );
    }
}

#[test]
fn one_divisions_hp_change_does_not_visit_fifteen_hundred_countries() {
    let mut first = frame(100);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows.extend((31..=1527).map(|id| replay_data::rows::CountryRow {
            id,
            canonical_slot: id,
            tag: format!("country {id}").into_bytes(),
            ..Default::default()
        }));
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *first.tables[TableId::CountryLinks.index()]
    {
        rows.extend((31..=1527).map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        }));
    }
    let mut quiet = first.clone();
    quiet.hour = 101;
    let mut another_quiet = quiet.clone();
    another_quiet.hour = 102;
    let mut second = first.clone();
    second.hour = 103;
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *second.tables[TableId::Divisions.index()]
    {
        rows[1].hit_points += 100_000;
    }
    let mut third = second.clone();
    third.hour = 104;
    assert_eq!(
        parity(
            "1500 countries",
            &[first, quiet, another_quiet, second, third],
            None,
            None
        ),
        [1500, 0, 0, 1, 0]
    );
}

#[test]
fn every_segment_starts_a_projector() {
    let replay = super::block::test_fixtures::recording();
    let projector = Prepared::default().projector(&replay).unwrap();
    let walk = Synthetic::new(TABLES, 1, 100).unwrap();
    assert!(projector.starts_at(Start::Run { continues: true }));
    assert!(projector.starts_at(Start::Restart(walk.state())));
    assert!(projector.starts_at(Start::Run { continues: false }));
}

#[test]
fn a_later_run_opening_with_refused_identities_reads_its_census_on_recovery() {
    let replay = super::block::test_fixtures::recording();
    let mut inputs: Vec<_> = [100, 101, 110, 111, 112].into_iter().map(frame).collect();
    for input in &mut inputs[2..] {
        if let TableView::Complete(TableRows::DivisionCensus(rows)) =
            &mut *input.tables[TableId::DivisionCensus.index()]
        {
            rows[1].grouped = 9;
        }
    }
    inputs[2].tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic identities refusal".into(),
    }
    .into();
    for restart in [None, Some(3)] {
        parity_on(
            &format!("a later run's refused opening, restart={restart:?}"),
            &inputs,
            restart,
            None,
            &replay,
        );
    }
}

#[test]
fn coverage_returns_at_the_first_hour_after_a_gap_even_when_nothing_changes() {
    let replay = super::block::test_fixtures::recording();
    // Hours 108 and 109 fall in the recording's gap; 108 changes a division, then nothing does.
    let mut inputs: Vec<_> = (100..=111).map(frame).collect();
    for input in &mut inputs[8..] {
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *input.tables[TableId::Divisions.index()]
        {
            rows[1].hit_points = 1_500_000;
        }
    }
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for input in &inputs[..=10] {
        prepared
            .observe(input, &replay, &tracker.observe(input, replay.format))
            .unwrap();
    }
    assert!(!prepared.scan.live.is_empty(), "coverage after the gap");
    for restart in [None, Some(9), Some(10)] {
        parity_on(
            &format!("a quiet hour after a gap, restart={restart:?}"),
            &inputs,
            restart,
            None,
            &replay,
        );
    }
}

#[test]
fn a_quiet_hour_before_a_gap_stays_in_its_runs_wherever_blocks_start() {
    let replay = super::block::test_fixtures::recording();
    // Nothing changes after hour 100; hours 108 and 109 fall in the recording's gap.
    let inputs: Vec<_> = (100..=111).map(frame).collect();
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for input in &inputs[..=7] {
        prepared
            .observe(input, &replay, &tracker.observe(input, replay.format))
            .unwrap();
    }
    let live = prepared.scan.live.clone();
    assert!(!live.is_empty());
    prepared
        .observe(
            &inputs[8],
            &replay,
            &tracker.observe(&inputs[8], replay.format),
        )
        .unwrap();
    for tag in &live {
        assert_eq!(
            prepared.scan.tracks[tag].runs.last().map(|run| run.last),
            Some(107),
            "{tag}"
        );
    }
    let whole = row_payloads(&inputs, &[], &replay);
    for start in [8, 9, 10] {
        assert_payloads_equal(
            &row_payloads(&inputs, &[start], &replay),
            &whole,
            &format!("a row block from hour {}", inputs[start].hour),
        );
        parity_on(
            &format!(
                "a quiet hour before a gap, restart at {}",
                inputs[start].hour
            ),
            &inputs,
            Some(start),
            None,
            &replay,
        );
    }
}

#[test]
fn carried_runs_and_gap_hours_match_the_row_paths_fast_path() {
    let replay = super::block::test_fixtures::recording();
    let inputs: Vec<_> = [100, 101, 110, 111, 119, 121]
        .into_iter()
        .map(frame)
        .collect();
    parity_on("crossing runs", &inputs, None, None, &replay);
    let mut inputs: Vec<_> = (100..=111).map(frame).collect();
    for input in &mut inputs[9..=10] {
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *input.tables[TableId::Divisions.index()]
        {
            rows[1].hit_points = 1_500_000;
        }
    }
    parity_on("observed hours inside gaps", &inputs, None, None, &replay);
}
