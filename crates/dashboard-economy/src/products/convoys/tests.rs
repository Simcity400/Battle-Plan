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
