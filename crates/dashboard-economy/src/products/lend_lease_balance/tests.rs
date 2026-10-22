use super::*;
use crate::products::test_support::*;
use replay_data::rows::*;

fn history(country: u32, target: u32, sent: i64, received: i64) -> WarLendLeaseHistoryRow {
    WarLendLeaseHistoryRow {
        country,
        target,
        industry_sent: sent,
        fuel_sent: 0,
        industry_received: received,
        fuel_received: 0,
    }
}

fn ask(state: &mut Prepared, holder: Option<&str>, through: i32) -> LendLeaseBalanceReply {
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    query(
        &fixture.store,
        &fixture.key,
        &LendLeaseBalanceParams {
            branch_id: "1".into(),
            holder: holder.map(str::to_owned),
        },
        Some(through),
        &flag,
    )
    .unwrap()
}

fn at(hour: i32, rows: Vec<WarLendLeaseHistoryRow>) -> DecodedFrame {
    let mut at = frame(hour);
    countries(&mut at, &[(1, 10, "AAA"), (2, 20, "BBB"), (3, 30, "CCC")]);
    set(&mut at, TableRows::WarLendLeaseHistories(rows));
    at
}

#[test]
fn totals_sum_each_owners_own_records_and_never_both_sides_of_a_pair() {
    let mut state = Prepared::default();
    // AAA records sending to BBB and CCC; BBB records the same delivery.
    state.step(&at(5, vec![
        history(1, 2, 300, 0),
        history(1, 3, 100, -1),
        history(2, 1, 0, 300),
    ]));
    let reply = ask(&mut state, None, 5);
    assert_eq!(reply.known, vec![EconomyRun(5, 5, "1".into())]);
    let aaa = reply.histories.iter().find(|row| row.holder == "10").unwrap();
    assert_eq!(aaa.values[0][0].2, "400");
    // A signed correction stays as recorded.
    assert_eq!(aaa.values[2][0].2, "-1");
    let bbb = reply.histories.iter().find(|row| row.holder == "20").unwrap();
    assert_eq!(bbb.values[2][0].2, "300");
    assert_eq!(bbb.values[0][0].2, "0");
    // CCC is a partner only: its own history is not recorded, not zero.
    assert!(reply.histories.iter().all(|row| row.holder != "30"));
}

#[test]
fn a_holders_partners_and_a_country_leaving_the_rosters_becomes_unknown() {
    let mut state = Prepared::default();
    state.step(&at(5, vec![history(1, 2, 300, 0)]));
    state.step(&at(6, vec![history(1, 2, 500, 0)]));
    state.step(&at(7, vec![]));
    let partners = ask(&mut state, Some("10"), 7);
    assert_eq!(partners.histories.len(), 1);
    assert_eq!(partners.histories[0].holder, "20");
    assert_eq!(
        partners.histories[0].values[0],
        vec![EconomyRun(5, 5, "300".into()), EconomyRun(6, 6, "500".into())]
    );
}

#[test]
fn an_unreadable_table_names_its_refusal() {
    let mut state = Prepared::default();
    let mut frame = at(5, vec![history(1, 2, 3, 0)]);
    unavailable(&mut frame, TableId::WarLendLeaseHistories);
    state.step(&frame);
    let reply = ask(&mut state, None, 5);
    assert!(reply.histories.is_empty());
    // Not read: unknown, not "no lend-lease".
    assert!(reply.known.is_empty());
    assert!(reply.warnings[0].contains("histories unavailable"));
}
