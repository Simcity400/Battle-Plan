use super::*;

#[test]
fn wake_matching_uses_sequence_or_exact_ticket_not_the_longest_job() {
    let ticket = Ticket::diagnostic(1, 2, 3);
    let mut hour = Hour {
        wait: Some(Wait {
            address: Address::Readiness,
            comparison: 7,
            observed: 9,
            begin: 100,
            end: 500,
            ..Default::default()
        }),
        ..Default::default()
    };
    for (sequence, at) in [(7, 120), (10, 200), (8, 501), (9, 300), (8, 250)] {
        hour.signal(Signal {
            readiness: sequence,
            wake_begin: at,
            ..Default::default()
        });
    }
    assert_eq!(hour.matching_signals, 2);
    assert_eq!(hour.signal.unwrap().readiness, 8);
    let wait = Wait {
        address: Address::Slot,
        ticket: Some(ticket),
        begin: 100,
        end: 500,
        ..Default::default()
    };
    let signal = Signal {
        ticket: Some(ticket),
        slot: true,
        released: 99,
        wake_begin: 110,
        ..Default::default()
    };
    assert!(
        signal.matches(wait),
        "wake-before-wait must remain attributable"
    );
    assert!(
        !Signal {
            ticket: Some(Ticket::diagnostic(1, 2, 4)),
            ..signal
        }
        .matches(wait)
    );
    assert!(
        !Signal {
            ticket: Some(Ticket::diagnostic(2, 2, 3)),
            ..signal
        }
        .matches(wait)
    );
}

#[test]
fn detached_wake_storage_counts_overflow_and_resets_only_after_settlement() {
    let events = Events::new(2).unwrap();
    let mut hour = Hour {
        wait: Some(Wait {
            comparison: 1,
            observed: 4,
            end: 50,
            ..Default::default()
        }),
        ..Default::default()
    };
    std::thread::scope(|scope| {
        for readiness in 2..=4 {
            let events = &events;
            scope.spawn(move || {
                events.record(Signal {
                    readiness,
                    wake_begin: 10,
                    wake_end: 11,
                    ..Default::default()
                })
            });
        }
    });
    // SAFETY: all event writers were joined by the scope.
    unsafe { events.drain_into(&mut hour) };
    assert!(hour.tracing_available);
    assert_eq!(hour.matching_signals, 2);
    assert_eq!(hour.dropped_signals, 1);
    let mut next = Hour::default();
    // SAFETY: the synthetic drained log has no publishers.
    unsafe { events.drain_into(&mut next) };
    assert_eq!(next.dropped_signals, 0);
    assert_eq!(next.matching_signals, 0);
}

#[test]
fn wait_status_distinguishes_missing_measurements_and_publisher_return_after_collector_return() {
    let missing = serde_json::to_value(Hour::default().status(10)).unwrap();
    assert!(missing["longest_address_wait"].is_null());
    assert!(missing["dropped_signals"].is_null());
    let wait = Wait {
        begin: 100,
        end: 200,
        ..Default::default()
    };
    let signal = Signal {
        published: 110,
        released: 120,
        wake_begin: 130,
        wake_end: 210,
        ..Default::default()
    };
    let status = serde_json::to_value(signal.status(10, wait)).unwrap();
    assert_eq!(status["publish_to_wait_return_ns"], 9_000_000_000_u64);
    assert_eq!(status["wake_call_ns"], 8_000_000_000_u64);
    assert!(status["readiness_sequence"].is_null());
    let readiness = serde_json::to_value(
        Signal {
            readiness: 7,
            handoff: true,
            ..signal
        }
        .status(10, wait),
    )
    .unwrap();
    assert_eq!(readiness["readiness_sequence"], 7);
    assert_eq!(readiness["source"], "inputs_handoff");
    assert!(
        serde_json::to_value(
            Signal {
                published: 0,
                ..signal
            }
            .status(10, wait)
        )
        .unwrap()["publish_to_wait_return_ns"]
            .is_null()
    );
}

#[test]
fn family_keeps_measured_totals_and_the_four_largest_tables_with_stable_ties() {
    use replay_data::rows::TableId;
    let family = Family::new(TableId::ALL[..6].iter().copied().zip([3, 9, 4, 9, 1, 0]));
    assert_eq!(family.total, 26);
    assert_eq!(family.top.map(|table| table.index), [1, 3, 2, 0]);
    assert_eq!(family.len, 4);
}
