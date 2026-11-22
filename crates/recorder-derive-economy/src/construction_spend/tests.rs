use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run, world};
use recorder_derive_world::identity::TagHolders;

const INPUTS: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::Provinces, TableId::ConstructionLineOutput];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(TagHolders::default()), Box::new(ConstructionSpend::default())]
}

fn building(line: u64, country: u32, construction: i64, since: i32, exact: bool) -> ConstructionLineOutputRow {
    ConstructionLineOutputRow {
        line,
        construction_raw: construction,
        since_hour: since,
        exact,
        country: Recorded::Value(Some(country)),
        building_type: Recorded::Value(Some(5)),
        target_state: Recorded::Value(Some(100)),
        ..Default::default()
    }
}

fn railway(line: u64, country: u32, construction: i64) -> ConstructionLineOutputRow {
    ConstructionLineOutputRow {
        line,
        railway: true,
        construction_raw: construction,
        exact: true,
        country: Recorded::Value(Some(country)),
        building_type: Recorded::Value(Some(9)),
        route_first: Recorded::Value(Some(7)),
        route_last: Recorded::Value(Some(8)),
        ..Default::default()
    }
}

fn province(id: u32, state: u32) -> ProvinceRow {
    ProvinceRow { id, state, ..Default::default() }
}

fn spend(run: &Run) -> Vec<(u32, u32, bool, bool, u32, u32, u32, Option<i64>, i64)> {
    run.rows::<ConstructionSpendRow>(TableId::ConstructionSpend)
        .iter()
        .map(|row| {
            let total = match row.total {
                Recorded::Value(total) => Some(total),
                Recorded::NotRecorded => None,
            };
            (row.slot, row.country, row.repair, row.railway, row.building_type, row.state, row.to_state, total, row.unknown)
        })
        .collect()
}

fn built() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::Provinces, province(7, 200))
        .put(TableId::Provinces, province(8, 300))
        .put(TableId::ConstructionLineOutput, building(1, 10, 1_000, 0, true))
        .put(TableId::ConstructionLineOutput, building(2, 11, 500, 0, true))
        .put(TableId::ConstructionLineOutput, railway(3, 20, 300));
    edits
}

#[test]
fn construction_spend_sums_each_lines_output_by_identity_work_type_and_state() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&built());
    assert_eq!(
        spend(&run),
        [(1, 10, false, false, 5, 100, NONE, Some(1_500), 0), (2, 20, false, true, NONE, 200, 300, Some(300), 0)],
        "GER's alias names GER; nothing was repaired, so no repair row"
    );
    let mut edits = Edits::default();
    edits.put(TableId::ConstructionLineOutput, building(1, 10, 1_600, 0, true)).put(TableId::ConstructionLineOutput, building(2, 11, 600, 0, false));
    run.hour(&edits);
    assert_eq!(spend(&run)[..2], [(1, 10, false, false, 5, 100, NONE, Some(2_100), 1), (1, 10, true, false, 5, 100, NONE, Some(0), 1)]);
    let mut edits = Edits::default();
    edits.put(TableId::ConstructionLineOutput, building(2, 11, 800, 0, true));
    run.hour(&edits);
    assert_eq!(spend(&run)[..2], [(1, 10, false, false, 5, 100, NONE, Some(2_400), 0), (1, 10, true, false, 5, 100, NONE, Some(0), 0)]);
    let mut edits = Edits::default();
    edits.delete(TableId::ConstructionLineOutput, 1).put(TableId::ConstructionLineOutput, building(2, 11, 50, 3, true));
    run.hour(&edits);
    assert_eq!(spend(&run)[0], (1, 10, false, false, 5, 100, NONE, Some(2_450), 0));
}

#[test]
fn a_resumed_construction_spend_writer_writes_the_table_of_one_never_stopped() {
    let mut unbroken = Run::new(writers, INPUTS);
    unbroken.hour(&built());
    let mut stopped = Run::new(writers, INPUTS);
    stopped.hour(&built());
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits.put(TableId::ConstructionLineOutput, building(1, 10, 1_200, 0, true)).put(TableId::ConstructionLineOutput, railway(3, 20, 900));
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(spend(&unbroken), spend(&resumed));
    assert_eq!(spend(&resumed), [(1, 10, false, false, 5, 100, NONE, Some(1_700), 0), (2, 20, false, true, NONE, 200, 300, Some(900), 0)]);
}

#[test]
fn construction_spend_keeps_states_and_building_types_past_24_bits_whole() {
    const WIDE: u32 = (1 << 24) + 5;
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::Provinces, province(7, WIDE))
        .put(TableId::Provinces, province(8, u32::MAX - 1))
        .put(TableId::ConstructionLineOutput, ConstructionLineOutputRow {
            building_type: Recorded::Value(Some(1 << 30)),
            target_state: Recorded::Value(Some(WIDE + 1)),
            ..building(1, 10, 1_000, 0, true)
        })
        .put(TableId::ConstructionLineOutput, railway(3, 20, 300));
    let mut run = Run::new(writers, INPUTS);
    run.hour(&edits);
    assert_eq!(
        spend(&run),
        [(1, 10, false, false, 1 << 30, WIDE + 1, NONE, Some(1_000), 0), (2, 20, false, true, NONE, WIDE, u32::MAX - 1, Some(300), 0)]
    );
    let key = ConstructionSpendRow { slot: 2, country: 20, railway: true, building_type: NONE, state: WIDE, to_state: u32::MAX - 1, ..Default::default() }.key();
    assert_eq!(key >> 114, 0, "the key is 114 bits");
}
