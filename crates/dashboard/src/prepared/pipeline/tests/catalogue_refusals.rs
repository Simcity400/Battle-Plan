//! Published content refusals are each family's own: another consumer's
//! refusal or artwork and uncovered reads, the lane a family runs on and a
//! restore from a commit leave them unchanged.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;

/// What another consumer of the shared catalogue refuses mid-run.
const OTHER: &str = "synthetic other consumer refusal";
/// The artwork-effect note another consumer's render leaves on the stack.
const OTHER_ARTWORK: &str = "synthetic other consumer artwork-effect note";
/// A file outside the recorded coverage policy that another consumer reads.
const OTHER_READ: &str = "outside_policy/other_consumer.txt";

/// A sinking from hour 101 in a content strategic region whose name the
/// plain view refuses, so the naval family's own reads refuse `mod_sea`.
fn sinking_frame(hour: i32) -> DecodedFrame {
    use replay_data::{rows::SunkShipEntry, rows::SunkShipsRow};
    let mut frame = crate::test_fixtures::source(1, hour, Some(i64::from(hour) * 1000));
    let entries = if hour >= 101 {
        vec![SunkShipEntry {
            ship_name: b"synthetic refused-place sinking".to_vec(),
            victim_country: 9001,
            attacker_country: 9001,
            sink_hour: 101,
            province: 300,
            province_present: 1,
            ..Default::default()
        }]
    } else {
        Vec::new()
    };
    *frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }]));
    frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
    frame
}

/// Two blocks of one branch (hours 100-101, then 102-103) recorded against a
/// synthetic content folder.
fn refusing_fixture() -> (Fixture, crate::products::ContentOptions) {
    let frames: Vec<_> = (100..=103).map(sinking_frame).collect();
    let fixture = Fixture::new(
        &Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..2].to_vec())]).unwrap(),
    );
    let game = fixture.root.join("content");
    for (path, text) in [
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
        (
            "map/strategicregions/zone.txt",
            "strategic_region={id=8 name=mod_sea provinces={300}}",
        ),
        (
            "localisation/english/sea_l_english.yml",
            "\u{feff}l_english:\n mod_sea:0 \"a¤b\"\n",
        ),
        (OTHER_READ, "read by another consumer"),
    ] {
        let file = game.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let (options, identity) = crate::test_fixtures::content_identity(&game);
    let mut input = records(1, frames[..2].to_vec());
    let replay::spool::Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let first = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    let replay = first
        .append(&[continuation_records(&first, 1, frames[2..].to_vec())])
        .unwrap();
    fixture.replace(&replay);
    (fixture, options)
}

/// Before `family` observes `hour`, another consumer of the same source
/// refuses a text through the shared catalogue, leaves an artwork-effect note
/// on the content stack and reads a file outside its coverage.
fn other_consumer(
    source: &Arc<Source>,
    options: &crate::products::ContentOptions,
    (family, hour): (&'static str, i32),
) -> lanes::Hook {
    let source = Arc::clone(source);
    let options = options.clone();
    Arc::new(move |name, frame, _changes| {
        if name == family && frame.hour == hour {
            let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
            catalogue.refuse(OTHER, "OTHER_CONSUMER");
            catalogue
                .content
                .layers
                .note_shader_warning(OTHER_ARTWORK.into());
            catalogue.content.layers.read(OTHER_READ).unwrap();
        }
        lanes::Fault::Continue
    })
}

/// Prepares through `through_hours`, publishing and committing at every
/// block end, at `parallelism` (two observe serially).
fn prepare(
    source: &Arc<Source>,
    options: &crate::products::ContentOptions,
    parallelism: usize,
    other: Option<(&'static str, i32)>,
    through_hours: u64,
) {
    let hook = other.map(|other| other_consumer(source, options, other));
    independent::with_test_hourly(|| {
        lanes::with_test_lanes(parallelism, hook, || {
            run_measured_through(
                source,
                Some(options),
                &AtomicBool::new(false),
                false,
                Duration::ZERO,
                Some(through_hours),
                |_| Ok(()),
                |_| {},
            )
        })
    })
    .unwrap();
    if other.is_some() {
        let shared = crate::catalogue::catalogue(source, options)
            .unwrap()
            .refusal_lines();
        for marker in [OTHER, OTHER_ARTWORK, OTHER_READ] {
            assert!(
                shared.iter().any(|line| line.contains(marker)),
                "the other consumer's {marker} reached the shared catalogue: {shared:#?}"
            );
        }
    }
}

/// Every published payload; the store is cleared for the next run.
fn published(
    source: Arc<Source>,
    options: &crate::products::ContentOptions,
) -> BTreeMap<(String, u64), Vec<u8>> {
    let published = payloads(&source, Some(options));
    super::super::super::files(&source, Some(options))
        .unwrap()
        .0
        .clear()
        .unwrap();
    drop(source);
    published
}

/// The sinking manifest names the naval family's own place refusal, and no
/// payload names the other consumer's refusal or reads.
fn assert_own_refusals_only(published: &BTreeMap<(String, u64), Vec<u8>>) {
    let plain =
        |bytes: &Vec<u8>| zstd::decode_all(bytes.as_slice()).unwrap_or_else(|_| bytes.clone());
    let (_, manifest) = published
        .iter()
        .find(|((name, _), _)| name.starts_with("ships-sunk-manifest-"))
        .expect("the naval family publishes its sinking manifest");
    let manifest: serde_json::Value = serde_json::from_slice(&plain(manifest)).unwrap();
    let warnings: Vec<_> = manifest["contentWarnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning.as_str().unwrap())
        .collect();
    assert!(
        warnings.iter().any(|warning| {
            warning.starts_with("1 texts are not shown: ") && warning.ends_with("; mod_sea")
        }),
        "{warnings:#?}"
    );
    for ((name, range), bytes) in published {
        let bytes = plain(bytes);
        for marker in [OTHER, OTHER_ARTWORK, OTHER_READ] {
            assert!(
                !bytes
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes()),
                "{name}#{range} names another consumer's {marker}"
            );
        }
    }
}

#[test]
fn another_consumers_refusal_leaves_every_published_payload_unchanged() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    let (mut fixture, options) = refusing_fixture();
    for parallelism in [2, 4] {
        let control = fixture.source("own-refusals");
        prepare(&control, &options, parallelism, None, 4);
        let control = published(control, &options);
        assert_own_refusals_only(&control);
        // Before the naval family's first observation, and after its last.
        for other in [("projects", 100), ("equipment_losses", 103)] {
            let source = fixture.source("own-refusals");
            prepare(&source, &options, parallelism, Some(other), 4);
            let refused = published(source, &options);
            assert_own_refusals_only(&refused);
            assert_payloads_equal(
                &refused,
                &control,
                &format!("{other:?} refusing at parallelism {parallelism}"),
            );
        }
    }
}

#[test]
fn serial_and_lane_preparation_publish_the_same_refusals_whenever_others_refuse() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    let (mut fixture, options) = refusing_fixture();
    // The other consumer's refusal lands late in the serial run and before the
    // first block's publication in the lane run, as lane timing may place it.
    let serial = fixture.source("lane-refusals");
    prepare(&serial, &options, 2, Some(("equipment_losses", 103)), 4);
    let serial = published(serial, &options);
    assert_own_refusals_only(&serial);
    let parallel = fixture.source("lane-refusals");
    prepare(&parallel, &options, 4, Some(("projects", 100)), 4);
    assert_payloads_equal(&published(parallel, &options), &serial, "lanes vs serial");
}

#[test]
fn restored_families_publish_the_refusals_continued_ones_do() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    let (mut fixture, options) = refusing_fixture();
    for parallelism in [2, 4] {
        let continued = fixture.source("restored-refusals");
        prepare(
            &continued,
            &options,
            parallelism,
            Some(("projects", 100)),
            4,
        );
        let continued = published(continued, &options);
        assert_own_refusals_only(&continued);
        // A new process: the first block commits, and a fresh source, whose
        // catalogue has refused nothing yet, restores every family for the second.
        let first = fixture.source("restored-refusals");
        prepare(&first, &options, parallelism, Some(("projects", 100)), 2);
        drop(first);
        let restored = fixture.source("restored-refusals");
        prepare(&restored, &options, parallelism, None, 4);
        assert_payloads_equal(
            &published(restored, &options),
            &continued,
            &format!("restored vs continued at parallelism {parallelism}"),
        );
    }
}

/// The data of `product`'s prepared reply to `params`, as it is served.
fn served(
    source: &Source,
    options: &crate::products::ContentOptions,
    product: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = independent::with_test_hourly(|| {
        super::super::super::query::query(source, &request, Some(options), &AtomicBool::new(false))
            .unwrap_or_else(|error| panic!("{product}: {error}"))
    });
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

/// After preparation, another consumer leaves an artwork-effect note and an
/// uncovered read on the stack. A reply that keeps its content warnings
/// somewhere other than a top-level `warnings` list serves both, once, at
/// each place it keeps them; recorded battle warnings stay battle warnings,
/// and the published payloads are unchanged.
#[test]
fn served_replies_carry_the_stacks_live_diagnostics_where_they_keep_content_warnings() {
    use serde_json::json;
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    let (mut fixture, options) = refusing_fixture();
    let source = fixture.source("served-diagnostics");
    prepare(&source, &options, 2, None, 4);
    let before = payloads(&source, Some(&options));
    let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
    catalogue
        .content
        .layers
        .note_shader_warning(OTHER_ARTWORK.into());
    catalogue.content.layers.read(OTHER_READ).unwrap();
    let branch = json!({"branchId": "1"});
    let replies: [(&str, serde_json::Value, &[&str]); 12] = [
        (
            "land-battles-manifest",
            branch.clone(),
            &["/contentWarnings"],
        ),
        (
            "land-battles",
            json!({"branchId": "1", "window": 0}),
            &["/contentWarnings"],
        ),
        (
            "naval-battles-manifest",
            branch.clone(),
            &["/contentWarnings"],
        ),
        (
            "division-log-manifest",
            branch.clone(),
            &["/contentWarnings"],
        ),
        (
            "ships-sunk-manifest",
            branch.clone(),
            &["/contentWarnings", "/classes/warnings"],
        ),
        (
            "ships-sunk",
            json!({"branchId": "1", "hour": 101}),
            &["/reply/warnings"],
        ),
        (
            "series-state",
            json!({"branchId": "1", "family": {"kind": "game-score"}, "reads": [{"hour": 101}]}),
            &["/reply/warnings"],
        ),
        (
            "scoreboard",
            json!({"selection": {"branchId": "1", "columns": [], "tags": [], "top": null},
                "hour": 101, "sort": "factories", "descending": true, "index": true, "bars": false}),
            &["/data/warnings"],
        ),
        (
            "series-state",
            json!({"branchId": "1", "family": {"kind": "country-fuel"},
                "reads": [{"tag": "AAA", "hour": 101}, {"tag": "BBB", "hour": 101}]}),
            &["/replies/0/warnings", "/replies/1/warnings"],
        ),
        // Politics content refuses on this stack, so neither reply adds the
        // stack's lines itself.
        (
            "party-history",
            json!({"branchId": "1", "tag": "AAA", "mode": "summary",
                "from": 100, "to": 103, "width": 10}),
            &["/reply/warnings"],
        ),
        (
            "event-feed",
            json!({"branchId": "1", "lastHour": 103, "pageSize": 256,
                "categories": ["milestones"]}),
            &["/reply/warnings"],
        ),
        // Two holders answer one catalogue. The equipment database loads
        // here and names the stack's lines itself; they stay once.
        (
            "equipment-stockpile-catalogue",
            json!({"branchId": "1", "holders": [{"tag": "AAA"}, {"tag": "BBB"}]}),
            &["/warnings"],
        ),
    ];
    let mut missing = Vec::new();
    for (product, params, pointers) in replies {
        let data = served(&source, &options, product, params);
        for pointer in pointers {
            let lines: Vec<_> = data
                .pointer(pointer)
                .and_then(serde_json::Value::as_array)
                .unwrap_or_else(|| panic!("{product} keeps no {pointer}: {data:#}"))
                .iter()
                .map(|line| line.as_str().unwrap())
                .collect();
            for marker in [OTHER_ARTWORK, OTHER_READ] {
                let served = lines.iter().filter(|line| line.contains(marker)).count();
                if served != 1 {
                    missing.push(format!("{product}{pointer} serves {marker} {served} times"));
                }
            }
        }
    }
    // Where a reply's own content refuses, nothing else names the stack's
    // lines: a stockpile hour's reply and a naval summary's sampled reply
    // each gain them where they are served.
    let shapes = [
        (
            "equipment-stockpile-at",
            "equipment-stockpile-at",
            json!({"warnings": ["kept"]}),
            &["/warnings"][..],
        ),
        (
            "series-summary",
            "naval-losses",
            json!({"series": [], "sampled": {"warnings": ["kept"]}}),
            &["/sampled/warnings"][..],
        ),
    ];
    for (product, name, mut shape, pointers) in shapes {
        crate::prepared::query::append_stack_warnings(&source, product, name, &mut shape);
        for pointer in pointers {
            let lines = shape.pointer(pointer).unwrap().as_array().unwrap();
            for marker in [OTHER_ARTWORK, OTHER_READ] {
                let served = lines
                    .iter()
                    .filter(|line| line.as_str().unwrap().contains(marker))
                    .count();
                if served != 1 {
                    missing.push(format!("{name}{pointer} serves {marker} {served} times"));
                }
            }
        }
        assert!(shape.to_string().contains("\"kept\""), "{shape}");
    }
    assert!(missing.is_empty(), "{missing:#?}");
    let window = served(
        &source,
        &options,
        "land-battles",
        json!({"branchId": "1", "window": 0}),
    );
    assert!(
        window["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|warning| !warning.is_string()),
        "{window:#}"
    );
    assert_payloads_equal(
        &payloads(&source, Some(&options)),
        &before,
        "served replies",
    );
}
