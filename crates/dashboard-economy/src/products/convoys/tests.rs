use super::*;
use replay_lanes::lanes::Step;

struct Row(HashMap<usize, i64>);

impl Cells for Row {
    fn number(&self, column: usize, _slot: u32) -> Option<i64> {
        self.0.get(&column).copied()
    }
    fn key(&self, _slot: u32) -> Option<u128> {
        Some(0)
    }
    fn slot(&self, _key: u128) -> Option<u32> {
        Some(0)
    }
    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        each(0);
    }
}

#[test]
fn only_a_lend_lease_client_names_its_receiver_and_an_unrecorded_category_names_none() {
    let row = |category: Option<i64>, other: i64| Row(category.map(|category| (CLIENT_CATEGORY, category)).into_iter().chain([(CLIENT_OTHER, other)]).collect());
    assert_eq!(lease_receiver(&row(Some(LEND_LEASE), 7), 0).value(), Some(7));
    assert_eq!(lease_receiver(&row(Some(1), 7), 0).value(), None);
    assert_eq!(lease_receiver(&row(None, 7), 0).value(), None);
}

#[test]
fn a_handle_reused_for_another_importer_names_each_country_only_over_its_own_hours() {
    let owned = owned([(5, vec![(100, 149, Some(1)), (150, 159, None), (160, 199, Some(2))]), (9, vec![(100, 199, Some(1))])]);
    assert_eq!(owned[&1], vec![(5, 100, 149), (9, 100, 199)]);
    assert_eq!(owned[&2], vec![(5, 160, 199)]);
    let rows = rows_of(&owned, 1, 120, 180);
    assert_eq!(rows.into_iter().collect::<Vec<_>>(), vec![(5, vec![(120, 149)]), (9, vec![(120, 180)])]);
    assert!(rows_of(&owned, 3, 100, 199).is_empty());
}

#[test]
fn a_deals_efficiency_is_its_factors_product_cut_to_its_importers_hours_and_never_made_up() {
    let cells = |a: Option<i64>, b: i64| Some(vec![a.map_or(Cell::Unrecorded, Cell::Number), Cell::Number(b)].into_boxed_slice());
    let steps = [
        Step { hour: 100, cells: cells(Some(50_000), 80_000) },
        Step { hour: 120, cells: cells(None, 80_000) },
        Step { hour: 130, cells: cells(Some(99_999), 99_999) },
        Step { hour: 140, cells: None },
        Step { hour: 150, cells: cells(Some(100_000), 100_000) },
    ];
    let value = |cells: &[Cell]| match (cells[0], cells[1]) {
        (Cell::Number(a), Cell::Number(b)) => product(a, b),
        _ => None,
    };
    let runs = steps_runs(&steps, 199, value);
    assert_eq!(numbers(&runs, &[(100, 134), (150, 199)]), vec![(100, 119, 40_000), (130, 134, 99_998), (150, 199, 100_000)]);
    assert_eq!(product(-3, 50_000), Some(-1));
}

#[test]
fn hours_outside_the_held_ones_are_unheld() {
    let span = |first_hour, last_hour| HourSpan { first_hour, last_hour };
    assert_eq!(outside(&[(110, 119), (115, 130), (150, 160)], 100, 170), vec![span(100, 109), span(131, 149), span(161, 170)]);
    assert_eq!(outside(&[], 5, 6), vec![span(5, 6)]);
    assert!(outside(&[(0, 10)], 0, 10).is_empty());
}

#[test]
fn convoy_health_distinguishes_no_traffic_unknown_traffic_and_full_efficiency() {
    use replay_data::rows::{ConvoyClientRow, CountryLinkRow, ResourceOriginRow, TableRows};
    use replay_frame::frame::{Metrics, TableView, not_selected};

    let make_frame = |clients: Vec<ConvoyClientRow>, origins: Vec<ResourceOriginRow>, routes: Option<Vec<TradeRouteRow>>| {
        let mut frame = DecodedFrame { branch: 1, hour: 100, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow { id: 10, canonical_slot: 1 }]));
        *frame.tables[TableId::ConvoyClients.index()] = TableView::Complete(TableRows::ConvoyClients(clients));
        *frame.tables[TableId::ResourceOrigins.index()] = TableView::Complete(TableRows::ResourceOrigins(origins));
        if let Some(routes) = routes {
            *frame.tables[TableId::TradeRoutes.index()] = TableView::Complete(TableRows::TradeRoutes(routes));
        }
        frame
    };

    let active_client = ConvoyClientRow { category: IMPORT, owner: 10, route_class: 1, route_country: 10, route_other: 2, convoys_assigned: 5, ..Default::default() };
    let naval = TradeRouteRow { country: 10, class: 1, other: 2, kind: NAVAL_ROUTE, from: 100, to: 200, path: vec![7, 8], ..Default::default() };
    let complete = make_frame(vec![active_client.clone()], vec![], Some(vec![naval]));
    assert_eq!(traffic_in_frame(&complete, 1), TrafficProof { crossed: BTreeSet::from([7, 8]), unknown: None });

    let incomplete_routes = make_frame(vec![active_client.clone()], vec![], None);
    assert_eq!(traffic_in_frame(&incomplete_routes, 1), TrafficProof::unknown("Trade routes were not recorded at this hour"));

    let unrecorded_path = make_frame(
        vec![active_client],
        vec![],
        Some(vec![TradeRouteRow { country: 10, class: 1, other: 2, kind: NAVAL_ROUTE, from: 0, to: 0, path: vec![], ..Default::default() }]),
    );
    assert_eq!(traffic_in_frame(&unrecorded_path, 1), TrafficProof::unknown(PORTS_NOT_RECORDED));

    let known = [HourSpan { first_hour: 0, last_hour: 99 }];
    let traffic = vec![
        (0, 49, TrafficProof { crossed: BTreeSet::from([7]), unknown: None }),
        (50, 99, TrafficProof::unknown(TRAFFIC_NOT_RECORDED)),
    ];
    let eff_bad = [(20, 29, 62_000), (60, 69, 40_000)];
    let cells_r7 = build_region_cells(7, &eff_bad, &traffic, &known, &[], &[], 0, 99);
    assert_eq!(
        cells_r7,
        vec![
            ConvoyCellRun { first_hour: 0, last_hour: 19, state: ConvoyCellState::Efficiency { value: 100_000 } },
            ConvoyCellRun { first_hour: 20, last_hour: 29, state: ConvoyCellState::Efficiency { value: 62_000 } },
            ConvoyCellRun { first_hour: 30, last_hour: 49, state: ConvoyCellState::Efficiency { value: 100_000 } },
            ConvoyCellRun { first_hour: 50, last_hour: 59, state: ConvoyCellState::NotRecorded { reason: TRAFFIC_NOT_RECORDED.to_owned() } },
            ConvoyCellRun { first_hour: 60, last_hour: 69, state: ConvoyCellState::Efficiency { value: 40_000 } },
            ConvoyCellRun { first_hour: 70, last_hour: 99, state: ConvoyCellState::NotRecorded { reason: TRAFFIC_NOT_RECORDED.to_owned() } },
        ]
    );
    let cells_r9 = build_region_cells(9, &[], &traffic, &known, &[], &[], 0, 99);
    assert_eq!(
        cells_r9,
        vec![
            ConvoyCellRun { first_hour: 0, last_hour: 49, state: ConvoyCellState::NoTraffic },
            ConvoyCellRun { first_hour: 50, last_hour: 99, state: ConvoyCellState::NotRecorded { reason: TRAFFIC_NOT_RECORDED.to_owned() } },
        ]
    );
}

#[test]
fn witnessed_naval_route_is_proven_even_when_another_client_has_no_route() {
    use replay_data::rows::{ConvoyClientRow, CountryLinkRow, TableRows};
    use replay_frame::frame::{Metrics, TableView, not_selected};

    let mut frame = DecodedFrame { branch: 1, hour: 100, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow { id: 10, canonical_slot: 36 }]));
    *frame.tables[TableId::ConvoyClients.index()] = TableView::Complete(TableRows::ConvoyClients(vec![
        ConvoyClientRow { category: IMPORT, owner: 10, route_class: 1, route_country: 10, route_other: 2, convoys_assigned: 4, ..Default::default() },
        ConvoyClientRow { category: IMPORT, owner: 10, route_class: 1, route_country: 10, route_other: 3, convoys_assigned: 6, ..Default::default() },
    ]));
    *frame.tables[TableId::ResourceOrigins.index()] = TableView::Complete(TableRows::ResourceOrigins(vec![]));
    *frame.tables[TableId::TradeRoutes.index()] = TableView::Complete(TableRows::TradeRoutes(vec![
        TradeRouteRow { country: 10, class: 1, other: 2, kind: NO_ROUTE, ..Default::default() },
        TradeRouteRow { country: 10, class: 1, other: 3, kind: NAVAL_ROUTE, from: 100, to: 200, path: vec![59], ..Default::default() },
    ]));

    let proof = traffic_in_frame(&frame, 36);
    let known = [HourSpan { first_hour: 100, last_hour: 100 }];
    let cells_r59 = build_region_cells(59, &[], &[(100, 100, proof.clone())], &known, &[], &[], 100, 100);
    assert_eq!(cells_r59, vec![ConvoyCellRun { first_hour: 100, last_hour: 100, state: ConvoyCellState::Efficiency { value: 100_000 } }]);

    let cells_r99 = build_region_cells(99, &[], &[(100, 100, proof)], &known, &[], &[], 100, 100);
    assert_eq!(cells_r99, vec![ConvoyCellRun { first_hour: 100, last_hour: 100, state: ConvoyCellState::NotRecorded { reason: NO_PATH.to_owned() } }]);
}

#[test]
fn traffic_hours_outside_a_complete_table_name_that_table() {
    let complete = vec![vec![(0, 99)], vec![(0, 39), (60, 99)], vec![(0, 99)], vec![(0, 79)]];
    let spans = complete.iter().skip(1).fold(complete[0].clone(), |held, other| intersect_spans(&held, other));
    assert_eq!(spans, vec![(0, 39), (60, 79)]);
    assert_eq!(
        missing_traffic(&complete, &spans, 0, 99),
        vec![
            (40, 59, TrafficProof::unknown("Convoy clients were not recorded at this hour")),
            (80, 99, TrafficProof::unknown("Trade routes were not recorded at this hour")),
        ]
    );
}

#[test]
fn a_recording_gap_is_taken_out_of_the_unheld_hours_and_keeps_its_own_reason() {
    let span = |first_hour, last_hour| HourSpan { first_hour, last_hour };
    let gaps = [(span(27, 39), "Hours not read")];
    assert_eq!(without(&[span(24, 45)], &gaps), vec![span(24, 26), span(40, 45)]);
    let known = [span(24, 45)];
    let cells = build_region_cells(17, &[(24, 45, 50_000)], &[], &known, &[span(43, 45)], &gaps, 24, 45);
    assert_eq!(
        cells,
        vec![
            ConvoyCellRun { first_hour: 24, last_hour: 26, state: ConvoyCellState::Efficiency { value: 50_000 } },
            ConvoyCellRun { first_hour: 27, last_hour: 39, state: ConvoyCellState::NotRecorded { reason: "Hours not read".into() } },
            ConvoyCellRun { first_hour: 40, last_hour: 42, state: ConvoyCellState::Efficiency { value: 50_000 } },
            ConvoyCellRun { first_hour: 43, last_hour: 45, state: ConvoyCellState::NotRecorded { reason: "No country holds this tag at this hour".into() } },
        ]
    );
}

#[test]
fn recording_gaps_are_not_mislabelled_as_unheld_tag_hours() {
    use dashboard_base::{Source, products::Product, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryLinkRow, CountryRow, RegionalConvoyEfficiencyRow, TableSchema};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let make_frame = |hour: i32, has_ger: bool| {
        let mut frame = source(1, hour, Some(1));
        let set_complete = |f: &mut DecodedFrame, idx: usize, rows| {
            *f.tables[idx] = TableView::Complete(rows);
            f.metrics.table_ns[idx] = 1;
        };
        let countries = if has_ger {
            vec![CountryRow { id: 10, canonical_slot: 1, tag: b"GER".to_vec(), ..Default::default() }]
        } else {
            vec![]
        };
        let links = if has_ger { vec![CountryLinkRow { id: 10, canonical_slot: 1 }] } else { vec![] };
        set_complete(&mut frame, TableId::Countries.index(), tables::Countries::wrap(countries));
        set_complete(&mut frame, TableId::CountryLinks.index(), tables::CountryLinks::wrap(links));
        set_complete(&mut frame, TableId::RegionalConvoyEfficiency.index(), tables::RegionalConvoyEfficiency::wrap(vec![RegionalConvoyEfficiencyRow { country: 10, region: 17, efficiency: 50_000 }]));
        set_complete(&mut frame, TableId::ConvoyClients.index(), tables::ConvoyClients::wrap(vec![]));
        set_complete(&mut frame, TableId::ResourceOrigins.index(), tables::ResourceOrigins::wrap(vec![]));
        set_complete(&mut frame, TableId::TradeRoutes.index(), tables::TradeRoutes::wrap(vec![]));
        frame
    };

    let mut frames = Vec::new();
    for h in 24..=26 {
        frames.push(make_frame(h, true));
    }
    for h in 40..=42 {
        frames.push(make_frame(h, true));
    }
    for h in 43..=45 {
        frames.push(make_frame(h, false));
    }
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "convoy-gap".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = ConvoyHealth::load(
        &src,
        &ConvoyHealthParams {
            branch_id: "1".into(),
            view: ConvoyView::Regions,
            country: Country::Tag("GER".into()),
            from: Some(24),
            to: Some(45),
            regions: Some(vec![17]),
            pinned_regions: None,
            hidden_regions: None,
            top: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    let region = reply.regions.iter().find(|r| r.region == 17).unwrap();
    let gap_cell = region.cells.iter().find(|c| c.first_hour <= 30 && 30 <= c.last_hour).unwrap();
    match &gap_cell.state {
        ConvoyCellState::NotRecorded { reason } => assert_ne!(reason, "No country holds this tag at this hour", "recording gap must not claim tag unheld"),
        other => panic!("expected NotRecorded in recording gap, got {other:?}"),
    }
    let unheld_cell = region.cells.iter().find(|c| c.first_hour <= 44 && 44 <= c.last_hour).unwrap();
    assert_eq!(unheld_cell.state, ConvoyCellState::NotRecorded { reason: "No country holds this tag at this hour".into() });
}

#[test]
fn a_witnessed_efficiency_stands_where_the_table_read_was_truncated() {
    let known = [HourSpan { first_hour: 0, last_hour: 9 }];
    let cells = build_region_cells(17, &[(0, 19, 62_000)], &[], &known, &[], &[], 0, 19);
    assert_eq!(cells, vec![ConvoyCellRun { first_hour: 0, last_hour: 19, state: ConvoyCellState::Efficiency { value: 62_000 } }]);
}
