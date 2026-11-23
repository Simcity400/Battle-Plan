use super::*;
use replay_data::{rows::CountryFactoriesRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::Metrics, frame::TableView};

fn save(ger: &str, ita: &str) -> Save {
    content_reader_save::parse(
        format!(
            r#"HOI4txt
date="1936.1.2.17" game_unique_id="campaign" version="build" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={{ GER={{ {ger} buildings={{ arms={{level=2 healthy_levels=2 partial_health=0}} }} }} ITA={{ {ita} }} }}
states={{
  1={{owner="GER" buildings={{ civ={{level=5 healthy_levels=3 partial_health=0}} arms={{level=1 healthy_levels=1 partial_health=0}} infrastructure={{level=4 healthy_levels=4 partial_health=0}} }} }}
  2={{owner="ITA" controller="GER" buildings={{ civ={{level=2 healthy_levels=2 partial_health=0}} yard={{level=3 healthy_levels=3 partial_health=0}} }} }}
  3={{owner="ITA" buildings={{ both={{level=1 healthy_levels=1 partial_health=0}} }} }}
}}
provinces={{}}
"#
        )
        .as_bytes(),
    )
    .unwrap()
}

const CLEAN: &str = "refresh=8 production={ dirty=no }";

fn content(extra: &str) -> Buildings {
    content_reader_key_names::buildings::parse(
        format!("buildings={{ civ={{general_production=1}} arms={{military_production=1}} yard={{naval_production=1}} infrastructure={{}} both={{military_production=1 general_production=1}} {extra} }}")
            .as_bytes(),
        &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap()
}

fn row(country: u32, levels: [i32; 3]) -> CountryFactoriesRow {
    CountryFactoriesRow {
        country,
        military_levels: levels[0],
        naval_levels: levels[1],
        civilian_levels: levels[2],
        civilian_total_raw: -1,
        consumer_goods: -1,
        ..Default::default()
    }
}

fn run(saved: &Save, content: &Buildings, rows: Vec<CountryFactoriesRow>) -> io::Result<Report> {
    run_with_membership(
        saved,
        content,
        rows,
        &content_reader_save::MapMembership {
            provinces: BTreeMap::from([(1, Some(1)), (2, Some(2)), (3, Some(3))]),
        }
        .into(),
    )
}

fn run_with_membership(
    saved: &Save,
    content: &Buildings,
    rows: Vec<CountryFactoriesRow>,
    membership: &content_reader_save::PartialMapMembership,
) -> io::Result<Report> {
    run_with_states(saved, content, rows, membership, Vec::new())
}

fn run_with_states(
    saved: &Save,
    content: &Buildings,
    rows: Vec<CountryFactoriesRow>,
    membership: &content_reader_save::PartialMapMembership,
    states: Vec<replay_data::rows::StateRow>,
) -> io::Result<Report> {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 2,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(rows));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(states));
    let germany = CountryRow {
        id: 5,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let italy = CountryRow {
        id: 6,
        tag: b"ITA".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(5, &germany), (6, &italy)]);
    let mut report = Report::default();
    report.membership_content(membership);
    compare(&frame, saved, membership, content, &links, &mut report)?;
    Ok(report)
}

#[test]
fn unowned_state_factories_follow_positive_control_and_never_charge_null_control() {
    content_reader_script::test_game::enter();
    let mut saved = save(CLEAN, CLEAN);
    let state = saved.states.get_mut(&2).unwrap();
    state.owner = None;
    state.controller = None;
    let membership = content_reader_save::MapMembership {
        provinces: BTreeMap::from([(1, Some(1)), (2, Some(2)), (3, Some(3))]),
    }
    .into();
    for (controller, levels) in [(0, [3, 0, 5]), (u32::MAX, [3, 0, 5]), (5, [3, 3, 7])] {
        let states = vec![replay_data::rows::StateRow {
            id: 2,
            controller,
            ..Default::default()
        }];
        let mut rows = vec![row(5, levels), row(6, [1, 0, 1])];
        let report = run_with_states(
            &saved,
            &content(""),
            rows.clone(),
            &membership,
            states.clone(),
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryFactories"], 2);
        rows[0].civilian_levels += 1;
        assert_eq!(
            run_with_states(&saved, &content(""), rows, &membership, states)
                .unwrap()
                .differences
                .len(),
            1
        );
    }
}

#[test]
fn an_excluded_state_keeps_all_saved_factory_input_comparisons() {
    content_reader_script::test_game::enter();
    let membership = super::super::fixtures::membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}} \
         state={id=3 provinces={3}}",
        3,
    );
    let saved = save(CLEAN, CLEAN);
    let report = run_with_membership(
        &saved,
        &content(""),
        vec![row(5, [99, 99, 99]), row(6, [1, 0, 2])],
        &membership,
    )
    .unwrap();
    assert_eq!(report.compared_tables["CountryFactories"], 2);
    assert!(report.membership_skips.is_empty());
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
    for field in [
        "GER.factories.military_levels",
        "GER.factories.naval_levels",
        "GER.factories.civilian_levels",
        "ITA.factories.civilian_levels",
    ] {
        assert!(report.differences.iter().any(|row| row.contains(field)));
    }
    assert!(report.coverage_line().contains("incomplete"));
}

#[test]
fn excluded_state_unsaved_control_requires_capture_and_missing_factory_capture_is_reported() {
    content_reader_script::test_game::enter();
    let membership = super::super::fixtures::membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}} \
         state={id=3 provinces={3}}",
        3,
    );
    let mut saved = save(CLEAN, CLEAN);
    let report =
        run_with_membership(&saved, &content(""), vec![row(6, [1, 0, 1])], &membership).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("GER.factories"));
    let state = saved.states.get_mut(&2).unwrap();
    state.owner = None;
    state.controller = None;
    let error = run_with_membership(
        &saved,
        &content(""),
        vec![row(5, [3, 3, 7]), row(6, [1, 0, 1])],
        &membership,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unowned state 2 has no captured controller")
    );
}

#[test]
fn an_excluded_state_without_factory_inputs_does_not_skip_a_country() {
    content_reader_script::test_game::enter();
    let membership = super::super::fixtures::membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}} \
         state={id=3 provinces={3}}",
        3,
    );
    let mut saved = save(CLEAN, CLEAN);
    saved
        .construction
        .state_buildings
        .retain(|(state, _), _| *state != 2);
    let report = run_with_membership(
        &saved,
        &content(""),
        vec![row(5, [3, 0, 5]), row(6, [1, 0, 1])],
        &membership,
    )
    .unwrap();
    assert_eq!(report.compared_tables["CountryFactories"], 2);
    assert!(report.membership_skips.is_empty());
    assert!(report.differences.is_empty());
    assert!(report.coverage_line().contains("incomplete"));
}

#[test]
fn history_building_effects_compare_saved_levels_without_waiving_a_difference() {
    content_reader_script::test_game::enter();
    let membership = super::super::fixtures::membership(
        "state={id=1 provinces={1} history={buildings={civ=99} \
         1936.1.1={IF={limit={} remove_building={type=civ level=2} \
         strategic_state_location={authored_location=1} \
         strategic_province_location={authored_location=1}}}}} \
         state={id=2 provinces={2}} state={id=3 provinces={3}}",
        3,
    );
    let saved = save(CLEAN, CLEAN);
    let mut rows = vec![row(5, [3, 3, 7]), row(6, [1, 0, 1])];
    let report = run_with_membership(&saved, &content(""), rows.clone(), &membership).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryFactories"], 2);
    rows[0].civilian_levels -= 1;
    let report = run_with_membership(&saved, &content(""), rows, &membership).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("GER.factories.civilian_levels"));
}

#[test]
fn controlled_and_offsite_levels_compare_per_group() {
    content_reader_script::test_game::enter();
    let saved = save(CLEAN, CLEAN);
    let rows = vec![row(5, [1 + 2, 3, 5 + 2]), row(6, [1, 0, 1])];
    let report = run(&saved, &content(""), rows.clone()).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryFactories"], 2);
    let mut off = rows;
    off[0].naval_levels = 2;
    off[1].civilian_levels = 0;
    let report = run(&saved, &content(""), off).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
}

#[test]
fn pending_recalculations_and_ambiguous_types_are_listed_not_compared() {
    content_reader_script::test_game::enter();
    let rows = || vec![row(5, [0; 3]), row(6, [0; 3])];
    for ger in [
        "refresh=10 production={ dirty=no }",
        "refresh=512 production={ dirty=no }",
        "refresh=0 production={ dirty=yes }",
    ] {
        let report = run(&save(ger, CLEAN), &content(""), rows()).unwrap();
        assert_eq!(report.compared_tables["CountryFactories"], 1, "{ger}");
        assert_eq!(report.differences.len(), 2, "{ger}");
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("recalculation pending"))
        );
    }
    let report = run(
        &save(CLEAN, CLEAN),
        &content("both={general_production=1}"),
        vec![row(5, [3, 3, 7]), row(6, [0; 3])],
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryFactories"], 1);
}

#[test]
fn missing_rows_inputs_or_content_are_not_silently_compared() {
    content_reader_script::test_game::enter();
    let report = run(&save(CLEAN, CLEAN), &content(""), vec![row(5, [3, 3, 7])]).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let rows = || vec![row(5, [3, 3, 7]), row(6, [1, 0, 1])];
    assert!(
        run(
            &save("production={ dirty=no }", CLEAN),
            &content(""),
            rows()
        )
        .is_err()
    );
    assert!(run(&save("refresh=0", CLEAN), &content(""), rows()).is_err());
    let undeclared = content_reader_key_names::buildings::parse(
        b"buildings={ civ={} arms={} yard={} infrastructure={} }",
        &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap();
    assert!(run(&save(CLEAN, CLEAN), &undeclared, rows()).is_err());
}
