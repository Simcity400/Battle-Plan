use super::*;
use replay_data::{
    rows::AirWingGameScoreStatsRow, rows::CountryGameScoreInputRow, rows::GameScoreFleetCount,
    rows::GameScoreMultiplierRow, rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

fn inputs(frame: &mut DecodedFrame, rows: Vec<CountryGameScoreInputRow>) {
    *frame.tables[TableId::CountryGameScoreInputs.index()] =
        TableView::Complete(TableRows::CountryGameScoreInputs(rows));
}

fn row() -> CountryGameScoreInputRow {
    CountryGameScoreInputRow {
        country: 101,
        army_count: Some(0),
        navy_count: Some(0),
        land_count: Some(2),
        air_state: CountryGameScoreInputRow::AIR_ABSENT,
        ..Default::default()
    }
}

#[test]
fn game_score_rosters_compare_order_counts_and_missing_zero() {
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    let tag = saved.countries.keys().next().unwrap().clone();
    saved
        .counter_inputs
        .division_orders
        .insert(tag.clone(), vec![11, 12]);
    saved.counter_inputs.fleet_orders.insert(tag, vec![9, 2]);
    saved.counter_inputs.fleets = BTreeMap::from([
        (
            9,
            content_reader_save::counter_inputs::Fleet {
                task_forces: vec![31, 30],
                ..Default::default()
            },
        ),
        (2, content_reader_save::counter_inputs::Fleet::default()),
    ]);
    let mut input = row();
    input.army_count = Some(2);
    input.navy_count = Some(2);
    input.fleets = vec![
        GameScoreFleetCount {
            fleet: 9,
            task_forces: Some(2),
            error: Vec::new(),
        },
        GameScoreFleetCount {
            fleet: 2,
            task_forces: Some(0),
            error: Vec::new(),
        },
    ];
    inputs(&mut frame, vec![input.clone()]);
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    rosters(&frame, &saved, &context.links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    input.fleets.reverse();
    input.fleets[0].task_forces = None;
    input.fleets[0].error = b"unreadable fleet header".to_vec();
    input.navy_count = None;
    input.navy_error = b"unreadable fleet header".to_vec();
    input.army_count = Some(1);
    drop(context);
    inputs(&mut frame, vec![input]);
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    rosters(&frame, &saved, &context.links, &mut report).unwrap();
    for field in ["army_count", "fleet_order", "fleet 2.task_forces"] {
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(field))
        );
    }
    drop(context);
    inputs(&mut frame, Vec::new());
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    rosters(&frame, &saved, &context.links, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("saved country present"))
    );
}

#[test]
fn game_score_land_keeps_unowned_provinces_out_of_positive_country_counts() {
    let (mut frame, mut saved, membership, _) = super::super::fixtures::fixture();
    saved.states.get_mut(&1).unwrap().owner = None;
    saved.states.get_mut(&1).unwrap().controller = None;
    saved.provinces.clear();
    let mut input = row();
    input.land_count = Some(0);
    inputs(&mut frame, vec![input.clone()]);
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    land(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["CountryGameScoreInputs.land_count"],
        1
    );
    drop(context);
    input.land_count = Some(1);
    inputs(&mut frame, vec![input]);
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    land(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn game_score_land_uses_state_owner_omissions_and_refuses_unknown_membership() {
    let (mut frame, mut saved, mut membership, _) = super::super::fixtures::fixture();
    let mut other = saved.countries.values().next().unwrap().clone();
    other.tag = b"BBB".to_vec();
    saved.countries.insert(other.tag.clone(), other);
    saved.states.get_mut(&1).unwrap().controller = Some(b"BBB".to_vec());
    inputs(
        &mut frame,
        vec![
            row(),
            CountryGameScoreInputRow {
                country: 202,
                land_count: Some(0),
                ..row()
            },
        ],
    );
    let context = super::super::country_context(&frame).unwrap();
    let other = CountryRow {
        id: 202,
        tag: b"BBB".to_vec(),
        ..Default::default()
    };
    let mut links = context.links;
    links.insert(202, &other);
    let mut report = Report::default();
    land(&frame, &saved, &membership, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    membership.provinces.insert(
        1,
        ProvinceMembership::Unknown(std::collections::BTreeSet::from([1])),
    );
    assert!(
        land(&frame, &saved, &membership, &links, &mut report)
            .unwrap_err()
            .to_string()
            .contains("1 unknown provinces")
    );
}

#[test]
fn game_score_wings_compare_zero_missing_and_extra_without_inventing_cached_stats() {
    let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
    saved.air_wings.insert(
        11,
        content_reader_save::AirWing {
            government_in_exile_tag: None,
            manpower: None,

            reinforcement_setting: Some(0),
            id: 11,
            owner: b"GER".to_vec(),
            country_override: b"GER".to_vec(),
            air_base: 20,
            planes: 0,
            mission_mask: 0,
            name: Some(Vec::new()),
            mission_region: None,
            mission_active: None,
            stop_training_at_max_xp: false,
            region_change_penalty: None,
            transfer_progress: None,
            transferring_to: None,
            pool_ordinal: 0,
        },
    );
    let mut value = AirWingGameScoreStatsRow {
        id: 11,
        planes: Some(0),
        stat_error: b"unreadable selected stat".to_vec(),
        mission_error: b"unreadable selector".to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::AirWingGameScoreStats.index()] =
        TableView::Complete(TableRows::AirWingGameScoreStats(vec![value.clone()]));
    let mut report = Report::default();
    wings(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|limitation| limitation.contains("stat_raw / mission_index"))
    );
    *frame.tables[TableId::AirWingGameScoreStats.index()] =
        TableView::Complete(TableRows::AirWingGameScoreStats(Vec::new()));
    let mut report = Report::default();
    wings(&frame, &saved, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("absent"))
    );
    value.planes = None;
    value.planes_error = b"unreadable count".to_vec();
    *frame.tables[TableId::AirWingGameScoreStats.index()] =
        TableView::Complete(TableRows::AirWingGameScoreStats(vec![value]));
    let mut report = Report::default();
    wings(&frame, &saved, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains(".planes"))
    );
    saved.air_wings.clear();
    let mut report = Report::default();
    wings(&frame, &saved, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("absent"))
    );
    *frame.tables[TableId::GameScoreMultipliers.index()] =
        TableView::Complete(TableRows::GameScoreMultipliers(vec![
            GameScoreMultiplierRow {
                total_raw: Some(0),
                army_raw: Some(0),
                navy_raw: Some(0),
                air_raw: Some(0),
                industry_raw: Some(0),
                land_raw: Some(0),
                ..Default::default()
            },
        ]));
    multipliers(&frame, &mut report).unwrap();
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|limitation| limitation == "GameScoreMultipliers")
    );
}

#[test]
fn game_score_oracle_names_unsaved_industry_effective_totals() {
    let (mut frame, _, _, _) = super::super::fixtures::fixture();
    *frame.tables[TableId::GameScoreMultipliers.index()] = TableView::Unavailable {
        reason: replay_data::format::GAME_SCORE_DISABLED_REASON.into(),
    };
    let mut report = Report::default();
    assert!(multipliers(&frame, &mut report).is_err());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|limitation| limitation
                == "Game score Industry / CountryFactories effective totals and transfers")
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("production save writer")
                && reason.contains("saved building inputs"))
    );
}

#[test]
fn game_score_reserved_object_does_not_block_positive_country_comparisons() {
    let (mut frame, saved, membership, _) = super::super::fixtures::fixture();
    inputs(
        &mut frame,
        vec![
            CountryGameScoreInputRow {
                country: 0,
                ..row()
            },
            row(),
        ],
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    rosters(&frame, &saved, &context.links, &mut report).unwrap();
    land(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["CountryGameScoreInputs saved rosters"],
        1
    );
    assert_eq!(
        report.compared_tables["CountryGameScoreInputs.land_count"],
        1
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("omits reserved slot zero"))
    );
    drop(context);
    inputs(
        &mut frame,
        vec![
            CountryGameScoreInputRow {
                country: 0,
                ..row()
            },
            CountryGameScoreInputRow {
                army_count: Some(1),
                land_count: Some(3),
                ..row()
            },
        ],
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    rosters(&frame, &saved, &context.links, &mut report).unwrap();
    land(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("army_count"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("land_count"))
    );
    assert!(
        country_inputs(&frame, &BTreeMap::new(), &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("country object 101")
    );
}
