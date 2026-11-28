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
        let mut rows = vec![RaidAirScoreRow { country: 9001, region: 17, friendly: 5, enemy: 4, ..Default::default() }];
        if matches!(hour, 142 | 146) {
            rows.push(RaidAirScoreRow { country: 9001, region: 22, friendly: 9, enemy: 4, ..Default::default() });
        }
        *frame.tables[TableId::RaidAirScores.index()] = TableView::Truncated { rows: TableRows::RaidAirScores(rows), missing: 1 };
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
    let frames: Vec<_> = (100..=102)
        .map(|hour| {
            let mut frame = frame(1, hour);
            *frame.tables[TableId::AirRegionWings.index()] = TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full };
            frame
        })
        .collect();
    let current: Vec<usize> = TableId::ALL.into_iter().map(TableId::field_count).collect();
    let mut without_wings = current.clone();
    without_wings[TableId::AirRegionWings.index()] = 0;
    let writers = recorder_derive::writers_within(&without_wings, &current);
    let replay = Replay::from_records(writers, CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Arc::new(Source::from_replay(SourceKey { source_id: "air-unrecorded-wings".into(), revision: "1".into() }, "air.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
    let load = LoadContext { content: None };
    let request = json!({"branchId":"1","panel":"air-superiority","mode":"sides","pinnedRegions":[17]});
    let panel = (AIR_SUPERIORITY_AIRSUPERIORITYAT.panel.unwrap().open)(&source, &load, &request).unwrap();
    let mut original = frame_panel::<crate::products::air_superiority::AirSuperiorityAt>(&source, &load, &request).unwrap();
    let reads = played(source.replay_ref(), panel.exact().unwrap(), 100, 102);
    for read in &reads {
        let expected = materialized_frame(&mut *original, source.replay_ref(), 1, read.hour);
        assert_eq!(dashboard_base::test_fixtures::streamed(&read.value, read.fresh), dashboard_base::test_fixtures::streamed(&expected, read.fresh), "hour {}", read.hour);
        let reply = read.value.as_ref().as_ref().unwrap();
        assert!(reply["unlisted"].as_str().is_some_and(|why| why.contains(&format!("hour {}", read.hour))), "{reply}");
        assert!(reply["warnings"].as_array().unwrap().iter().any(|line| line.as_str().is_some_and(|line| line.contains(&format!("hour {}", read.hour)))), "{reply}");
    }
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
        if request["every"].is_null() {
            let seed = (exact.advance)(100, true, &false.into(), None);
            assert!(seed.lanes.iter().any(|lane| lane.table == TableId::RaidAirScores.index()), "{request}");
            assert!(seed.lanes.iter().any(|lane| lane.table == TableId::CountryLinks.index()), "{request}");
        }
        let reads = played(source.replay_ref(), exact, 100, 149);
        for read in reads.iter().filter(|r| r.hour > 100) {
            assert_eq!(*read.value, (exact.cold)(read.hour, &false.into()), "{request} at {}", read.hour);
            if matches!(read.hour, 103..=106 | 109) && request["theatres"].is_null() && request["every"].is_null() {
                assert_eq!(read.rows, 0, "{request} at {} holds unchanged lane rows", read.hour);
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
                } else {
                    let row_22 = reply["rows"].as_array().unwrap().iter().find(|row| row["region"] == 22);
                    if read.hour == 142 {
                        assert_eq!(row_22.and_then(|row| row["readout"]["kind"].as_str()), Some("leading"), "{reply}");
                    } else {
                        assert_ne!(
                            row_22.and_then(|row| row["readout"]["kind"].as_str()),
                            Some("leading"),
                            "{request} at 143 must not carry omitted region 22's score from hour 142 into truncated bucket end 143: {reply}",
                        );
                    }
                }
            }
            if read.hour == 149 && !request["every"].is_null() {
                let reply = read.value.as_ref().as_ref().unwrap();
                let row_22 = reply["rows"].as_array().unwrap().iter().find(|row| row["region"] == 22);
                assert_ne!(
                    row_22.and_then(|row| row["readout"]["kind"].as_str()),
                    Some("leading"),
                    "{request} at 149 must not carry region 22's score from truncated hour 146 into complete bucket end 149: {reply}",
                );
            }
        }
        let sought = (exact.advance)(101, true, &false.into(), None);
        assert!(sought.fresh);
        assert_eq!(*sought.value, (exact.cold)(101, &false.into()));
        if request["mode"] == "sides" {
            let advance = exact.advance.clone();
            let enemy = dashboard_base::products::at_hour::field_columns(TableId::RaidAirScores, &["enemy"]);
            let partial = dashboard_follow::panel::AtHour {
                cold: exact.cold.clone(),
                read: exact.read.clone(),
                advance: Arc::new(move |hour, seed, token, input| {
                    let mut read = advance(hour, seed, token, input.map(|input| dashboard_follow::panel::ProductStep { held: Some(&[]), ..input }));
                    if input.is_none() {
                        let scores = read.lanes.iter_mut().find(|lane| lane.table == TableId::RaidAirScores.index()).unwrap();
                        scores.columns.clone_from(&enemy);
                    }
                    read
                }),
            };
            let reads = played(source.replay_ref(), &partial, 100, 107);
            assert_eq!(*reads.last().unwrap().value, (exact.cold)(107, &false.into()));
        }
    }
}

#[test]
fn the_followed_air_tile_matches_each_exact_closed_day_prefix() {
    let registration = AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE;
    let source = recording(&[1]);
    let load = LoadContext { content: None };
    let request = |last_hour| json!({"branchId": "1", "mode": "sides", "every": true, "firstHour": 100, "lastHour": last_hour});
    let mut subscription = request(143);
    subscription["panel"] = registration.key.into();
    let panel = (registration.panel.unwrap().open)(&source, &load, &subscription).unwrap();
    let exact = panel.exact().unwrap();
    let before = Value::Null;
    let prefix = registration.answer(&source, request(119), &load, &false.into()).unwrap();
    let closed = registration.answer(&source, request(143), &load, &false.into()).unwrap();
    let reads = played(source.replay_ref(), exact, 100, 149);
    for read in &reads {
        let expected = if read.hour < 120 { &before } else if read.hour < 144 { &prefix } else { &closed };
        assert!(!read.pending);
        assert_eq!(read.value.as_ref().as_ref().unwrap(), expected, "prefix at {}", read.hour);
    }
    assert_eq!(reads.iter().filter(|read| read.fresh && read.hour > 100).map(|read| read.hour).collect::<Vec<_>>(), [120, 144]);
}

#[test]
fn the_air_day_rebuilds_on_read_errors_gaps_and_new_runs() {
    let segments = [(1, 96..=107), (2, 109..=125)].map(|(branch, hours)| {
        records(branch, hours.map(|hour| {
            let mut snapshot = frame(branch, hour);
            if (98..100).contains(&hour) {
                let mut rows = snapshot.complete::<tables::RaidAirScores>().unwrap().to_vec();
                rows.push(RaidAirScoreRow { country: 9001, region: 22, friendly: 13, enemy: 1, ..Default::default() });
                *snapshot.tables[TableId::RaidAirScores.index()] = TableView::Complete(TableRows::RaidAirScores(rows));
            }
            snapshot
        }).collect())
    });
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Arc::new(Source::from_replay(SourceKey { source_id: "gapped-air".into(), revision: "1".into() }, "air.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
    let request = json!({"panel": "air-superiority-timeline", "branchId": "1", "mode": "sides", "top": 7});
    let panel = (AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE.panel.unwrap().open)(&source, &LoadContext { content: None }, &request).unwrap();
    let exact = panel.exact().unwrap();
    let advance = exact.advance.clone();
    let with_error = dashboard_follow::panel::AtHour {
        cold: exact.cold.clone(),
        read: exact.read.clone(),
        advance: Arc::new(move |hour, seed, token, step| {
            if hour == 104 {
                let failed = advance(hour, seed, &true.into(), step);
                assert!(failed.value.is_err());
            }
            advance(hour, seed, token, step)
        }),
    };
    let reads = played(source.replay_ref(), &with_error, 96, 125);
    for hour in [100, 101, 102, 104, 107, 108, 109, 125] {
        let read = reads.iter().find(|read| read.hour == hour).unwrap();
        assert!(read.fresh, "hour {hour} rebuilds the current day");
        assert_eq!(*read.value, (exact.cold)(hour, &false.into()));
    }
    assert!(reads.iter().find(|read| read.hour == 104).unwrap().rows > 0, "an error discards the projected rows");
    let reply = |hour| reads.iter().find(|read| read.hour == hour).unwrap().value.as_ref().as_ref().unwrap();
    assert!(!reply(100)["rows"].as_array().unwrap().iter().any(|row| row["region"] == 22));
    assert!(reply(101)["rows"].as_array().unwrap().iter().any(|row| row["region"] == 22), "the summary can retain a deleted score at the bucket close");
    assert_eq!(reply(108)["gaps"], json!([{"firstHour": 108, "lastHour": 108}]));
}

#[test]
fn military_air_head_follows_changes_and_dates_quiet_replies_without_reloading() {
    let frames = (100..=120).map(|hour| {
        let mut frame = source(1, hour, Some(i64::from(hour)));
        let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
        countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"SOV".to_vec(), ..Default::default() });
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
        let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
        links.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
        let events = match hour {
            100 => Some((1, 9001, 200_000)),
            107 => Some((2, 9002, 100_000)),
            112 => Some((3, 9001, 100_000)),
            _ => None,
        }.into_iter().map(|(id, country, amount_raw)| AirLossEventRow {
            id, game_hour: hour, country, region: 1, native_cause: 1, amount_raw, ..Default::default()
        }).collect();
        *frame.tables[TableId::AirLossEvents.index()] = TableView::Complete(TableRows::AirLossEvents(events));
        frame.metrics.table_ns[TableId::AirLossEvents.index()] = 1;
        frame
    }).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Arc::new(Source::from_replay(SourceKey { source_id: "military-air-head".into(), revision: "1".into() }, "air.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
    let load = LoadContext { content: None };
    let registration = AIR_LEDGER_AT_MILITARYAIRAT;
    for (ranked, selection) in [(false, json!({"countries":["GER"]})), (true, json!({"top":1}))] {
        let mut request = selection;
        request["branchId"] = "1".into();
        request["family"] = "airLosses".into();
        request["metric"] = "taken".into();
        let mut follow_request = request.clone();
        follow_request["panel"] = registration.key.into();
        let follow = (registration.panel.unwrap().open)(&source, &load, &follow_request).unwrap();
        for read in played(source.replay_ref(), follow.exact().unwrap(), 100, 120) {
            request["hour"] = read.hour.into();
            let reply = registration.answer(&source, request.clone(), &load, &false.into()).unwrap();
            assert_eq!(read.value.as_ref().as_ref().unwrap(), &reply, "{follow_request} at {}", read.hour);
            match read.hour {
                100 if ranked => assert!(reply["validUntil"].as_i64().unwrap() >= 107, "the runner-up does not change the shown rank"),
                101..=106 => assert_eq!(read.product_calls, 0, "quiet hour {}", read.hour),
                107 => assert_eq!(read.product_calls, u64::from(ranked), "a ranked proof stops at the runner-up's event"),
                112 => assert_eq!(read.product_calls, 1, "the new event is included at its read hour"),
                _ => {}
            }
        }
    }
}
