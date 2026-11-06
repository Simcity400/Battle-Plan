use super::*;
use dashboard_base::{
    prepared::{
        coverage::{Coverage, SCHEMA},
        prefix::Prefix,
        query::progressive,
        series::tests::Fixture,
    },
    test_fixtures::{CAMPAIGN, records, world_frame},
};
use dashboard_keyed::{Observer as _, Scan, Values};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, AirWingRow, ModifierDefinitionRow,
        Recorded, RegionCombatWeather, TableId, TableRows, WeatherRegionRow,
    },
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

pub(super) fn row(id: u32) -> WeatherRegionRow {
    WeatherRegionRow {
        id,
        air_detection: Recorded::Value(Some(None)),
        map_modifiers_known: Recorded::Value(true),
        naval_detection: Recorded::Value(None),
        air_attack: Recorded::Value(None),
        air_defence: Recorded::Value(None),
        combat: Recorded::Value(Some(RegionCombatWeather::default())),
        ..Default::default()
    }
}

pub(super) fn frame(hour: i32, rows: Option<Vec<WeatherRegionRow>>) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    *frame.tables[TableId::WeatherRegions.index()] = match rows {
        Some(rows) => TableView::Complete(TableRows::WeatherRegions(rows)),
        None => TableView::Unavailable {
            reason: "synthetic unavailable weather".into(),
        },
    };
    frame
}

fn values(frame: &DecodedFrame) -> Option<BTreeMap<String, i128>> {
    let mut observer = block::Observer::default();
    let mut values = Values::default();
    observer
        .observe(FormatVersion::CURRENT, frame, &mut values)
        .unwrap()
        .then(|| {
            values
                .iter()
                .map(|(name, raw)| (name.to_owned(), raw))
                .collect()
        })
}

fn publish(fixture: &Fixture, frames: &[DecodedFrame]) -> Scan<block::Observer> {
    let mut scan = Scan::<block::Observer>::default();
    let mut writer = fixture.writer();
    for frame in frames {
        scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
    }
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    scan
}

pub(super) fn source(frames: Vec<DecodedFrame>) -> dashboard_base::Source {
    let replay =
        replay::container::Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "weather-test".into(),
            revision: "1".into(),
        },
        "weather-test.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

pub(super) fn answer(
    fixture: &Fixture,
    source: &dashboard_base::Source,
    params: &Value,
    through: Option<i32>,
    content: Option<&dashboard_core::config::ContentOptions>,
) -> io::Result<Value> {
    let branch = &source.metadata().unwrap().branches[0];
    let prefix = Prefix {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: branch.first_hour,
            last_hour: through.unwrap_or(branch.last_hour),
            complete: through.is_none(),
        },
        pending: vec![],
    };
    let cancelled = AtomicBool::new(false);
    let request = Query {
        source,
        params,
        content,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    respond(&request, || request.open::<block::Observer>())
}

#[test]
fn concurrent_flags_have_no_precedence_and_known_missing_keys_are_explicit_zero() {
    let mut weather = row(7913);
    weather.light_rain = 1;
    weather.heavy_rain = 2;
    weather.snow = 3;
    weather.blizzard = 4;
    weather.sandstorm = 5;
    weather.arctic_water = 255;
    let values = values(&frame(23, Some(vec![weather]))).unwrap();
    assert_eq!(values.len(), 14);
    for (flag, raw) in [
        ("light_rain", 1),
        ("heavy_rain", 2),
        ("snow", 3),
        ("blizzard", 4),
        ("sandstorm", 5),
        ("arctic_water", 255),
    ] {
        assert_eq!(values[&format!("7913|{flag}")], raw);
    }
    for (modifier, _, _) in content::MODIFIERS {
        assert_eq!(values[&format!("7913|mod:{modifier}")], 0);
    }
    assert!(!values.keys().any(|key| key.ends_with("|temp")));
}

#[test]
fn modifier_witnesses_are_independent_and_do_not_admit_unwitnessed_scalars() {
    let mut a = row(9001);
    a.air_detection = Recorded::Value(Some(Some(-123_456)));
    a.map_modifiers_known = Recorded::Value(false);
    a.air_attack = Recorded::Value(Some(999));
    a.combat = Recorded::Value(None);
    let a = values(&frame(0, Some(vec![a]))).unwrap();
    assert_eq!(a["9001|mod:air_detection"], -123_456);
    assert!(!a.contains_key("9001|mod:air_attack"));
    assert!(!a.contains_key("9001|mod:army_speed_factor"));
    assert_eq!(a["9001|snow"], 0);

    let mut b = row(9001);
    b.air_detection = Recorded::Value(None);
    b.air_attack = Recorded::Value(Some(i64::MIN));
    b.air_defence = Recorded::NotRecorded;
    b.combat = Recorded::Value(Some(RegionCombatWeather {
        army_attack_factor: Some(i64::MAX),
        ..Default::default()
    }));
    let b = values(&frame(1, Some(vec![b]))).unwrap();
    assert!(!b.contains_key("9001|mod:air_detection"));
    assert_eq!(b["9001|mod:naval_detection"], 0);
    assert_eq!(b["9001|mod:air_attack"], i128::from(i64::MIN));
    assert!(!b.contains_key("9001|mod:air_defence"));
    assert_eq!(b["9001|mod:army_attack_factor"], i128::from(i64::MAX));
    assert_eq!(b["9001|mod:breakthrough_factor"], 0);

    let mut c = row(9001);
    c.air_detection = Recorded::NotRecorded;
    c.map_modifiers_known = Recorded::NotRecorded;
    c.naval_detection = Recorded::Value(Some(17));
    c.combat = Recorded::NotRecorded;
    let c = values(&frame(2, Some(vec![c]))).unwrap();
    assert_eq!(c.len(), 6);
    assert!(values(&frame(3, None)).is_none());
    assert!(values(&frame(4, Some(vec![]))).unwrap().is_empty());
}

#[test]
fn runs_keep_absent_unknown_gaps_carried_reads_and_prefix_across_resume() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<block::Observer>::default();
    let mut writer = fixture.writer();
    scan.prepare(
        FormatVersion::CURRENT,
        &frame(20, Some(vec![row(8888)])),
        true,
    )
    .unwrap();
    scan.advance_quiet(21, false).unwrap();
    scan.advance_quiet(22, true).unwrap();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    scan = dashboard_base::prepared::series::read_catalogue::<Scan<block::Observer>>(
        &fixture.store,
        &fixture.key,
        block::Observer::NAME,
        1,
    )
    .unwrap()
    .unwrap()
    .1;
    let mut writer = fixture.writer();
    scan.prepare(FormatVersion::CURRENT, &frame(23, Some(vec![])), true)
        .unwrap();
    scan.prepare(FormatVersion::CURRENT, &frame(24, None), true)
        .unwrap();
    scan.prepare(
        FormatVersion::CURRENT,
        &frame(27, Some(vec![row(8888)])),
        false,
    )
    .unwrap();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut opened =
        dashboard_keyed::query::open::<block::Observer>(&fixture.store, &fixture.key, 1, &flag)
            .unwrap()
            .unwrap();
    let window = opened.window(20, 27, None, |_| true).unwrap();
    assert_eq!(
        window
            .known
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(20, 23), (27, 27)]
    );
    assert_eq!(window.reads, [(20, 21, 20), (22, 22, 22), (23, 23, 23)]);
    assert_eq!(
        window
            .series
            .iter()
            .find(|series| series.name == "8888|snow")
            .unwrap()
            .runs,
        [(20, 22, "0".into(), true), (27, 27, "0".into(), true)]
    );
    let clipped = opened.window(20, 27, Some(21), |_| true).unwrap();
    assert!(
        clipped
            .series
            .iter()
            .all(|series| series.runs == [(20, 21, "0".into(), true)])
    );
}

#[test]
fn daily_endpoint_never_falls_back_to_an_earlier_known_weather_hour() {
    let fixture = Fixture::new();
    let mut first = row(73);
    first.snow = 1;
    let mut second = row(73);
    second.sandstorm = 1;
    let frames = vec![
        frame(22, Some(vec![first])),
        frame(23, None),
        frame(24, None),
        frame(25, Some(vec![second])),
    ];
    publish(&fixture, &frames);
    let source = source(frames);
    let reply = answer(
        &fixture,
        &source,
        &json!({"branchId":"1", "daily":true, "show":"conditions"}),
        None,
        None,
    )
    .unwrap();
    // The first day's endpoint is unknown at 23, despite known snow at 22.
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":24,"lastHour":25}])
    );
    assert_eq!(reply["window"]["reads"], json!([[24, 25, 25]]));
    // A partial day's endpoint is 25; it covers its whole reached day, even
    // though 24 itself was weather-unknown. It never invents hour 47.
    assert_eq!(
        reply_runs(&reply, "73|sandstorm"),
        json!([[24, 25, "1", true]])
    );
    assert_eq!(reply_runs(&reply, "73|snow"), json!([[24, 25, "0", true]]));
}

pub(super) fn reply_runs(reply: &Value, name: &str) -> Value {
    reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|series| series["name"] == name)
        .map_or(json!([]), |series| series["runs"].clone())
}

#[test]
fn daily_sampling_uses_last_reached_hour_coalesces_days_and_clips_to_window() {
    let fixture = Fixture::new();
    let frames: Vec<_> = (16..=55)
        .map(|hour| {
            let mut weather = row(73);
            weather.snow = match hour {
                23 | 47 => 6,
                55 => 9,
                _ => 1,
            };
            frame(hour, Some(vec![weather]))
        })
        .collect();
    publish(&fixture, &frames);
    let source = source(frames);
    let params = json!({"branchId":"1", "firstHour":18,"daily":true,"show":"conditions"});
    let reply = answer(&fixture, &source, &params, None, None).unwrap();
    assert_eq!(reply["dayHours"], daily::DAY_HOURS);
    assert_eq!(reply["window"]["firstHour"], 18);
    assert_eq!(reply["window"]["lastHour"], 55);
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":18,"lastHour":55}])
    );
    assert_eq!(
        reply["window"]["reads"],
        json!([[18, 23, 23], [24, 47, 47], [48, 55, 55]])
    );
    assert_eq!(
        reply_runs(&reply, "73|snow"),
        json!([[18, 47, "6", true], [48, 55, "9", false]])
    );
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 6);
    assert!(
        reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|series| !series["name"].as_str().unwrap().contains("mod:"))
    );

    let limited = answer(&fixture, &source, &params, Some(30), None).unwrap();
    // Preparation has reached 30, not the later known value at 47. Neither
    // runs nor reads extend into the pending tail of that calendar day.
    assert_eq!(
        limited["window"]["known"],
        json!([{"firstHour":18,"lastHour":30}])
    );
    assert_eq!(
        limited["window"]["reads"],
        json!([[18, 23, 23], [24, 30, 30]])
    );
    assert_eq!(
        reply_runs(&limited, "73|snow"),
        json!([[18, 23, "6", true], [24, 30, "1", false]])
    );

    let pending = answer(&fixture, &source, &params, Some(17), None).unwrap();
    assert_eq!(pending["window"]["known"], json!([]));
    assert_eq!(pending["window"]["reads"], json!([]));
    assert_eq!(pending["window"]["series"], json!([]));

    let requested = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","firstHour":19,"lastHour":29,"daily":true,"show":"conditions"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        reply_runs(&requested, "73|snow"),
        json!([[19, 23, "6", true], [24, 29, "1", false]])
    );
}

#[test]
fn a_day_with_only_unknown_hours_remains_a_gap_in_series_known_and_reads() {
    let fixture = Fixture::new();
    let frames: Vec<_> = (0..=71)
        .map(|hour| frame(hour, (!(24..48).contains(&hour)).then(|| vec![row(73)])))
        .collect();
    publish(&fixture, &frames);
    let source = source(frames);
    let reply = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","daily":true,"show":"modifier","modifier":"air_detection"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        reply["window"]["known"],
        json!([
            {"firstHour":0,"lastHour":23},{"firstHour":48,"lastHour":71}
        ])
    );
    assert_eq!(reply["window"]["reads"], json!([[0, 23, 23], [48, 71, 71]]));
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 1);
    assert_eq!(
        reply_runs(&reply, "73|mod:air_detection"),
        json!([[0, 23, "0", true], [48, 71, "0", true]])
    );
}

#[test]
fn show_selects_only_requested_names_and_counts_only_the_selected_witness() {
    let fixture = Fixture::new();
    let mut unknown = row(73);
    unknown.air_detection = Recorded::Value(None);
    let frames = vec![frame(1, Some(vec![unknown]))];
    publish(&fixture, &frames);
    let source = source(frames);
    let refused = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","show":"modifier","modifier":"air_detection"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(refused["window"]["series"], json!([]));
    assert!(
        refused["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                == "1 weather modifier histories not shown: regional modifier witness unavailable")
    );
    // Without a chosen modifier the view shows the catalogue's first, as the widget's default says.
    let automatic = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","show":"modifier"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(automatic["modifiers"][0]["id"], "air_detection");
    assert_eq!(automatic["window"], refused["window"]);
    assert_eq!(automatic["warnings"], refused["warnings"]);
    let selected = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","show":"modifier","modifier":"air_attack"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(selected["window"]["series"].as_array().unwrap().len(), 1);
    assert_eq!(
        reply_runs(&selected, "73|mod:air_attack"),
        json!([[1, 1, "0", true]])
    );
    assert!(
        !selected["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("modifier histories"))
    );
    let conditions = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","show":"conditions"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(conditions["window"]["series"].as_array().unwrap().len(), 6);
    assert!(
        !conditions["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("modifier histories"))
    );
    let all = answer(&fixture, &source, &json!({"branchId":"1"}), None, None).unwrap();
    assert_eq!(all["window"]["series"].as_array().unwrap().len(), 13);
    assert_eq!(selected["flags"], all["flags"]);
    assert_eq!(selected["modifiers"], all["modifiers"]);
    for bad in [
        json!({"branchId":"1","show":"modifier","modifier":"no_such_field"}),
        json!({"branchId":"1","show":"unknown"}),
        json!({"branchId":"1","daily":"true"}),
    ] {
        assert!(
            answer(&fixture, &source, &bad, None, None).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn daily_sampling_keeps_independent_modifier_refusals_and_known_absence() {
    let fixture = Fixture::new();
    let frames: Vec<_> = (0..=71)
        .map(|hour| {
            let mut weather = row(73);
            weather.air_detection = if hour == 23 {
                Recorded::Value(None)
            } else {
                Recorded::Value(Some(Some(-123_456)))
            };
            frame(hour, Some(if hour == 71 { vec![] } else { vec![weather] }))
        })
        .collect();
    publish(&fixture, &frames);
    let source = source(frames);
    let reply = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","daily":true,"show":"modifier","modifier":"air_detection"}),
        None,
        None,
    )
    .unwrap();
    // Flags prove a region exists on day zero but its selected witness does not.
    // Day two is known, with an absent region, not a refused modifier or zero.
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":0,"lastHour":71}])
    );
    assert_eq!(
        reply_runs(&reply, "73|mod:air_detection"),
        json!([[24, 47, "-123456", true]])
    );
    assert!(
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                == "1 weather modifier histories not shown: regional modifier witness unavailable")
    );
}

#[test]
fn daily_projection_respects_recording_gaps_negative_hours_carried_reads_and_integer_edges() {
    use dashboard_base::products::country_groups::HourSpan;
    let mut window = KeyedWindow {
        first_hour: -26,
        last_hour: 76,
        known: vec![
            HourSpan {
                first_hour: -26,
                last_hour: 27,
            },
            HourSpan {
                first_hour: 75,
                last_hour: 76,
            },
        ],
        reads: vec![(-26, 27, -26), (75, 76, 75)],
        series: vec![KeyedSeries {
            name: "73|snow".into(),
            runs: vec![(-26, 27, "3".into(), true), (75, 76, "3".into(), true)],
        }],
    };
    daily::Days::new(&window.known, -25, 76)
        .sample(&mut window, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        window
            .known
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(-25, 47), (72, 76)]
    );
    assert_eq!(window.reads, [(-25, 47, -26), (72, 76, 75)]);
    assert_eq!(
        window.series[0].runs,
        [(-25, 47, "3".into(), true), (72, 76, "3".into(), true)]
    );
    // A requested slice after the day's last recorded hour still uses that
    // same day's sample, not a previous or following calendar day's value.
    let mut slice = window.clone();
    slice.known = vec![HourSpan {
        first_hour: 24,
        last_hour: 27,
    }];
    slice.reads = vec![(24, 27, 24)];
    slice.series[0].runs = vec![(24, 27, "7".into(), true)];
    daily::Days::new(&slice.known, 30, 40)
        .sample(&mut slice, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(slice.series[0].runs, [(30, 40, "7".into(), true)]);
    assert_eq!(slice.reads, [(30, 40, 24)]);
    assert_eq!(
        daily::Days::new(&[], 0, 0)
            .sample(&mut slice, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(daily::day_start(-1), -daily::DAY_HOURS);
    assert_eq!(daily::day_start(i32::MIN), i32::MIN);
    for hour in [i32::MIN, i32::MAX] {
        let span = HourSpan {
            first_hour: hour,
            last_hour: hour,
        };
        let mut edge = KeyedWindow {
            first_hour: hour,
            last_hour: hour,
            known: vec![span],
            reads: vec![(hour, hour, hour)],
            series: vec![KeyedSeries {
                name: "73|snow".into(),
                runs: vec![(hour, hour, "1".into(), true)],
            }],
        };
        daily::Days::new(&[span], hour, hour)
            .sample(&mut edge, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(edge.series[0].runs, [(hour, hour, "1".into(), true)]);
        assert_eq!(edge.reads, [(hour, hour, hour)]);
    }
}

#[test]
fn daily_query_uses_sparse_recorded_hours_not_weather_coverage_as_its_calendar() {
    let fixture = Fixture::new();
    let first: Vec<_> = (0..=20)
        .map(|hour| frame(hour, Some(vec![row(73)])))
        .collect();
    let later: Vec<_> = (72..=76)
        .map(|hour| frame(hour, Some(vec![row(73)])))
        .collect();
    let mut frames = first.clone();
    frames.extend(later.iter().cloned());
    publish(&fixture, &frames);
    let replay =
        replay::container::Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    let continuation = dashboard_base::test_fixtures::continuation_records(&replay, 1, later);
    let replay = replay.append(&[continuation]).unwrap();
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "weather-gap".into(),
            revision: "1".into(),
        },
        "weather-gap.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","daily":true,"show":"conditions"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        reply["window"]["known"],
        json!([
            {"firstHour":0,"lastHour":23},{"firstHour":72,"lastHour":76}
        ])
    );
    assert_eq!(reply["window"]["reads"], json!([[0, 23, 20], [72, 76, 76]]));
    assert_eq!(
        reply_runs(&reply, "73|snow"),
        json!([[0, 23, "0", true], [72, 76, "0", true]])
    );
    let slice = answer(
        &fixture,
        &source,
        &json!({"branchId":"1","firstHour":21,"lastHour":23,"daily":true,"show":"conditions"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(reply_runs(&slice, "73|snow"), json!([[21, 23, "0", true]]));
    assert_eq!(slice["window"]["reads"], json!([[21, 23, 20]]));
}

#[test]
fn schema_fields_watch_all_witnesses_and_the_weather_family_is_country_independent() {
    let fields = block::Observer::fields().unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].fields.count_ones(), 12);
    for field in [
        "combat",
        "air_detection",
        "map_modifiers_known",
        "naval_detection",
        "air_attack",
        "air_defence",
    ] {
        let watched =
            replay::container::TableFields::named(TableId::WeatherRegions, &[field]).unwrap();
        assert_ne!(fields[0].fields & watched.fields, 0);
    }
    assert!(!fields[0].read);
    let frame = frame(0, Some(vec![row(7)]));
    let mut changes = dashboard_base::prepared::frame_changes::FrameChanges::alone(
        &frame,
        FormatVersion::CURRENT,
    );
    changes.rows_changed.fill(false);
    changes.slot_read[TableId::WeatherProvinces.index()] = false;
    assert!(!block::Observer::changed(&changes));
    changes.rows_changed[TableId::Countries.index()] = true;
    assert!(!block::Observer::changed(&changes));
    changes.slot_read[TableId::WeatherProvinces.index()] = true;
    assert!(block::Observer::changed(&changes));
    changes.slot_read[TableId::WeatherProvinces.index()] = false;
    changes.rows_changed[TableId::WeatherProvinces.index()] = true;
    assert!(block::Observer::changed(&changes));
    changes.rows_changed[TableId::WeatherProvinces.index()] = false;
    changes.rows_changed[TableId::WeatherRegions.index()] = true;
    assert!(block::Observer::changed(&changes));
}

#[test]
fn missing_modifier_warnings_are_selected_and_window_local() {
    let fixture = Fixture::new();
    let mut unknown = row(73);
    unknown.air_detection = Recorded::Value(None);
    publish(
        &fixture,
        &[frame(1, Some(vec![unknown])), frame(2, Some(vec![row(73)]))],
    );
    let flag = AtomicBool::new(false);
    let mut opened =
        dashboard_keyed::query::open::<block::Observer>(&fixture.store, &fixture.key, 1, &flag)
            .unwrap()
            .unwrap();
    let mut warnings = vec![];
    let params = serde_json::from_value(json!({"branchId":"1"})).unwrap();
    modifier_refusals(
        &opened.window(1, 2, None, |_| true).unwrap(),
        &params,
        &mut warnings,
    );
    assert_eq!(
        warnings,
        ["1 weather modifier histories not shown: regional modifier witness unavailable"]
    );
    warnings.clear();
    modifier_refusals(
        &opened.window(2, 2, None, |_| true).unwrap(),
        &params,
        &mut warnings,
    );
    assert!(warnings.is_empty());
}

#[test]
fn cold_query_contract_validation_and_cancellation() {
    let fixture = Fixture::new();
    let source = source(vec![frame(10, Some(vec![]))]);
    let params = json!({"branchId":"1", "regions":["7001"]});
    let reply = answer(&fixture, &source, &params, Some(9), None).unwrap();
    let mut keys: Vec<_> = reply
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "dayHours",
            "flags",
            "modifiers",
            "regions",
            "temperature",
            "view",
            "warnings",
            "window"
        ]
    );
    assert_eq!(reply["window"]["firstHour"], 10);
    assert_eq!(reply["window"]["known"], json!([]));
    assert_eq!(reply["window"]["series"], json!([]));
    assert_eq!(reply["flags"].as_array().unwrap().len(), 6);
    assert_eq!(reply["modifiers"].as_array().unwrap().len(), 8);
    assert_eq!(
        reply["regions"][0],
        json!({"id":"7001", "name":"Region name unavailable"})
    );
    assert_eq!(reply["temperature"], TEMPERATURE_REFUSAL);
    assert_eq!(reply["dayHours"], replay_data::cadence::STAGGER_PERIOD);
    for bad in [
        json!({"branchId":"bad"}),
        json!({"branchId":"1", "regions":["07"]}),
        json!({"branchId":"1", "regions":["-1"]}),
        json!({"branchId":"1", "firstHour":3,"lastHour":2}),
        json!({"branchId":"2"}),
    ] {
        assert!(answer(&fixture, &source, &bad, None, None).is_err());
    }
    let prefix = Prefix {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 10,
            complete: true,
        },
        pending: vec![],
    };
    let cancelled = AtomicBool::new(true);
    let request = Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    let error = respond(&request, || request.open::<block::Observer>()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

fn theatre_frame() -> DecodedFrame {
    let mut frame = frame(20, Some(vec![row(73)]));
    *frame.tables[TableId::AirTheatres.index()] =
        TableView::Complete(TableRows::AirTheatres(vec![
            AirTheatreRow {
                id: 2,
                name: b"Empty theatre".to_vec(),
                ..Default::default()
            },
            AirTheatreRow {
                id: u64::MAX,
                name: b"Synthetic theatre".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
            id: 9,
            theatre: u64::MAX,
            ..Default::default()
        }]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![
            AirTheatreMemberRow {
                wing: 81,
                group: 9,
                ordinal: 0,
            },
            AirTheatreMemberRow {
                wing: 82,
                group: 9,
                ordinal: 1,
            },
        ]));
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![
        AirWingRow {
            id: 81,
            mission_region: Some(73),
            ..Default::default()
        },
        AirWingRow {
            id: 82,
            mission_region: Some(73),
            ..Default::default()
        },
    ]));
    frame
}

#[test]
fn theatre_membership_deduplicates_regions_keeps_empty_and_refuses_broken_joins() {
    let mut frame = theatre_frame();
    let mut loc = installed_content_localisation::localisation::Localisation::default();
    loc.language = "english".into();
    let mut warnings = vec![];
    let theatres =
        content::theatre_membership(&frame, Some(&loc), "l_english", &mut warnings).unwrap();
    assert_eq!(theatres[1].id, u64::MAX.to_string());
    assert_eq!(theatres[1].name, "Synthetic theatre");
    assert_eq!(theatres[1].regions, ["73"]);
    assert!(theatres[0].regions.is_empty());
    assert!(warnings.is_empty());
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![]));
    assert!(
        content::theatre_membership(&frame, Some(&loc), "english", &mut warnings)
            .unwrap_err()
            .contains("group is not recorded")
    );
    let mut french = installed_content_localisation::localisation::Localisation::default();
    french.language = "french".into();
    let theatres =
        content::theatre_membership(&theatre_frame(), Some(&french), "english", &mut warnings)
            .unwrap();
    assert_eq!(theatres[0].name, "Air theatre name unavailable");
    assert!(warnings.iter().any(|warning| warning.contains("english")));
}

pub(super) fn write(root: &std::path::Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn mod_replace_paths_region_names_empty_titles_and_recorded_modifier_labels() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported read-only installation");
    let fixture = Fixture::new();
    let stack = installed_content_core::test_support::Stack::new("regional-weather-mod");
    let base = stack.base();
    let overlay = stack.addon();
    let root = base.parent().unwrap();
    write(
        &base,
        "map/strategicregions/base.txt",
        "strategic_region={id=9000 name=BASE provinces={1}}",
    );
    write(
        &overlay,
        "map/strategicregions/mod.txt",
        "strategic_region={id=7001 name=MOD_TITLE provinces={1 2}} strategic_region={id=7002 name=EMPTY provinces={3}} strategic_region={id=7003 name=MISSING provinces={4}} strategic_region={id=7004 name=EMPTY provinces={}}",
    );
    write(
        &base,
        "localisation/base_l_english.yml",
        "\u{feff}l_english:\n MOD_TITLE:0 \"Base title\"\n",
    );
    write(
        &overlay,
        "localisation/replace/mod_l_english.yml",
        "\u{feff}l_english:\n MOD_TITLE:0 \"Overlay region\"\n EMPTY:0 \"\"\n weather_snow:0 \"Synthetic frozen condition\"\n MOD_AIR_TITLE:0 \"Synthetic $ARG$ modifier\"\n",
    );
    write(root, "user/settings.txt", "language=l_english");
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: base.clone(),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let mut frame = theatre_frame();
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                index: engine_executable::modifier_keys::REGION_AIR_DETECTION,
                name_key: b"MOD_AIR_TITLE".to_vec(),
                argument: b"detection".to_vec(),
                ..Default::default()
            },
        ]));
    publish(&fixture, &[frame.clone()]);
    let source = source(vec![frame]);
    source.set_test_content(
        options.options.clone(),
        Arc::new(installed_content_core::content::ResolvedContent {
            layers: installed_content_core::ResolvedLayers::from_folders(&[
                (&base, &[][..]),
                (&overlay, &["map/strategicregions"][..]),
            ])
            .unwrap(),
            stats: Default::default(),
            changed: Vec::new(),
            notes: Vec::new(),
        }),
    );
    let params = json!({"branchId":"1", "regions":["73"]});
    let reply = answer(&fixture, &source, &params, None, Some(&options)).unwrap();
    // Recorded 73 is not a region of the mod's map and 7004 has no provinces: neither is a choice.
    assert_eq!(
        reply["regions"],
        json!([
            {"id":"7001", "name":"Overlay region"},
            {"id":"7002", "name":""}, {"id":"7003", "name":"Region name unavailable"},
        ])
    );
    assert_eq!(reply["flags"][2]["name"], "Synthetic frozen condition");
    assert_eq!(
        reply["modifiers"][0]["name"],
        "Synthetic detection modifier"
    );
    assert!(
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("MISSING"))
    );
    assert!(
        reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|series| series["name"].as_str().unwrap().starts_with("73|"))
    );
    assert!(
        reply["theatres"]
            .as_array()
            .unwrap()
            .iter()
            .any(|theatre| theatre["id"].as_str() == Some(u64::MAX.to_string().as_str())
                && theatre["regions"] == json!(["73"]))
    );
    // Cached auxiliary reads do not decode a branch again on a repeated query.
    let (cached, refused_decode) = progressive::without_block_decodes(|| {
        answer(&fixture, &source, &params, None, Some(&options))
    });
    assert!(!refused_decode);
    assert_eq!(cached.unwrap(), reply);
}

#[test]
fn prepared_query_keeps_numbers_when_content_fails_and_never_reads_past_prefix() {
    let fixture = Fixture::new();
    let mut a = row(7001);
    a.air_attack = Recorded::Value(Some(-321_000));
    let mut b = row(7002);
    b.snow = 1;
    let frames = vec![
        frame(10, Some(vec![a, b])),
        frame(11, Some(vec![row(7001), row(7002)])),
    ];
    publish(&fixture, &frames);
    let source = source(frames);
    let reply = answer(
        &fixture,
        &source,
        &json!({"branchId":"1", "regions":["7001"]}),
        Some(10),
        None,
    )
    .unwrap();
    let series = reply["window"]["series"].as_array().unwrap();
    assert_eq!(series.len(), 14);
    assert!(
        series
            .iter()
            .all(|series| series["name"].as_str().unwrap().starts_with("7001|"))
    );
    let attack = series
        .iter()
        .find(|series| series["name"] == "7001|mod:air_attack")
        .unwrap();
    assert_eq!(attack["runs"], json!([[10, 10, "-321000", true]]));
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":10,"lastHour":10}])
    );
    assert!(reply["warnings"].as_array().unwrap().iter().any(|warning| {
        warning
            .as_str()
            .unwrap()
            .contains("game folders are not chosen")
    }));
    let empty = answer(
        &fixture,
        &source,
        &json!({"branchId":"1", "regions":[]}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(empty["window"]["series"], json!([]));
    assert_eq!(empty["regions"].as_array().unwrap().len(), 2);
}

#[test]
fn independent_witness_changes_end_and_restart_only_the_affected_series() {
    let fixture = Fixture::new();
    let mut refused = row(73);
    refused.air_detection = Recorded::Value(None);
    publish(
        &fixture,
        &[
            frame(0, Some(vec![row(73)])),
            frame(1, Some(vec![refused])),
            frame(2, Some(vec![row(73)])),
        ],
    );
    let flag = AtomicBool::new(false);
    let mut opened =
        dashboard_keyed::query::open::<block::Observer>(&fixture.store, &fixture.key, 1, &flag)
            .unwrap()
            .unwrap();
    let window = opened.window(0, 2, None, |_| true).unwrap();
    let runs = |name| {
        &window
            .series
            .iter()
            .find(|series| series.name == name)
            .unwrap()
            .runs
    };
    assert_eq!(runs("73|snow"), &[(0, 2, "0".into(), true)]);
    assert_eq!(runs("73|mod:air_attack"), &[(0, 2, "0".into(), true)]);
    assert_eq!(
        runs("73|mod:air_detection"),
        &[(0, 0, "0".into(), true), (2, 2, "0".into(), true)]
    );
}

#[test]
fn a_truncated_weather_table_does_not_certify_absence_or_zero() {
    let mut partial = frame(0, Some(vec![]));
    *partial.tables[TableId::WeatherRegions.index()] = TableView::Truncated {
        rows: TableRows::WeatherRegions(vec![row(73)]),
        missing: 1,
    };
    assert!(values(&partial).is_none());
}

#[test]
fn content_check_and_query_share_label_readers_with_independent_region_failure() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported read-only installation");
    let stack = installed_content_core::test_support::Stack::new("regional-weather-refusal");
    let root = stack.base();
    write(
        &root,
        "map/strategicregions/bad.txt",
        "strategic_region={id=7001 name=BAD provinces={0}}",
    );
    write(
        &root,
        "localisation/test_l_english.yml",
        "\u{feff}l_english:\n weather_snow:0 \"Synthetic snow label\"\n",
    );
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let warnings = content_warnings(&layers, "english").unwrap();
    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains("weather_snow"))
    );
}
