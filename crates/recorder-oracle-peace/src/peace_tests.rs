use super::*;
use replay_data::rows::*;
use replay_frame::{frame::Metrics, frame::TableView};

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [b"MOD_A".as_slice(), b"MOD_B", b"MOD_C"]
            .into_iter()
            .enumerate()
            .map(|(i, tag)| CountryRow {
                id: 1001 + i as u32,
                canonical_slot: 1 + i as u32,
                tag: tag.to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..3)
            .map(|i| CountryLinkRow {
                id: 1001 + i,
                canonical_slot: 1 + i,
            })
            .collect(),
    ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 99,
            literal: b"custom_demand".to_vec(),
        }]));
    frame
}
fn row(hour: i32) -> PeaceSettlementRow {
    PeaceSettlementRow {
        sequence: 1,
        conference: Some((3_u64 << 32) | 7),
        hour: Some(hour),
        actions: vec![SettledPeaceAction {
            token: 99,
            country: 1002,
            taker: 1002,
            giver: 1001,
            negotiator: 1003,
            cost: -2,
            status: 3,
            state: Some(17),
            flags: Some([1, 0, 0, 0]),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn resident_bid_cached_name_compares_saved_override_without_waiving_a_difference() {
    let parsed = content_reader_core::parser::parse(br#"countries={} peace_conference={active_peace={id={id=3 type=7} history={MOD_B={bidding={country=MOD_B actions={custom_demand={name="Saved type" taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=17 demilitarized_zone=yes}}}}}}}"#).unwrap();
    let saved = projects::parse(&parsed.items).unwrap();
    let mut frame = frame(100);
    let mut bid = row(100);
    bid.actions[0].cached_name = Some(b"Saved type".to_vec());
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid.clone()]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    bid.actions[0].cached_name = Some(b"Wrong type".to_vec());
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("cached_name"));
}

fn recorded_hours<'a>(
    frames: &'a [&'a DecodedFrame],
) -> impl FnMut(Hour) -> io::Result<Option<&'a DecodedFrame>> + 'a {
    move |hour| Ok(frames.iter().copied().find(|frame| frame.hour == hour))
}

fn empty_settlement_save() -> Save {
    let mut saved = save::parse(br#"HOI4txt date="1936.1.1.1" game_unique_id="fixture" version="build" game_unique_seed=1
        debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6
        military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9
        country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
        countries={MOD_A={} MOD_B={} MOD_C={}} states={} provinces={}"#).unwrap();
    saved.projects.conferences = Some(BTreeMap::new());
    saved
}

#[test]
fn vanished_bid_conference_requires_capture_or_counted_loss() {
    let saved = empty_settlement_save();
    let mut previous = frame(100);
    *previous.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    let mut current = frame(101);
    let mut report = Report::default();
    compare_settlements_after(&current, recorded_hours(&[&previous]), &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("finalized capture"));
    *current.tables[TableId::PeaceSettlements.index()] = TableView::Truncated {
        rows: TableRows::PeaceSettlements(vec![]),
        missing: 1,
    };
    let mut report = Report::default();
    assert!(
        compare_settlements_after(&current, recorded_hours(&[&previous]), &saved, &mut report)
            .is_err()
    );
    assert!(
        report.differences.is_empty(),
        "loss is counted, but truncation still fails the oracle"
    );
}

#[test]
fn finalized_resident_conferences_use_their_earlier_publication_after_queue_cleanup() {
    let saved = empty_settlement_save();
    let mut hours: Vec<_> = (99..104).map(frame).collect();
    let id = row(100).conference.unwrap();
    for prior in &mut hours {
        let mut bid = row(prior.hour);
        if prior.hour >= 100 {
            bid.actions.clear();
        }
        *prior.tables[TableId::PeaceBids.index()] =
            TableView::Complete(TableRows::PeaceBids(vec![bid]));
        *prior.tables[TableId::PeaceConferences.index()] =
            TableView::Complete(TableRows::PeaceConferences(vec![PeaceConferenceRow {
                id,
                finalized: u8::from(prior.hour >= 100),
                ..Default::default()
            }]));
    }
    *hours[1].tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    for at in [101, 104] {
        let priors: Vec<_> = hours.iter().filter(|frame| frame.hour < at).collect();
        let mut report = Report::default();
        compare_settlements_after(&frame(at), recorded_hours(&priors), &saved, &mut report)
            .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(
            report.compared_tables["PeaceSettlements previously finalized resident conferences"],
            1
        );
        assert_eq!(
            report.compared_tables["PeaceSettlements finalized conference census"],
            1
        );
    }
    *hours[1].tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![]));
    let priors: Vec<_> = hours.iter().collect();
    let mut report = Report::default();
    compare_settlements_after(&frame(104), recorded_hours(&priors), &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains(&id.to_string()));
}

#[test]
fn already_finalized_at_the_observation_boundary_is_a_named_limitation() {
    let saved = empty_settlement_save();
    let mut first = frame(100);
    let mut bid = row(100);
    bid.actions.clear();
    *first.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid]));
    *first.tables[TableId::PeaceConferences.index()] =
        TableView::Complete(TableRows::PeaceConferences(vec![PeaceConferenceRow {
            id: row(100).conference.unwrap(),
            finalized: 1,
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare_settlements_after(&frame(101), recorded_hours(&[&first]), &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("pre-observation"))
    );
    assert_eq!(
        report.compared_tables["PeaceSettlements previously finalized resident conferences"],
        0
    );
    *first.tables[TableId::PeaceConferences.index()] = TableView::Unavailable {
        reason: "unread first roster".into(),
    };
    let mut report = Report::default();
    compare_settlements_after(&frame(101), recorded_hours(&[&first]), &saved, &mut report).unwrap();
    assert_eq!(
        report.differences.len(),
        1,
        "unknown is not a finalized boundary witness"
    );
}

#[test]
fn earlier_publications_require_continuous_residency_and_full_conference_identity() {
    let saved = empty_settlement_save();
    let mut earlier = frame(100);
    *earlier.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    *earlier.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    let mut previous = frame(102);
    *previous.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(102)]));
    let absent = frame(101);
    let mut report = Report::default();
    compare_settlements_after(
        &frame(103),
        recorded_hours(&[&earlier, &absent, &previous]),
        &saved,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        report.differences.len(),
        1,
        "an older lifetime cannot certify this disappearance"
    );
    previous.hour = 101;
    let mut reused = row(101);
    reused.conference = Some((4_u64 << 32) | 7);
    *previous.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![reused]));
    let mut report = Report::default();
    compare_settlements_after(
        &frame(102),
        recorded_hours(&[&earlier, &previous]),
        &saved,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        report.differences.len(),
        1,
        "a reused low identity word is not the same conference"
    );
}

#[test]
fn previously_published_events_keep_their_busy_span_and_failure_checks() {
    let saved = empty_settlement_save();
    let mut hours: Vec<_> = (100..105).map(frame).collect();
    for prior in &mut hours {
        *prior.tables[TableId::PeaceBids.index()] =
            TableView::Complete(TableRows::PeaceBids(vec![row(prior.hour)]));
    }
    for prior in &mut hours[1..3] {
        *prior.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
            reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
        };
    }
    *hours[3].tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    let priors: Vec<_> = hours.iter().collect();
    let mut report = Report::default();
    compare_settlements_after(&frame(105), recorded_hours(&priors), &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    let mut current = frame(105);
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    assert!(
        compare_settlements_after(
            &current,
            recorded_hours(&priors),
            &saved,
            &mut Report::default(),
        )
        .is_err()
    );
    drop(priors);
    *hours[1].tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![]));
    let priors: Vec<_> = hours.iter().collect();
    assert!(
        compare_settlements_after(
            &frame(105),
            recorded_hours(&priors),
            &saved,
            &mut Report::default(),
        )
        .is_err(),
        "a successful earlier drain closes that old publication's span"
    );
    drop(priors);
    let mut failed = row(103);
    failed.failure = Some(b"unread finalizer".to_vec());
    *hours[3].tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![failed]));
    let priors: Vec<_> = hours.iter().collect();
    assert!(
        compare_settlements_after(
            &frame(105),
            recorded_hours(&priors),
            &saved,
            &mut Report::default(),
        )
        .is_err(),
        "failed old manifests never certify cleanup"
    );
}

#[test]
fn busy_drain_allows_one_extra_hour_and_keeps_completeness_pending() {
    let saved = empty_settlement_save();
    let mut older = frame(100);
    *older.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    let mut previous = frame(101);
    *previous.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
        reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
    };
    let mut report = Report::default();
    report.recorded_tables.insert(
        TableId::PeaceSettlements,
        Err(replay_data::rows::PEACE_DRAIN_BUSY.into()),
    );
    compare_settlements_after(&previous, recorded_hours(&[&older]), &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report.outcome().is_ok(),
        "a validated deferral must not remain an availability failure"
    );
    report
        .recorded_tables
        .insert(TableId::States, Err("unreadable owners".into()));
    assert!(
        report.outcome().is_err(),
        "a busy peace drain cannot waive another missing table"
    );
    assert!(report.limitations.keys().any(|r| r.contains("busy lease")));
    let mut current = frame(102);
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    let mut report = Report::default();
    compare_settlements_after(
        &current,
        recorded_hours(&[&previous, &older]),
        &saved,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|r| r.contains("preceding drain"))
    );
    assert!(compare_settlements(&current, &saved, &mut Report::default()).is_err());
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![]));
    let mut report = Report::default();
    compare_settlements_after(
        &current,
        recorded_hours(&[&previous, &older]),
        &saved,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        report.differences.len(),
        1,
        "the deferred conference still needs its row"
    );
}

#[test]
fn busy_drain_span_matches_long_delays_and_keeps_every_conference_in_the_census() {
    let mut saved = empty_settlement_save();
    saved.states.insert(
        17,
        save::State {
            id: 17,
            owner: Some(b"MOD_B".to_vec()),
            controller: None,
            demilitarized: None,
            is_border_conflict: None,
            manpower_available: None,
            manpower_locked: None,
            population: None,
            category: None,
            name: None,
            strategic_locations: Vec::new(),
            contested_owners: Vec::new(),
        },
    );
    let mut hours: Vec<_> = (100..104).map(frame).collect();
    *hours[0].tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    for prior in &mut hours[1..] {
        *prior.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
            reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
        };
    }
    let mut second = row(102);
    second.conference = Some((4_u64 << 32) | 7);
    second.sequence = 2;
    *hours[2].tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![second.clone()]));
    let priors: Vec<_> = hours.iter().collect();
    let mut current = frame(104);
    *current.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
        reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
    };
    let mut report = Report::default();
    report.recorded_tables.insert(
        TableId::PeaceSettlements,
        Err(replay_data::rows::PEACE_DRAIN_BUSY.into()),
    );
    compare_settlements_after(&current, recorded_hours(&priors), &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(report.outcome().is_ok());
    assert_eq!(
        report.compared_tables["PeaceSettlements conferences deferred by busy drain"],
        2
    );
    let mut settled = row(100);
    settled.transfers.push(SettledPeaceTransfer {
        state: 17,
        taker: 1002,
        automatic: true,
    });
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![settled, second]));
    let mut report = Report::default();
    compare_settlements_after(&current, recorded_hours(&priors), &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["PeaceSettlements finalized conference census"],
        2
    );
    assert_eq!(
        report.compared_tables["PeaceSettlements events published after busy drain"],
        2
    );
    assert_eq!(
        report.compared_tables["PeaceSettlements retained state owners"],
        1
    );
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![]));
    let mut report = Report::default();
    compare_settlements_after(&current, recorded_hours(&priors), &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn busy_drain_span_stops_at_the_first_normal_hour() {
    let saved = empty_settlement_save();
    let mut older = frame(100);
    *older.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
        reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
    };
    *older.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    let normal = frame(101);
    let mut busy = frame(102);
    *busy.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
        reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
    };
    let priors = [&older, &normal, &busy];
    let mut visited = Vec::new();
    let mut current = frame(103);
    let mut report = Report::default();
    compare_settlements_after(
        &current,
        |hour| {
            visited.push(hour);
            recorded_hours(&priors)(hour)
        },
        &saved,
        &mut report,
    )
    .unwrap();
    assert_eq!(visited, [102, 101]);
    assert!(
        report.differences.is_empty(),
        "an older span is not part of this census"
    );
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(101)]));
    compare_settlements_after(
        &current,
        recorded_hours(&priors),
        &saved,
        &mut Report::default(),
    )
    .unwrap();
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    assert!(
        compare_settlements_after(
            &current,
            recorded_hours(&priors),
            &saved,
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn busy_drain_span_never_bridges_an_unrecorded_hour_or_another_branch() {
    let saved = empty_settlement_save();
    let mut busy = frame(102);
    *busy.tables[TableId::PeaceSettlements.index()] = TableView::Unavailable {
        reason: replay_data::rows::PEACE_DRAIN_BUSY.into(),
    };
    let mut current = frame(103);
    *current.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![row(100)]));
    assert!(
        compare_settlements_after(
            &current,
            recorded_hours(&[&busy]),
            &saved,
            &mut Report::default(),
        )
        .is_err()
    );
    busy.branch = 2;
    assert!(
        compare_settlements_after(
            &current,
            recorded_hours(&[&busy]),
            &saved,
            &mut Report::default(),
        )
        .is_err()
    );
}

fn settlement_replay(frames: Vec<DecodedFrame>) -> Replay {
    use replay_capture_spool::spool::Record;
    use replay_data::{
        identity::CaptureMode, identity::Identity, identity::Roster, identity::SampleBoundary,
        identity::SessionStart, identity::Verification,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        }),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(frames[0].hour - 1),
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for mut frame in frames {
        for table in TableId::ALL {
            if !matches!(
                table,
                TableId::Countries
                    | TableId::CountryLinks
                    | TableId::PeaceBids
                    | TableId::PeaceSettlements
            ) {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "outside the peace fixture".into(),
                };
            }
        }
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: frame.hour,
                ..Default::default()
            }]));
        *frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: b"campaign".to_vec(),
            }]));
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap()
}

#[test]
fn recorded_settlements_refuse_an_in_branch_gap_and_note_the_branch_start() {
    let saved = empty_settlement_save();
    let replay = settlement_replay(vec![frame(100), frame(101), frame(103)]);
    let error = compare_recorded_settlements(&replay, &frame(103), &saved, &mut Report::default())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("previous hourly PeaceBids sample unavailable"),
        "{error}"
    );
    let mut report = Report::default();
    compare_recorded_settlements(&replay, &frame(100), &saved, &mut report).unwrap();
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.starts_with("previous hourly PeaceBids sample unavailable"))
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn recorded_event_loss_stops_finalizer_completeness_crossing_it() {
    use replay_recording::control::ControlRecord;
    let saved = empty_settlement_save();
    let mut resident = frame(100);
    *resident.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    let check = |loss: Option<(u64, i32, i32)>| {
        let mut replay = settlement_replay(vec![frame(99), resident.clone(), frame(101)]);
        if let Some((branch, after_hour, hour)) = loss {
            replay.controls.push(ControlRecord::EventLoss {
                session: 1,
                branch,
                after_hour,
                hour,
            });
        }
        let mut report = Report::default();
        compare_recorded_settlements(&replay, &frame(101), &saved, &mut report).unwrap();
        report
    };
    let crossed = check(Some((1, 100, 101)));
    assert!(crossed.differences.is_empty(), "{:?}", crossed.differences);
    assert!(
        crossed
            .limitations
            .keys()
            .any(|reason| reason.starts_with("a recorded event loss"))
    );
    for loss in [None, Some((2, 100, 101)), Some((1, 99, 100))] {
        let strict = check(loss);
        assert_eq!(strict.differences.len(), 1, "{loss:?}");
        assert!(strict.differences[0].contains("finalized capture"));
    }
}

#[test]
fn bids_compare_native_operands_and_never_waive_a_changed_field() {
    let saved = projects::parse(&content_reader_core::parser::parse(br"countries={} peace_conference={active_peace={id={id=3 type=7} history={MOD_B={bidding={country=MOD_B actions={custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=17 demilitarized_zone=yes}}}}}}}").unwrap().items).unwrap();
    let mut frame = frame(100);
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![row(100)]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    let mut changed = row(100);
    changed.actions[0].negotiator = 1001;
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![changed]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("negotiator"));
    let mut failed = row(100);
    failed.failure = Some(b"guarded fault".to_vec());
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![failed]));
    assert!(compare_bids(&frame, &saved, &mut report).is_err());
}

#[test]
fn missing_flag_payload_is_not_four_saved_false_flags() {
    let saved = projects::parse(&content_reader_core::parser::parse(br"countries={} peace_conference={active_peace={id={id=3 type=7} history={MOD_B={bidding={country=MOD_B actions={custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=17}}}}}}}").unwrap().items).unwrap();
    let mut frame = frame(100);
    let mut bid = row(100);
    bid.actions[0].flags = None;
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("flags"));
}

#[test]
fn swapped_country_groups_are_a_difference_even_when_each_bid_matches() {
    let saved = projects::parse(&content_reader_core::parser::parse(br"countries={} peace_conference={active_peace={id={id=3 type=7} history={
        MOD_B={bidding={country=MOD_B actions={custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=17 demilitarized_zone=yes}}}}
        MOD_A={bidding={country=MOD_A actions={custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=17 demilitarized_zone=yes}}}}}}}").unwrap().items).unwrap();
    let mut bid = row(100);
    let mut second = bid.actions[0].clone();
    second.country = 1001;
    bid.actions.push(second);
    let mut frame = frame(100);
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid.clone()]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    bid.actions.swap(0, 1);
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("country_group_order"));
}

#[test]
fn repeated_mod_actions_keep_turn_order_country_operands_and_navy_values() {
    let saved = projects::parse(&content_reader_core::parser::parse(br"countries={} peace_conference={active_peace={id={id=3 type=7} history={MOD_B={
        bidding={country=MOD_B actions={
            custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=17 liberate=MOD_C demilitarized_zone=yes}
            custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 ship={id=9 type=11} ratio=-0.00003}}}
        bidding={country=MOD_B actions={custom_demand={taker=MOD_B giver=MOD_A negotiator=MOD_C cost=-2 status=3 state=18 puppet=MOD_A demilitarized_zone=yes}}}}}}}").unwrap().items).unwrap();
    let mut bid = row(100);
    bid.actions[0].country_operand = Some(1003);
    let mut navy = bid.actions[0].clone();
    navy.ordinal = 1;
    navy.state = None;
    navy.flags = None;
    navy.country_operand = None;
    navy.ship = Some((9_u64 << 32) | 11);
    navy.ratio = Some(-3);
    let mut puppet = bid.actions[0].clone();
    puppet.turn = 1;
    puppet.state = Some(18);
    puppet.country_operand = Some(1001);
    bid.actions.extend([navy, puppet]);
    let mut frame = frame(100);
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid.clone()]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    bid.actions[0].country_operand = Some(1001);
    bid.actions[1].ship = Some(0);
    bid.actions[1].ratio = Some(-4);
    bid.actions[2].turn = 0;
    *frame.tables[TableId::PeaceBids.index()] =
        TableView::Complete(TableRows::PeaceBids(vec![bid]));
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 4);
}

#[test]
fn bid_census_and_publication_failures_cannot_pass_as_empty_history() {
    let saved = projects::parse(
        &content_reader_core::parser::parse(
            br"countries={} peace_conference={active_peace={id={id=3 type=7}}}",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut frame = frame(100);
    let mut report = Report::default();
    compare_bids(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    for mut bid in [row(99), row(100)] {
        if bid.hour == Some(100) {
            bid.omitted = 1;
        }
        *frame.tables[TableId::PeaceBids.index()] =
            TableView::Complete(TableRows::PeaceBids(vec![bid]));
        assert!(compare_bids(&frame, &saved, &mut Report::default()).is_err());
    }
}

#[test]
fn retained_state_and_subject_outcomes_are_checked_and_destroyed_actions_are_named() {
    let mut saved = save::parse(br#"HOI4txt date="1936.1.1.1" game_unique_id="fixture" version="build" game_unique_seed=1
        debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6
        military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9
        country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
        countries={MOD_A={} MOD_B={} MOD_C={}} states={17={owner=MOD_B}} provinces={}"#).unwrap();
    let hour = saved.date.engine_hour().unwrap();
    let mut frame = frame(hour);
    let mut settled = row(hour);
    settled.transfers.push(SettledPeaceTransfer {
        state: 17,
        taker: 1002,
        automatic: true,
    });
    settled.subjects.push(SettledPeaceSubject {
        overlord: 1002,
        subject: 1003,
    });
    *frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![settled]));
    saved
        .intel
        .countries
        .entry(b"MOD_B".to_vec())
        .or_default()
        .diplomacy = Some(save::intel::Diplomacy {
        subjects: Some(vec![save::intel::Subject {
            roster_owner: b"MOD_B".to_vec(),
            target: b"MOD_C".to_vec(),
            first: b"MOD_B".to_vec(),
            second: b"MOD_C".to_vec(),
            state: None,
            autonomy_ratio: Decimal::parse(b"0").unwrap(),
            start_date: None,
            end_date: None,
            cancel: false,
        }]),
        ..Default::default()
    });
    let mut report = Report::default();
    compare_settlements(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(report.limitations.contains_key(DESTROYED_ACTIONS));
    let extra = saved.intel.countries[b"MOD_B".as_slice()]
        .diplomacy
        .as_ref()
        .unwrap()
        .subjects
        .as_ref()
        .unwrap()[0]
        .clone();
    let mut conflicting = extra;
    conflicting.first = b"MOD_A".to_vec();
    saved
        .intel
        .countries
        .get_mut(b"MOD_A".as_slice())
        .unwrap()
        .diplomacy = Some(save::intel::Diplomacy {
        subjects: Some(vec![conflicting]),
        ..Default::default()
    });
    let mut report = Report::default();
    compare_settlements(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("subject overlords"));
    saved
        .intel
        .countries
        .get_mut(b"MOD_A".as_slice())
        .unwrap()
        .diplomacy = None;
    let mut first = row(hour);
    first.transfers.push(SettledPeaceTransfer {
        state: 17,
        taker: 1001,
        automatic: false,
    });
    first.subjects.push(SettledPeaceSubject {
        overlord: 1001,
        subject: 1003,
    });
    let mut second = row(hour);
    second.sequence = 2;
    second.transfers.push(SettledPeaceTransfer {
        state: 17,
        taker: 1002,
        automatic: true,
    });
    second.subjects.push(SettledPeaceSubject {
        overlord: 1002,
        subject: 1003,
    });
    *frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![
            first.clone(),
            second.clone(),
        ]));
    let mut report = Report::default();
    compare_settlements(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    *frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![second, first]));
    assert!(compare_settlements(&frame, &saved, &mut Report::default()).is_err());
    let mut settled = row(hour);
    settled.transfers.push(SettledPeaceTransfer {
        state: 17,
        taker: 1002,
        automatic: true,
    });
    settled.subjects.push(SettledPeaceSubject {
        overlord: 1002,
        subject: 1003,
    });
    *frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![settled]));
    saved.states.get_mut(&17).unwrap().owner = Some(b"MOD_A".to_vec());
    saved
        .intel
        .countries
        .get_mut(b"MOD_B".as_slice())
        .unwrap()
        .diplomacy
        .as_mut()
        .unwrap()
        .subjects = Some(vec![]);
    let mut report = Report::default();
    compare_settlements(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(report.differences.iter().any(|d| d.contains("owner")));
    assert!(report.differences.iter().any(|d| d.contains("subject")));
    for mut invalid in [row(hour), row(hour - 2), row(hour + 1)] {
        if invalid.hour == Some(hour) {
            invalid.conference = None;
        }
        *frame.tables[TableId::PeaceSettlements.index()] =
            TableView::Complete(TableRows::PeaceSettlements(vec![invalid]));
        assert!(compare_settlements(&frame, &saved, &mut Report::default()).is_err());
    }
    let mut omitted = row(hour);
    omitted.omitted = 1;
    *frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![omitted]));
    assert!(compare_settlements(&frame, &saved, &mut Report::default()).is_err());
    saved.states.get_mut(&17).unwrap().owner = None;
    let mut null_recipient = row(hour);
    null_recipient.transfers.push(SettledPeaceTransfer {
        state: 17,
        taker: 0,
        automatic: true,
    });
    *frame.tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![null_recipient]));
    assert!(compare_settlements(&frame, &saved, &mut Report::default()).is_err());
}
