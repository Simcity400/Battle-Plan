use super::*;
use crate::products::regional_weather::{
    TEMPERATURE_SCALE, block,
    tests::{answer, frame, reply_runs, row, source, write},
};
use dashboard_base::{prepared::series::tests::Fixture, products::country_groups::HourSpan};
use dashboard_keyed::{
    Observer as _, Scan,
    native::testing::{Binding, operate_bound, publish_bound},
    query::KeyedWindow,
};
use replay::columnar_frames::synthetic::Synthetic;
use replay_data::{
    format::FormatVersion,
    rows::{
        ProvinceTemperature, Recorded, TableId, TableRows, TemperaturePass, WeatherProvinceRow,
        WeatherRegionRow,
    },
    temperature::{Province, Settings, Simulation},
};
use replay_frame::frame::TableView;
use serde_json::json;

fn bind_observer(
    observer: &mut block::Observer,
    source: &Source,
    options: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let resolved = resolve(source, options, &mut || {
        dashboard_text::metadata::check_cancel(cancelled)
    });
    match resolved {
        Ok(content) => dashboard_keyed::Observer::bind(observer, &Arc::new(content)),
        Err(error) => {
            observer.retire();
            Err(error)
        }
    }
}

fn bound(members: &[(u32, Vec<u32>)], count: usize) -> Bound {
    let members: BTreeMap<_, _> = members.iter().cloned().collect();
    Bound {
        inverse: Mutex::default(),
        members,
        pass: Content {
            provinces: (0..count)
                .map(|id| Province {
                    active: id > 0,
                    neighbours: vec![id as u32],
                    region: None,
                    terrain: Ok(None),
                })
                .collect(),
            settings: Settings {
                provinces_per_update: count as i32,
                ..Default::default()
            },
            ..Default::default()
        },
        warnings: vec![],
        plan: OnceLock::new(),
        layout: OnceLock::new(),
    }
}

fn states(temperatures: &[i64]) -> Vec<State> {
    temperatures
        .iter()
        .map(|&temperature| State {
            temperature,
            staged: temperature,
            ..Default::default()
        })
        .collect()
}

fn temperatures(
    branch: u64,
    anchor: i32,
    hour: i32,
    words: &[State],
    seeds: Vec<Option<u32>>,
    content: &Content,
) -> DecodedFrame {
    let mut simulation = Simulation::new(anchor, 1, words.to_vec());
    for &seed in &seeds {
        simulation.advance(content, seed).unwrap();
    }
    assert_eq!(simulation.hour, hour);
    let pass = TemperaturePass {
        anchor,
        cursor: 1,
        hours: seeds,
        witness: replay_data::rows::TemperatureWitness::new(simulation.witness()),
        ..Default::default()
    };
    let rows = words
        .iter()
        .enumerate()
        .map(|(id, &state)| WeatherProvinceRow {
            id: id as u32,
            temperature: ProvinceTemperature::anchored(state),
            temperature_pass: Recorded::Value((id == 0).then(|| pass.clone())),
            ..Default::default()
        })
        .collect();
    let mut frame = frame(hour, Some(vec![row(71), row(72)]));
    frame.branch = branch;
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(rows));
    frame
}

fn observe(observation: &mut Observation, frame: &DecodedFrame) -> BTreeMap<String, i128> {
    let mut values = Values::default();
    observation.observe(frame, &mut values).unwrap();
    values
        .iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn observation(content: Arc<Bound>) -> Observation {
    Observation {
        status: Some("available".into()),
        inverse: Some(
            content
                .lease_inverse(&mut || Ok(()))
                .unwrap(),
        ),
        content: Some(content),
        ..Default::default()
    }
}

#[test]
fn all_provinces_known_average_is_signed_wide_fixed5_and_never_an_empty_zero() {
    assert_eq!(TEMPERATURE_SCALE, 100_000);
    let words = states(&[0, -1_000_000, 2_000_000]);
    assert_eq!(average(&[1, 2], &words), Some(500_000));
    assert_eq!(average(&[0], &words), Some(0));
    assert_eq!(average(&[], &words), None);
    assert_eq!(average(&[1, 3], &words), None);
    assert_eq!(
        average(&[1, 2], &states(&[0, i64::MAX, i64::MAX])),
        Some(i128::from(i64::MAX))
    );
    assert_eq!(
        average(&[1, 2], &states(&[0, i64::MIN, i64::MIN])),
        Some(i128::from(i64::MIN))
    );
    assert_eq!(average(&[1, 2], &states(&[0, -2, -1])), Some(-1));
}

#[test]
fn mod_overrides_and_replace_path_supply_native_membership_without_backlink_inversion() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stack = installed_content_core::test_support::Stack::new("weather-temperature-membership");
    let base = stack.base();
    let overlay = stack.addon();
    write(
        &base,
        "map/strategicregions/a.txt",
        "strategic_region={id=71 provinces={1 2}}",
    );
    write(
        &base,
        "map/strategicregions/hidden.txt",
        "strategic_region={id=99 provinces={3}}",
    );
    write(
        &overlay,
        "map/strategicregions/a.txt",
        "strategic_region={id=71 provinces={2 3 2 99}} strategic_region={id=72 provinces={3}}",
    );
    let geometry = ProvinceGeometry {
        centroids: (1..=3).map(|id| (id, [0, 0])).collect(),
        ..Default::default()
    };
    let layers = stack.layers(&[]);
    let regions = regions::read(&layers).unwrap();
    let members = membership(&regions, &geometry);
    assert_eq!(members[&71], [2, 3]);
    assert_eq!(members[&72], [3]); // Both native vectors retain the shared province.
    assert!(members.contains_key(&99));
    let layers = stack.layers(&["map/strategicregions"]);
    let regions = regions::read(&layers).unwrap();
    let members = membership(&regions, &geometry);
    assert_eq!(members.len(), 2);
    assert_eq!(
        average(&members[&71], &states(&[0, 900_000, 100_000, 300_000])),
        Some(200_000)
    );
    assert!(!members.contains_key(&99));
}

#[test]
fn memo_reconstructs_committed_words_not_anchor_and_checks_every_hour() {
    let mut content = bound(&[(71, vec![1, 2])], 3);
    content.pass.settings.smoothing = 50_000;
    content.pass.provinces[1].neighbours = vec![2];
    content.pass.provinces[2].neighbours = vec![1];
    let content = Arc::new(content);
    let mut words = states(&[0, 100_000, 500_000]);
    words[1].staged = 300_000;
    words[2].staged = 900_000;
    let mut observation = observation(Arc::clone(&content));
    let first = temperatures(1, 10, 11, &words, vec![Some(7)], &content.pass);
    // Staged values with no period still smooth against committed neighbours.
    assert_eq!(observe(&mut observation, &first)["71|temp"], 450_000);
    let second = temperatures(1, 10, 12, &words, vec![Some(7), None], &content.pass);
    assert_eq!(observe(&mut observation, &second)["71|temp"], 450_000);
    assert_eq!(observation.memo.simulation().unwrap().hour, 12);
    let mut bad = temperatures(1, 10, 13, &words, vec![Some(7), None, None], &content.pass);
    if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *bad.tables[TableId::WeatherProvinces.index()]
        && let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass
    {
        pass.witness = replay_data::rows::TemperatureWitness::new(pass.witness.get().unwrap() ^ 1);
    }
    assert!(observe(&mut observation, &bad).is_empty());
    assert!(
        observation
            .warnings()
            .iter()
            .any(|warning| warning.contains("differ from the recorded witness"))
    );
    // A valid later hour recovers without reusing an unchecked result.
    let next = temperatures(
        1,
        10,
        14,
        &words,
        vec![Some(7), None, None, None],
        &content.pass,
    );
    assert_eq!(observe(&mut observation, &next)["71|temp"], 450_000);
}

#[test]
fn one_unknown_member_or_missing_pass_never_yields_a_partial_average() {
    let content = Arc::new(bound(&[(71, vec![1, 2])], 3));
    let mut observation = observation(Arc::clone(&content));
    let good = temperatures(
        1,
        10,
        10,
        &states(&[0, 100_000, 900_000]),
        vec![],
        &content.pass,
    );
    assert_eq!(observe(&mut observation, &good)["71|temp"], 500_000);
    for missing in [1, 2] {
        let mut unknown = good.clone();
        if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
            &mut *unknown.tables[TableId::WeatherProvinces.index()]
        {
            rows[missing].temperature = ProvinceTemperature::NOT_RECORDED;
        }
        assert!(observe(&mut observation, &unknown).is_empty());
    }
    let mut no_pass = good;
    if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *no_pass.tables[TableId::WeatherProvinces.index()]
    {
        rows[0].temperature_pass = Recorded::Value(None);
    }
    assert!(observe(&mut observation, &no_pass).is_empty());
    let warnings = observation.warnings();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("temperature not recorded"))
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("pass whole"))
    );
}

#[test]
fn interleaved_branches_keep_independent_memos_and_never_step_held_hours_by_date() {
    let content = Arc::new(bound(&[(71, vec![1, 2])], 3));
    let mut a = observation(Arc::clone(&content));
    let mut b = observation(Arc::clone(&content));
    let wa = states(&[0, -100_000, 300_000]);
    let wb = states(&[0, 900_000, 1_100_000]);
    assert_eq!(
        observe(
            &mut a,
            &temperatures(1, 10, 11, &wa, vec![None], &content.pass)
        )["71|temp"],
        100_000
    );
    assert_eq!(
        observe(
            &mut b,
            &temperatures(2, 10, 11, &wb, vec![None], &content.pass)
        )["71|temp"],
        1_000_000
    );
    assert_eq!(
        observe(
            &mut a,
            &temperatures(1, 10, 12, &wa, vec![None, None], &content.pass)
        )["71|temp"],
        100_000
    );
    assert_eq!(b.memo.simulation().unwrap().hour, 11);
    assert_eq!(a.memo.simulation().unwrap().hour, 12);
    // Even an accidentally reused observer resets its memo on a branch switch.
    assert_eq!(
        observe(
            &mut a,
            &temperatures(2, 10, 12, &wb, vec![None, None], &content.pass)
        )["71|temp"],
        1_000_000
    );
}

fn install_cached(source: &Source, content: Bound) -> ContentOptions {
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: "synthetic-temperature-game".into(),
            user_dir: "synthetic-temperature-user".into(),
            cache_dir: "synthetic-temperature-cache".into(),
        },
        allow_unverified: false,
        presentation: Default::default(),
    };
    source
        .state::<catalogue::KeyedResults<catalogue::RecordedBinding, Bound>>()
        .lock()
        .unwrap()
        .get_or_init(&catalogue::recorded_binding(&options), |_| Ok(content))
        .unwrap();
    options
}

#[test]
fn prepared_temperature_runs_query_selection_prefix_daily_and_rebound_continuation() {
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let content = bound(&[(71, vec![1, 2]), (72, vec![2])], 3);
    let words = states(&[0, -100_000, 300_000]);
    let frames: Vec<_> = (22..=25)
        .map(|hour| {
            temperatures(
                1,
                22,
                hour,
                &words,
                vec![None; (hour - 22) as usize],
                &content.pass,
            )
        })
        .collect();
    let source = source(frames.clone());
    let options = install_cached(&source, content);
    let mut scan = Scan::<block::Observer>::default();
    bind_observer(&mut scan.observer, &source, Some(&options), &cancelled).unwrap();
    for frames in [&frames[..2], &frames[2..]] {
        let mut writer = fixture.writer();
        for frame in frames {
            scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
        }
        scan.flush(&mut writer, 1, &cancelled).unwrap();
        writer.commit().unwrap();
        scan = dashboard_base::prepared::series::read_catalogue::<Scan<block::Observer>>(
            &fixture.store,
            &fixture.key,
            "regional-weather",
            1,
        )
        .unwrap()
        .unwrap()
        .1;
        // Runtime memo/content are absent after restore; rebind before folding.
        bind_observer(&mut scan.observer, &source, Some(&options), &cancelled).unwrap();
    }
    let reply = answer(
        &fixture,
        &source,
        &json!({"branchId":"1", "regions":["71"], "show":"temperature"}),
        Some(24),
        None,
    )
    .unwrap();
    assert_eq!(reply["temperature"], "available");
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 1);
    assert_eq!(
        reply_runs(&reply, "71|temp"),
        json!([[22, 24, "100000", true]])
    );
    assert!(
        !reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("modifier histories"))
    );
    let daily = answer(
        &fixture,
        &source,
        &json!({"branchId":"1", "regions":["72"], "show":"temperature", "daily":true}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        reply_runs(&daily, "72|temp"),
        json!([[22, 25, "300000", true]])
    );
    assert_eq!(scan.observer.temperature_status(), "available");
}

#[test]
fn province_refusals_keep_flags_and_do_not_carry_temperature_through_an_unknown_hour() {
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let content = bound(&[(71, vec![1, 2])], 3);
    let mut frames: Vec<_> = (10..=12)
        .map(|hour| {
            temperatures(
                1,
                10,
                hour,
                &states(&[0, 100_000, 300_000]),
                vec![None; (hour - 10) as usize],
                &content.pass,
            )
        })
        .collect();
    *frames[1].tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
        reason: "synthetic province refusal".into(),
    };
    let source = source(frames.clone());
    let options = install_cached(&source, content);
    let mut scan = Scan::<block::Observer>::default();
    bind_observer(&mut scan.observer, &source, Some(&options), &cancelled).unwrap();
    let mut writer = fixture.writer();
    for frame in &frames {
        scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
    }
    scan.flush(&mut writer, 1, &cancelled).unwrap();
    writer.commit().unwrap();
    let reply = answer(&fixture, &source, &json!({"branchId":"1"}), None, None).unwrap();
    assert_eq!(reply_runs(&reply, "71|snow"), json!([[10, 12, "0", true]]));
    assert_eq!(
        reply_runs(&reply, "71|temp"),
        json!([[10, 10, "200000", true], [12, 12, "200000", true]])
    );
    let daily = answer(
        &fixture,
        &source,
        &json!({"branchId":"1", "show":"temperature", "daily":true}),
        Some(11),
        None,
    )
    .unwrap();
    assert_eq!(reply_runs(&daily, "71|temp"), json!([]));
    assert!(!daily["window"]["known"].as_array().unwrap().is_empty());
    assert!(reply["warnings"].as_array().unwrap().iter().any(|warning| {
        warning
            .as_str()
            .unwrap()
            .contains("synthetic province refusal")
    }));
}

#[test]
fn content_binding_failure_is_precise_independent_retryable_on_new_binding_and_cached() {
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let mut observer = block::Observer::default();
    bind_observer(&mut observer, &source, None, &AtomicBool::new(false)).unwrap();
    assert!(
        observer
            .temperature_status()
            .contains("province geometry, adjacency, terrain, weather settings")
    );
    let mut values = Values::default();
    assert!(
        observer
            .observe(
                FormatVersion::CURRENT,
                &frame(10, Some(vec![row(71)])),
                &mut values
            )
            .unwrap()
    );
    assert_eq!(values.len(), 14);
    let options = install_cached(&source, bound(&[(71, vec![1])], 2));
    bind_observer(
        &mut observer,
        &source,
        Some(&options),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(observer.temperature_status(), "available");
    bind_observer(
        &mut observer,
        &source,
        Some(&options),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        bind_observer(&mut observer, &source, Some(&options), &cancelled)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn temperature_fields_watch_pass_and_every_anchor_word() {
    let fields = block::Observer::fields().unwrap();
    let provinces = fields
        .iter()
        .find(|fields| fields.table == TableId::WeatherProvinces)
        .unwrap();
    assert!(provinces.read);
    for field in [4, 10, 11, 12, 13] {
        assert_ne!(provinces.fields & (1 << field), 0);
    }
    assert_eq!(
        block::Observer::TABLES,
        &[TableId::WeatherRegions, TableId::WeatherProvinces]
    );
}

fn bmp() -> Vec<u8> {
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

#[test]
fn bind_reads_recorded_layered_temperature_content_once_and_refuses_unverified_source() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stack = installed_content_core::test_support::Stack::new("weather-temperature-content-binding");
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
    std::fs::write(base.join("map/provinces.bmp"), bmp()).unwrap();
    std::fs::write(base.join("map/terrain.bmp"), bmp()).unwrap();
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
        "map/strategicregions/base.txt",
        "strategic_region={id=99 provinces={1 2 3}}",
    );
    write(
        &overlay,
        "map/strategicregions/mod.txt",
        "strategic_region={id=71 provinces={2 3 2}} strategic_region={id=72 provinces={1}}",
    );
    let layers = Arc::new(installed_content_core::content::ResolvedContent {
        layers: stack.layers(&["map/strategicregions"]),
        stats: Default::default(),
        changed: Vec::new(),
        notes: Vec::new(),
    });
    let mut records =
        dashboard_base::test_fixtures::records(1, vec![frame(10, Some(vec![row(71)]))]);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        panic!("fixture session")
    };
    session.identity.verification = replay_data::identity::Verification::Unchecked;
    let replay = replay::campaign::Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[records],
    )
    .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "weather-unverified-binding".into(),
            revision: "1".into(),
        },
        "synthetic-unverified.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: base.clone(),
            user_dir: base.parent().unwrap().join("user"),
            cache_dir: base.parent().unwrap().join("cache"),
        },
        allow_unverified: false,
        presentation: Default::default(),
    };
    source.set_test_content(options.options.clone(), layers);
    let cancelled = AtomicBool::new(false);
    let mut a = Observation::default();
    a.bind_resolved(
        &resolve(&source, Some(&options), &mut || {
            dashboard_text::metadata::check_cancel(&cancelled)
        })
        .unwrap(),
    )
    .unwrap();
    assert!(a.status().contains("unverified content"), "{}", a.status());
    options.allow_unverified = true;
    a.bind_resolved(
        &resolve(&source, Some(&options), &mut || {
            dashboard_text::metadata::check_cancel(&cancelled)
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(a.status(), "available", "{:?}", a.warnings());
    let content = a.content.as_ref().unwrap();
    assert_eq!(
        content.members,
        BTreeMap::from([(71, vec![2, 3]), (72, vec![1])])
    );
    assert_eq!(content.pass.provinces[2].region, Some(71));
    assert_eq!(content.pass.settings.provinces_per_update, 4);
    let frame = temperatures(
        1,
        10,
        10,
        &states(&[0, -100_000, 100_000, 300_000]),
        vec![],
        &content.pass,
    );
    assert_eq!(
        observe(&mut a, &frame),
        BTreeMap::from([("71|temp".into(), 200_000), ("72|temp".into(), -100_000)])
    );
    let mut b = Observation::default();
    b.bind_resolved(
        &resolve(&source, Some(&options), &mut || {
            dashboard_text::metadata::check_cancel(&cancelled)
        })
        .unwrap(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        a.content.as_ref().unwrap(),
        b.content.as_ref().unwrap()
    ));
    assert!(b.memo.simulation().is_none());
    assert!(
        a.warnings()
            .iter()
            .any(|warning| warning.contains("UNVERIFIED CONTENT"))
    );
}

#[test]
fn unchanged_pass_at_a_new_hour_is_rechecked_not_carried_as_proven_temperature() {
    let content = bound(&[(71, vec![1, 2])], 3);
    let first = temperatures(
        1,
        10,
        10,
        &states(&[0, 100_000, 300_000]),
        vec![],
        &content.pass,
    );
    let mut stale = first.clone();
    stale.hour = 11;
    let source = source(vec![first.clone(), stale.clone()]);
    let options = install_cached(&source, content);
    let mut scan = Scan::<block::Observer>::default();
    bind_observer(
        &mut scan.observer,
        &source,
        Some(&options),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for frame in [&first, &stale] {
        let facts = changes.observe(frame, FormatVersion::CURRENT);
        scan.prepare_changed(FormatVersion::CURRENT, frame, &facts)
            .unwrap();
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    let reply = answer(&fixture, &source, &json!({"branchId":"1"}), None, None).unwrap();
    assert_eq!(
        reply_runs(&reply, "71|temp"),
        json!([[10, 10, "200000", true]])
    );
    assert_eq!(reply_runs(&reply, "71|snow"), json!([[10, 11, "0", true]]));
    assert!(
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("frame is hour 11"))
    );
}

/// Hour `hour` of branch 1, both its tables read: the regions, and the provinces' words
/// anchored at `anchor` with no pass moving them since, or the provinces refused.
fn weather_hour(
    hour: i32,
    regions: Vec<WeatherRegionRow>,
    provinces: Option<(i32, &[i64])>,
    content: &Content,
) -> DecodedFrame {
    let mut weather = match provinces {
        Some((anchor, words)) => temperatures(
            1,
            anchor,
            hour,
            &states(words),
            vec![None; (hour - anchor) as usize],
            content,
        ),
        None => {
            let mut refused = frame(hour, None);
            *refused.tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
                reason: "synthetic bound-temperature refusal".into(),
            };
            refused
        }
    };
    *weather.tables[TableId::WeatherRegions.index()] =
        TableView::Complete(TableRows::WeatherRegions(regions));
    for table in block::Observer::TABLES {
        weather.metrics.table_ns[table.index()] = 1;
    }
    weather
}

/// From 10: the provinces refused at 11 and anchored anew at 12; then only the pass moving, or
/// one province's words (14), while a region's flags move (13, 17), one appears (14) and one
/// ends (15).
fn bound_weather(content: &Content) -> Vec<DecodedFrame> {
    let rain = |id, light_rain, heavy_rain| WeatherRegionRow {
        light_rain,
        heavy_rain,
        ..row(id)
    };
    let first: &[i64] = &[0, -100_000, 300_000];
    let anchored: &[i64] = &[0, -300_000, 100_000];
    let warmed: &[i64] = &[0, 100_000, 100_000];
    vec![
        weather_hour(10, vec![row(71), row(72)], Some((10, first)), content),
        weather_hour(11, vec![row(71), row(72)], None, content),
        weather_hour(12, vec![row(71), row(72)], Some((12, anchored)), content),
        weather_hour(13, vec![rain(71, 3, 0), row(72)], Some((12, anchored)), content),
        weather_hour(14, vec![rain(71, 3, 0), row(72), row(73)], Some((14, warmed)), content),
        weather_hour(15, vec![rain(71, 3, 0), row(73)], Some((14, warmed)), content),
        weather_hour(16, vec![rain(71, 3, 0), row(73)], Some((14, warmed)), content),
        weather_hour(17, vec![rain(71, 3, 2), row(73)], Some((14, warmed)), content),
    ]
}

fn walked_weather(walk: &mut Synthetic, frame: &DecodedFrame) {
    dashboard_keyed::native::testing::write::<tables::WeatherRegions>(walk, frame);
    dashboard_keyed::native::testing::write::<tables::WeatherProvinces>(walk, frame);
}

/// The values `window` holds at `hour`, by name.
fn held_at(window: &KeyedWindow, hour: i32) -> BTreeMap<String, i128> {
    window
        .series
        .iter()
        .filter_map(|series| {
            let (_, _, value, _) = series
                .runs
                .iter()
                .find(|(first, last, _, _)| (*first..=*last).contains(&hour))?;
            Some((series.name.clone(), value.parse().unwrap()))
        })
        .collect()
}

/// A bound observer's native publication, its hours kept as the provinces and regions they
/// changed, holds at every hour what observing each frame whole gives, literal temperatures
/// included, and at every cut the same observer, its refusal and content warnings and its
/// status, whatever segments start, wherever it is cut and after a resume at the refused hour.
#[test]
fn native_bound_publication_holds_whole_observation_through_refusal_and_resume() {
    let mut content = bound(&[(71, vec![1, 2]), (72, vec![2])], 3);
    content.warnings = vec!["synthetic bound-content warning".into()];
    let frames = bound_weather(&content.pass);
    let source = source(frames.clone());
    let options = install_cached(&source, content);
    let refusal = "1 weather temperature observations not shown: hour 11 WeatherProvinces is unavailable: synthetic bound-temperature refusal";
    let mut whole = block::Observer::default();
    whole
        .bind(&source, Some(&options), &AtomicBool::new(false))
        .unwrap();
    let mut expected = BTreeMap::new();
    for frame in &frames {
        let hour = frame.hour;
        let mut values = Values::default();
        assert!(
            whole
                .observe(FormatVersion::CURRENT, frame, &mut values)
                .unwrap()
        );
        let values: BTreeMap<String, i128> = values
            .iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        let literal: BTreeMap<&str, i128> = match hour {
            10 => BTreeMap::from([("71|temp", 100_000), ("72|temp", 300_000)]),
            11 => BTreeMap::new(),
            12 | 13 => BTreeMap::from([("71|temp", -100_000), ("72|temp", 100_000)]),
            _ => BTreeMap::from([("71|temp", 100_000), ("72|temp", 100_000)]),
        };
        assert_eq!(
            values
                .iter()
                .filter(|(name, _)| name.ends_with("|temp"))
                .map(|(name, &value)| (name.as_str(), value))
                .collect::<BTreeMap<_, _>>(),
            literal,
            "hour {hour}"
        );
        let mut warnings = vec!["synthetic bound-content warning"];
        if hour >= 11 {
            warnings.push(refusal);
        }
        assert_eq!(whole.temperature_warnings(), warnings, "hour {hour}");
        assert_eq!(whole.temperature_status(), "available");
        expected.insert(hour, (values, serde_json::to_value(&whole).unwrap()));
    }
    let binding = Binding::<block::Observer>::new(&source, Some(&options)).unwrap();
    let first = frames[0].hour;
    for (starts, cuts, resume) in [
        (&[][..], &[17][..], None),
        // A resumed emit's next hour starts a segment, as the driver resumes.
        (&[2, 6][..], &[10, 11, 14, 17][..], Some(11)),
        (&[6][..], &[11, 15, 17][..], Some(15)),
        (
            &[1, 2, 3, 4, 5, 6, 7][..],
            &[10, 11, 12, 13, 14, 15, 16, 17][..],
            Some(11),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let hours =
            operate_bound::<block::Observer>(&frames, starts, walked_weather, Some(&binding));
        let published = publish_bound(hours, 1, cuts, resume, Some(&binding));
        for (&cut, published) in cuts.iter().zip(&published) {
            let (scan, window) = published.read::<block::Observer>(first, cut, None);
            assert_eq!(
                window.known,
                [HourSpan {
                    first_hour: first,
                    last_hour: cut
                }],
                "{label}: known through {cut}"
            );
            assert_eq!(
                window.reads,
                (first..=cut).map(|hour| (hour, hour, hour)).collect::<Vec<_>>(),
                "{label}: reads through {cut}"
            );
            for hour in first..=cut {
                assert_eq!(
                    held_at(&window, hour),
                    expected[&hour].0,
                    "{label}: hour {hour} through {cut}"
                );
            }
            // The reply's temperature status and warnings are the catalogued observer's.
            let observer = &scan.observer;
            assert_eq!(
                serde_json::to_value(observer).unwrap(),
                expected[&cut].1,
                "{label}: the observer at {cut}"
            );
            assert_eq!(observer.temperature_status(), "available", "{label}: {cut}");
            let mut warnings = vec!["synthetic bound-content warning"];
            if cut >= 11 {
                warnings.push(refusal);
            }
            assert_eq!(observer.temperature_warnings(), warnings, "{label}: {cut}");
        }
    }
}

#[test]
fn incremental_region_sums_keep_duplicate_and_overlapping_membership_exact() {
    let mut content = bound(
        &[
            (71, vec![1, 2, 1]),
            (72, vec![2, 3]),
            (73, vec![]),
            (74, vec![9]),
        ],
        4,
    );
    content.pass.settings.provinces_per_update = 2;
    content.pass.settings.smoothing = 50_000;
    content.pass.provinces[1].neighbours = vec![2];
    content.pass.provinces[2].neighbours = vec![3];
    content.pass.provinces[3].neighbours = vec![1];
    let content = Arc::new(content);
    let words = states(&[0, 100_001, -500_000, 900_000]);
    let seeds: Vec<_> = (0..12).map(|at| (at % 4 != 2).then_some(at + 7)).collect();
    let mut observation = observation(Arc::clone(&content));
    let mut reference = Simulation::new(10, 1, words.clone());
    let mut previous: Option<DecodedFrame> = None;
    for step in 0..=12 {
        if step > 0 {
            reference.advance(&content.pass, seeds[step - 1]).unwrap();
        }
        let frame = temperatures(
            1,
            10,
            10 + step as i32,
            &words,
            seeds[..step].to_vec(),
            &content.pass,
        );
        let rows = frame.complete::<tables::WeatherProvinces>().unwrap();
        let keys: Vec<_> = rows
            .iter()
            .enumerate()
            .filter_map(|(at, row)| {
                (previous
                    .as_ref()
                    .and_then(|frame| frame.complete::<tables::WeatherProvinces>().ok())
                    .and_then(|rows| rows.get(at))
                    != Some(row))
                .then_some(at as u128)
            })
            .collect();
        observation
            .refresh(
                &frame,
                previous.as_ref().map(|_| keys.as_slice()),
                |_, _| {},
            )
            .unwrap();
        for (&id, members) in &content.members {
            assert_eq!(
                observation.aggregates.as_ref().unwrap().regions[&id].mean,
                average(members, &reference.states),
                "region {id}, step {step}"
            );
        }
        assert_eq!(observation.aggregates.as_ref().unwrap().missing, 2);
        if step > 0 {
            assert!(observation.memo.changes().is_some());
        }
        previous = Some(frame);
    }
    assert_eq!(
        content
            .inverse
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .regions(1)
            .collect::<Vec<_>>(),
        [71, 71]
    );
    assert_eq!(
        content
            .inverse
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .regions(2)
            .collect::<Vec<_>>(),
        [71, 72]
    );
    assert_eq!(observation.empty_regions, BTreeSet::from([73]));
    assert_eq!(observation.unavailable[&9].first_hour, 10);
    assert_eq!(observation.unavailable[&9].count, 13);
    assert!(observation.refused.is_empty());
}

fn missing_reason_publication(
    content: Bound,
    frames: &[DecodedFrame],
    warnings: &[Vec<&str>],
) -> Vec<BTreeMap<String, i128>> {
    let mut frames = frames.to_vec();
    for frame in &mut frames {
        for table in block::Observer::TABLES {
            frame.metrics.table_ns[table.index()] = 1;
        }
    }
    let source = source(frames.clone());
    let options = install_cached(&source, content);
    let mut whole = block::Observer::default();
    whole
        .bind(&source, Some(&options), &AtomicBool::new(false))
        .unwrap();
    let mut expected = Vec::new();
    let mut observers = Vec::new();
    for (at, frame) in frames.iter().enumerate() {
        let mut values = Values::default();
        whole
            .observe(FormatVersion::CURRENT, frame, &mut values)
            .unwrap();
        expected.push(
            values
                .iter()
                .filter(|(name, _)| name.ends_with("|temp"))
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        );
        assert_eq!(whole.temperature_warnings(), warnings[at]);
        observers.push(serde_json::to_value(&whole).unwrap());
    }
    let binding = Binding::<block::Observer>::new(&source, Some(&options)).unwrap();
    let first = frames[0].hour;
    let cuts: Vec<_> = frames.iter().map(|frame| frame.hour).collect();
    for (starts, resume) in [(&[][..], None), (&[1, 3][..], Some(cuts[2]))] {
        let hours =
            operate_bound::<block::Observer>(&frames, starts, walked_weather, Some(&binding));
        let published = publish_bound(hours, 1, &cuts, resume, Some(&binding));
        for (at, published) in published.iter().enumerate() {
            let (scan, window) = published.read::<block::Observer>(first, cuts[at], None);
            assert_eq!(scan.observer.temperature_warnings(), warnings[at]);
            assert_eq!(serde_json::to_value(&scan.observer).unwrap(), observers[at]);
            for hour in 0..=at {
                assert_eq!(
                    held_at(&window, cuts[hour])
                        .into_iter()
                        .filter(|(name, _)| name.ends_with("|temp"))
                        .collect::<BTreeMap<_, _>>(),
                    expected[hour]
                );
            }
        }
    }
    expected
}

#[test]
fn empty_regions_are_named_once_through_hours_segments_and_resume() {
    let content = bound(&[(71, vec![1]), (73, vec![]), (74, vec![])], 2);
    let frames: Vec<_> = (10..=13)
        .map(|hour| {
            temperatures(
                1,
                10,
                hour,
                &states(&[0, 100_000]),
                vec![None; (hour - 10) as usize],
                &content.pass,
            )
        })
        .collect();
    let warnings = vec![
        vec![
            "weather temperature not shown: region 73 has no member provinces (permanent)",
            "weather temperature not shown: region 74 has no member provinces (permanent)",
        ];
        frames.len()
    ];
    let means = missing_reason_publication(content, &frames, &warnings);
    assert_eq!(
        means,
        vec![BTreeMap::from([("71|temp".into(), 100_000)]); 4]
    );
}

#[test]
fn unavailable_members_name_each_province_first_hour_and_count_without_partial_means() {
    let content = bound(
        &[(71, vec![1, 2]), (72, vec![2, 9, 9, 10]), (73, vec![9])],
        3,
    );
    let mut frames: Vec<_> = (10..=13)
        .map(|hour| {
            temperatures(
                1,
                10,
                hour,
                &states(&[0, 100_000, 300_000]),
                vec![None; (hour - 10) as usize],
                &content.pass,
            )
        })
        .collect();
    for (at, missing) in [(1, vec![1, 2]), (2, vec![2])] {
        if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
            &mut *frames[at].tables[TableId::WeatherProvinces.index()]
        {
            for province in missing {
                rows[province].temperature = ProvinceTemperature::NOT_RECORDED;
            }
        }
    }
    let warnings = [
        vec![
            "1 weather temperature observations not shown: province 9's temperature unavailable at hour 10 (first affected hour)",
            "1 weather temperature observations not shown: province 10's temperature unavailable at hour 10 (first affected hour)",
        ],
        vec![
            "1 weather temperature observations not shown: WeatherProvinces temperature not recorded",
            "1 weather temperature observations not shown: province 1's temperature unavailable at hour 11 (first affected hour)",
            "1 weather temperature observations not shown: province 2's temperature unavailable at hour 11 (first affected hour)",
            "2 weather temperature observations not shown: province 9's temperature unavailable at hour 10 (first affected hour)",
            "2 weather temperature observations not shown: province 10's temperature unavailable at hour 10 (first affected hour)",
        ],
        vec![
            "2 weather temperature observations not shown: WeatherProvinces temperature not recorded",
            "1 weather temperature observations not shown: province 1's temperature unavailable at hour 11 (first affected hour)",
            "2 weather temperature observations not shown: province 2's temperature unavailable at hour 11 (first affected hour)",
            "3 weather temperature observations not shown: province 9's temperature unavailable at hour 10 (first affected hour)",
            "3 weather temperature observations not shown: province 10's temperature unavailable at hour 10 (first affected hour)",
        ],
        vec![
            "2 weather temperature observations not shown: WeatherProvinces temperature not recorded",
            "1 weather temperature observations not shown: province 1's temperature unavailable at hour 11 (first affected hour)",
            "2 weather temperature observations not shown: province 2's temperature unavailable at hour 11 (first affected hour)",
            "4 weather temperature observations not shown: province 9's temperature unavailable at hour 10 (first affected hour)",
            "4 weather temperature observations not shown: province 10's temperature unavailable at hour 10 (first affected hour)",
        ],
    ];
    let means = missing_reason_publication(content, &frames, &warnings);
    assert_eq!(
        means,
        [
            BTreeMap::from([("71|temp".into(), 200_000)]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::from([("71|temp".into(), 200_000)]),
        ]
    );
}

fn cached_bound(source: &Source, options: &ContentOptions) -> Arc<Bound> {
    source
        .state::<catalogue::KeyedResults<catalogue::RecordedBinding, Bound>>()
        .lock()
        .unwrap()
        .get_or_init(&catalogue::recorded_binding(options), |_| {
            unreachable!("fixture installed the binding")
        })
        .unwrap()
}

#[test]
fn inverse_leases_retire_while_source_and_old_bindings_stay_cached() {
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 1]), (72, vec![2])], 3));
    let cached = cached_bound(&source, &options);
    let mut first = Observation::default();
    let mut second = Observation::default();
    first
        .bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    second
        .bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    assert!(Arc::ptr_eq(
        first.inverse.as_ref().unwrap().index.as_ref().unwrap(),
        second.inverse.as_ref().unwrap().index.as_ref().unwrap()
    ));
    let old = crate::allocation_watch::Watch::new(Arc::as_ptr(
        first.inverse.as_ref().unwrap().index.as_ref().unwrap(),
    ));
    first.retire();
    assert!(!old.freed());
    drop(second);
    assert!(old.freed());
    drop(old);
    assert!(cached.inverse.lock().unwrap().is_none());

    first
        .bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    let retired = crate::allocation_watch::Watch::new(Arc::as_ptr(
        first.inverse.as_ref().unwrap().index.as_ref().unwrap(),
    ));
    let mut other_options = options.clone();
    other_options.allow_unverified = true;
    source
        .state::<catalogue::KeyedResults<catalogue::RecordedBinding, Bound>>()
        .lock()
        .unwrap()
        .get_or_init(&catalogue::recorded_binding(&other_options), |_| {
            Ok(bound(&[(72, vec![1, 2, 1])], 3))
        })
        .unwrap();
    first
        .bind_in(&source, Some(&other_options), &mut || Ok(()))
        .unwrap();
    assert!(retired.freed());
    drop(retired);
    assert!(cached.inverse.lock().unwrap().is_none());
    assert_eq!(
        first
            .inverse
            .as_ref()
            .unwrap()
            .regions(1)
            .collect::<Vec<_>>(),
        [72, 72]
    );
    first.retire();
    assert!(
        cached_bound(&source, &other_options)
            .inverse
            .lock()
            .unwrap()
            .is_none()
    );
}

#[test]
fn cancelled_initialization_caches_no_failure() {
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 2])], 3));
    let cached = cached_bound(&source, &options);
    let mut observer = Observation::default();
    assert!(cached.inverse.lock().unwrap().is_none());
    observer
        .bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    let cancelled = observer
        .bind_in(
            &source,
            Some(&options),
            &mut || Err(io::ErrorKind::Interrupted.into()),
        )
        .unwrap_err();
    assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
    assert!(observer.inverse.is_none());
    assert!(cached.inverse.lock().unwrap().is_none());
    observer
        .bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    assert!(observer.inverse.is_some());
    drop(observer);
}

#[test]
fn concurrent_inverse_binders_share_one_index_and_retire_the_last_lease() {
    use std::sync::Barrier;
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 1]), (72, vec![1, 2])], 3));
    let cached = cached_bound(&source, &options);
    let start = Barrier::new(4);
    let observers = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    let mut observer = Observation::default();
                    observer
                        .bind_in(&source, Some(&options), &mut || Ok(()))
                        .unwrap();
                    observer
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    for observer in &observers[1..] {
        assert!(Arc::ptr_eq(
            observers[0]
                .inverse
                .as_ref()
                .unwrap()
                .index
                .as_ref()
                .unwrap(),
            observer.inverse.as_ref().unwrap().index.as_ref().unwrap()
        ));
    }
    assert_eq!(
        observers[0]
            .inverse
            .as_ref()
            .unwrap()
            .regions(1)
            .collect::<Vec<_>>(),
        [71, 71, 72]
    );
    drop(observers);
    assert!(cached.inverse.lock().unwrap().is_none());
}

#[test]
fn source_cached_cancelled_drill_never_holds_an_aggregate_inverse_lease() {
    use super::super::provinces::{Temperatures, temperatures_checked};
    use dashboard_base::prepared::{
        coverage::{Coverage, SCHEMA},
        prefix::Prefix,
    };
    use dashboard_keyed::query::Query;
    use std::sync::atomic::Ordering;

    let content = bound(&[(71, vec![1, 2])], 3);
    let frame = temperatures(1, 10, 10, &states(&[0, -7, 21]), vec![], &content.pass);
    let source = source(vec![frame.clone()]);
    let options = install_cached(&source, content);
    let cached = cached_bound(&source, &options);
    let mut prepared = Observation::default();
    prepared
        .bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    let allocation = crate::allocation_watch::Watch::new(Arc::as_ptr(
        prepared.inverse.as_ref().unwrap().index.as_ref().unwrap(),
    ));
    let fixture = Fixture::new();
    let params = json!({"branchId":"1", "view":"provinces", "region":"71", "hour":10});
    let prefix = Prefix {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 9,
            complete: false,
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
    let mut checks = 0;
    let error = temperatures_checked(&query, &frame, &[1, 2], &mut vec![], &mut || {
        checks += 1;
        if checks == 2 {
            cancelled.store(true, Ordering::Relaxed);
        }
        dashboard_text::metadata::check_cancel(&cancelled)
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(checks, 2);
    {
        let state = source.state::<Temperatures>();
        let state = state.lock().unwrap();
        let drill = state.0.get(&1).unwrap();
        assert!(drill.inverse.is_none());
        assert_eq!(drill.memo.simulation().unwrap().hour, 10);
        assert_eq!(drill.memo.simulation().unwrap().states[1].temperature, -7);
        assert!(drill.content.is_some());
    }
    assert!(!allocation.freed());
    prepared.retire();
    assert!(allocation.freed());
    assert!(cached.inverse.lock().unwrap().is_none());
    assert_eq!(source.state::<Temperatures>().lock().unwrap().0.len(), 1);
}

#[test]
fn final_inverse_retirement_and_concurrent_rebind_fit_one_allocation() {
    use std::sync::Barrier;
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 1]), (72, vec![2])], 3));
    let cached = cached_bound(&source, &options);
    let mut old = Observation::default();
    old.bind_in(&source, Some(&options), &mut || Ok(()))
        .unwrap();
    let gate = Barrier::new(2);
    let gate = &gate;
    let current = std::thread::scope(|scope| {
        let retiring = scope.spawn(move || {
            gate.wait();
            drop(old);
        });
        let binding = scope.spawn(|| {
            gate.wait();
            let mut current = Observation::default();
            current
                .bind_in(&source, Some(&options), &mut || Ok(()))
                .unwrap();
            current
        });
        retiring.join().unwrap();
        binding.join().unwrap()
    });
    assert_eq!(
        current
            .inverse
            .as_ref()
            .unwrap()
            .regions(1)
            .collect::<Vec<_>>(),
        [71, 71]
    );
    let allocation = crate::allocation_watch::Watch::new(Arc::as_ptr(
        current.inverse.as_ref().unwrap().index.as_ref().unwrap(),
    ));
    drop(current);
    assert!(allocation.freed());
    assert!(cached.inverse.lock().unwrap().is_none());
}

fn weather(walk: &mut replay::columnar_frames::synthetic::Synthetic, frame: &DecodedFrame) {
    dashboard_keyed::native::testing::write::<tables::WeatherRegions>(walk, frame);
    dashboard_keyed::native::testing::write::<tables::WeatherProvinces>(walk, frame);
}

/// The native publication, its temperatures derived by each segment's keeper, holds what the
/// walk's scan holds and counts the same refusals, wherever segments start (mid-anchor ones
/// deriving from the anchor), the publisher cuts and the emit resumes: two anchors, an
/// unavailable hour, a wrong witness, a gap, overlapping, duplicate and missing members.
#[test]
fn segment_keepers_derive_the_temperatures_the_walk_derives() {
    let mut content = bound(
        &[
            (71, vec![1, 2, 1]),
            (72, vec![2, 3]),
            (73, vec![]),
            (74, vec![9]),
        ],
        4,
    );
    content.pass.settings.provinces_per_update = 2;
    content.pass.settings.smoothing = 50_000;
    content.pass.settings.change_smoothing = 30_000;
    content.pass.provinces[1].neighbours = vec![2];
    content.pass.provinces[2].neighbours = vec![3, 1];
    content.pass.provinces[3].neighbours = vec![1];
    let first = states(&[0, 100_001, -500_000, 900_000]);
    let second = states(&[0, -300_000, 250_000, 40_000]);
    let seeds = |count: i32| -> Vec<Option<u32>> {
        (0..count)
            .map(|at| (at % 4 != 2).then_some(at as u32 * 13 + 7))
            .collect()
    };
    let mut frames = Vec::new();
    for hour in 10..=17 {
        frames.push(temperatures(1, 10, hour, &first, seeds(hour - 10), &content.pass));
    }
    let mut refused = frame(18, Some(vec![row(71), row(72)]));
    *refused.tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
        reason: "synthetic native temperature refusal".into(),
    };
    frames.push(refused);
    for hour in 19..=25 {
        let mut frame = temperatures(1, 19, hour, &second, seeds(hour - 19), &content.pass);
        if hour == 23 {
            let rows = match &mut *frame.tables[TableId::WeatherProvinces.index()] {
                TableView::Complete(TableRows::WeatherProvinces(rows)) => rows,
                _ => unreachable!(),
            };
            if let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass {
                pass.witness =
                    replay_data::rows::TemperatureWitness::new(pass.witness.get().unwrap() ^ 1);
            }
        }
        frames.push(frame);
    }
    for hour in 28..=31 {
        frames.push(temperatures(1, 28, hour, &first, seeds(hour - 28), &content.pass));
    }
    for frame in &mut frames {
        frame.metrics.table_ns[TableId::WeatherRegions.index()] = 1;
        frame.metrics.table_ns[TableId::WeatherProvinces.index()] = 1;
    }
    let source = source(frames.clone());
    let options = install_cached(&source, content);
    let binding = Binding::<block::Observer>::new(&source, Some(&options)).unwrap();
    for (starts, cuts, resume) in [
        (&[][..], &[31][..], None),
        (&[3, 9, 12][..], &[12, 22, 31][..], Some(22)),
        (&[5, 10, 15][..], &[17, 19, 25, 31][..], Some(19)),
        (&(1..frames.len()).collect::<Vec<_>>()[..], &[23, 31][..], Some(23)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (native, walked) =
            dashboard_keyed::native::testing::assert_bound_published_as_walked::<block::Observer>(
                &frames,
                weather,
                starts,
                cuts,
                resume,
                &label,
                Some(&binding),
            );
        assert_eq!(
            native.observer.temperature_warnings(),
            walked.observer.temperature_warnings(),
            "{label}: refusals"
        );
        assert_eq!(native.observer.temperature_status(), "available", "{label}");
        let warnings = native.observer.temperature_warnings();
        for refusal in ["synthetic native temperature refusal", "recorded witness"] {
            assert!(
                warnings.iter().any(|warning| warning.contains(refusal)),
                "{label}: {refusal} in {warnings:?}"
            );
        }
    }
}

#[test]
fn resolved_binding_preserves_source_binding_status_and_warnings() {
    for case in 0..3 {
        let source = source(vec![frame(10, Some(vec![row(71)]))]);
        let options = match case {
            0 => {
                let mut content = bound(&[(71, vec![1])], 2);
                content.warnings.push("synthetic content warning".into());
                Some(install_cached(&source, content))
            }
            1 => None,
            _ => {
                let mut options = install_cached(&source, bound(&[(71, vec![1])], 2));
                options.options.game_dir = "unavailable-temperature-stack".into();
                source
                    .state::<catalogue::KeyedResults<catalogue::RecordedBinding, Bound>>()
                    .lock()
                    .unwrap()
                    .get_or_init(&catalogue::recorded_binding(&options), |_| {
                        Err(dashboard_core::invalid(
                            "synthetic unavailable recorded stack",
                        ))
                    })
                    .err()
                    .expect("unavailable stack");
                Some(options)
            }
        };
        let mut old = block::Observer::default();
        old.bind(&source, options.as_ref(), &AtomicBool::new(false))
            .unwrap();
        let resolved = Arc::new(resolve(&source, options.as_ref(), &mut || Ok(())).unwrap());
        let mut observer = block::Observer::default();
        dashboard_keyed::Observer::bind(&mut observer, &resolved).unwrap();
        assert_eq!(observer.temperature_status(), old.temperature_status());
        assert_eq!(observer.temperature_warnings(), old.temperature_warnings());
        if case == 0 {
            assert_eq!(observer.temperature_status(), "available");
            assert_eq!(
                observer.temperature_warnings(),
                ["synthetic content warning"]
            );
        } else {
            assert!(
                observer
                    .temperature_status()
                    .starts_with("Temperature unavailable: ")
            );
        }
    }
}

#[test]
fn native_bindings_use_resolved_content_without_the_source_cache() {
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 2])], 3));
    let resolved = Arc::new(resolve(&source, Some(&options), &mut || Ok(())).unwrap());
    let cached = cached_bound(&source, &options);
    let cache = source.state::<catalogue::KeyedResults<catalogue::RecordedBinding, Bound>>();
    let _locked = cache.lock().unwrap();
    let mut keeper = <block::Observer as dashboard_keyed::Observer>::native_keeper().unwrap();
    keeper.bind(&resolved).unwrap();
    assert!(cached.inverse.lock().unwrap().is_none());
    let mut emit =
        dashboard_keyed::native::Emit::<block::Observer>::new(FormatVersion::CURRENT, 1).unwrap();
    emit.bind(&resolved).unwrap();
    assert!(cached.inverse.lock().unwrap().is_some());
    drop(emit);
    assert!(cached.inverse.lock().unwrap().is_none());
}
