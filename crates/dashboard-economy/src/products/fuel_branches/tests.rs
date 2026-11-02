//! The row census's own scenarios ([`super::row`]), the oracle the columnar path is held to.
use super::{row::{Prepared, reply}, *};
use crate::products::{keyed::EconomyRun, test_support::*};
use replay_data::rows::*;
use replay_frame::frame::DecodedFrame;
use std::sync::atomic::AtomicBool;

fn consumer(country: u32, index: u32, requested: i64, received: i64) -> CountryFuelConsumerRow {
    CountryFuelConsumerRow {
        country,
        index,
        priority: index as i32,
        requested_raw: requested,
        received_raw: received,
    }
}

fn run(first: i32, last: i32, value: &str) -> EconomyRun {
    EconomyRun(first, last, value.into())
}

fn step(state: &mut Prepared, at: &DecodedFrame, read: bool) {
    state.keyed.step(at.hour).unwrap();
    state.observe(at, read);
}

fn ask(state: &mut Prepared, tags: &[&str], through: i32) -> FuelBranchesReply {
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    reply(
        &fixture.store,
        &fixture.key,
        &FuelBranchesParams {
            branch_id: "1".into(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        },
        Some(through),
        [Some("Land".into()), Some("Air".into()), Some("Sea".into())],
        Vec::new(),
        &flag,
    )
    .unwrap()
}

#[test]
fn consumers_hold_between_reads_and_keep_receipt_over_request_exact() {
    let mut state = Prepared::default();
    for (hour, read, rows) in [
        (10, true, vec![consumer(7, 0, 500_000, 250_000), consumer(7, 1, 0, 0)]),
        (11, false, vec![consumer(7, 0, 500_000, 250_000), consumer(7, 1, 0, 0)]),
        // A receipt above the request stays as recorded.
        (34, true, vec![consumer(7, 0, 100, i64::MAX), consumer(7, 1, 0, 0)]),
    ] {
        let mut at = frame(hour);
        countries(&mut at, &[(7, 3, "AAA")]);
        set(&mut at, TableRows::CountryFuelConsumers(rows));
        step(&mut state, &at, read);
    }
    let reply = ask(&mut state, &["AAA"], 34);
    let country = &reply.countries[0];
    assert_eq!(country.holder, "3");
    let army = &country.branches[0];
    // 11 follows 10 without a change; 12..33 were not recorded: a gap.
    assert_eq!(army.requested, vec![run(10, 11, "500000"), run(34, 34, "100")]);
    assert_eq!(army.received[1], run(34, 34, &i64::MAX.to_string()));
    // A known zero request is a value, not a missing branch.
    assert_eq!(country.branches[1].requested, vec![run(10, 11, "0"), run(34, 34, "0")]);
    // The read hour holds between reads.
    assert_eq!(country.read, vec![run(10, 11, "10"), run(34, 34, "34")]);
    assert_eq!(reply.consumer_names[0].as_deref(), Some("Land"));
}

#[test]
fn an_unavailable_read_leaves_consumers_unknown_and_names_its_refusal() {
    let mut state = Prepared::default();
    let mut at = frame(10);
    countries(&mut at, &[(7, 3, "AAA")]);
    set(&mut at, TableRows::CountryFuelConsumers(vec![consumer(7, 0, 5, 5)]));
    step(&mut state, &at, true);
    let mut at = frame(11);
    countries(&mut at, &[(7, 3, "AAA")]);
    unavailable(&mut at, TableId::CountryFuelConsumers);
    step(&mut state, &at, true);
    let reply = ask(&mut state, &[], 11);
    assert_eq!(reply.countries[0].branches[0].requested, vec![run(10, 10, "5")]);
    // The table is known only where it was read complete.
    assert_eq!(reply.known, vec![run(10, 10, "1")]);
    assert!(reply.warnings.iter().any(|line| line.contains("fuel consumers unavailable")));
}

#[test]
fn a_reused_tag_reads_only_the_hours_its_holder_had_it_and_indexes_come_from_the_recording() {
    let mut state = Prepared::default();
    for (hour, slot) in [(10, 3), (11, 4)] {
        let mut at = frame(hour);
        countries(&mut at, &[(7, slot, "AAA")]);
        // A fourth consumer index, beyond the fuel panel's labels.
        set(&mut at, TableRows::CountryFuelConsumers(vec![consumer(7, 3, 9, 9)]));
        step(&mut state, &at, true);
    }
    let reply = ask(&mut state, &["AAA"], 11);
    assert_eq!(reply.countries.len(), 2);
    assert_eq!(reply.countries[0].branches[0].requested, vec![run(10, 10, "9")]);
    assert_eq!(reply.countries[1].branches[0].requested, vec![run(11, 11, "9")]);
    assert_eq!(reply.consumer_names.len(), 4);
    assert_eq!(reply.consumer_names[3], None);
    assert!(reply.warnings.iter().any(|line| line.contains("beyond the fuel panel")));
}

#[test]
fn countries_without_consumers_are_left_out() {
    let mut state = Prepared::default();
    let mut at = frame(10);
    many_countries(&mut at, 50);
    set(
        &mut at,
        TableRows::CountryFuelConsumers((0..5).map(|n| consumer(n * 10 + 1, 0, 1, 1)).collect()),
    );
    step(&mut state, &at, true);
    assert_eq!(ask(&mut state, &[], 10).countries.len(), 5);
}

#[test]
fn an_unidentified_consumer_keeps_the_others_but_proves_no_country_without_consumers() {
    let mut state = Prepared::default();
    for (hour, rows) in [
        (10, vec![consumer(7, 0, 5, 5)]),
        // A consumer of raw country 9, which no link resolves.
        (11, vec![consumer(7, 0, 6, 6), consumer(9, 0, 8, 8)]),
    ] {
        let mut at = frame(hour);
        countries(&mut at, &[(7, 3, "AAA"), (8, 4, "BBB")]);
        set(&mut at, TableRows::CountryFuelConsumers(rows));
        step(&mut state, &at, true);
    }
    let reply = ask(&mut state, &[], 11);
    assert_eq!(reply.countries.len(), 1);
    assert_eq!(reply.countries[0].branches[0].requested, vec![run(10, 10, "5"), run(11, 11, "6")]);
    // BBB had no consumers at 10; at 11 the unidentified one may be its.
    assert_eq!(reply.known, vec![run(10, 10, "1")]);
    assert!(reply.warnings.iter().any(|line| line.contains("a fuel consumer's country has no identity")));
}
