use super::*;
use crate::products::regional_weather::{
    RegionalWeatherResponse,
    tests::{answer, frame, row, source, write},
};
use dashboard_base::{
    prepared::{
        coverage::{Coverage, SCHEMA},
        prefix::Prefix,
        query::{frames, progressive},
        series::tests::Fixture,
    },
    products::ReplyWarnings,
    test_fixtures::{CAMPAIGN, continuation_records, records},
};
use dashboard_core::config::ContentOptions;
use installed_content_core::{content::ResolvedContent, test_support::Stack};
use replay_data::{
    rows::{
        ProvinceNameRow, ProvinceRow, ProvinceTemperature, Recorded, TableRows, TemperaturePass,
        WeatherProvinceRow,
    },
    temperature::{State, checksum},
};
use replay_frame::frame::TableView;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

fn bitmap() -> Vec<u8> {
    let mut bytes = vec![0_u8; 66];
    bytes[..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&66_u32.to_le_bytes());
    bytes[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bytes[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&4_i32.to_le_bytes());
    bytes[22..26].copy_from_slice(&1_i32.to_le_bytes());
    bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24_u16.to_le_bytes());
    for (pixel, id) in [1, 2, 3, 3].into_iter().enumerate() {
        bytes[54 + pixel * 3 + 2] = id;
    }
    bytes
}

fn content_stack() -> Stack {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported read-only installation");
    let stack = Stack::new("weather-province-drill");
    let base = stack.base();
    let overlay = stack.addon();
    write(
        &base,
        "map/default.map",
        "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp adjacencies=adjacencies.csv adjacency_rules=adjacency_rules.txt",
    );
    write(
        &base,
        "map/definition.csv",
        "0;0;0;0;land;false;synthetic;0\n1;1;0;0;land;false;synthetic;0\n2;2;0;0;land;false;synthetic;0\n3;3;0;0;land;false;synthetic;0\n",
    );
    std::fs::write(base.join("map/provinces.bmp"), bitmap()).unwrap();
    std::fs::write(base.join("map/terrain.bmp"), bitmap()).unwrap();
    write(
        &base,
        "map/adjacencies.csv",
        "header\n-1;-1;;-1;0;0;0;0;;\n",
    );
    write(&base, "map/adjacency_rules.txt", "");
    write(
        &base,
        "common/terrain/synthetic.txt",
        "categories={synthetic={movement_cost=1}}",
    );
    write(
        &base,
        "common/weather.txt",
        "settings={temperature={max_temperature_change=1} performance={provinces_per_update=4}}",
    );
    write(
        &base,
        "common/defines/mod.lua",
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NBuildings={MAX_BUILDING_LEVELS=15}}",
    );
    write(
        &base,
        "map/strategicregions/base.txt",
        "strategic_region={id=71 name=BASE_REGION provinces={1}} strategic_region={id=99 provinces={1 2 3}}",
    );
    write(
        &overlay,
        "map/strategicregions/mod.txt",
        "strategic_region={id=71 name=MOD_REGION provinces={2 3 2 99}} strategic_region={id=72 name=OTHER_REGION provinces={1}} strategic_region={id=73 provinces={}}",
    );
    write(
        &base,
        "localisation/base_l_english.yml",
        "\u{feff}l_english:\n MOD_PROVINCE_2:0 \"Base province\"\n",
    );
    write(
        &overlay,
        "localisation/replace/mod_l_english.yml",
        "\u{feff}l_english:\n MOD_PROVINCE_1:0 \"Overlay one\"\n MOD_PROVINCE_2:0 \"Overlay two\"\n MOD_PROVINCE_3:0 \"Overlay three\"\n MOD_REGION:0 \"Overlay region\"\n",
    );
    write(
        base.parent().unwrap(),
        "user/settings.txt",
        "language=l_english",
    );
    stack
}

fn bind(source: &dashboard_base::Source, stack: &Stack) -> ContentOptions {
    let base = stack.base();
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: base.clone(),
            user_dir: base.parent().unwrap().join("user"),
            cache_dir: base.parent().unwrap().join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    source.set_test_content(
        options.options.clone(),
        Arc::new(ResolvedContent {
            layers: stack.layers(&["map/strategicregions"]),
            stats: Default::default(),
            changed: Vec::new(),
        }),
    );
    options
}

fn weather(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour, Some(vec![row(71)]));
    let words: Vec<_> = [0, -100_000, 0, 300_000]
        .into_iter()
        .map(|temperature| State {
            temperature,
            staged: temperature,
            ..Default::default()
        })
        .collect();
    let pass = TemperaturePass {
        anchor: hour,
        cursor: 1,
        checksum: checksum(words.iter().map(|state| state.temperature)),
        ..Default::default()
    };
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(
            words
                .into_iter()
                .enumerate()
                .map(|(id, state)| WeatherProvinceRow {
                    id: id as u32,
                    snow: if id == 2 { 0 } else { -123_456 },
                    permanent_snow: 999_999,
                    display_snow: 244,
                    raw_mud: Recorded::Value(if id == 2 { 0 } else { 17 }),
                    display_mud: 255,
                    temperature: ProvinceTemperature::anchored(state),
                    temperature_pass: Recorded::Value((id == 0).then(|| pass.clone())),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=3)
            .map(|id| ProvinceRow {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ProvinceNames.index()] = TableView::Complete(TableRows::ProvinceNames(
        (1..=3)
            .map(|province| ProvinceNameRow {
                province,
                name: format!("MOD_PROVINCE_{province}").into_bytes(),
            })
            .collect(),
    ));
    frame.metrics.table_ns[TableId::WeatherProvinces.index()] = 1;
    frame
}

fn params(hour: i32) -> Value {
    json!({"branchId":"1", "view":"provinces", "region":"71", "hour":hour})
}

fn warning(reply: &Value, reason: &str) -> bool {
    reply["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains(reason))
}

#[test]
fn mod_overlay_membership_names_raw_scales_and_cold_prefix_are_exact() {
    let stack = content_stack();
    let source = source(vec![weather(10)]);
    let options = bind(&source, &stack);
    let fixture = Fixture::new();
    let reply = answer(&fixture, &source, &params(10), Some(9), Some(&options)).unwrap();
    let mut keys: Vec<_> = reply
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "hour",
            "provinces",
            "readHour",
            "region",
            "view",
            "warnings"
        ]
    );
    assert_eq!(reply["view"], "provinces");
    assert_eq!(reply["hour"], 10);
    assert_eq!(reply["region"], "71");
    assert_eq!(reply["readHour"], 10);
    assert_eq!(
        reply["provinces"],
        json!([
            {"id":"2", "name":"Overlay two", "snow":"0", "mud":false, "temperature":"0"},
            {"id":"3", "name":"Overlay three", "snow":"-123456", "mud":true, "temperature":"300000"}
        ])
    );
    assert!(!warning(&reply, "not shown"), "{reply}");
    // The duplicate and nonexistent authored ids do not become weather rows.
    // replace_path hides the base region, and region selection is not inverted.
    let mut request = params(10);
    request["region"] = json!("99");
    let absent = answer(&fixture, &source, &request, None, Some(&options));
    assert!(absent.is_ok());
    let absent = absent.unwrap();
    assert_eq!(absent["provinces"], json!([]));
    assert!(warning(&absent, "region 99 is absent"));
    request["region"] = json!("73");
    let empty = answer(&fixture, &source, &request, None, Some(&options)).unwrap();
    assert_eq!(empty["provinces"], json!([]));
    assert!(!warning(&empty, "roster not shown"));
}

#[test]
fn missing_raw_mud_and_bad_checksum_refuse_only_their_fields() {
    let stack = content_stack();
    let mut frame = weather(10);
    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!()
    };
    rows[3].raw_mud = Recorded::NotRecorded;
    let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass else {
        unreachable!()
    };
    pass.checksum ^= 1;
    let source = source(vec![weather(10)]);
    let options = bind(&source, &stack);
    let fixture = Fixture::new();
    let params = params(10);
    let prefix = Prefix {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 10,
            complete: true,
        },
        pending: vec![],
    };
    let cancelled = AtomicBool::new(false);
    let query = Query {
        source: &source,
        params: &params,
        content: Some(&options),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    let reply = at_frame(
        &query,
        &frame,
        &[2, 3],
        RegionalWeatherProvincesReply {
            hour: 10,
            read_hour: None,
            region: "71".into(),
            provinces: vec![],
            warnings: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        reply["provinces"][0],
        json!({"id":"2", "name":"Overlay two", "snow":"0", "mud":false, "temperature":null})
    );
    assert_eq!(
        reply["provinces"][1],
        json!({"id":"3", "name":"Overlay three", "snow":"-123456", "mud":null, "temperature":null})
    );
    assert!(warning(
        &reply,
        "2 weather province fields not shown: temperature:"
    ));
    assert!(warning(&reply, "checksum"));
    assert!(warning(
        &reply,
        "1 weather province fields not shown: raw mud:"
    ));
}

#[test]
fn temperature_content_failure_keeps_membership_snow_mud_and_names() {
    let stack = content_stack();
    // The mod can supply geometry and membership without supplying a usable
    // temperature graph. Membership does not demand this independent input.
    write(
        &stack.addon(),
        "map/default.map",
        "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp",
    );
    let source = source(vec![weather(10)]);
    let options = bind(&source, &stack);
    let reply = answer(&Fixture::new(), &source, &params(10), None, Some(&options)).unwrap();
    assert_eq!(reply["provinces"].as_array().unwrap().len(), 2);
    assert_eq!(reply["provinces"][0]["snow"], "0");
    assert_eq!(reply["provinces"][0]["mud"], false);
    assert_eq!(reply["provinces"][0]["name"], "Overlay two");
    assert!(reply["provinces"][0]["temperature"].is_null());
    assert!(warning(&reply, "Temperature unavailable"));
}

#[test]
fn missing_weather_row_and_unavailable_table_are_not_zero_or_empty_membership() {
    let stack = content_stack();
    let mut missing = weather(10);
    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *missing.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!()
    };
    rows.pop();
    let mut unavailable = weather(11);
    *unavailable.tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
        reason: "synthetic weather refusal".into(),
    };
    let source = source(vec![missing, unavailable]);
    let options = bind(&source, &stack);
    let fixture = Fixture::new();
    let missing = answer(&fixture, &source, &params(10), None, Some(&options)).unwrap();
    assert_eq!(missing["provinces"][0]["snow"], "0");
    assert!(missing["provinces"][1]["snow"].is_null());
    assert!(missing["provinces"][1]["mud"].is_null());
    assert!(missing["provinces"][0]["temperature"].is_null());
    assert!(warning(&missing, "province has no recorded weather row"));
    let unavailable = answer(&fixture, &source, &params(11), None, Some(&options)).unwrap();
    assert_eq!(unavailable["provinces"].as_array().unwrap().len(), 2);
    assert!(unavailable["readHour"].is_null());
    assert_eq!(unavailable["provinces"][0]["name"], "Overlay two");
    assert!(unavailable["provinces"][0]["snow"].is_null());
    assert!(unavailable["provinces"][0]["mud"].is_null());
    assert!(warning(&unavailable, "synthetic weather refusal"));
}

#[test]
fn reached_hour_reads_only_its_block_and_reuses_frame_cache_without_preparation() {
    let stack = content_stack();
    let replay =
        replay::container::Replay::from_records(CAMPAIGN.into(), &[records(1, vec![weather(10)])])
            .unwrap();
    let later = continuation_records(&replay, 1, vec![weather(100), weather(101)]);
    let replay = replay.append(&[later]).unwrap();
    assert_eq!(replay.index.len(), 2);
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "weather-drill-blocks".into(),
            revision: "1".into(),
        },
        "weather-drill-blocks.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let options = bind(&source, &stack);
    let fixture = Fixture::new();
    assert_eq!(frames::reads(&source), (0, 0));
    let (cold, blocked) = progressive::without_block_decodes(|| {
        answer(&fixture, &source, &params(100), Some(9), Some(&options))
    });
    assert!(blocked);
    assert_eq!(cold.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(frames::reads(&source), (0, 0));
    let reply = answer(&fixture, &source, &params(100), Some(9), Some(&options)).unwrap();
    assert_eq!(reply["provinces"][1]["temperature"], "300000");
    assert_eq!(frames::reads(&source), (0, 1));
    let (cached, blocked) = progressive::without_block_decodes(|| {
        answer(&fixture, &source, &params(100), Some(9), Some(&options))
    });
    assert!(!blocked);
    assert_eq!(cached.unwrap(), reply);
    assert_eq!(frames::reads(&source), (0, 1));
    let cache = source.state::<KeyedResults<catalogue::RecordedBinding, Membership>>();
    assert_eq!(
        cache
            .lock()
            .unwrap()
            .loads(&catalogue::recorded_binding(&options)),
        1
    );
    assert_eq!(source.state::<Temperatures>().lock().unwrap().0.len(), 1);
    // A gap is not replaced by the prior or following frame.
    let gap = answer(&fixture, &source, &params(50), None, Some(&options)).unwrap();
    assert!(gap["readHour"].is_null());
    assert_eq!(gap["provinces"].as_array().unwrap().len(), 2);
    assert!(gap["provinces"][0]["snow"].is_null());
    assert!(warning(&gap, "no usable recorded frame at hour 50"));
}

#[test]
fn validation_cancellation_and_retry() {
    let fixture = Fixture::new();
    let source = source(vec![weather(10)]);
    for bad in [
        json!({"branchId":"1", "view":"provinces", "region":"71"}),
        json!({"branchId":"1", "view":"provinces", "hour":10}),
        json!({"branchId":"1", "view":"provinces", "region":"071", "hour":10}),
        json!({"branchId":"1", "view":"provinces", "region":71, "hour":10}),
        params(9),
        params(11),
    ] {
        assert!(
            answer(&fixture, &source, &bad, None, None).is_err(),
            "{bad}"
        );
    }
    let prefix = Prefix {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 9,
            complete: false,
        },
        pending: vec![],
    };
    let params = params(10);
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
    assert_eq!(
        crate::products::regional_weather::query(&request)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(frames::reads(&source), (0, 0));
    cancelled.store(false, Ordering::Relaxed);
    let reply = crate::products::regional_weather::query(&request).unwrap();
    assert_eq!(reply["provinces"], json!([]));
    assert!(warning(
        &reply,
        "province membership needs the recorded content stack"
    ));
    let mut reply = RegionalWeatherResponse::Provinces(RegionalWeatherProvincesReply {
        hour: 10,
        read_hour: None,
        region: "71".into(),
        provinces: vec![],
        warnings: vec![],
    });
    reply.warnings_mut().push("test warning".into());
    assert_eq!(
        serde_json::to_value(reply).unwrap()["warnings"],
        json!(["test warning"])
    );
}

#[test]
fn successful_empty_name_and_refused_name_do_not_change_numeric_fields() {
    let stack = content_stack();
    write(
        &stack.addon(),
        "localisation/replace/mod_l_english.yml",
        "\u{feff}l_english:\n MOD_PROVINCE_2:0 \"Overlay two\"\n MOD_PROVINCE_3:0 \"\"\n",
    );
    let mut first = weather(10);
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *first.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows.pop();
    let source = source(vec![first, weather(11)]);
    let options = bind(&source, &stack);
    let fixture = Fixture::new();
    let missing = answer(&fixture, &source, &params(10), None, Some(&options)).unwrap();
    assert!(missing["provinces"][1]["name"].is_null());
    assert_eq!(missing["provinces"][1]["snow"], "-123456");
    assert_eq!(missing["provinces"][1]["mud"], true);
    assert_eq!(missing["provinces"][1]["temperature"], "300000");
    assert!(warning(
        &missing,
        "1 weather province fields not shown: name:"
    ));
    let empty = answer(&fixture, &source, &params(11), None, Some(&options)).unwrap();
    assert_eq!(empty["provinces"][1]["name"], "");
    assert_eq!(empty["provinces"][1]["temperature"], "300000");
}

#[test]
fn shared_name_fallback_survives_an_unavailable_primary_name_input() {
    let stack = content_stack();
    // The shared name ladder refuses duplicate region memberships, unlike the
    // weather roster's native-vector deduplication. Keep this fallback unique.
    write(
        &stack.addon(),
        "map/strategicregions/mod.txt",
        "strategic_region={id=71 name=MOD_REGION provinces={2 3}} strategic_region={id=72 name=OTHER_REGION provinces={1}}",
    );
    let mut frame = weather(10);
    *frame.tables[TableId::ProvinceNames.index()] = TableView::Unavailable {
        reason: "synthetic province-name refusal".into(),
    };
    let source = source(vec![frame]);
    let options = bind(&source, &stack);
    let reply = answer(&Fixture::new(), &source, &params(10), None, Some(&options)).unwrap();
    // The shared resolver proves a strategic-region fallback independently of
    // the unavailable province override; do not discard that known name.
    assert_eq!(reply["provinces"][0]["name"], "Overlay region", "{reply}");
    assert_eq!(reply["provinces"][1]["name"], "Overlay region", "{reply}");
    assert_eq!(reply["provinces"][0]["snow"], "0");
    assert_eq!(reply["provinces"][0]["mud"], false);
    assert_eq!(reply["provinces"][0]["temperature"], "0");
    let refusals: Vec<_> = reply["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|warning| warning.contains("weather province fields not shown: name:"))
        .collect();
    assert_eq!(refusals.len(), 2, "{reply}");
    assert!(refusals.iter().all(|warning| warning.starts_with("1 ")));
    assert!(warning(&reply, "synthetic province-name refusal"));
}

#[test]
fn carried_weather_read_is_dated_without_reading_a_previous_block() {
    let stack = content_stack();
    let first = weather(10);
    let mut held = first.clone();
    held.hour = 11;
    held.metrics.table_ns[TableId::WeatherProvinces.index()] = 0;
    let source = source(vec![first, held]);
    let options = bind(&source, &stack);
    let reply = answer(&Fixture::new(), &source, &params(11), None, Some(&options)).unwrap();
    assert_eq!(reply["readHour"], 10);
    assert_eq!(reply["provinces"][0]["snow"], "0");
    assert_eq!(reply["provinces"][0]["mud"], false);
    // The unchanged pass reaches hour 10, not 11. It must not prove a held
    // temperature merely because its snow/mud table has a certified read.
    assert!(reply["provinces"][0]["temperature"].is_null());
    assert!(warning(&reply, "frame is hour 11"));
    assert_eq!(frames::reads(&source), (0, 1));
}

#[test]
fn suspended_pass_reaches_requested_hour_and_backwards_queries_reset_the_memo() {
    let stack = content_stack();
    let first = weather(10);
    let mut later = first.clone();
    later.hour = 12;
    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *later.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!()
    };
    let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass else {
        unreachable!()
    };
    pass.hours = vec![None, None];
    let source = source(vec![first, later]);
    let options = bind(&source, &stack);
    let fixture = Fixture::new();
    let later = answer(&fixture, &source, &params(12), None, Some(&options)).unwrap();
    assert_eq!(later["provinces"][1]["temperature"], "300000");
    // Scrubbing backwards reuses this branch memo without fabricating extra
    // passes or retaining the later hour's state as the earlier value.
    let first = answer(&fixture, &source, &params(10), None, Some(&options)).unwrap();
    assert_eq!(first["provinces"][1]["temperature"], "300000");
    assert_eq!(source.state::<Temperatures>().lock().unwrap().0.len(), 1);
}

#[test]
fn nonanchor_temperature_is_reconstructed_not_read_from_the_anchor_word() {
    let stack = content_stack();
    write(
        &stack.addon(),
        "common/weather.txt",
        "settings={temperature={temperature_neighbor_smoothing=0.5} performance={provinces_per_update=4}}",
    );
    let layers = stack.layers(&["map/strategicregions"]);
    let (graph, geometry) = installed_content_core::province_graph::ProvinceGraph::read(&layers).unwrap();
    let content =
        installed_content_weather::weather::temperature::read_with(&layers, &graph, &geometry).unwrap();
    let first = weather(10);
    let words: Vec<_> = first
        .complete::<tables::WeatherProvinces>()
        .unwrap()
        .iter()
        .map(|row| {
            let replay_data::rows::TemperatureRecord::Anchored(state) =
                row.temperature.record().unwrap()
            else {
                unreachable!()
            };
            state
        })
        .collect();
    let mut simulation = replay_data::temperature::Simulation::new(10, 1, words.clone());
    simulation.advance(&content, Some(51)).unwrap();
    assert_ne!(simulation.states[2].temperature, words[2].temperature);
    let mut reached = first.clone();
    reached.hour = 11;
    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *reached.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!()
    };
    let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass else {
        unreachable!()
    };
    pass.hours = vec![Some(51)];
    pass.checksum = simulation.checksum();
    let source = source(vec![first, reached]);
    let options = bind(&source, &stack);
    let reply = answer(
        &Fixture::new(),
        &source,
        &params(11),
        Some(9),
        Some(&options),
    )
    .unwrap();
    for (at, id) in [2, 3].into_iter().enumerate() {
        assert_eq!(
            reply["provinces"][at]["temperature"],
            simulation.states[id].temperature.to_string()
        );
    }
}

#[test]
fn wide_signed_values_remain_decimal_text_without_display_conversion() {
    let stack = content_stack();
    let mut frame = weather(10);
    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!()
    };
    rows[2].snow = i64::MIN;
    rows[3].temperature = ProvinceTemperature::anchored(State {
        temperature: i64::MAX,
        staged: i64::MAX,
        ..Default::default()
    });
    let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass else {
        unreachable!()
    };
    pass.checksum = checksum([0, -100_000, 0, i64::MAX]);
    let source = source(vec![frame]);
    let options = bind(&source, &stack);
    let reply = answer(&Fixture::new(), &source, &params(10), None, Some(&options)).unwrap();
    assert_eq!(reply["provinces"][0]["snow"], i64::MIN.to_string());
    assert_eq!(reply["provinces"][1]["temperature"], i64::MAX.to_string());
    assert_eq!(reply["provinces"][0]["mud"], false);
}
