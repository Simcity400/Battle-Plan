use super::*;
use recorder_derive_framework::testing::deployed::{Edits, Run, country, link, world};
use recorder_derive_world::identity::TagHolders;
use replay_columnar::manifest::Status;
use replay_data::rows::{Recorded, TagHolderRow};

const NONE: u32 = TagHolderRow::NONE;

const INPUTS: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::ResourceExchanges];

fn writers() -> Vec<Box<dyn Writer>> {
    vec![Box::new(TagHolders::default()), Box::new(TradeFlows::default())]
}

fn deal(id: u64, exporter: u32, importer: u32, resource: u32, delivered: i64, factories: i32) -> ResourceExchangeRow {
    ResourceExchangeRow { id, exporter, importer, resource, delivered, assigned_factories: factories, convoys_assigned: 1, convoys_required: 2, ..Default::default() }
}

fn known(cell: &Recorded<i64>) -> Option<i64> {
    match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    }
}

fn flows(run: &Run) -> Vec<((u32, u32, u32, u32, u32), [Option<i64>; 4], i64)> {
    run.rows::<TradeFlowRow>(TableId::TradeFlows)
        .iter()
        .map(|row| {
            (
                (row.seller_slot, row.seller, row.buyer_slot, row.buyer, row.resource),
                [known(&row.delivered), known(&row.factories), known(&row.convoys_assigned), known(&row.convoys_required)],
                row.deals,
            )
        })
        .collect()
}

fn traded() -> Edits {
    let mut edits = Edits::default();
    world(&mut edits)
        .put(TableId::ResourceExchanges, deal(1, 10, 20, 1, 100, 2))
        .put(TableId::ResourceExchanges, deal(2, 11, 20, 1, 50, 1))
        .put(TableId::ResourceExchanges, deal(3, 30, 99, 2, 7, 0));
    edits
}

#[test]
fn trade_flows_sum_deals_by_holder_identity_and_resource() {
    let mut run = Run::new(writers, INPUTS);
    run.hour(&traded());
    assert_eq!(run.status(TableId::TradeFlows), Status::Complete);
    assert_eq!(
        flows(&run),
        [((1, 10, 2, 20, 1), [Some(150), Some(3), Some(2), Some(4)], 2), ((3, 30, NONE, NONE, 2), [Some(7), Some(0), Some(1), Some(2)], 1)]
    );
    let mut edits = Edits::default();
    edits.delete(TableId::ResourceExchanges, 2).put(TableId::ResourceExchanges, deal(3, 30, 10, 2, 9, 1));
    run.hour(&edits);
    assert_eq!(
        flows(&run),
        [((1, 10, 2, 20, 1), [Some(100), Some(2), Some(1), Some(2)], 1), ((3, 30, 1, 10, 2), [Some(9), Some(1), Some(1), Some(2)], 1)],
        "the deleted deal leaves its flow; the moved deal leaves the unresolved flow, which is deleted"
    );
    let mut edits = Edits::default();
    edits.delete(TableId::Countries, 10).put(TableId::Countries, country(12, 1, b"GER")).put(TableId::CountryLinks, link(12, 1));
    run.hour(&edits);
    assert_eq!(
        flows(&run).iter().map(|flow| flow.0).collect::<Vec<_>>(),
        [(1, 12, 2, 20, 1), (3, 30, 1, 12, 2)],
        "GER's new holder takes its flows under its own identity"
    );
    let mut edits = Edits::default();
    edits.unavailable(TableId::ResourceExchanges, b"not read");
    run.hour(&edits);
    assert!(matches!(run.status(TableId::TradeFlows), Status::Unavailable { .. }));
    assert_eq!(flows(&run).len(), 2);
    let mut edits = Edits::default();
    edits.status(TableId::ResourceExchanges, Status::Complete).delete(TableId::ResourceExchanges, 1);
    run.hour(&edits);
    assert_eq!(run.status(TableId::TradeFlows), Status::Complete);
    assert_eq!(flows(&run).iter().map(|flow| flow.0).collect::<Vec<_>>(), [(3, 30, 1, 12, 2)]);
}

#[test]
fn a_trade_flow_sum_past_i64_is_not_recorded() {
    let mut run = Run::new(writers, INPUTS);
    let mut edits = Edits::default();
    world(&mut edits).put(TableId::ResourceExchanges, deal(1, 10, 20, 1, i64::MAX, 1)).put(TableId::ResourceExchanges, deal(2, 10, 20, 1, 1, 1));
    run.hour(&edits);
    assert_eq!(flows(&run), [((1, 10, 2, 20, 1), [None, Some(2), Some(2), Some(4)], 2)]);
}

#[test]
fn a_resumed_trade_flows_writer_writes_the_table_of_one_never_stopped() {
    let mut unbroken = Run::new(writers, INPUTS);
    unbroken.hour(&traded());
    let mut stopped = Run::new(writers, INPUTS);
    stopped.hour(&traded());
    let mut resumed = stopped.resumed();
    let mut edits = Edits::default();
    edits.put(TableId::ResourceExchanges, deal(2, 11, 30, 1, 50, 1)).put(TableId::ResourceExchanges, deal(4, 20, 30, 1, 5, 5));
    unbroken.hour(&edits);
    resumed.hour(&edits);
    assert_eq!(flows(&unbroken), flows(&resumed));
    assert_eq!(flows(&resumed).iter().map(|flow| flow.0).collect::<Vec<_>>(), [(1, 10, 2, 20, 1), (1, 10, 3, 30, 1), (2, 20, 3, 30, 1), (3, 30, NONE, NONE, 2)]);
}
