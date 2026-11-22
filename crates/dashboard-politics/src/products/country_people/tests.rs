use super::*;
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableView};

const DEFINES: RecruitmentDefines = RecruitmentDefines { speed: 1000, noncore_base: 2000, noncore_max: 100_000 };
const MOD: Owner = Owner { slot: 1, id: 101 };

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 100, tables: std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])).into()), metrics: Metrics::default() };
    let mut set = |id: TableId, rows| *frame.tables[id.index()] = TableView::Complete(rows);
    set(
        TableId::Countries,
        TableRows::Countries(vec![
            CountryRow { id: 101, canonical_slot: 1, tag: b"MOD".to_vec(), manpower_residual: 7, ..Default::default() },
            CountryRow { id: 201, canonical_slot: 2, tag: b"HST".to_vec(), ..Default::default() },
        ]),
    );
    set(
        TableId::CountryLinks,
        TableRows::CountryLinks(vec![CountryLinkRow { id: 101, canonical_slot: 1 }, CountryLinkRow { id: 102, canonical_slot: 1 }, CountryLinkRow { id: 201, canonical_slot: 2 }]),
    );
    set(TableId::States, TableRows::States(vec![StateRow { id: 1, manpower_available: -9, manpower_locked: (-3).into(), population: 100_000.into(), ..Default::default() }]));
    set(
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow { country: 101, owned_state_count: 1, controlled_states: Some(vec![1]).into(), recruitment_modifiers: Some([None; 4]).into(), ..Default::default() }]),
    );
    set(
        TableId::CountryStabilityInputs,
        TableRows::CountryStabilityInputs(vec![CountryStabilityInputRow {
            country: 101,
            recruitment_ratio: Some(250_000).into(),
            exile_host: Some(201).into(),
            exile_legitimacy: Some(12345).into(),
            exile_manpower: Some(400).into(),
            exile_manpower_cap: Some(900).into(),
            exile_leaders: Some(2).into(),
            ..Default::default()
        }]),
    );
    set(TableId::SliceReads, TableRows::SliceReads(vec![SliceReadRow { table: TableId::CountryStabilityInputs.index() as u32, owner: 101, sample_hour: 85 }]));
    set(
        TableId::StateResourceInputs,
        TableRows::StateResourceInputs(vec![StateResourceInputRow {
            state: 1,
            local_manpower: Some(0).into(),
            local_noncore_manpower: Some(0).into(),
            recruitment_add: Some(0).into(),
            recruitment_factor: Some(0).into(),
            ..Default::default()
        }]),
    );
    set(TableId::StateCores, TableRows::StateCores(vec![StateCoreRow { state: 1, cores: vec![102], ..Default::default() }]));
    set(TableId::Politics, TableRows::Politics(vec![PoliticsRow { country: 102, has_elections: true.into(), election_interval: 47.into(), ..Default::default() }]));
    set(
        TableId::CountryDeployedManpower,
        TableRows::CountryDeployedManpower(vec![
            CountryDeployedManpowerRow { slot: 1, army: 37.into(), navy: 55.into(), navy_required: 77.into(), air: 22.into(), exile_air: 100.into() },
            CountryDeployedManpowerRow { slot: 2, army: 0.into(), navy: 0.into(), navy_required: 0.into(), air: 0.into(), exile_air: Recorded::NotRecorded },
        ]),
    );
    frame
}

fn value<'a>(facts: &'a [PeopleFact], label: &str) -> Option<&'a str> {
    facts.iter().find(|fact| fact.label == label).unwrap().value.as_deref()
}

#[test]
fn a_countrys_people_join_its_pools_recruitment_elections_exiles_and_the_recorders_deployed_sums() {
    let reply = read(&frame(), "1", &Country::Slot(MOD.slot), "MOD", MOD, Some(DEFINES)).unwrap();
    assert_eq!(value(&reply.manpower, "Available manpower"), Some("-2"));
    assert_eq!(value(&reply.manpower, "Effective recruitable population"), Some("2507"));
    assert_eq!(reply.states[0].recruitable, Some(2500));
    assert_eq!(reply.read_hour, Some(85), "recruitment is dated by its own slice read");
    assert_eq!(value(&reply.manpower, "Deployed army"), Some("37"));
    assert_eq!(value(&reply.manpower, "Deployed navy"), Some("55"));
    assert_eq!(value(&reply.manpower, "Required navy crews"), Some("77"));
    assert_eq!(value(&reply.manpower, "Deployed air force"), Some("22"));
    assert_eq!(value(&reply.manpower, "Deployed exile air force"), Some("100"));
    assert_eq!(value(&reply.politics, "Election interval"), Some("47"));
    assert_eq!(value(&reply.politics, "Elections"), Some("Allowed"));
    assert_eq!(reply.exiles.len(), 1);
    assert_eq!((reply.exiles[0].tag.as_str(), reply.exiles[0].host.as_deref()), ("MOD", Some("HST")));
    assert_eq!(reply.exiles[0].deployed_air, Some(100));
    assert_eq!((reply.exiles[0].manpower, reply.exiles[0].cap, reply.exiles[0].leaders), (Some(400), Some(900), Some(2)));
    assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
    let host = read(&frame(), "1", &Country::Slot(2), "HST", Owner { slot: 2, id: 201 }, None).unwrap();
    assert_eq!(host.exiles.len(), 1);
    assert_eq!(value(&host.manpower, "Deployed exile air force"), None, "an unrecorded sum is missing, never zero");
    assert!(host.warnings.iter().any(|warning| warning.contains("Deployed exile air force")));
}

#[test]
fn an_unknown_state_or_deployed_row_leaves_its_totals_missing_never_zero() {
    let mut frame = frame();
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) = &mut *frame.tables[TableId::CountryCounterInputs.index()] {
        rows[0].controlled_states = Some(vec![1, 2]).into();
    }
    if let TableView::Complete(TableRows::CountryDeployedManpower(rows)) = &mut *frame.tables[TableId::CountryDeployedManpower.index()] {
        rows.remove(0);
    }
    let reply = read(&frame, "1", &Country::Slot(MOD.slot), "MOD", MOD, Some(DEFINES)).unwrap();
    assert_eq!(value(&reply.manpower, "Available manpower"), None);
    assert_eq!(value(&reply.manpower, "Effective recruitable population"), None);
    assert!(reply.warnings.iter().any(|warning| warning.contains("controlled state 2")));
    for label in DEPLOYED {
        assert_eq!(value(&reply.manpower, label), None, "{label}");
    }
    assert_eq!(reply.exiles[0].deployed_air, None);
    assert_eq!(reply.states[0].recruitable, Some(2500), "the known state stays exact");
}

#[test]
fn a_country_owning_no_states_holds_no_elections_and_an_unread_count_leaves_them_unknown() {
    fn counters(frame: &mut DecodedFrame) -> &mut Vec<CountryCounterInputRow> {
        match &mut *frame.tables[TableId::CountryCounterInputs.index()] {
            TableView::Complete(TableRows::CountryCounterInputs(rows)) => rows,
            _ => unreachable!(),
        }
    }
    let mut frame = frame();
    counters(&mut frame)[0].owned_state_count = 0;
    let reply = read(&frame, "1", &Country::Slot(MOD.slot), "MOD", MOD, Some(DEFINES)).unwrap();
    assert_eq!(value(&reply.politics, "Elections"), Some("Not held"));
    assert_eq!(value(&reply.politics, "Next election"), Some("Not held"));
    counters(&mut frame).clear();
    let reply = read(&frame, "1", &Country::Slot(MOD.slot), "MOD", MOD, Some(DEFINES)).unwrap();
    assert_eq!(value(&reply.politics, "Elections"), None, "eligibility without the country's state count is unknown, never allowed");
    assert!(reply.warnings.iter().any(|warning| warning.contains("election details")));
}

#[test]
fn manpower_rules_keep_native_order_scaling_wrapping_and_the_election_cache() {
    assert_eq!(recruitment_target(2500, 50_000), 3750);
    assert_eq!(recruitment_target(-2500, 0), 0);
    assert_eq!(mobilisation_change(200_000, 2500, 1000, 0), 1000);
    assert_eq!(mobilisation_change(250_001, 2500, 1000, 0), -1);
    assert_eq!(applied_fraction(250_099), 2500);
    assert_eq!(state_recruitment(250_000, [50_000, 7000, 1000, 50_000], false, 1000, DEFINES), 787);
    use game_model::calendar::HOUR_EPOCH;
    let january_31 = HOUR_EPOCH + 1936 * 365 * 24 + 30 * 24 + 7;
    assert_eq!(election_date(january_31, 1), HOUR_EPOCH + 1936 * 365 * 24 + 61 * 24 + 7);
    for last in [engine_native::cgame_date::NULL_RAW_HOUR, engine_native::cpolitical_status::RESET_LAST_ELECTION] {
        assert_eq!(next_election(last, Some(48), Some(Some(12345))), Some(12345));
        assert_eq!(next_election(last, Some(48), Some(None)), None);
    }
}
