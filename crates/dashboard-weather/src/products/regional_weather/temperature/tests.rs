use super::*;
use crate::products::regional_weather::{
    TEMPERATURE_SCALE, block,
    tests::{answer, frame, reply_runs, row, source, write},
};
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_keyed::{Observer as _, Scan};
use replay_data::{
    format::FormatVersion,
    rows::{
        ProvinceTemperature, Recorded, TableId, TableRows, TemperaturePass, WeatherProvinceRow,
    },
    temperature::{Province, Settings, Simulation},
};
use replay_frame::frame::TableView;
use serde_json::json;

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
        checksum: simulation.checksum(),
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
                .lease_inverse(&MemoryBudget::preparation(), &mut || Ok(()))
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
        pass.checksum ^= 1;
    }
    assert!(observe(&mut observation, &bad).is_empty());
    assert!(
        observation
            .warnings()
            .iter()
            .any(|warning| warning.contains("checksum"))
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
    scan.observer
        .bind(&source, Some(&options), &cancelled)
        .unwrap();
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
        scan.observer
            .bind(&source, Some(&options), &cancelled)
            .unwrap();
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
    scan.observer
        .bind(&source, Some(&options), &cancelled)
        .unwrap();
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
    observer
        .bind(&source, None, &AtomicBool::new(false))
        .unwrap();
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
    observer
        .bind(&source, Some(&options), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(observer.temperature_status(), "available");
    observer
        .bind(&source, Some(&options), &AtomicBool::new(false))
        .unwrap();
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        observer
            .bind(&source, Some(&options), &cancelled)
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
    });
    let mut records =
        dashboard_base::test_fixtures::records(1, vec![frame(10, Some(vec![row(71)]))]);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        panic!("fixture session")
    };
    session.identity.verification = replay_data::identity::Verification::Unchecked;
    let replay = replay::container::Replay::from_records(
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
    a.bind(&source, Some(&options), &cancelled).unwrap();
    assert!(a.status().contains("unverified content"), "{}", a.status());
    options.allow_unverified = true;
    a.bind(&source, Some(&options), &cancelled).unwrap();
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
    b.bind(&source, Some(&options), &cancelled).unwrap();
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
    scan.observer
        .bind(&source, Some(&options), &AtomicBool::new(false))
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

#[test]
fn compact_bound_observers_preserve_literal_temperatures_and_refusals_after_resume() {
    let mut content = bound(&[(71, vec![1, 2]), (72, vec![2])], 3);
    content.warnings = vec!["synthetic bound-content warning".into()];
    let first_words = states(&[0, -100_000, 300_000]);
    let resumed_words = states(&[0, -300_000, 100_000]);
    let first = temperatures(1, 10, 10, &first_words, vec![], &content.pass);
    let mut refused = frame(11, Some(vec![row(71), row(72)]));
    *refused.tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
        reason: "synthetic bound-temperature refusal".into(),
    };
    let resumed = temperatures(1, 12, 12, &resumed_words, vec![], &content.pass);
    let frames = vec![first, refused, resumed];
    let source = source(frames.clone());
    let options = install_cached(&source, content);
    let cancelled = AtomicBool::new(false);
    let mut whole = block::Observer::default();
    let mut compact = block::Observer::default();
    for observer in [&mut whole, &mut compact] {
        observer.bind(&source, Some(&options), &cancelled).unwrap();
        assert_eq!(observer.temperature_status(), "available");
    }
    let mut keeper = <block::Observer as dashboard_keyed::Observer>::keeper().unwrap();
    for (at, frame) in frames.into_iter().enumerate() {
        let province_view = frame.tables[TableId::WeatherProvinces.index()].identity();
        let region_view = frame.tables[TableId::WeatherRegions.index()].identity();
        let input = keeper.keep(FormatVersion::CURRENT, &frame).unwrap();
        let mut expected = Values::default();
        assert!(
            whole
                .observe(FormatVersion::CURRENT, &frame, &mut expected)
                .unwrap()
        );
        drop(frame);
        assert!(!province_view.is_alive());
        assert!(!region_view.is_alive());
        let mut actual = Values::default();
        assert!(input.observe(&mut compact, &mut actual).unwrap());
        assert!(expected.iter().eq(actual.iter()));
        let literal = match at {
            0 => BTreeMap::from([("71|temp", 100_000), ("72|temp", 300_000)]),
            1 => BTreeMap::new(),
            2 => BTreeMap::from([("71|temp", -100_000), ("72|temp", 100_000)]),
            _ => unreachable!(),
        };
        let warnings = if at == 0 {
            vec!["synthetic bound-content warning"]
        } else {
            vec![
                "synthetic bound-content warning",
                "1 weather temperature observations not shown: hour 11 WeatherProvinces is unavailable: synthetic bound-temperature refusal",
            ]
        };
        for (observer, values) in [(&whole, &expected), (&compact, &actual)] {
            assert_eq!(
                values
                    .iter()
                    .filter(|(name, _)| name.ends_with("|temp"))
                    .collect::<BTreeMap<_, _>>(),
                literal,
            );
            assert_eq!(observer.temperature_warnings(), warnings);
            assert_eq!(observer.temperature_status(), "available");
        }
        assert_eq!(
            serde_json::to_value(&whole).unwrap(),
            serde_json::to_value(&compact).unwrap()
        );
        if at == 1 {
            whole = serde_json::from_value(serde_json::to_value(&whole).unwrap()).unwrap();
            compact = serde_json::from_value(serde_json::to_value(&compact).unwrap()).unwrap();
            for observer in [&mut whole, &mut compact] {
                observer.bind(&source, Some(&options), &cancelled).unwrap();
                assert_eq!(observer.temperature_warnings(), warnings);
            }
            keeper = <block::Observer as dashboard_keyed::Observer>::keeper().unwrap();
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
    assert_eq!(
        observation.refused["region membership is empty or a member province temperature is unavailable"],
        26
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
    let expected = Inverse::allocation_bytes(3).unwrap();
    let budget = MemoryBudget::new(expected);
    let mut first = Observation::default();
    let mut second = Observation::default();
    first
        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
        .unwrap();
    second
        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
        .unwrap();
    assert!(Arc::ptr_eq(
        first.inverse.as_ref().unwrap().index.as_ref().unwrap(),
        second.inverse.as_ref().unwrap().index.as_ref().unwrap()
    ));
    let old = crate::allocation_watch::Watch::new(Arc::as_ptr(
        first.inverse.as_ref().unwrap().index.as_ref().unwrap(),
    ));
    assert_eq!(budget.used(), expected);
    first.retire();
    assert_eq!(budget.used(), expected);
    assert!(!old.freed());
    drop(second);
    assert_eq!(budget.used(), 0);
    assert!(old.freed());
    drop(old);
    assert!(cached.inverse.lock().unwrap().is_none());

    first
        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
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
        .bind_in(&source, Some(&other_options), &mut || Ok(()), &budget)
        .unwrap();
    assert!(retired.freed());
    drop(retired);
    assert!(cached.inverse.lock().unwrap().is_none());
    assert_eq!(budget.used(), expected);
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
    assert_eq!(budget.used(), 0);
    assert!(
        cached_bound(&source, &other_options)
            .inverse
            .lock()
            .unwrap()
            .is_none()
    );
}

#[test]
fn inverse_denial_and_cancelled_initialization_release_credit_without_caching_failure() {
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 2])], 3));
    let cached = cached_bound(&source, &options);
    let budget = MemoryBudget::new(Inverse::allocation_bytes(2).unwrap());
    let mut observer = Observation::default();
    let denied = observer
        .bind_in(
            &source,
            Some(&options),
            &mut || Ok(()),
            &MemoryBudget::new(0),
        )
        .unwrap_err();
    assert_eq!(denied.kind(), io::ErrorKind::WouldBlock);
    assert!(denied.to_string().contains("shared inverse membership"));
    assert!(observer.inverse.is_none());
    assert!(cached.inverse.lock().unwrap().is_none());
    let cancelled = observer
        .bind_in(
            &source,
            Some(&options),
            &mut || {
                if budget.used() > 0 {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Ok(())
                }
            },
            &budget,
        )
        .unwrap_err();
    assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
    assert_eq!(budget.used(), 0);
    assert!(observer.inverse.is_none());
    assert!(cached.inverse.lock().unwrap().is_none());
    observer
        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
        .unwrap();
    assert!(budget.used() > 0);
    let cancelled = observer
        .bind_in(
            &source,
            Some(&options),
            &mut || Err(io::ErrorKind::Interrupted.into()),
            &budget,
        )
        .unwrap_err();
    assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
    assert!(observer.inverse.is_none());
    assert_eq!(budget.used(), 0);
    assert!(cached.inverse.lock().unwrap().is_none());
    observer
        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
        .unwrap();
    assert!(observer.inverse.is_some());
    drop(observer);
    assert_eq!(budget.used(), 0);
}

#[test]
fn concurrent_inverse_binders_share_one_credit_and_retire_the_last_lease() {
    use std::sync::Barrier;
    let source = source(vec![frame(10, Some(vec![row(71)]))]);
    let options = install_cached(&source, bound(&[(71, vec![1, 1]), (72, vec![1, 2])], 3));
    let cached = cached_bound(&source, &options);
    let expected = Inverse::allocation_bytes(4).unwrap();
    let budget = MemoryBudget::new(expected);
    let start = Barrier::new(4);
    let observers = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    let mut observer = Observation::default();
                    observer
                        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
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
    assert_eq!(budget.used(), expected);
    assert_eq!(budget.peak(), expected);
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
    assert_eq!(budget.used(), 0);
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
    let expected = Inverse::allocation_bytes(2).unwrap();
    let budget = MemoryBudget::new(expected);
    let mut prepared = Observation::default();
    prepared
        .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
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
    assert_eq!(budget.used(), 0);
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
    let expected = Inverse::allocation_bytes(3).unwrap();
    let budget = MemoryBudget::new(expected);
    let mut old = Observation::default();
    old.bind_in(&source, Some(&options), &mut || Ok(()), &budget)
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
                .bind_in(&source, Some(&options), &mut || Ok(()), &budget)
                .unwrap();
            current
        });
        retiring.join().unwrap();
        binding.join().unwrap()
    });
    assert_eq!(budget.used(), expected);
    assert_eq!(budget.peak(), expected);
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
    assert_eq!(budget.used(), 0);
    assert!(allocation.freed());
    assert!(cached.inverse.lock().unwrap().is_none());
}
