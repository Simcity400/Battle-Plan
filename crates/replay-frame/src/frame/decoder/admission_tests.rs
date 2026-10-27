use super::*;
use crate::frame::endpoint_step::{EndpointStep, EndpointWriter};
use crate::frame::{Frame, FrameEncoder, TableView};
use crate::rows::{CountryRow, TableRows};
use std::cell::Cell;

const TABLES: &[TableId] = &[TableId::Countries];

fn source(hour: Hour) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture omitted table".into(),
            }
            .into()
        }),
    };
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=40)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("{id:03}").into_bytes(),
                command_power_raw: i64::from(hour),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame
}

fn updates() -> Vec<Frame> {
    let mut encoder = FrameEncoder::new();
    (0..=2)
        .map(|hour| encoder.encode(source(hour), hour == 0).unwrap())
        .collect()
}

fn bounded_checks(checks: &Cell<usize>) -> bool {
    checks.set(checks.get() + 1);
    assert!(
        checks.get() <= 3,
        "admission waited on its own pinned predecessor"
    );
    false
}

fn assert_hour(decoder: &FrameDecoder, hour: Hour) {
    let frame = decoder.current().unwrap();
    assert_eq!(frame.hour, hour);
    let TableView::Complete(TableRows::Countries(rows)) =
        &*frame.tables[TableId::Countries.index()]
    else {
        panic!("complete countries");
    };
    assert_eq!(rows.len(), 40);
    assert_eq!(rows[0].id, 1);
    assert_eq!(rows[39].id, 40);
    assert!(
        rows.iter()
            .all(|row| row.command_power_raw == i64::from(hour))
    );
}

#[test]
fn advancing_reclaims_exclusive_facts_and_refuses_a_pinned_predecessor_without_waiting() {
    for held in [false, true] {
        let budget = allocation::credit::MemoryBudget::new(3072);
        let mut decoder = FrameDecoder::for_tables(TABLES).with_change_facts(budget.clone());
        let updates = updates();
        decoder.advance(&updates[0]).unwrap();
        decoder.advance(&updates[1]).unwrap();
        let held = held.then(|| decoder.shared_current().unwrap().clone());
        let checks = Cell::new(0);
        let result = decoder.advance_cancellable(&updates[2], &|| bounded_checks(&checks));
        if held.is_some() {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
            assert_eq!(
                error.to_string(),
                "display change-fact working set remains pinned; common paging is required"
            );
            assert_hour(&decoder, 1);
            drop(held);
            decoder.advance(&updates[2]).unwrap();
        } else {
            assert_eq!(result.unwrap().hour, 2);
        }
        assert_hour(&decoder, 2);
        assert!(budget.peak() <= 3072);
        drop(decoder);
        assert_eq!(budget.used(), 0);
    }
}

fn endpoints() -> Vec<Vec<u8>> {
    let mut native = FrameDecoder::for_tables(TABLES);
    let mut writer = EndpointWriter::for_tables(TABLES);
    updates()
        .iter()
        .map(|update| {
            let frame = native.advance(update).unwrap();
            writer.observe(update, frame).unwrap();
            let mut bytes = Vec::new();
            writer.finish(frame, &mut bytes).unwrap();
            bytes
        })
        .collect()
}

#[test]
fn endpoint_admission_reclaims_or_names_its_pinned_predecessor() {
    for held in [false, true] {
        let budget = allocation::credit::MemoryBudget::new(3072);
        let mut decoder = FrameDecoder::for_tables(TABLES).with_change_facts(budget.clone());
        let endpoints = endpoints();
        for bytes in &endpoints[..2] {
            decoder
                .advance_endpoint(EndpointStep::decode(bytes).unwrap())
                .unwrap();
        }
        let held = held.then(|| decoder.shared_current().unwrap().clone());
        let checks = Cell::new(0);
        let result = decoder.advance_endpoint_parallel_cancellable(
            EndpointStep::decode(&endpoints[2]).unwrap(),
            1,
            &|| {},
            &|| bounded_checks(&checks),
        );
        if held.is_some() {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
            assert_eq!(
                error.to_string(),
                "display change-fact working set remains pinned; common paging is required"
            );
            assert_hour(&decoder, 1);
            drop(held);
            decoder
                .advance_endpoint(EndpointStep::decode(&endpoints[2]).unwrap())
                .unwrap();
        } else {
            assert_eq!(result.unwrap().hour, 2);
        }
        assert_hour(&decoder, 2);
        drop(decoder);
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn restore_releases_its_input_facts_before_admission_and_never_waits_for_its_caller() {
    for held in [false, true] {
        let budget = allocation::credit::MemoryBudget::new(3072);
        let mut decoder = FrameDecoder::for_tables(TABLES).with_change_facts(budget.clone());
        let updates = updates();
        decoder.advance(&updates[0]).unwrap();
        decoder.advance(&updates[1]).unwrap();
        let frame = if held {
            decoder.current().unwrap().clone()
        } else {
            Arc::try_unwrap(decoder.current.take().unwrap()).unwrap()
        };
        let checks = Cell::new(0);
        let result = FrameDecoder::resume_for_tables_with_change_facts_cancellable(
            TABLES,
            frame,
            budget.clone(),
            &|| bounded_checks(&checks),
        );
        if held {
            let error = result.err().expect("caller pins old metadata");
            assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
            assert_eq!(
                error.to_string(),
                "display change-fact working set remains pinned; common paging is required"
            );
            assert_hour(&decoder, 1);
        } else {
            let resumed = result.unwrap();
            assert_hour(&resumed, 1);
            drop(resumed);
        }
        drop(decoder);
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn multi_endpoint_attempt_refuses_opaque_older_caller_pins_after_a_small_step() {
    let budget = allocation::credit::MemoryBudget::new(4608);
    let mut encoder = FrameEncoder::new();
    let mut native = FrameDecoder::for_tables(TABLES);
    let mut writer = EndpointWriter::for_tables(TABLES);
    let mut endpoints = Vec::new();
    for hour in 0..=3 {
        let mut frame = source(if hour == 2 { 1 } else { hour });
        frame.hour = hour;
        if hour == 2 {
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                panic!("complete fixture countries");
            };
            rows[0].command_power_raw = 2;
        }
        let update = encoder.encode(frame, hour == 0).unwrap();
        let frame = native.advance(&update).unwrap();
        writer.observe(&update, frame).unwrap();
        let mut bytes = Vec::new();
        writer.finish(frame, &mut bytes).unwrap();
        endpoints.push(bytes);
    }
    let mut original = FrameDecoder::for_tables(TABLES).with_change_facts(budget.clone());
    for bytes in &endpoints[..2] {
        original
            .advance_endpoint(EndpointStep::decode(bytes).unwrap())
            .unwrap();
    }
    let mut attempt = original.clone();
    let checks = Cell::new(0);
    attempt
        .advance_endpoint_parallel_cancellable(
            EndpointStep::decode(&endpoints[2]).unwrap(),
            1,
            &|| {},
            &|| bounded_checks(&checks),
        )
        .unwrap();
    let current = attempt.current().unwrap();
    assert_eq!(current.hour, 2);
    assert_eq!(
        current.tables[TableId::Countries.index()]
            .2
            .as_ref()
            .unwrap()
            .changed()
            .len(),
        1
    );
    assert_eq!(
        original.current().unwrap().tables[TableId::Countries.index()]
            .2
            .as_ref()
            .unwrap()
            .changed()
            .len(),
        40
    );
    checks.set(0);
    let error = attempt
        .advance_endpoint_parallel_cancellable(
            EndpointStep::decode(&endpoints[3]).unwrap(),
            1,
            &|| {},
            &|| bounded_checks(&checks),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert_eq!(
        error.to_string(),
        "display change-fact working set remains pinned; common paging is required"
    );
    assert_hour(&original, 1);
    assert_eq!(attempt.current().unwrap().hour, 2);
    assert_eq!(checks.get(), 2);
    drop(original);
    attempt
        .advance_endpoint(EndpointStep::decode(&endpoints[3]).unwrap())
        .unwrap();
    assert_hour(&attempt, 3);
    assert!(budget.peak() <= 4608);
    drop(attempt);
    assert_eq!(budget.used(), 0);
}

#[test]
fn opaque_credit_pressure_refuses_without_waiting_or_consuming_the_baseline() {
    let budget = allocation::credit::MemoryBudget::new(3072);
    let mut decoder = FrameDecoder::for_tables(TABLES).with_change_facts(budget.clone());
    let updates = updates();
    decoder.advance(&updates[0]).unwrap();
    decoder.advance(&updates[1]).unwrap();
    let other = budget.admit(budget.limit() - budget.used()).unwrap();
    let checks = Cell::new(0);
    let error = change_credit(&budget, 128, Some(&|| bounded_checks(&checks))).unwrap_err();
    assert!(FrameDecoder::change_fact_admission_refused(&error));
    assert!(!FrameDecoder::change_fact_admission_refused(
        &io::Error::from(io::ErrorKind::OutOfMemory)
    ));
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert_eq!(checks.get(), 1);
    assert_hour(&decoder, 1);
    drop(other);
    assert_eq!(change_credit(&budget, 128, None).unwrap().bytes(), 128);
    drop(decoder);
    assert_eq!(budget.used(), 0);
}
