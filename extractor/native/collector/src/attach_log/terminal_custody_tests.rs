use super::*;
use compact_contract::domain_outcome::DomainOutcomeLedger;

const HOUR_INDEX: i64 = 1936 * 365 * 24;
const HOUR: i32 = compact_contract::calendar::DATE_EPOCH_CONST + HOUR_INDEX as i32;

#[test]
fn terminal_custody_d75_keeps_the_raw_label_separate_from_the_payload_index() {
    let hours = D75TerminalFrameHours::from_raw(HOUR).unwrap();
    assert_eq!(hours.raw, HOUR as u64);
    assert_eq!(hours.index, HOUR_INDEX as u64);
    assert_ne!(hours.raw, hours.index);
    for invalid in [0, -1, HOUR_INDEX as i32, i32::MAX] {
        assert!(D75TerminalFrameHours::from_raw(invalid).is_err());
    }
}

fn pending_frame(domains: &[u16]) -> Vec<u8> {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
    let mut records = Vec::new();
    for (index, domain) in domains.iter().enumerate() {
        let gap = DomainGapRecordV1 {
            part: DomainGapPart::Both, reason: DomainGapReason::HourLabelMismatch,
            target_hour_raw: HOUR, range_start_hour_raw: HOUR, range_end_hour_raw: HOUR,
            tick_before: (HOUR + 1) as u64, tick_after: (HOUR + 1) as u64,
            queue_capacity: 0, queue_depth: 0, queue_dropped: 0, outcome_ordinal: index as u64,
        }.encode().unwrap();
        append_compact_record(&mut records, index as u64 + 1, HOUR, *domain,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP, 0, &gap);
    }
    append_compact_frame_end_record(&mut records, domains.len() as u64 + 1, HOUR, 100,
        0, domains.len() as u32, domains.len() as u32, 0);
    records
}

fn folded(records: &[u8]) -> serde_json::Value {
    folded_with_outcomes(records, &DomainOutcomeLedger::new())
}

fn folded_with_outcomes(records: &[u8], outcomes: &DomainOutcomeLedger) -> serde_json::Value {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut records = records.to_vec();
    if let FrameCompletenessVerdict::Degraded(domains) = verdict(&records, outcomes) {
        let mut next = resequence_compact_records(&mut records, 1).unwrap();
        insert_frame_degraded_notice_before_frame_end(&mut records, &mut next, HOUR, domains).unwrap();
    }
    let scratch = std::env::temp_dir().join(format!("term-unit-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&scratch).unwrap();
    let input = scratch.join("stream.bin");
    std::fs::write(&input, &records).unwrap();
    let result = recorder_host::stream_fold::fold_compact_stream_file(
        &input, "terminal-custody-test", &scratch.join("snapshots"));
    let snapshot = result.map(|(hours, stats)| {
        assert!(stats.stop_reason.is_none(), "{:?}", stats.stop_reason);
        assert_eq!(stats.malformed_payload_records, 0, "{:?}", stats.malformed_payload_domains);
        assert_eq!(hours.len(), 1);
        serde_json::from_slice(&std::fs::read(&hours[0].path).unwrap()).unwrap()
    });
    std::fs::remove_dir_all(&scratch).unwrap();
    snapshot.unwrap()
}

fn verdict(records: &[u8], outcomes: &DomainOutcomeLedger) -> FrameCompletenessVerdict {
    classify_frame_completeness(&compact_frame_completeness_findings(records, HOUR, outcomes))
}

fn degraded_domains(verdict: &FrameCompletenessVerdict) -> Vec<u16> {
    match verdict {
        FrameCompletenessVerdict::Degraded(domains) => {
            domains.iter().map(|domain| domain.domain_id).collect()
        }
        other => panic!("expected a degraded frame, got {other:?}"),
    }
}

fn perf_status(records: &[u8], domain_id: u16) -> u32 {
    let mut offset = 0;
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF && header.domain_id == domain_id {
            let payload = &records[offset + compact_contract::COMPACT_RECORD_HEADER_LEN..];
            return u32::from_le_bytes(payload[32..36].try_into().unwrap());
        }
        offset += header.record_len();
    }
    panic!("no PERF record for domain {domain_id}");
}

fn domain_problem(snapshot: &serde_json::Value, domain_id: u16, problem: &str) -> bool {
    snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["domain_id"] == domain_id && entry["problem"] == problem)
}

#[test]
fn terminal_custody_missing_pairs_are_inserted_and_resequenced_atomically() {
    let mut records = pending_frame(&[55, 67, 86, 90, 75, 64]);
    for domain in [55, 67, 86, 90, 75, 64] {
        assert!(terminal_domain_data_payload(&records, HOUR, domain).unwrap().is_none());
        replace_terminal_hook_domain_records(
            &mut records, HOUR, 100, domain, &[1, 2, 3], 1, 0, "terminal_custody",
        ).unwrap();
        assert_eq!(terminal_domain_data_payload(&records, HOUR, domain).unwrap(), Some(vec![1, 2, 3]));
    }
    let mut offset = 0;
    let mut sequence = 1;
    let mut perf = BTreeSet::new();
    let mut data = BTreeSet::new();
    let mut frame_end_sequence = 0;
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        assert_eq!(header.sequence, sequence);
        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF {
            assert!(perf.insert(header.domain_id));
            assert_eq!(perf_status(&records, header.domain_id), compact_contract::COMPACT_STATUS_TERMINAL_CUSTODY);
        }
        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA { assert!(data.insert(header.domain_id)); }
        if header.record_type == COMPACT_RECORD_TYPE_FRAME_END { frame_end_sequence = header.sequence; }
        offset += header.record_len();
        sequence += 1;
    }
    assert_eq!(perf, data);
    assert_eq!(data.len(), 6);
    assert_eq!(frame_end_sequence, 13);
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));

    // The cursor the frame was staged with is stale once custody records are inserted;
    // the re-derived one is what every later before-FRAME_END insertion checks against.
    let domain = compact_contract::frame_degraded::FrameDegradedDomain::from_status(
        90, compact_contract::COMPACT_STATUS_PARTIAL_DATA,
        compact_contract::frame_degraded::FrameDegradedDiagnosis::new(
            compact_contract::diagnostic_channel::Predicate::undeclared(),
            compact_contract::diagnostic_channel::Authority::SelfDomain, "test",
        ),
    ).unwrap();
    let mut stale_next_sequence = 2u64;
    let mut stale = records.clone();
    assert!(insert_frame_degraded_notice_before_frame_end(&mut stale, &mut stale_next_sequence, HOUR, vec![domain.clone()]).is_err());
    let first = compact_contract::parse_compact_record_header(&records).unwrap().sequence;
    let mut next_sequence = resequence_compact_records(&mut records, first).unwrap();
    assert_eq!(next_sequence, 14);
    insert_frame_degraded_notice_before_frame_end(&mut records, &mut next_sequence, HOUR, vec![domain]).unwrap();
    assert_eq!(next_sequence, 15);

    let before = records.clone();
    assert!(replace_terminal_hook_domain_records(&mut records, HOUR + 1, 100, 55, &[], 0, 0, "terminal_custody").is_err());
    assert_eq!(records, before);
}

/// One sealed row per D55 terminal carrier: width, current loss, and two panel
/// observations of one combat side, in capture order.
pub(super) fn d55_sealed_rows(game_hour_raw: i32) -> (
    land_detail55::WidthObservationRow,
    land_detail55::CurrentLossObservationRow,
    Vec<land_detail55::PanelObservationRow>,
) {
    use land_detail55::{CombatSide, CurrentLossObservationRow, PanelObservationRow, WidthObservationRow};
    let width = WidthObservationRow {
        observed_game_hour_raw: Some(game_hour_raw),
        combat_id: 17, generation: 3, side: CombatSide::Attacker, capture_sequence: 1,
        ordinal: 0, wrapper_count: 1, division_id: 42, control_flags: land_detail55::WIDTH_REQUIRED_FLAG_MASK,
        wrapper_width_raw: 12_000, final_width_raw: 12_000,
    };
    let loss = CurrentLossObservationRow {
        observed_game_hour_raw: Some(game_hour_raw),
        combat_id: 17, generation: 3, side: CombatSide::Attacker, capture_sequence: 2,
        ordinal: 0, equipment_count: 1, equipment_id: 7,
        control_flags: land_detail55::CURRENT_LOSS_REQUIRED_FLAG_MASK | land_detail55::CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE,
        manpower_before_raw: 100, manpower_after_raw: 90, equipment_before_raw: 50, equipment_after_raw: 40,
    };
    let panel = PanelObservationRow {
        observed_game_hour_raw: Some(game_hour_raw - 1),
        combat_id: 17, generation: 3, side: CombatSide::Attacker, capture_sequence: 3,
        leader_id: 19, modifier_bits: 0, control_flags: land_detail55::PANEL_REQUIRED_FLAG_MASK,
    };
    let later_panel = PanelObservationRow {
        observed_game_hour_raw: Some(game_hour_raw), capture_sequence: 4, leader_id: 20, ..panel
    };
    (width, loss, vec![panel, later_panel])
}

#[test]
fn terminal_custody_d55_sealed_rows_fold_without_a_scheduled_side_set() {
    let (width, loss, panels) = d55_sealed_rows(HOUR);
    let (panel, later_panel) = (panels[0], panels[1]);
    let mut records = pending_frame(&[55]);
    merge_terminal_d55_records(
        &mut records, HOUR, 100, Some(&land_detail55::a729_hook::TerminalDrain::default()),
        Some(land_detail55::x2_carriers::WidthTerminalDrain { rows: vec![width], ..Default::default() }),
        Some(land_detail55::x2_carriers::LossTerminalDrain { rows: vec![loss], ..Default::default() }),
        Some(land_detail55::x2_carriers::PanelDrain { observations: vec![panel, later_panel], ..Default::default() }),
    ).unwrap();
    let payload = terminal_domain_data_payload(&records, HOUR, 55).unwrap().unwrap();
    let decoded = land_detail55::decode_detail_events_payload(0, &payload).unwrap();
    assert!(decoded.side_detail_rows.is_empty());
    assert_eq!(decoded.width_observation_rows, vec![width]);
    assert_eq!(decoded.current_loss_observation_rows, vec![loss]);
    assert_eq!(decoded.panel_observation_rows, vec![panel, later_panel]);
    assert!(compact_contract::compact_status_is_terminal_custody(perf_status(&records, 55)));
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    let custody = &snapshot["_r2_land_combat_terminal_custody"];
    assert_eq!(custody["base_status"], "terminal_custody_base_unobserved");
    assert_eq!(custody["width_observation_rows"][0]["division_id"], 42);
    assert_eq!(custody["current_loss_observation_rows"][0]["equipment_after_raw"], 40);
    assert_eq!(custody["panel_observation_rows"].as_array().unwrap().len(), 2);
    assert_eq!(custody["panel_observation_rows"][0]["observed_game_hour_raw"], HOUR - 1);
    assert_eq!(custody["panel_observation_rows"][1]["observed_game_hour_raw"], HOUR);
    assert!(domain_problem(&snapshot, 55, "terminal_custody_base_unobserved"), "{snapshot}");
}

#[test]
fn terminal_custody_d55_without_its_terminal_writer_still_publishes_the_other_carriers() {
    use land_detail55::{CombatSide, WidthObservationRow};
    let width = WidthObservationRow {
        observed_game_hour_raw: Some(HOUR),
        combat_id: 17, generation: 3, side: CombatSide::Defender, capture_sequence: 1,
        ordinal: 0, wrapper_count: 1, division_id: 43, control_flags: land_detail55::WIDTH_REQUIRED_FLAG_MASK,
        wrapper_width_raw: 12_000, final_width_raw: 12_000,
    };
    let mut records = pending_frame(&[55]);
    merge_terminal_d55_records(
        &mut records, HOUR, 100, None,
        Some(land_detail55::x2_carriers::WidthTerminalDrain { rows: vec![width], ..Default::default() }),
        None, None,
    ).unwrap();
    let payload = terminal_domain_data_payload(&records, HOUR, 55).unwrap().unwrap();
    let decoded = land_detail55::decode_detail_events_payload(0, &payload).unwrap();
    assert_eq!(decoded.width_observation_rows, vec![width]);
    assert!(decoded.terminal_record_rows.is_empty());
    assert_ne!(decoded.status_flags & compact_contract::DOMAIN55_STATUS_EVENT_CARRIER_INCOMPLETE, 0);
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    assert_eq!(snapshot["_r2_land_combat_terminal_custody"]["width_observation_rows"][0]["division_id"], 43);
}

#[test]
fn terminal_custody_d55_replaces_pending_panel_observations() {
    let (_, _, panels) = d55_sealed_rows(HOUR);
    let sides = [land_detail55::SideDetailRow {
        combat_id: panels[0].combat_id,
        generation: panels[0].generation,
        side: panels[0].side,
        detail: super::tests::golden_side_detail(1, 0),
        reserve_members_known: false,
    }];
    for custody in [false, true] {
        let status = land_detail55::STATUS_DELTA_DATA
            | if custody { compact_contract::COMPACT_STATUS_TERMINAL_CUSTODY } else { 0 };
        let payload = land_detail55::encode_detail_events_payload_v10(
            90, status, &[], if custody { &[] } else { &sides },
            &[], &[], &[], &[], &[], &panels[..1], &Default::default(),
            if custody { Default::default() } else { land_detail55::hour_custody::FrameHourCustody::witnessed(HOUR) },
        );
        land_detail55::decode_detail_events_payload(0, &payload).unwrap();
        let mut records = Vec::new();
        append_compact_record(&mut records, 1, HOUR, 55, COMPACT_RECORD_TYPE_DOMAIN_PERF, 0, &[0; 40]);
        append_compact_record(&mut records, 2, HOUR, 55, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &payload);
        append_compact_frame_end_record(&mut records, 3, HOUR, 90, 0, 1, 1, 0);
        merge_terminal_d55_records(
            &mut records, HOUR, 100, Some(&land_detail55::a729_hook::TerminalDrain::default()),
            Some(Default::default()), Some(Default::default()),
            Some(land_detail55::x2_carriers::PanelDrain { observations: panels.clone(), ..Default::default() }),
        ).unwrap();
        let merged = terminal_domain_data_payload(&records, HOUR, 55).unwrap().unwrap();
        let decoded = land_detail55::decode_detail_events_payload(0, &merged).unwrap();
        assert_eq!(decoded.panel_observation_rows, panels);
    }
}

pub(super) fn zero_removal(
    owner: u32,
    observed_game_hour_raw: i32,
) -> compact_contract::dynamic_modifiers::DynamicModifierZeroRemoval {
    compact_contract::dynamic_modifiers::DynamicModifierZeroRemoval {
        owner_tag_db_id: owner, modifier_id: "terminal_modifier".into(), enabled: true,
        values_raw: vec![-100_000, 9], observed_game_hour_raw,
    }
}

#[test]
fn terminal_custody_d67_zero_removals_fold_without_a_current_roster_or_base() {
    let rows = vec![zero_removal(7, HOUR)];
    let mut records = pending_frame(&[67]);
    let mut outcomes = DomainOutcomeLedger::new();
    merge_terminal_d67_records(&mut records, HOUR, 100, &rows, 0, &mut outcomes).unwrap();
    let payload = terminal_domain_data_payload(&records, HOUR, 67).unwrap().unwrap();
    let decoded = compact_contract::dynamic_modifiers::decode_payload(&payload).unwrap();
    assert!(decoded.instances.is_empty());
    assert!(decoded.partiality.is_empty());
    assert_eq!(decoded.zero_removals, rows);
    assert!(outcomes.get(67).is_none());
    assert_eq!(perf_status(&records, 67), compact_contract::COMPACT_STATUS_TERMINAL_CUSTODY);
    assert!(matches!(verdict(&records, &outcomes), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    let custody = &snapshot["_r2_dynamic_modifier_lifecycle_live"];
    assert_eq!(custody["base_status"], "terminal_custody_base_unobserved", "{snapshot}");
    assert_eq!(custody["zero_removals"][0]["owner_tag_db_id"], 7);
    assert_eq!(custody["zero_removals"][0]["values_raw"], serde_json::json!([-100_000, 9]));
    assert!(domain_problem(&snapshot, 67, "terminal_custody_base_unobserved"));
}

#[test]
fn terminal_custody_d67_names_an_unclosed_carrier_as_a_frame_wide_gap() {
    let rows = vec![zero_removal(7, HOUR)];
    let causes = domain67_cause::bit(domain67_cause::ZERO_RESERVATION_ABSENT_OR_POISONED)
        | domain67_cause::bit(domain67_cause::ZERO_EVENT_NOT_A_ZERO_REMOVAL);
    let mut records = pending_frame(&[67]);
    let mut outcomes = DomainOutcomeLedger::new();
    merge_terminal_d67_records(&mut records, HOUR, 100, &rows, causes, &mut outcomes).unwrap();
    let payload = terminal_domain_data_payload(&records, HOUR, 67).unwrap().unwrap();
    let decoded = compact_contract::dynamic_modifiers::decode_payload(&payload).unwrap();
    assert_eq!(decoded.zero_removals, rows);
    assert_eq!(decoded.partiality, vec![compact_contract::dynamic_modifiers::CountryPartiality {
        owner_tag_db_id: 0, causes, first_tick: HOUR_INDEX as u64, last_tick: HOUR_INDEX as u64,
    }]);
    let status = perf_status(&records, 67);
    assert_eq!(status, decoded.status_flags);
    assert!(compact_contract::compact_status_is_terminal_custody(status));
    assert_ne!(status & compact_contract::COMPACT_STATUS_PARTIAL_DATA, 0);
    assert_eq!(status & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE, 0);
    let outcome = outcomes.get(67).unwrap();
    assert!(outcome.is_failure());
    outcome.audit_published_status(status).unwrap();
    assert_eq!(degraded_domains(&verdict(&records, &outcomes)), vec![67]);
    let snapshot = folded_with_outcomes(&records, &outcomes);
    let custody = &snapshot["_r2_dynamic_modifier_lifecycle_live"];
    assert_eq!(custody["partiality"][0]["causes"], causes, "{snapshot}");
    assert_eq!(custody["zero_removals"][0]["owner_tag_db_id"], 7);
}

/// One `FocusCompleted` direct transition captured through the lane's own
/// carrier, so its instance ids and sequence are the carrier's.
pub(super) fn d45_focus_completed_rows(
    game_hour_raw: i32,
) -> Vec<direct_frd_transitions_live::DirectEventRow> {
    use direct_frd_transitions_live::{DirectEventCarrier, DirectEventKind, DirectEventStateSummary, UnsequencedDirectEventRow};
    let mut carrier = DirectEventCarrier::new();
    let owner = carrier.allocate_instance_id().unwrap();
    let subject = carrier.allocate_instance_id().unwrap();
    carrier.capture(UnsequencedDirectEventRow {
        kind: DirectEventKind::FocusCompleted, thread_id: 23, game_hour_raw,
        root_rva: DirectEventKind::FocusCompleted.expected_root_rva_a729(),
        owner_instance_id: owner, subject_instance_id: subject,
        argument_instance_id: direct_frd_transitions_live::OpaqueInstanceId::NONE,
        owner_key: 1, subject_token_hash: 1, subject_key: 0, input_bits: 0, return_bits: 1,
        before: DirectEventStateSummary::default(),
        after: DirectEventStateSummary { state_bits: 1 << 63, ..Default::default() },
    }).unwrap();
    carrier.drain_rows().unwrap()
}

#[test]
fn terminal_custody_d45_builds_its_model_from_the_sealed_hook_batch() {
    use direct_frd_transitions_live::{DirectEventBatch, DirectEventKind};
    let rows = d45_focus_completed_rows(HOUR);
    let batch = DirectEventBatch {
        hook_epoch: 1, drain_sequence: 1,
        installed_root_mask: direct_frd_transitions_live::D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK,
        observed_kind_mask: DirectEventKind::FocusCompleted.root_mask_bit(), sequence_cursor: 2,
        producer_complete: true, evidence_fixture: false, rows: rows.clone(),
    };
    let mut records = pending_frame(&[45]);
    let mut model = None;
    merge_terminal_d45_records(&mut records, HOUR, 100, &mut model,
        direct_frd_transitions_live::a729_hook::DirectEventBatchReservation {
            reservation_id: 1, batch, overflow_dropped_rows: 0,
        }).unwrap();
    let read = model.unwrap();
    assert_eq!(read.direct_transitions, rows);
    assert!(read.focus_events.is_empty());
    assert_eq!(read.status_flags, compact_contract::COMPACT_STATUS_TERMINAL_CUSTODY);
    assert_eq!(read.depth_status_flags, COMPACT_FRD_STATUS_TERMINAL_CUSTODY);
    assert_eq!(perf_status(&records, 45), compact_contract::COMPACT_STATUS_TERMINAL_CUSTODY);
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    assert_eq!(snapshot["_r2_direct_frd_transitions"]["transitions"].as_array().unwrap_or_else(|| panic!("{snapshot}")).len(), 1);
    assert_eq!(snapshot["_r2_direct_frd_terminal_custody"]["transitions"].as_array().unwrap().len(), 1);
    assert!(domain_problem(&snapshot, 45, "terminal_custody_base_unobserved"), "{snapshot}");
}

use sunk_ship_history_live_lane::*;
fn sunk_frame() -> SunkShipFrame {
    SunkShipFrame {
        elapsed_wall_ms: 7,
        frame_hour: HOUR_INDEX as u64,
        status_flags: 0,
        authority: AuthorityState::StaticExact20Receipt,
        snapshot: Some(SunkShipSnapshot {
            source_sequence: 2,
            producer_complete: true,
            records_with_unavailable_text: 0,
            producer_hour_raw: Some(HOUR),
            producer_loss: None,
            events: vec![SunkShipEvent {
                event_sequence: 2,
                combat_join_present: true,
                combat_id: 4,
                combat_generation: 2,
                victim_country_id_raw: 3,
                attacker_country_id_raw: 9,
                ui_source_country_id_raw: 5,
                sink_date_raw: HOUR - 60,
                victim_member_battle_slot_index: 7,
                flags: EVENT_FLAG_KILLED,
                victim_ship_reference_raw: [11, 12],
                damage_source_reference_raw: [13, 14],
                task_force_id_raw: 15,
                terminal_context_raw: [16, 17],
                cause_record_flag: false,
                ship_name: "HMS Exact".into(),
                ship_class_name: "Destroyer".into(),
                attribution_source_name: "KMS Exact".into(),
                attribution_source_class_name: "Cruiser".into(),
            }],
        }),
    }
}

#[test]
fn terminal_custody_d86_snapshot_folds_without_a_pending_pair() {
    let frame = sunk_frame();
    let payload = sunk_ship_history_live_lane::encode_payload(&frame).unwrap();
    let mut records = pending_frame(&[86]);
    replace_terminal_hook_domain_records(&mut records, HOUR, 100, 86, &payload, 1,
        frame.status_flags, "d86_terminal_custody").unwrap();
    assert_eq!(decode_payload(&terminal_domain_data_payload(&records, HOUR, 86).unwrap().unwrap()).unwrap(), frame);
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    assert_eq!(snapshot["_r2_sunk_ship_history_live"]["events"][0]["ship_name"], "HMS Exact", "{snapshot}");
}

use naval4950::{ConvoyD70MutationKind, ConvoyD70MutationProof, ConvoyTransportAttackRow,
    ConvoyTransportAttackWitness, ConvoyTerminalAttributionCarrier, ConvoyTerminalCombat,
    CombatEpoch, ConvoyTerminalEntityHandle, TERMINAL_SUNK_STATE, ConvoyTerminalRoute,
    LiveRouteResolution, RouteProducerKind, RouteEndpoint, RouteEndpointKind,
    encode_convoy_transport_attack_batch, decode_convoy_transport_attack_batch};
fn convoy_row(sequence: u64, kind: ConvoyD70MutationKind) -> ConvoyTransportAttackRow {
    ConvoyTransportAttackRow {
        carrier: ConvoyTerminalAttributionCarrier {
            producer_sequence: sequence,
            game_hour_raw: HOUR,
            combat: ConvoyTerminalCombat::Bound(CombatEpoch {
                combat_id: 21,
                generation: 4,
            }),
            convoy: ConvoyTerminalEntityHandle {
                id: 158_065,
                type_tag: 4_713,
            },
            owner_tag_db_id: 2,
            killer_tag_db_id: 51,
            client: ConvoyTerminalEntityHandle {
                id: 106_381,
                type_tag: 4_713,
            },
            convoy_index: 8,
            member_state_at_entry: TERMINAL_SUNK_STATE - 1,
            battle_slot: 8,
            route: ConvoyTerminalRoute::Bound(LiveRouteResolution {
                route_key: ConvoyTerminalEntityHandle {
                    id: 106_381,
                    type_tag: 4_713,
                },
                producer_kind: RouteProducerKind::LendLease,
                source: RouteEndpoint {
                    kind: RouteEndpointKind::State,
                    id: 282,
                },
                destination: RouteEndpoint {
                    kind: RouteEndpointKind::State,
                    id: 64,
                },
            }),
        },
        d70: match kind {
            ConvoyD70MutationKind::IncrementExisting => ConvoyD70MutationProof {
                kind,
                ordinal: 440,
                before_rows: 441,
                after_rows: 441,
                before_count: 2,
                after_count: 3,
            },
            ConvoyD70MutationKind::AppendFirstSeen => ConvoyD70MutationProof {
                kind,
                ordinal: 441,
                before_rows: 441,
                after_rows: 442,
                before_count: 0,
                after_count: 1,
            },
        },
    }
}

fn convoy_witness(rows: &[ConvoyTransportAttackRow]) -> ConvoyTransportAttackWitness {
    let emitted = rows
        .last()
        .map(|row| row.carrier.producer_sequence)
        .unwrap_or(0);
    ConvoyTransportAttackWitness {
        hook_installed: true,
        queue_drain_complete: true,
        producer_sequence_next: emitted + 1,
        drained_rows: rows.len() as u32,
        events_seen_this_drain: u64::from(rows.len() as u32),
        events_emitted_this_drain: u64::from(rows.len() as u32),
        resolver_publication_sequence: 3,
        resolver_game_hour_raw: HOUR,
        resolver_member_bindings: 4,
        resolver_route_bindings: 2,
        ..ConvoyTransportAttackWitness::default()
    }
}

#[test]
fn terminal_custody_d90_drain_folds_without_a_pending_pair() {
    let rows = vec![convoy_row(1, ConvoyD70MutationKind::AppendFirstSeen)];
    let payload = encode_convoy_transport_attack_batch(&rows, convoy_witness(&rows)).unwrap();
    let mut records = pending_frame(&[90]);
    replace_terminal_hook_domain_records(&mut records, HOUR, 100, 90, &payload, 1,
        0, "d90_terminal_custody").unwrap();
    let decoded = decode_convoy_transport_attack_batch(&terminal_domain_data_payload(&records, HOUR, 90).unwrap().unwrap()).unwrap();
    assert_eq!(decoded.rows, rows);
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    assert!(snapshot.to_string().contains("158065"), "{snapshot}");
}

#[test]
fn terminal_custody_d90_inexact_drain_publishes_its_rows_under_a_derived_partial_word() {
    let rows = vec![convoy_row(1, ConvoyD70MutationKind::AppendFirstSeen)];
    let witness = ConvoyTransportAttackWitness { queue_drain_complete: false, ..convoy_witness(&rows) };
    let payload = encode_convoy_transport_attack_batch(&rows, witness).unwrap();
    let decoded = decode_convoy_transport_attack_batch(&payload).unwrap();
    assert!(!decoded.exact_events_complete);
    let (outcome, status) = terminal_d90_inexact_outcome(&decoded);
    assert_eq!(status, compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE | compact_contract::COMPACT_STATUS_PARTIAL_DATA);
    let mut records = pending_frame(&[90]);
    replace_terminal_hook_domain_records(&mut records, HOUR, 100, 90, &payload, 1, status, "d90_terminal_custody").unwrap();
    let mut outcomes = DomainOutcomeLedger::new();
    outcomes.record_superseding(outcome);
    let published = perf_status(&records, 90);
    assert_eq!(published, status | compact_contract::COMPACT_STATUS_TERMINAL_CUSTODY);
    outcomes.get(90).unwrap().audit_published_status(published).unwrap();
    assert_eq!(decode_convoy_transport_attack_batch(&terminal_domain_data_payload(&records, HOUR, 90).unwrap().unwrap()).unwrap().rows, rows);
    assert_eq!(degraded_domains(&verdict(&records, &outcomes)), vec![90]);
}

/// One DEL2 lifecycle row as the terminal D64 drain encodes it.
pub(super) struct LifecycleTrailerRow {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub reason: u32,
    pub owner_tag_db_id: u32,
    pub event_id: u32,
    pub originator_tag_db_id: u32,
    pub scope_random: u64,
    pub due_before: u32,
    pub due_after: Option<u32>,
}

/// A retained-decrement lifecycle row (reason 1, due 9 -> 8) at `sequence`.
pub(super) fn retained_decrement_lifecycle_row(sequence: u64, game_hour_raw: i32) -> LifecycleTrailerRow {
    LifecycleTrailerRow {
        sequence, game_hour_raw, reason: 1, owner_tag_db_id: 1, event_id: 100_007,
        originator_tag_db_id: 1, scope_random: 5, due_before: 9, due_after: Some(8),
    }
}

/// The DEL2 trailer the terminal D64 drain produces for a complete drain of
/// `rows` under `hook_epoch` (`a729_delayed_event_lifecycle::encode_terminal_del2`).
pub(super) fn del2_trailer(hook_epoch: u64, rows: &[LifecycleTrailerRow]) -> Vec<u8> {
    let mut del2 = Vec::new();
    del2.extend_from_slice(b"DEL2");
    for value in [2u16, 48, 48, 0] { del2.extend_from_slice(&value.to_le_bytes()); }
    del2.extend_from_slice(&1u32.to_le_bytes());
    del2.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    del2.extend_from_slice(&hook_epoch.to_le_bytes());
    del2.extend_from_slice(&rows.first().map_or(0, |row| row.sequence).to_le_bytes());
    del2.extend_from_slice(&rows.last().map_or(0, |row| row.sequence).to_le_bytes());
    del2.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        del2.extend_from_slice(&row.sequence.to_le_bytes());
        del2.extend_from_slice(&row.game_hour_raw.to_le_bytes());
        for value in [row.reason, row.owner_tag_db_id, row.event_id, row.originator_tag_db_id,
            u32::from(row.due_after.is_some())] {
            del2.extend_from_slice(&value.to_le_bytes());
        }
        del2.extend_from_slice(&row.scope_random.to_le_bytes());
        del2.extend_from_slice(&row.due_before.to_le_bytes());
        del2.extend_from_slice(&row.due_after.unwrap_or(0).to_le_bytes());
    }
    del2
}

#[test]
fn terminal_custody_d75_d64_rows_fold_without_pending_pairs() {
    let telemetry = fired_event_popups_live_lane::HookTelemetry {
        fire_hook_validated: true,
        option_hook_validated: true,
        materialisation_hook_validated: true,
        timeout_hook_validated: true,
        all_event_classes_covered: true,
        player_ai_timeout_no_choice_covered: true,
        capture_start_baseline_complete: true,
        queue_capacity: 1024,
        queue_high_water_mark: 3,
        ..Default::default()
    };
    let frame = fired_event_popups_live_lane::FiredEventFrame {
        elapsed_wall_ms: 7,
        frame_hour: HOUR_INDEX as u64,
        producer_manifest_sha256: fired_event_popups_live_lane::PRODUCER_MANIFEST_SHA256,
        telemetry,
        fired_events_complete: true,
        capture_start_fired_event_ids: Some(Vec::new()),
        capture_start_pending_unresolved: Vec::new(),
        rows: vec![fired_event_popups_live_lane::FiredEventRow {
            event_id: "germany.1".into(),
            scope_country_tag_db_id: 1,
            recipient: Some(fired_event_popups_live_lane::FiredEventRecipient { tag_db_id: 1, tag: "GER".into() }),
            fire_date: "1936.1.1.12".into(),
            fire_hour: HOUR_INDEX as u64,
            fire_tick: 43_800_000,
            event_sequence: 1,
            visible_option_ordinals: vec![0, 2],
            chosen_option_ordinal: Some(2),
            selected_desc_key: Some("germany.1.d".into()),
            shown_picture_gfx: Some("GFX_report_event_german_reich".into()),
            popup_subtype: fired_event_popups_live_lane::PopupSubtype::Country,
            effective_timeout_hours: 312,
            timeout_tick: 43_800_312,
            resolved_tick: Some(43_800_001),
            resolution_reason: fired_event_popups_live_lane::ResolutionReason::Selected,
            runtime_title_text_context_id: Some("EventWindow::Title::4".into()),
            runtime_description_text_context_id: Some("EventWindow::Description::4".into()),
            runtime_option_text_contexts: vec![fired_event_popups_live_lane::OptionRuntimeTextContext {
                definition_ordinal: 2,
                context_id: "EventWindow::Name::9".into(),
            }],
        }],
    };
    let payload = fired_event_popups_live_lane::encode_payload(&frame).unwrap();
    let del2 = del2_trailer(1, &[retained_decrement_lifecycle_row(1, HOUR)]);
    let mut records = pending_frame(&[75, 64]);
    merge_terminal_d75_d64_payloads(&mut records, HOUR, &frame, &payload, 1, 1,
        Sha256::digest(&payload).into(), [64; 32], &del2, Sha256::digest(&del2).into()).unwrap();
    assert_eq!(terminal_domain_data_payload(&records, HOUR, 75).unwrap().unwrap(), payload);
    let d64 = terminal_domain_data_payload(&records, HOUR, 64).unwrap().unwrap();
    let mut state = wire_format::d64::State::new();
    let port = wire_format::d64::fold_payload(&mut state, &d64).unwrap();
    assert_eq!(port.table, wire_format::d64::TablePart::StoppedUnavailable);
    assert_eq!(state.last_sequence(), Some(1));
    assert!(!state.history_failed());
    assert!(compact_contract::compact_status_is_terminal_custody(perf_status(&records, 75)));
    assert!(compact_contract::compact_status_is_terminal_custody(perf_status(&records, 64)));
    assert!(matches!(verdict(&records, &DomainOutcomeLedger::new()), FrameCompletenessVerdict::Complete));
    let snapshot = folded(&records);
    assert_eq!(snapshot["world_events"]["fired_events"][0]["event_id"], "germany.1", "{snapshot}");
    assert!(!snapshot["_r2_delayed_event_lifecycle_live"].is_null(), "{snapshot}");
    assert!(domain_problem(&snapshot, 64, "terminal_custody_base_unobserved"), "{snapshot}");
}
