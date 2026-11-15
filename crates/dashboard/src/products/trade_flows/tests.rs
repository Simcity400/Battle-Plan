use super::*;
use replay_data::rows::{TradeFlowRow, key_field};

#[test]
fn the_schema_numbers_a_flows_cells_and_its_key_holds_both_holders_and_the_resource() {
    let field = |name| dashboard_declare::field(TableId::TradeFlows, name).unwrap();
    assert_eq!(COLUMNS, [field("delivered"), field("factories"), field("convoys_assigned"), field("convoys_required"), field("deals")]);
    let row = TradeFlowRow { seller_slot: 3, seller: 30, buyer_slot: 4, buyer: 40, resource: 7, ..TradeFlowRow::default() };
    let key = row.key();
    assert_eq!((holder(key, 80), holder(key, 32), key as u32), (Some((3, 30)), Some((4, 40)), 7));
    let unresolved = TradeFlowRow { buyer_slot: TagHolderRow::NONE, ..row };
    assert_eq!(holder(unresolved.key(), 32), None);
    assert_eq!(key_field(TagHolderRow::NONE), u128::from(replay_data::rows::KEY_FIELD_NONE));
}

fn step(hour: i32, cells: Option<[Option<i64>; 5]>) -> Step {
    Step { hour, cells: cells.map(|cells| cells.map(|cell| cell.map_or(Cell::Unrecorded, Cell::Number)).into()) }
}

#[test]
fn a_flows_runs_are_its_steps_terms_and_a_sum_not_recorded_leaves_its_deals_out() {
    let terms = |delivered| Some([Some(delivered), Some(1), Some(0), Some(2), Some(3)]);
    let steps = [
        step(100, terms(500)),
        step(110, terms(500)),
        step(120, Some([None, Some(1), Some(0), Some(2), Some(3)])),
        step(130, None),
        step(140, terms(700)),
    ];
    assert_eq!(step_runs(&steps, 159), vec![(100, 119, Ok([500, 1, 0, 2, 3])), (120, 129, Err(3)), (140, 159, Ok([700, 1, 0, 2, 3]))]);
}

#[test]
fn a_run_is_cut_where_a_sides_tag_changes_and_hours_no_tag_holds_a_side_are_unnamed() {
    let tag = |name: &str, a, b| (name.to_owned(), a, b);
    let seller = [tag("OLD", 100, 149), tag("NEW", 150, 199)];
    let buyer = [tag("BUY", 120, 199)];
    assert_eq!(tagged(&seller, &buyer, 100, 199), vec![(100, 119, None), (120, 149, Some(("OLD", "BUY"))), (150, 199, Some(("NEW", "BUY")))]);
    assert_eq!(tagged(&seller, &[], 100, 110), vec![(100, 110, None)]);
}

#[test]
fn left_out_deals_are_summed_per_hour() {
    let span = |first_hour, last_hour, deals| PartialSpan { first_hour, last_hour, deals };
    assert_eq!(partial(&[(100, 119, 2), (110, 129, 1), (130, 139, 3), (150, 150, 0)]), vec![span(100, 109, 2), span(110, 119, 3), span(120, 129, 1), span(130, 139, 3)]);
    assert!(partial(&[]).is_empty());
}

#[test]
fn flows_list_their_tags_once_in_byte_order_and_index_each_run() {
    let gathered = BTreeMap::from([(((3, 30), (4, 40), 7), vec![(150, 199, "NEW".to_owned(), "BUY".to_owned(), [700, 1, 0, 0, 1]), (120, 149, "OLD".to_owned(), "BUY".to_owned(), [500, 1, 0, 0, 1])])]);
    let (countries, flows) = flows(gathered);
    assert_eq!(countries, ["BUY", "NEW", "OLD"]);
    let flow = &flows[0];
    assert_eq!((flow.seller, flow.buyer, flow.seller_identity.as_str(), flow.buyer_identity.as_str()), (2, 0, "[3,30]", "[4,40]"));
    assert_eq!((flow.first_hours.clone(), flow.last_hours.clone(), flow.seller_tags.clone()), (vec![120, 150], vec![149, 199], vec![2, 1]));
    assert_eq!(flow.delivered_raw, ["500", "700"]);
}

#[test]
fn trade_is_known_only_where_the_deals_were_read_and_the_flows_table_was_available() {
    let read = [(0, 9), (20, 29)];
    let written = [(5, 24), (28, 40)];
    assert_eq!(both(&read, &written), [(5, 9), (20, 24), (28, 29)], "a read hour the writer withheld is not known, nor an available hour not read");
    assert!(both(&read, &[]).is_empty(), "no hour the flows were written: nothing is known, not no trade");
    assert_eq!(both(&[(3, 3)], &[(0, 100)]), [(3, 3)]);
}
