use super::*;
use replay_data::{
    event_feed_rows::EventFeedCounts, event_feed_rows::EventFeedEvent, rows::CountryLinkRow,
    rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

#[test]
fn script_occurrences_preserve_repeats_signed_choices_scopes_and_separate_retained_membership() {
    use replay_data::event_feed_rows::{
        EventFeedEventIdentity, EventFeedScope, EventFeedScript, EventFeedScriptChange,
    };
    let mut events = vec![
        event(1, EventFeedSource::ScriptEvent, 105),
        event(2, EventFeedSource::NewsEvent, 105),
        event(3, EventFeedSource::EventOption, 105),
        event(4, EventFeedSource::DecisionCompleted, 105),
        event(5, EventFeedSource::ScriptState, 105),
    ];
    for event in &mut events {
        event.script = Some(EventFeedScript {
            token_len: 11,
            scope_len: 1,
            native_event: Some(EventFeedEventIdentity { kind: 1, id: 9 }),
            instance: Some(7),
            ..Default::default()
        });
    }
    events[2].script.as_mut().unwrap().option = Some(-2);
    events[3].script.as_mut().unwrap().lifecycle = Some(3);
    events[3].script.as_mut().unwrap().days = Some(-8);
    events[3].script.as_mut().unwrap().numeric_target = Some(-19);
    events[4].script.as_mut().unwrap().change = Some(EventFeedScriptChange::FiredMember);
    events[4].script.as_mut().unwrap().instance = None;
    events[4].script.as_mut().unwrap().scope_len = 0;
    let mut row = pack(events);
    row.script_tokens = b"synthetic.1".to_vec();
    row.script_scopes = vec![EventFeedScope {
        random: [3, 4],
        ..Default::default()
    }];
    let mut builder = Builder::new(100, 120);
    append(&mut builder, &row, &BTreeMap::from([(105, frame(105, 2))])).unwrap();
    assert_eq!(builder.feed.rows.len(), 5);
    assert_eq!(builder.feed.rows[1].kind, EventFeedKind::NewsEventFired);
    assert!(
        builder.feed.rows[2]
            .cause
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.contains("-2"))
    );
    assert!(
        builder.feed.rows[3]
            .cause
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.contains("-8 days"))
    );
    assert!(
        builder.feed.rows[0]
            .cause
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.contains("ROOT 0, FROM 0, PREV 0"))
    );
    assert_eq!(builder.feed.rows[0].hour, 105);
    let notes = &builder.feed.rows[3].cause.as_ref().unwrap().notes;
    assert!(notes.iter().any(|note| note == "Numeric target: -19."));
    assert!(!notes.iter().any(|note| note.starts_with("State target:")));
    assert_eq!(builder.feed.rows[0].read_hour, 120);
}

fn identity(index: u32) -> EventFeedCountry {
    EventFeedCountry {
        reference: 100_000 + index,
        id: 100_000 + index,
        instance: 7 + index,
    }
}
fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "synthetic unused table".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    for rows in [
        TableRows::Countries(
            (0..count)
                .map(|index| CountryRow {
                    id: identity(index).id,
                    canonical_slot: index + 1,
                    tag: format!("synthetic / country {index}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::CountryLinks(
            (0..count)
                .map(|index| CountryLinkRow {
                    id: identity(index).id,
                    canonical_slot: index + 1,
                })
                .collect(),
        ),
        TableRows::CountryExperienceGains(
            (0..count)
                .map(|index| CountryExperienceGainRow {
                    country: identity(index).id,
                    instance: identity(index).instance,
                    complete: 0,
                    ..Default::default()
                })
                .collect(),
        ),
    ] {
        let id = rows.table();
        *frame.tables[id.index()] = TableView::Complete(rows);
        frame.metrics.table_ns[id.index()] = 1;
    }
    frame
}
fn event(sequence: u64, source: EventFeedSource, hour: i32) -> EventFeedEvent {
    EventFeedEvent {
        script: None,
        sequence,
        generation: 9,
        hour,
        source,
        actor: identity(0),
        target: identity(1),
        cause: Recorded::Value(NativeCause::default()),
    }
}
fn pack(events: Vec<EventFeedEvent>) -> EventFeedHistoryRow {
    let settled = events.len() as u64;
    EventFeedHistoryRow {
        script_refusals: 0,
        script_tokens: Vec::new(),
        script_scopes: Vec::new(),
        generation: 9,
        read_hour: 120,
        coverage: EventFeedCoverage::Complete,
        counts: Some(EventFeedCounts {
            acquired: settled,
            settled,
            ..Default::default()
        }),
        outstanding: 0,
        failure: None,
        events,
    }
}
fn append(
    builder: &mut Builder,
    pack: &EventFeedHistoryRow,
    frames: &BTreeMap<i32, DecodedFrame>,
) -> io::Result<()> {
    Projection::default().append(
        builder,
        pack,
        1,
        |hour| frames.get(&hour),
        &AtomicBool::new(false),
    )
}

#[test]
fn native_causes_keep_actor_order_aliases_and_each_distinct_occurrence() {
    let mut sample = frame(105, 2);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *sample.tables[TableId::CountryLinks.index()]
    {
        rows.insert(
            0,
            CountryLinkRow {
                id: 77,
                canonical_slot: 2,
            },
        );
    }
    let mut events: Vec<_> = [
        EventFeedSource::Declaration,
        EventFeedSource::Join,
        EventFeedSource::CivilWarWarTransfer,
        EventFeedSource::Annex,
        EventFeedSource::Declaration,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, source)| event(index as u64 + 1, source, 105))
    .collect();
    for event in &mut events {
        event.target.reference = 77
    }
    let mut builder = Builder::new(100, 130);
    builder.row(
        EventFeedKind::WarStarted,
        super::super::RowDate {
            hour: 105,
            read_hour: 120,
            name_hour: 105,
            exact: true,
        },
        vec![
            "synthetic / country 0".into(),
            "synthetic / country 1".into(),
        ],
        None,
        None,
    );
    append(
        &mut builder,
        &pack(events),
        &BTreeMap::from([(105, sample)]),
    )
    .unwrap();
    assert_eq!(
        builder
            .feed
            .rows
            .iter()
            .map(|row| row.kind)
            .collect::<Vec<_>>(),
        vec![
            EventFeedKind::WarStarted,
            EventFeedKind::WarDeclared,
            EventFeedKind::WarJoined,
            EventFeedKind::CivilWarWarTransferred,
            EventFeedKind::Annexed,
            EventFeedKind::WarDeclared,
        ]
    );
    assert_eq!(builder.feed.rows.len(), 6);
    for (index, row) in builder.feed.rows[1..].iter().enumerate() {
        assert_eq!(row.id, format!("native:9:{}", index + 1));
        assert_eq!(row.tags, ["synthetic / country 0", "synthetic / country 1"]);
        assert_eq!(
            row.participants,
            Some([Some(row.tags[0].clone()), Some(row.tags[1].clone())])
        );
        assert_eq!(
            (row.hour, row.read_hour, row.name_hour, row.exact),
            (105, 120, 105, true)
        );
    }
    assert!(builder.feed.unavailable.is_empty());
}

#[test]
fn native_projection_resume_preserves_late_dates_duplicates_and_sequence_guard() {
    let frames = BTreeMap::from([(105, frame(105, 2)), (106, frame(106, 2))]);
    let first = pack(vec![
        event(1, EventFeedSource::Declaration, 105),
        event(2, EventFeedSource::Declaration, 105),
    ]);
    let mut second = pack(vec![event(3, EventFeedSource::WarEnded, 106)]);
    second.read_hour = 121;
    let cancelled = AtomicBool::new(false);
    let project =
        |projection: &mut Projection, builder: &mut Builder, pack: &EventFeedHistoryRow| {
            projection.append(builder, pack, 1, |hour| frames.get(&hour), &cancelled)
        };
    let mut oracle = Builder::new(100, 130);
    let mut original = Projection::default();
    project(&mut original, &mut oracle, &first).unwrap();
    project(&mut original, &mut oracle, &second).unwrap();
    let mut prepared = Builder::new(100, 130);
    let mut resumed = Projection::default();
    project(&mut resumed, &mut prepared, &first).unwrap();
    resumed = serde_json::from_slice(&serde_json::to_vec(&resumed).unwrap()).unwrap();
    project(&mut resumed, &mut prepared, &second).unwrap();
    assert_eq!(
        serde_json::to_value(&prepared.feed.rows).unwrap(),
        serde_json::to_value(&oracle.feed.rows).unwrap()
    );
    assert_eq!(prepared.war_ends, oracle.war_ends);
    assert_eq!(prepared.feed.rows.len(), 3);
    assert_eq!(
        prepared
            .feed
            .rows
            .iter()
            .map(|row| (row.hour, row.read_hour))
            .collect::<Vec<_>>(),
        [(105, 120), (105, 120), (106, 121)]
    );
    assert_ne!(prepared.feed.rows[0].id, prepared.feed.rows[1].id);
    assert_eq!(
        project(&mut resumed, &mut prepared, &second)
            .unwrap_err()
            .to_string(),
        "Native event occurrence sequence differs"
    );
    assert_eq!(prepared.feed.rows.len(), 3);
}

#[test]
fn full_instance_read_and_branch_guards_never_use_later_or_reused_aliases() {
    for mode in 0..6 {
        let mut sample = frame(105, 2);
        match mode {
            0 => {
                if let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
                    &mut *sample.tables[TableId::CountryExperienceGains.index()]
                {
                    rows[0].instance += 1
                }
            }
            1 => sample.hour = 106,
            2 => sample.branch = 2,
            3 => sample.metrics.table_ns[TableId::CountryExperienceGains.index()] = 0,
            4 => {
                *sample.tables[TableId::CountryExperienceGains.index()] = TableView::Unavailable {
                    reason: "census refused".into(),
                }
            }
            _ => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *sample.tables[TableId::Countries.index()]
                {
                    rows[0].id += 5
                }
            }
        }
        let mut builder = Builder::new(100, 130);
        append(
            &mut builder,
            &pack(vec![event(1, EventFeedSource::Annex, 105)]),
            &BTreeMap::from([(105, sample), (106, frame(106, 2))]),
        )
        .unwrap();
        assert_eq!(
            builder.feed.rows[0].participants.as_ref().unwrap()[0],
            None,
            "mode {mode}"
        );
        assert_eq!(builder.feed.rows[0].kind, EventFeedKind::Annexed);
        assert!(
            builder
                .feed
                .unavailable
                .iter()
                .any(|item| item.kind == "nativeEventCountry")
        );
    }
}

#[test]
fn ambiguous_tag_census_alias_or_reserved_zero_preserves_unidentified_roles() {
    for mode in 0..4 {
        let mut sample = frame(105, 2);
        let mut at = event(1, EventFeedSource::Declaration, 105);
        match mode {
            0 => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *sample.tables[TableId::Countries.index()]
                {
                    rows[1].tag = rows[0].tag.clone()
                }
            }
            1 => {
                if let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
                    &mut *sample.tables[TableId::CountryExperienceGains.index()]
                {
                    rows.push(rows[0].clone())
                }
            }
            2 => {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *sample.tables[TableId::CountryLinks.index()]
                {
                    rows.insert(
                        0,
                        CountryLinkRow {
                            id: at.actor.reference,
                            canonical_slot: 2,
                        },
                    )
                }
            }
            _ => {
                at.actor = EventFeedCountry {
                    reference: 0,
                    id: 0,
                    instance: 7,
                }
            }
        }
        let mut builder = Builder::new(100, 130);
        append(
            &mut builder,
            &pack(vec![at]),
            &BTreeMap::from([(105, sample)]),
        )
        .unwrap();
        assert_eq!(builder.feed.rows.len(), 1);
        assert_eq!(
            builder.feed.rows[0].participants.as_ref().unwrap()[0],
            None,
            "mode {mode}"
        );
        assert!(
            !builder.feed.rows[0]
                .tags
                .contains(&"synthetic / country 0".into())
        );
    }
}

#[test]
fn incomplete_coverage_keeps_known_actions_and_counts_read_intervals_without_guessing_events() {
    for coverage in [EventFeedCoverage::Partial, EventFeedCoverage::Unavailable] {
        let mut observed = pack(vec![event(1, EventFeedSource::Join, 105)]);
        observed.coverage = coverage;
        observed.failure = Some(b"native writer observation refused".to_vec());
        let mut builder = Builder::new(100, 130);
        append(&mut builder, &observed, &BTreeMap::new()).unwrap();
        assert_eq!(builder.feed.rows.len(), 1);
        assert_eq!(builder.feed.rows[0].participants, Some([None, None]));
        assert_eq!(builder.feed.rows[0].tags, Vec::<String>::new());
        let unavailable = builder
            .feed
            .unavailable
            .iter()
            .find(|item| item.kind == "nativeEventCoverage")
            .unwrap();
        assert_eq!(unavailable.count, Some(1));
        assert!(
            unavailable
                .reason
                .contains("Missing rows do not prove no event occurred")
        );
        assert_eq!(
            builder
                .feed
                .unavailable
                .iter()
                .find(|item| item.kind == "nativeEventCountry")
                .unwrap()
                .count,
            Some(2)
        );
    }
}

#[test]
fn occurrence_range_and_sequence_validation_are_independent_of_read_visibility() {
    let mut builder = Builder::new(106, 130);
    append(
        &mut builder,
        &pack(vec![event(1, EventFeedSource::Declaration, 105)]),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(builder.feed.rows.is_empty());
    let mut projection = Projection::default();
    let first = pack(vec![event(5, EventFeedSource::Declaration, 105)]);
    let mut builder = Builder::new(100, 130);
    projection
        .append(&mut builder, &first, 1, |_| None, &AtomicBool::new(false))
        .unwrap();
    assert!(
        projection
            .append(&mut builder, &first, 1, |_| None, &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(builder.feed.rows.len(), 1);
    assert!(
        append(
            &mut builder,
            &pack(vec![event(6, EventFeedSource::Join, 121)]),
            &BTreeMap::new()
        )
        .is_err()
    );
    assert_eq!(builder.feed.rows.len(), 1);
}

#[test]
fn five_thousand_holder_events_share_occurrence_witnesses_and_existing_reply_windows() {
    let sample = frame(105, 5000);
    let observed = pack(
        (0..5000)
            .map(|index| EventFeedEvent {
                script: None,
                actor: identity(index),
                target: identity((index + 1) % 5000),
                ..event(u64::from(index) + 1, EventFeedSource::Declaration, 105)
            })
            .collect(),
    );
    let mut builder = Builder::new(100, 130);
    let mut frame_reads = 0;
    Projection::default()
        .append(
            &mut builder,
            &observed,
            1,
            |_| {
                frame_reads += 1;
                Some(&sample)
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(frame_reads, 1);
    assert_eq!(builder.feed.rows.len(), 5000);
    assert_eq!(builder.feed.rows[4999].tags[0], "synthetic / country 4999");
    let feed = builder.finish().unwrap();
    assert_eq!(
        feed.windows.iter().map(|window| window.count).sum::<u32>(),
        5000
    );
    assert!(super::super::window_fits(
        &feed,
        "1",
        105,
        105,
        super::super::MAX_PRODUCT_REPLY_BYTES
    ));
}

#[test]
fn complete_empty_history_is_distinct_from_an_unobserved_empty_interval() {
    for coverage in [
        EventFeedCoverage::Complete,
        EventFeedCoverage::Partial,
        EventFeedCoverage::Unavailable,
    ] {
        let mut observed = pack(Vec::new());
        observed.coverage = coverage;
        if coverage == EventFeedCoverage::Unavailable {
            observed.failure = Some(b"producer unavailable".to_vec())
        }
        let mut builder = Builder::new(100, 130);
        append(&mut builder, &observed, &BTreeMap::new()).unwrap();
        assert!(builder.feed.rows.is_empty());
        assert_eq!(
            builder
                .feed
                .unavailable
                .iter()
                .any(|item| item.kind == "nativeEventCoverage"),
            coverage != EventFeedCoverage::Complete
        );
    }
}

#[test]
fn registered_history_loads_later_publication_without_using_the_later_country_instance() {
    use crate::test_fixtures::{CAMPAIGN, records};
    use replay::container::Replay;
    use replay_data::rows::CampaignRow;
    let mut frames = Vec::new();
    for hour in 105..=120 {
        let mut sample = frame(hour, 2);
        *sample.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: CAMPAIGN.into(),
            }]));
        let observed = if hour == 120 {
            pack(vec![event(4, EventFeedSource::Annex, 105)])
        } else {
            EventFeedHistoryRow {
                read_hour: hour,
                ..pack(Vec::new())
            }
        };
        *sample.tables[TableId::EventFeedHistory.index()] =
            TableView::Complete(TableRows::EventFeedHistory(vec![observed]));
        if hour > 105
            && let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
                &mut *sample.tables[TableId::CountryExperienceGains.index()]
        {
            rows[0].instance = 99;
        }
        frames.push(sample);
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut builder = Builder::new(105, 120);
    scan(&replay, 1, &mut builder, &AtomicBool::new(false)).unwrap();
    assert_eq!(builder.feed.rows.len(), 1);
    let row = &builder.feed.rows[0];
    assert_eq!((row.hour, row.read_hour), (105, 120));
    assert_eq!(
        row.participants,
        Some([
            Some("synthetic / country 0".into()),
            Some("synthetic / country 1".into())
        ])
    );
}

/// A recorded cause reaches the row in words with its ally's exact
/// occurrence-hour tag; an ally without a witness stays unnamed and is counted.
#[test]
fn recorded_causes_name_their_route_ally_and_operands() {
    let frames = BTreeMap::from([(105, frame(105, 3))]);
    let mut join = event(1, EventFeedSource::Join, 105);
    join.cause = Recorded::Value(NativeCause {
        route: EventFeedRoute::CallAlly,
        within: EventFeedRoute::DiplomaticCommand,
        ally: Some(identity(2)),
        reason: Some(2),
        flags: 1,
        ..Default::default()
    });
    let mut transfer = event(2, EventFeedSource::StateTransfer, 105);
    transfer.cause = Recorded::Value(NativeCause {
        route: EventFeedRoute::StateAnnex,
        within: EventFeedRoute::PeaceAnnex,
        conference: Some(0x0000_0007_0000_0031),
        state: Some(64),
        flags: 1,
        ..Default::default()
    });
    let mut unwitnessed = event(3, EventFeedSource::Join, 105);
    unwitnessed.cause = Recorded::Value(NativeCause {
        route: EventFeedRoute::AddToWarEffect,
        ally: Some(identity(9)),
        ..Default::default()
    });
    let mut builder = Builder::new(100, 130);
    append(
        &mut builder,
        &pack(vec![join, transfer, unwitnessed]),
        &frames,
    )
    .unwrap();
    let rows = &builder.feed.rows;
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        [
            EventFeedKind::WarJoined,
            EventFeedKind::StateTransferred,
            EventFeedKind::WarJoined
        ]
    );
    assert_eq!(
        rows[0].cause,
        Some(EventFeedCause {
            how: Some("a call to arms".into()),
            within: Some("a diplomatic action".into()),
            conference: None,
            state: None,
            ally: Some("synthetic / country 2".into()),
            reason: Some("ally".into()),
            notes: vec!["against every enemy of the ally's side".into()],
        })
    );
    let transfer = rows[1].cause.as_ref().unwrap();
    assert_eq!(
        (
            transfer.how.as_deref(),
            transfer.state,
            transfer.conference.as_deref()
        ),
        (Some("an annexation"), Some(64), Some("30064771121"))
    );
    // A state transfer's first byte is SetOwner's, not a truce or troops.
    assert!(transfer.notes.is_empty());
    assert_eq!(rows[2].cause.as_ref().unwrap().ally, None);
    assert!(
        builder
            .feed
            .unavailable
            .iter()
            .any(|item| item.reason.contains("The ally of event 3"))
    );
}

#[test]
fn every_route_but_none_has_words() {
    for route in EventFeedRoute::ALL {
        assert_eq!(
            route_words(route).is_none(),
            route == EventFeedRoute::None,
            "{route:?}"
        );
    }
}

/// A partial interval ending at a recorded event loss names the detached
/// recorder as its reason; any other partial interval keeps the generic one.
#[test]
fn a_partial_interval_after_a_recorded_event_loss_names_the_detached_recorder() {
    let reason = |partial: &EventFeedHistoryRow, event_losses: Vec<(i32, i32)>| {
        let mut builder = Builder::new(100, 130);
        Projection {
            event_losses,
            ..Projection::default()
        }
        .append_named(&mut builder, partial, |_, _| None, &AtomicBool::new(false))
        .unwrap();
        builder.feed.unavailable[0].reason.clone()
    };
    let mut partial = pack(Vec::new());
    partial.coverage = EventFeedCoverage::Partial;
    let lost = reason(&partial, vec![(119, 120)]);
    assert!(lost.contains("recorder was detached"), "{lost}");
    assert!(lost.contains("between 119 and 120"), "{lost}");
    for other in [vec![], vec![(120, 121)]] {
        let generic = reason(&partial, other);
        assert!(generic.contains("not fully observed"), "{generic}");
    }
    // An independent failure of the resumed interval is kept beside the loss.
    partial.coverage = EventFeedCoverage::Unavailable;
    partial.failure = Some(b"Event feed acquired events unsettled".to_vec());
    let both = reason(&partial, vec![(119, 120)]);
    assert!(both.contains("recorder was detached"), "{both}");
    assert!(both.contains("acquired events unsettled"), "{both}");
}

#[test]
fn complete_transport_still_names_and_counts_every_refused_script_family_per_interval() {
    let mut builder = Builder::new(100, 130);
    let mut pack = pack(Vec::new());
    pack.script_refusals = replay_data::event_feed_rows::DECISION_FAMILY_REFUSALS;
    let mut projection = Projection::default();
    for hour in [120, 121] {
        pack.read_hour = hour;
        projection
            .append_named(&mut builder, &pack, |_, _| None, &AtomicBool::new(false))
            .unwrap();
    }
    let refused: Vec<_> = builder
        .feed
        .unavailable
        .iter()
        .filter(|row| row.kind == "nativeScriptFamilyCoverage")
        .collect();
    assert_eq!(refused.len(), 5);
    assert!(refused.iter().all(|row| row.count == Some(2)));
    assert!(
        refused
            .iter()
            .any(|row| row.label.contains("removal and re-enable timers"))
    );
    assert!(
        !builder
            .feed
            .unavailable
            .iter()
            .any(|row| row.kind == "nativeEventCoverage")
    );
}

#[test]
fn certified_roster_rebuilds_are_visible_without_inventing_baseline_selections() {
    use replay_data::event_feed_rows::{
        EventFeedScript, EventFeedScriptChange,
        decisions::{DecisionChange, Operation},
    };
    let events = [Operation::Baseline, Operation::Ready, Operation::Rebuild]
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            let mut event = event(index as u64 + 1, EventFeedSource::ScriptState, 105);
            event.script = Some(EventFeedScript {
                change: Some(EventFeedScriptChange::Decision(DecisionChange::new(
                    operation,
                    if operation == Operation::Rebuild {
                        3
                    } else {
                        0
                    },
                ))),
                days: (operation == Operation::Rebuild).then_some(7),
                lifecycle: (operation == Operation::Rebuild).then_some(0),
                ..Default::default()
            });
            event
        })
        .collect();
    let row = pack(events);
    let mut builder = Builder::new(100, 120);
    append(&mut builder, &row, &BTreeMap::from([(105, frame(105, 2))])).unwrap();
    assert_eq!(
        builder
            .feed
            .rows
            .iter()
            .filter(|row| row.kind == EventFeedKind::DecisionRosterRebuilt)
            .count(),
        1
    );
    assert!(builder.feed.rows.iter().all(|row| matches!(
        row.kind,
        EventFeedKind::DecisionRosterRebuilt | EventFeedKind::ScriptStateObserved
    )));
    assert_eq!(builder.script_inputs.len(), 1);
}

fn decision_boundary(
    sequence: u64,
    operation: replay_data::event_feed_rows::decisions::Operation,
    phase: u8,
) -> EventFeedEvent {
    let mut value = event(sequence, EventFeedSource::ScriptState, 120);
    value.script = Some(replay_data::event_feed_rows::EventFeedScript {
        change: Some(
            replay_data::event_feed_rows::EventFeedScriptChange::Decision(
                replay_data::event_feed_rows::decisions::DecisionChange::new(operation, phase),
            ),
        ),
        ..Default::default()
    });
    value
}
#[test]
fn only_final_roster_phase_emits_an_occurrence_and_resume_carries_owner_certificate() {
    use replay_data::event_feed_rows::decisions::Operation;
    let mut baseline = pack(vec![
        decision_boundary(1, Operation::Baseline, 0),
        decision_boundary(2, Operation::Ready, 0),
    ]);
    baseline.read_hour = 120;
    let mut live = Projection::default();
    live.append_named(
        &mut Builder::new(100, 130),
        &baseline,
        |_, _| Some("SYN".into()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut resumed: Projection =
        serde_json::from_slice(&serde_json::to_vec(&live).unwrap()).unwrap();
    let mut intermediate = pack(
        (0..3)
            .map(|phase| decision_boundary(3 + phase as u64, Operation::Rebuild, phase))
            .collect(),
    );
    intermediate.read_hour = 121;
    let mut builder = Builder::new(100, 130);
    resumed
        .append_named(
            &mut builder,
            &intermediate,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(builder.feed.rows.is_empty());
    assert!(builder.script_inputs.is_empty());
    let mut final_row = pack(vec![decision_boundary(6, Operation::Rebuild, 3)]);
    final_row.read_hour = 122;
    resumed
        .append_named(
            &mut builder,
            &final_row,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(builder.feed.rows.len(), 1);
    let inputs = builder.script_inputs.values().next().unwrap();
    assert_eq!(
        serde_json::to_value(inputs).unwrap()["roster"]["complete"],
        true
    );
}
#[test]
fn retained_pending_due_after_end_fired_members_and_delayed_scopes_survive_resume() {
    use replay_data::event_feed_rows::{
        EventFeedEventIdentity, EventFeedScope, EventFeedScript, EventFeedScriptChange as C,
    };
    let changes = [
        C::PendingBaseline,
        C::PendingStored { due_hour: 999 },
        C::PendingReady,
        C::FiredBaseline,
        C::FiredMember,
        C::FiredReady,
        C::DelayedBaseline,
        C::DelayedStored { remaining: 500 },
        C::DelayedReady,
    ];
    let mut baseline = pack(
        changes
            .into_iter()
            .enumerate()
            .map(|(index, change)| {
                let mut value = event(index as u64 + 1, EventFeedSource::ScriptState, 120);
                let stored = matches!(change, C::PendingStored { .. } | C::DelayedStored { .. });
                value.script = Some(EventFeedScript {
                    change: Some(change),
                    native_event: matches!(
                        change,
                        C::PendingStored { .. } | C::DelayedStored { .. } | C::FiredMember
                    )
                    .then_some(EventFeedEventIdentity { kind: 7, id: 9 }),
                    token_len: if stored || change == C::FiredMember {
                        3
                    } else {
                        0
                    },
                    instance: stored.then_some(17),
                    scope_len: u32::from(stored),
                    ..Default::default()
                });
                value
            })
            .collect(),
    );
    baseline.script_tokens = b"mod".to_vec();
    baseline.script_scopes = vec![EventFeedScope {
        country: 100_000,
        random: [12, 13],
        ..Default::default()
    }];
    let mut projection = Projection::default();
    let mut builder = Builder::new(100, 130);
    projection
        .append_named(
            &mut builder,
            &baseline,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(builder.feed.rows.len(), 1);
    assert_eq!(
        builder.feed.rows[0].kind,
        EventFeedKind::ScriptStateObserved
    );
    assert!(!builder.feed.rows[0].exact);
    let notes = builder.feed.rows[0]
        .cause
        .as_ref()
        .unwrap()
        .notes
        .join("\n");
    assert!(notes.contains("due hour 999"));
    assert!(notes.contains("Fired-set member: mod"));
    assert!(notes.contains("remaining 500 hours"));
    assert!(notes.contains("ROOT 0, FROM 0, PREV 0"));
    assert!(notes.contains("no recorded fire or selection time"));
    let mut resumed: Projection =
        serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
    let mut tail = pack(vec![event(10, EventFeedSource::ScriptState, 121)]);
    tail.read_hour = 121;
    tail.events[0].script = Some(EventFeedScript {
        change: Some(C::DelayedAdvance {
            count: 1,
            finished: true,
        }),
        ..Default::default()
    });
    let mut expected = Builder::new(100, 130);
    let mut actual = Builder::new(100, 130);
    projection
        .append_named(
            &mut expected,
            &tail,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    resumed
        .append_named(
            &mut actual,
            &tail,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&actual.feed.rows).unwrap(),
        serde_json::to_value(&expected.feed.rows).unwrap()
    );
    let notes = actual.feed.rows[0].cause.as_ref().unwrap().notes.join("\n");
    assert!(notes.contains("remaining 499 hours"));
    assert!(notes.contains("due hour 999"));
}

#[test]
fn preparation_resume_keeps_a_lost_delayed_queue_retired_until_its_baseline() {
    use replay_data::event_feed_rows::{
        EventFeedEventIdentity, EventFeedScope, EventFeedScript, EventFeedScriptChange as C,
    };
    let delayed = |sequence: u64, hour: i32, change, timer: Option<(u32, u32)>| {
        let mut value = event(sequence, EventFeedSource::ScriptState, hour);
        value.script = Some(EventFeedScript {
            change: Some(change),
            native_event: timer.map(|(_, id)| EventFeedEventIdentity { kind: 7, id }),
            instance: timer.map(|(instance, _)| instance),
            token_len: if timer.is_some() { 3 } else { 0 },
            scope_len: u32::from(timer.is_some()),
            ..Default::default()
        });
        value
    };
    let hour = |read_hour: i32, events: Vec<EventFeedEvent>| {
        let mut row = pack(events);
        row.read_hour = read_hour;
        row.script_tokens = b"mod".to_vec();
        row.script_scopes = vec![EventFeedScope::default()];
        row
    };
    let rows = [
        hour(
            120,
            vec![
                delayed(1, 120, C::DelayedBaseline, None),
                delayed(2, 120, C::DelayedStored { remaining: 500 }, Some((17, 9))),
                delayed(3, 120, C::DelayedReady, None),
            ],
        ),
        // The loss is known on this row; its store meets no stale queue.
        {
            let mut row = hour(
                121,
                vec![delayed(
                    4,
                    121,
                    C::DelayedStored { remaining: 1 },
                    Some((17, 10)),
                )],
            );
            row.coverage = EventFeedCoverage::Unavailable;
            row.failure = Some(b"synthetic observation overlap".to_vec());
            row
        },
        hour(
            122,
            vec![delayed(
                5,
                122,
                C::DelayedStored { remaining: 2 },
                Some((17, 11)),
            )],
        ),
        hour(
            123,
            vec![
                delayed(6, 123, C::DelayedBaseline, None),
                delayed(7, 123, C::DelayedStored { remaining: 497 }, Some((18, 9))),
                delayed(8, 123, C::DelayedReady, None),
            ],
        ),
    ];
    let append = |projection: &mut Projection, builder: &mut Builder, row| {
        projection
            .append_named(
                builder,
                row,
                |_, _| Some("SYN".into()),
                &AtomicBool::new(false),
            )
            .unwrap()
    };
    let mut projection = Projection::default();
    let mut ignored = Builder::new(100, 130);
    for row in &rows[..2] {
        append(&mut projection, &mut ignored, row);
    }
    let mut resumed: Projection =
        serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
    let mut expected = Builder::new(100, 130);
    let mut actual = Builder::new(100, 130);
    for row in &rows[2..] {
        append(&mut projection, &mut expected, row);
        append(&mut resumed, &mut actual, row);
    }
    assert_eq!(
        serde_json::to_value(&actual.feed.rows).unwrap(),
        serde_json::to_value(&expected.feed.rows).unwrap()
    );
    assert_eq!(actual.feed.rows.len(), 2);
    let notes = |row: usize| {
        actual.feed.rows[row]
            .cause
            .as_ref()
            .unwrap()
            .notes
            .join("\n")
    };
    // The uncertified store is still reported as an observation.
    assert!(notes(0).contains("Observation 5 at 122: DelayedStored"));
    assert!(notes(0).contains("SYN delayed timers unavailable"));
    assert!(notes(1).contains("SYN: 1 delayed event timers."));
    assert!(notes(1).contains("SYN delayed instance 18: mod [9 (type 7)]; remaining 497 hours"));
}

#[test]
fn preparation_resume_preserves_an_open_native_timer_pass() {
    use replay_data::event_feed_rows::decisions::{Operation, TIMED};
    let mut stored = decision_boundary(2, Operation::Store, TIMED);
    let script = stored.script.as_mut().unwrap();
    script.instance = Some(17);
    script.token_len = 3;
    script.days = Some(10);
    script.lifecycle = Some(0);
    let mut begin = decision_boundary(4, Operation::PassBegin, TIMED);
    if let Some(replay_data::event_feed_rows::EventFeedScriptChange::Decision(ref mut change)) =
        begin.script.as_mut().unwrap().change
    {
        change.ordinal = 1;
    }
    let mut baseline = pack(vec![
        decision_boundary(1, Operation::Baseline, 0),
        stored,
        decision_boundary(3, Operation::Ready, 0),
        begin,
    ]);
    baseline.script_tokens = b"mod".to_vec();
    let mut projection = Projection::default();
    projection
        .append_named(
            &mut Builder::new(100, 130),
            &baseline,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    let mut resumed: Projection =
        serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
    let mut tail = pack(vec![
        decision_boundary(5, Operation::PassEnd, TIMED),
        decision_boundary(6, Operation::Rebuild, 3),
    ]);
    tail.read_hour = 121;
    let mut expected = Builder::new(100, 130);
    let mut actual = Builder::new(100, 130);
    projection
        .append_named(
            &mut expected,
            &tail,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    resumed
        .append_named(
            &mut actual,
            &tail,
            |_, _| Some("SYN".into()),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&actual.feed.rows).unwrap(),
        serde_json::to_value(&expected.feed.rows).unwrap()
    );
    let inputs = serde_json::to_value(actual.script_inputs.values().next().unwrap()).unwrap();
    assert_eq!(inputs["roster"]["complete"], true);
    assert_eq!(inputs["roster"]["members"][0]["days"], 9);
}
