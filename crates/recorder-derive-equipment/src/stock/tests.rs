use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run, link};
use replay_data::rows::{Recorded, StockByVariantRow};

const INPUTS: &[TableId] = &[TableId::CountryLinks, TableId::Stockpiles];

fn make() -> Vec<Box<dyn Writer>> {
    vec![Box::new(Stockpile::default())]
}

fn position(country: u32, ordinal: u32, variant: u64, amount: i64) -> StockpileRow {
    StockpileRow { country, ordinal, variant_handle: variant, amount_raw: amount }
}

fn value(cell: Recorded<i64>) -> Option<i64> {
    match cell {
        Recorded::Value(value) => Some(value),
        Recorded::NotRecorded => None,
    }
}

fn stock(run: &Run) -> Vec<(u32, u64, Option<i64>, Option<i64>)> {
    run.rows::<StockByVariantRow>(TableId::StockByVariant).into_iter().map(|row| (row.slot, row.variant, value(row.amount), value(row.positions))).collect()
}

#[test]
fn a_slots_stock_of_a_design_sums_its_countries_positions_and_follows_a_position_to_another_design() {
    let mut run = Run::new(make, INPUTS);
    let mut edits = Edits::default();
    edits
        .put(TableId::CountryLinks, link(10, 1))
        .put(TableId::CountryLinks, link(11, 1))
        .put(TableId::CountryLinks, link(20, 2))
        .put(TableId::Stockpiles, position(10, 0, 7, 100))
        .put(TableId::Stockpiles, position(10, 1, 8, 5))
        .put(TableId::Stockpiles, position(11, 0, 7, 50))
        .put(TableId::Stockpiles, position(20, 0, 9, -1))
        .put(TableId::Stockpiles, position(20, 1, 9, 1));
    run.hour(&edits);
    assert_eq!(stock(&run), [(1, 7, Some(150), Some(2)), (1, 8, Some(5), Some(1)), (2, 9, Some(0), Some(2))], "slot 2's design 9 sums to none, held at two positions");
    let mut edits = Edits::default();
    edits.put(TableId::Stockpiles, position(10, 1, 7, 5)).delete(TableId::Stockpiles, position(11, 0, 0, 0).key()).put(TableId::Stockpiles, position(20, 1, 9, 3));
    run.hour(&edits);
    assert_eq!(stock(&run), [(1, 7, Some(105), Some(2)), (2, 9, Some(2), Some(2))]);
    let mut resumed = run.resumed();
    let mut edits = Edits::default();
    edits.put(TableId::Stockpiles, position(30, 0, 9, 4));
    run.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(stock(&run), [(1, 7, None, None), (2, 9, None, None)], "a country no link resolves");
    assert_eq!(stock(&run), stock(&resumed));
    let mut edits = Edits::default();
    edits.put(TableId::CountryLinks, link(30, 2));
    run.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(stock(&run), [(1, 7, Some(105), Some(2)), (2, 9, Some(6), Some(3))]);
    assert_eq!(stock(&run), stock(&resumed));
}
