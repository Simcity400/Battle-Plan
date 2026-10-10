use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{
        LexicalTokenRow, Recorded, ReservedCountryNamingRow, TableId, TableRows,
        TransportMissionInputs,
    },
};

const HANDLE: u64 = (2_u64 << 32) | 61;

fn document(fields: &str) -> SaveDocument {
    let text = [
        "HOI4txt date=1936.1.1.1 game_unique_id=c version=b countries={X={units={fleet={id={id=1 type=61} task_force={id={id=2 type=61} mission={",
        fields,
        "} } } } } }",
    ].concat();
    SaveDocument::read(text.as_bytes()).unwrap()
}

fn receiver(cache: Option<[i64; 2]>) -> TransportMissionReceiver {
    TransportMissionReceiver {
        task_force: HANDLE,
        mission_owner: Some(7),
        logical_owner: Some(7),
        in_regions: Some(1),
        combat_references: Some(-1),
        combined_detection: Some(123),
        detection_cache: cache,
        received_supply_ratio: Some(-50_000),
        risk: Some(replay::rows::TransportReceiverRisk {
            coordination: Some(0),
            spotting_chance: Some(0),
            night_spotting_chance: Some(0),
            visibility: Some(0),
            speed_base: Some(0),
            speed_modifier: Some(0),
            terrain_class: Some(0),
            stats: Some(replay::rows::TransportTaskForceStats {
                detection_138: Some(0),
                visibility_148: Some(0),
                visibility_150: Some(0),
                carrier_detection_160: Some(0),
            }),
            roster_owner: Some(7),
            location: Some(1),
            region: Some(1),
            location_controller: Some(7),
            detached_activity: Some(0),
            ship_count: Some(0),
            targeted_spotting_modifier: Some(0),
            actual_terrain_speed: Some(0),
        }),
    }
}

fn frame(packet: Recorded<Option<TransportMissionInputs>>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        hour: 23,
        branch: 1,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "unused".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                transport_mission_inputs: packet,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: CACHE_TOKENS[0],
                literal: b"fixture_r".to_vec(),
            },
            LexicalTokenRow {
                token: CACHE_TOKENS[1],
                literal: b"fixture_a".to_vec(),
            },
        ]))
        .into();
    frame
}

fn packet(receiver: TransportMissionReceiver) -> Recorded<Option<TransportMissionInputs>> {
    Recorded::Value(Some(TransportMissionInputs {
        sample_hour: 23,
        navies: Some(Vec::new()),
        receivers: Some(vec![receiver]),
        naval_search: None,
        spotting_context: Some(replay::rows::TransportSpottingContext {
            sample_hour: 23,
            owners_complete: true,
            regions_complete: true,
            ..Default::default()
        }),
        air: Some(replay::rows::TransportAirInputs {
            sample_hour: 23,
            countries: Some(vec![]),
        }),
        risk_ships: Some(replay::rows::TransportRiskShips {
            sample_hour: 23,
            receivers: Some(vec![]),
        }),
    }))
}

#[test]
fn runtime_domain_admission_faults_survive_without_cache_or_save_key_demand() {
    let Recorded::Value(Some(mut packet)) = packet(receiver(None)) else {
        unreachable!()
    };
    packet.air.as_mut().unwrap().sample_hour -= 1;
    packet.spotting_context.as_mut().unwrap().regions_complete = false;
    packet.risk_ships.as_mut().unwrap().receivers = None;
    let mut report = Report::default();
    compare_runtime_domains(&packet, 23, &mut report);
    assert_eq!(report.differences.len(), 3);
    for field in [
        "air.sample_hour",
        "spotting_context.regions",
        "risk_ships.receivers",
    ] {
        assert!(
            report.differences.iter().any(|line| line.contains(field)),
            "{field}"
        );
    }
    assert!(
        report
            .limitations
            .keys()
            .any(|name| name.contains("writer lifetime"))
    );
}

#[test]
fn positive_cache_words_compare_exactly_and_omission_only_proves_nonpositive() {
    for (cache, text, differences) in [
        ([1, 712500], "fixture_r=0.00001 fixture_a=7.125", 0),
        ([2, 712500], "fixture_r=0.00001 fixture_a=7.125", 1),
        ([1, 0], "", 1),
        ([0, -123], "", 0),
        ([-1, 712500], "fixture_r=0.00001 fixture_a=7.125", 1),
    ] {
        let mut report = Report::default();
        compare(
            &frame(packet(receiver(Some(cache)))),
            &document(text),
            &mut report,
        )
        .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        if cache.iter().any(|&value| value <= 0) && differences == 0 {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("omission proves nonpositive"))
            );
        }
    }
}

#[test]
fn historical_absence_wrong_hour_and_current_refusal_remain_distinct() {
    let mut report = Report::default();
    compare(&frame(Recorded::NotRecorded), &document(""), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("not recorded"))
    );
    let mut report = Report::default();
    compare(&frame(Recorded::Value(None)), &document(""), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    let mut current = frame(packet(receiver(Some([1, 1]))));
    current.hour += 1;
    let mut report = Report::default();
    compare(&current, &document(""), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("sample_hour"));
}

#[test]
fn native_gate_skips_unreached_cache_and_missing_content_cannot_hide_current_refusal() {
    let mut original = receiver(None);
    original.in_regions = Some(0);
    original.combat_references = None;
    let mut current = frame(packet(original));
    current.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "missing tokens".into(),
    }
    .into();
    let mut report = Report::default();
    compare(&current, &document(""), &mut report).unwrap();
    assert!(report.differences.is_empty());
    let mut current = frame(packet(receiver(None)));
    current.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "missing tokens".into(),
    }
    .into();
    let mut report = Report::default();
    compare(&current, &document(""), &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("detection_cache"))
    );
}
