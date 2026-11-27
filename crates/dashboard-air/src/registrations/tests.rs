use super::*;
use dashboard_base::{Source, SourceKey, products::{LoadContext, Registered, at_hour::{AtHour, frame_panel}}, test_fixtures::{CAMPAIGN, materialized_frame, played, records, source}};
use replay::campaign::Replay;
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::Arc;

fn frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(i64::from(hour)));
    for table in TableId::ALL {
        if !table.is_derived() && table.key_bits() != 0 && matches!(frame.tables[table.index()].held(), TableView::Unavailable { .. }) {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    let strength = 10 * branch as i64 + i64::from(hour >= 107);
    let region = if hour < 107 { 17 } else { 22 };
    let mut set = |table: TableId, rows| {
        *frame.tables[table.index()] = if (110..115).contains(&hour) {
            TableView::Unavailable { reason: "damaged air table".into() }
        } else { TableView::Complete(rows) };
    };
    set(TableId::RaidAirScores, TableRows::RaidAirScores(vec![RaidAirScoreRow { country: 9001, region: 17, friendly: strength, enemy: 4, friendly_missions: hour as u16, ..Default::default() }]));
    set(TableId::AirAces, TableRows::AirAces(vec![AirAceRow { id: 42, country: 9001, assigned_wing: 42, alive: 1, name: format!("Ace {hour}").into_bytes(), callsign: format!("Pilot {strength}").into_bytes(), ..Default::default() }]));
    set(TableId::AirStatistics, TableRows::AirStatistics(vec![AirStatisticsRow { country: 9001, region: 17, channels: [strength; 24], ..Default::default() }]));
    set(TableId::AirWings, TableRows::AirWings(vec![AirWingRow { id: 42, owner: 9001, planes: strength as i32, mission_mask: 1, mission_region: Some(region), name: Recorded::Value(Some(b"Wing".to_vec())), ..Default::default() }]));
    set(TableId::AirRegionalMissions, TableRows::AirRegionalMissions(vec![AirRegionalMissionRow { wing: 42, ..Default::default() }]));
    set(TableId::AirTheatres, TableRows::AirTheatres(vec![AirTheatreRow { id: 1, owner: 9001, ..Default::default() }]));
    set(TableId::AirTheatreGroups, TableRows::AirTheatreGroups(vec![AirTheatreGroupRow { id: 2, theatre: 1, ..Default::default() }]));
    set(TableId::AirTheatreMembers, TableRows::AirTheatreMembers(vec![AirTheatreMemberRow { group: 2, wing: 42, ordinal: 0 }]));
    set(TableId::WingEquipment, TableRows::WingEquipment(vec![WingEquipmentRow { wing: 42, variant: 3, ..Default::default() }]));
    set(TableId::VariantDefinitions, TableRows::VariantDefinitions(vec![VariantDefinitionRow { id: 3, equipment_token: 4, ..Default::default() }]));
    set(TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 4, literal: b"aircraft".to_vec() }]));
    if hour >= 118 {
        let tables = [TableId::AirTheatres, TableId::AirTheatreGroups, TableId::AirTheatreMembers, TableId::AirWings, TableId::VariantDefinitions, TableId::LexicalTokens, TableId::CountryLinks, TableId::Countries];
        let offset = (hour - 118) as usize;
        if offset % 3 < 2 && let Some(&table) = tables.get(offset / 3) {
            *frame.tables[table.index()] = TableView::Unavailable { reason: "damaged dependency".into() };
        }
    }
    if (142..147).contains(&hour) {
        let rows = TableRows::RaidAirScores(vec![RaidAirScoreRow { country: 9001, region: 17, friendly: 5, enemy: 4, ..Default::default() }]);
        *frame.tables[TableId::RaidAirScores.index()] = TableView::Truncated { rows, missing: 1 };
    }
    frame
}

fn recording(branches: &[u64]) -> Arc<Source> {
    let branches: Vec<_> = branches.iter().map(|&branch| records(branch, (100..=149).map(|hour| frame(branch, hour)).collect())).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &branches).unwrap().into_timelines(&Default::default()).unwrap();
    Arc::new(Source::from_replay(SourceKey { source_id: "air-follow".into(), revision: "1".into() }, "air.battleplan".into(), Arc::new(replay), &false.into()).unwrap())
}

fn follows<P: AtHour>(registration: Registered, mut request: Value, live: bool) {
    let source = recording(&[1, 2]);
    let load = LoadContext { content: None };
    request["panel"] = P::KEY.into();
    let mut seeds = Vec::new();
    for branch in [1, 2] {
        request["branchId"] = branch.to_string().into();
        let panel = (registration.panel.unwrap().open)(&source, &load, &request).unwrap();
        let exact = panel.exact().expect("registered exact reader");
        let mut original = frame_panel::<P>(&source, &load, &request).unwrap();
        for (hour, seed) in (100..=149).map(|hour| (hour, hour == 100)).chain([(107, true), (100, true)]) {
            let expected = materialized_frame(&mut *original, source.replay_ref(), branch, hour);
            let step = (exact.advance)(hour, seed, &false.into(), None);
            if seed {
                assert!(step.fresh, "{} branch {branch} at {hour} needs a new seed", P::KEY);
                if live {
                    let groups = step.value.as_ref().as_ref().unwrap()["live"]["groups"].as_array().unwrap();
                    let values: Vec<_> = groups.iter().flat_map(|group| group["values"].as_array().unwrap()).filter_map(Value::as_i64).collect();
                    assert!(!values.is_empty(), "{} must seed its live values", P::KEY);
                    seeds.push(values);
                }
            }
            let materialized = (exact.advance)(hour, true, &false.into(), None);
            assert_eq!(*materialized.value, expected, "{} branch {branch} at {hour}", P::KEY);
        }
    }
    if live {
        assert_eq!(seeds[0], seeds[2], "{} seek back restores the first seed", P::KEY);
        assert_ne!(seeds[0], seeds[1], "{} seek changes live values", P::KEY);
        assert_ne!(seeds[2], seeds[3], "{} branch switch changes live values", P::KEY);
    }
    let source = recording(&[1]);
    request["branchId"] = "1".into();
    let panel = (registration.panel.unwrap().open)(&source, &load, &request).unwrap();
    let mut original = frame_panel::<P>(&source, &load, &request).unwrap();
    let reads = played(source.replay_ref(), panel.exact().unwrap(), 100, 149);
    let mut previous = None;
    for read in &reads {
        let value = materialized_frame(&mut *original, source.replay_ref(), 1, read.hour);
        let streamed = dashboard_base::test_fixtures::streamed;
        assert_eq!(streamed(&read.value, read.fresh), streamed(&value, read.fresh), "{} played at {}", P::KEY, read.hour);
        if let Some(last) = &previous {
            assert_eq!(read.product_calls, u64::from(value.is_ok() && last != &value), "{} at {}", P::KEY, read.hour);
        }
        previous = Some(value);
    }
}

#[test]
fn statistics_follow_the_selected_channels_and_reseed_their_live_values() {
    follows::<crate::products::air_statistics::AirStatisticsAt>(AIR_STATISTICS_AIRSTATISTICSAT, json!({"country":"GER","channel":1,"regions":[],"theatres":[]}), true);
}

#[test]
fn missions_follow_the_selected_country_and_reseed_their_live_values() {
    follows::<crate::products::air_missions::AirMissionsAt>(AIR_MISSIONS_AIRMISSIONSAT, json!({"country":"GER"}), true);
}

#[test]
fn aces_follow_the_recorded_roster() {
    follows::<crate::products::air_aces::AirAcesAt>(AIR_ACES_AIRACESAT, json!({}), false);
}

#[test]
fn superiority_follows_the_scores_and_reseeds_its_live_values() {
    follows::<crate::products::air_superiority::AirSuperiorityAt>(AIR_SUPERIORITY_AIRSUPERIORITYAT, json!({"mode":"viewpoint","viewpointCountry":true,"viewpoint":"GER","pinnedRegions":[17]}), true);
}

#[test]
fn the_current_air_day_follows_score_changes_and_dates_the_held_readouts() {
    let registration = AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE;
    let source = recording(&[1]);
    let load = LoadContext { content: None };
    for selection in [
        json!({"mode":"sides","pinnedRegions":[17],"top":7}),
        json!({"mode":"viewpoint","viewpoint":"GER","viewpointCountry":true,"pinnedRegions":[17]}),
        json!({"theatres":["1"],"pinnedRegions":[17],"top":7}),
        json!({"every":true}),
    ] {
        let mut request = selection;
        request["branchId"] = "1".into();
        request["panel"] = registration.key.into();
        let panel = (registration.panel.unwrap().open)(&source, &load, &request).unwrap();
        let exact = panel.exact().unwrap();
        let reads = played(source.replay_ref(), exact, 100, 149);
        for read in reads.iter().filter(|r| r.hour > 100) {
            assert_eq!(*read.value, (exact.cold)(read.hour, &false.into()), "{request} at {}", read.hour);
            if read.hour < 110 && request["theatres"].is_null() {
                assert_eq!(read.product_calls, u64::from(read.hour == 107), "{request} at {}", read.hour);
            }
            if matches!(read.hour, 110 | 113) {
                let reply = read.value.as_ref().as_ref().unwrap();
                assert_eq!(reply["days"], json!([null]), "{request} at {} must not treat unavailable scores as recorded", read.hour);
                assert!(reply["warnings"].as_array().unwrap().iter().any(|line| line.as_str().is_some_and(|line| line.contains("damaged air table"))), "{reply}");
                if request["every"].is_null() {
                    assert!(reply["rankMissing"].as_str().is_some_and(|why| why.contains("damaged air table")), "{reply}");
                }
            }
            if matches!(read.hour, 142 | 143) {
                let reply = read.value.as_ref().as_ref().unwrap();
                assert_eq!(reply["days"], json!([read.hour]), "{request} at {} keeps witnessed truncated scores", read.hour);
                assert!(reply["warnings"].as_array().unwrap().iter().any(|line| line.as_str().is_some_and(|line| line.contains("1 rows not read"))), "{reply}");
                if request["every"].is_null() {
                    assert!(reply["rankMissing"].as_str().is_some_and(|why| why.contains("1 rows not read")), "{reply}");
                }
            }
        }
        assert_eq!(*(exact.advance)(101, true, &false.into(), None).value, (exact.cold)(101, &false.into()));
    }
}
