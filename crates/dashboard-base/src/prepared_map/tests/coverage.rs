//! Production sparse checkpoints preserve every recently audited map input.
use super::*;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame};
use replay::container::{Replay, ReplayStream, SeekResult};
use replay_data::rows::*;

fn inputs() -> Vec<TableRows> {
    vec![
        TableRows::AirDailyRoster(vec![AirDailyRosterRow::default()]),
        TableRows::AirGroundCrews(vec![AirGroundCrewRow::default()]),
        TableRows::AvailableAdvisors(vec![AvailableAdvisorRow::default()]),
        TableRows::CapturedOperativeReferences(vec![CapturedOperativeReferenceRow::default()]),
        TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
            army_count: Some(0),
            navy_count: Some(0),
            land_count: Some(0),
            air_state: CountryGameScoreInputRow::AIR_ABSENT,
            ..Default::default()
        }]),
        TableRows::DecisionRandomItems(vec![DecisionRandomItemRow::default()]),
        TableRows::Decisions(vec![DecisionRow::default()]),
        TableRows::OccupationSettings(vec![OccupationSettingsRow::default()]),
        TableRows::OperativeMemberships(vec![OperativeMembershipRow::default()]),
        TableRows::OwnerTransportSpeeds(vec![OwnerTransportSpeedRow::default()]),
        TableRows::PowerBalanceRanges(vec![PowerBalanceRangeRow::default()]),
        TableRows::ProjectPrograms(vec![ProjectProgramRow::default()]),
        TableRows::ReserveModifierTrees(vec![ReserveModifierTreeRow::default()]),
        TableRows::ScientistSkills(vec![ScientistSkillRow::default()]),
        TableRows::Scientists(vec![ScientistRow::default()]),
        TableRows::TimedIdeas(vec![TimedIdeaRow::default()]),
        TableRows::WarOwnedRelations(vec![WarOwnedRelationRow::default()]),
    ]
}

#[test]
fn sparse_production_selection_preserves_rows_refusals_partial_empty_and_supplying_hours() {
    let selected = selected_tables();
    let frames = (100..=105)
        .map(|hour| {
            let mut frame = scale_frame(1, hour, 1);
            for rows in inputs() {
                let table = rows.table();
                assert!(selected.contains(&table), "{table:?} must be prepared");
                frame.tables[table.index()] = match hour {
                    101 => TableView::Truncated {
                        rows,
                        missing: 5_001,
                    },
                    102 => TableView::Unavailable {
                        reason: format!("synthetic {table:?} refused"),
                    },
                    103 => TableView::Complete(TableRows::empty(table)),
                    _ => TableView::Complete(rows),
                }
                .into();
                frame.metrics.table_ns[table.index()] = if hour == 105 { 0 } else { 17 };
            }
            frame
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut builder =
        Builder::new(NonZeroU32::new(2).unwrap(), selected, Progress::default()).unwrap();
    builder.set_ranges(&replay);
    let mut stream = ReplayStream::new(&[TableId::Campaign]).unwrap();
    let mut writer = fixture.writer();
    let mut next_frame = 0;
    stream
        .next_block_with(
            &replay,
            |raw, frame| {
                next_frame += 1;
                builder.accept_sparse(
                    &mut writer,
                    &replay,
                    &raw,
                    frame,
                    Position {
                        block: 0,
                        next_frame,
                    },
                    || Ok(()),
                )
            },
            || Ok(()),
        )
        .unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(
        Store::local(
            fixture
                .directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap(),
        fixture.key.clone(),
        builder.progress().clone(),
        Arc::clone(&replay),
        selected,
    )
    .unwrap();
    for hour in [100, 101, 102, 103, 104, 105, 101, 104] {
        let Some(Current::Frame(actual)) = reader.seek(1, hour).unwrap() else {
            panic!("prepared frame at {hour}")
        };
        let SeekResult::Frame(expected) = replay.seek_selected(1, hour, Some(selected)).unwrap()
        else {
            panic!("recorded frame")
        };
        assert_eq!(actual.as_ref(), expected.as_ref(), "native state at {hour}");
        for rows in inputs() {
            let table = rows.table();
            assert_eq!(
                actual.tables[table.index()].sampled_at(),
                expected.tables[table.index()].sampled_at(),
                "{table:?} supplying hour at {hour}"
            );
        }
        if hour % 2 == 0 {
            assert_eq!(reader.source_reads, 0, "exact checkpoint");
        }
    }
}

#[test]
fn builder_and_reader_require_the_same_accepted_selection() {
    let selected = selected_tables();
    let progress = Builder::new(NonZeroU32::new(720).unwrap(), selected, Progress::default())
        .unwrap()
        .progress()
        .clone();
    let changed: Vec<_> = selected
        .iter()
        .copied()
        .filter(|table| *table != TableId::CountryGameScoreInputs)
        .collect();
    assert!(Builder::new(NonZeroU32::new(720).unwrap(), &changed, progress.clone()).is_err());
    let fixture = crate::prepared::series::tests::Fixture::new();
    let replay = Arc::new(
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 100, 1)])]).unwrap(),
    );
    let error = Reader::new(
        Store::local(
            fixture
                .directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap(),
        fixture.key.clone(),
        progress,
        replay,
        &changed,
    )
    .err()
    .expect("selection mismatch must not look complete");
    assert!(error.to_string().contains("selection differs"));
}
