use crate::products::{
    air_missions::{self, AirMissions, AirMissionsObserver},
    air_statistics::{self, AirStatistics, AirStatisticsObserver},
};
use dashboard_base::{
    Source,
    prepared::{
        coverage::Coverage,
        prefix::Prefix,
        series::{self, tests::Fixture},
    },
    products::Product,
    test_fixtures::{self, world_frame},
};
use dashboard_keyed::{
    Observer, Scan, Values,
    query::{self, Query},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirRegionalMissionRow, AirStatisticsRow, AirWingRow, CountryLinkRow, CountryRow, Recorded,
        TableId, TableRows, tables,
    },
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

fn wing(id: u64, planes: i32, mask: u32) -> AirWingRow {
    AirWingRow {
        id,
        owner: 11,
        planes,
        mission_mask: mask,
        mission_region: Some(3),
        ..Default::default()
    }
}
fn missions(hour: i32, wings: Vec<AirWingRow>, order: &[u64]) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(wings));
    *frame.tables[TableId::AirRegionalMissions.index()] =
        TableView::Complete(TableRows::AirRegionalMissions(
            order
                .iter()
                .enumerate()
                .map(|(ordinal, &wing)| AirRegionalMissionRow {
                    wing,
                    ordinal: ordinal as u32,
                    active_order: None,
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::AirRegionalMissions.index()] = 1;
    frame
}
fn statistics(hour: i32, rows: Vec<AirStatisticsRow>) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    *frame.tables[TableId::AirStatistics.index()] =
        TableView::Complete(TableRows::AirStatistics(rows));
    frame.metrics.table_ns[TableId::AirStatistics.index()] = 1;
    frame
}
fn stat(region: u32, channels: [i64; 24]) -> AirStatisticsRow {
    AirStatisticsRow {
        country: 11,
        region,
        channels,
        ..Default::default()
    }
}
fn values<O: Observer>(frame: &DecodedFrame) -> BTreeMap<String, i128> {
    let mut values = Values::default();
    assert!(
        O::default()
            .observe(FormatVersion::CURRENT, frame, &mut values)
            .unwrap()
    );
    values
        .iter()
        .map(|(name, raw)| (name.to_owned(), raw))
        .collect()
}
fn publish<O: Observer>(fixture: &Fixture, frames: &[DecodedFrame]) -> Scan<O> {
    let mut scan = Scan::<O>::default();
    let mut writer = fixture.writer();
    for frame in frames {
        scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
    }
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    scan
}
fn source(frames: Vec<DecodedFrame>) -> Source {
    Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "air-test".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(test_fixtures::replay(frames)),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn answer(
    fixture: &Fixture,
    frames: Vec<DecodedFrame>,
    params: Value,
    stats: bool,
    through: Option<i32>,
) -> Value {
    let source = source(frames);
    let bounds = &source.metadata().unwrap().branches[0];
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: bounds.first_hour,
            last_hour: through.unwrap_or(bounds.last_hour),
            complete: through.is_none(),
        },
        None,
    );
    let cancelled = AtomicBool::new(false);
    let query = Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    if stats {
        air_statistics::query(&query)
    } else {
        air_missions::query(&query)
    }
    .unwrap()
}
fn runs<'a>(reply: &'a Value, name: &str) -> &'a Value {
    &reply["window"]["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|series| series["name"] == name)
        .unwrap()["runs"]
}

#[test]
fn missions_overlap_counts_unique_wings_zero_planes_and_pool_owner() {
    let mut volunteer = wing(1, 17, 5);
    volunteer.country_override = 21;
    let mut foreign = wing(3, 90, 1);
    foreign.owner = 21;
    foreign.country_override = 11;
    let frame = missions(
        100,
        vec![volunteer, wing(2, 0, 8), foreign, wing(4, 400, 1)],
        &[2, 1, 3],
    );
    let values = values::<AirMissionsObserver>(&frame);
    assert_eq!(values["GER@7|3|m0"], 17);
    assert_eq!(values["GER@7|3|m2"], 17);
    assert_eq!(values["GER@7|3|m3"], 0);
    assert_eq!(values["GER@7|3|wings"], 2);
    assert_eq!(values["ITA@8|3|wings"], 1);
    assert_eq!(values["ITA@8|3|m0"], 90);
}

#[test]
fn missions_keep_native_registration_order_signed_mask_negative_and_wrapping_totals() {
    let frame = missions(
        100,
        vec![
            wing(1, -4, 1),
            wing(2, 9, 1),
            wing(3, i32::MAX, 4),
            wing(4, 1, 4),
            wing(5, 100, 0x8000_0001),
        ],
        &[1, 2, 3, 4, 5],
    );
    let v = values::<AirMissionsObserver>(&frame);
    assert_eq!(v["GER@7|3|m0"], 9);
    assert_eq!(v["GER@7|3|m2"], i128::from(i32::MIN));
    assert_eq!(v["GER@7|3|wings"], 5);
    let frame = missions(101, vec![wing(1, -1, 1)], &[1]);
    assert_eq!(values::<AirMissionsObserver>(&frame)["GER@7|3|m0"], -1);
}

#[test]
fn mission_counts_match_shared_map_projection() {
    let frame = missions(
        100,
        vec![wing(1, -8, 1), wing(2, 7, 5), wing(3, 0, 8)],
        &[1, 3, 2],
    );
    let counts = replay_derive::air_mission_counts::regional_counts(
        frame.complete::<tables::AirRegionalMissions>().unwrap(),
        frame.complete::<tables::AirWings>().unwrap(),
        7,
        |raw| (raw == 11).then_some(7),
    )
    .unwrap()
    .0;
    let v = values::<AirMissionsObserver>(&frame);
    for bit in [0, 2, 3] {
        assert_eq!(v[&format!("GER@7|3|m{bit}")], i128::from(counts[&3][bit]));
    }
}

#[test]
fn missions_runs_absence_unknown_gaps_dates_and_saved_continuation() {
    let fixture = Fixture::new();
    let frames = [
        missions(100, vec![wing(1, 0, 1)], &[1]),
        missions(101, vec![], &[]),
        missions(102, vec![], &[9]),
        missions(104, vec![wing(1, 6, 1)], &[1]),
    ];
    publish::<AirMissionsObserver>(&fixture, &frames[..2]);
    let mut scan = series::read_catalogue::<Scan<AirMissionsObserver>>(
        &fixture.store,
        &fixture.key,
        AirMissionsObserver::NAME,
        1,
    )
    .unwrap()
    .unwrap()
    .1;
    let mut writer = fixture.writer();
    for frame in &frames[2..] {
        scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
    }
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    let cancelled = AtomicBool::new(false);
    let mut opened =
        query::open::<AirMissionsObserver>(&fixture.store, &fixture.key, 1, &cancelled)
            .unwrap()
            .unwrap();
    let window = opened.window(100, 104, None, |_| true).unwrap();
    assert_eq!(
        window
            .known
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (104, 104)]
    );
    assert_eq!(
        window
            .series
            .iter()
            .find(|s| s.name.ends_with("|m0"))
            .unwrap()
            .runs,
        [(100, 100, "0".into(), true), (104, 104, "6".into(), true)]
    );
    assert_eq!(
        window.reads,
        [(100, 100, 100), (101, 101, 101), (104, 104, 104)]
    );
    assert_eq!(opened.scan.observer.refusals.values().sum::<u64>(), 1);
}

#[test]
fn statistics_all_24_signed_channels_ring_state_and_both_divisors() {
    let channels = std::array::from_fn(|channel| {
        if channel == 0 {
            i64::MIN
        } else if channel == 23 {
            i64::MAX
        } else {
            -(channel as i64)
        }
    });
    let mut row = stat(3, channels);
    row.year_slot = 7;
    row.year_window_wrapped = 1;
    let v = values::<AirStatisticsObserver>(&statistics(100, vec![row]));
    assert_eq!(v.len(), 26);
    for (channel, raw) in channels.into_iter().enumerate() {
        assert_eq!(v[&format!("GER@7|3|c{channel}")], i128::from(raw));
    }
    assert_eq!(v["GER@7|3|slot"], 7);
    assert_eq!(v["GER@7|3|wrapped"], 1);
    assert_eq!(air_statistics::divisor(0), 1000);
    for channel in 1..12 {
        assert_eq!(air_statistics::divisor(channel), 100_000);
    }
}

#[test]
fn statistics_absent_is_not_zero_and_wrap_is_not_subtracted() {
    let fixture = Fixture::new();
    let mut wrapped = stat(3, [0; 24]);
    wrapped.year_slot = 1;
    wrapped.year_window_wrapped = 1;
    let mut unknown = statistics(103, vec![]);
    *unknown.tables[TableId::AirStatistics.index()] = TableView::Unavailable {
        reason: "synthetic missing statistics".into(),
    };
    let frames = vec![
        statistics(100, vec![stat(3, [0; 24])]),
        statistics(101, vec![]),
        statistics(102, vec![wrapped]),
        unknown,
    ];
    publish::<AirStatisticsObserver>(&fixture, &frames);
    let reply = answer(
        &fixture,
        frames,
        json!({"branchId":"1","tag":"GER","channel":2}),
        true,
        None,
    );
    assert_eq!(
        runs(&reply, "GER@7|3|c2"),
        &json!([[100, 100, "0", true], [102, 102, "0", true]])
    );
    assert_eq!(
        runs(&reply, "GER@7|3|wrapped"),
        &json!([[100, 100, "0", true], [102, 102, "1", true]])
    );
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 4);
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":100,"lastHour":102}])
    );
}

#[test]
fn queries_select_one_country_and_pair_and_clip_accepted_prefix() {
    let fixture = Fixture::new();
    let frames = vec![
        statistics(
            100,
            vec![
                stat(3, [10; 24]),
                AirStatisticsRow {
                    country: 21,
                    ..stat(4, [90; 24])
                },
            ],
        ),
        statistics(101, vec![stat(3, [20; 24])]),
    ];
    publish::<AirStatisticsObserver>(&fixture, &frames);
    let reply = answer(
        &fixture,
        frames,
        json!({"branchId":"1","tag":"GER@7","channel":10}),
        true,
        Some(100),
    );
    assert_eq!(
        runs(&reply, "GER@7|3|c10"),
        &json!([[100, 100, "10", true]])
    );
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 4);
    assert_eq!(reply["regions"].as_array().unwrap().len(), 1);
    assert!(
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("Game folders"))
    );
    assert!(
        reply["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["ours"] == "Channel name unavailable")
    );
}

#[test]
fn cold_prefix_has_empty_window_not_zero_and_keeps_channel_choices() {
    let fixture = Fixture::new();
    let frames = vec![
        statistics(100, vec![stat(3, [0; 24])]),
        statistics(101, vec![]),
    ];
    let reply = answer(
        &fixture,
        frames,
        json!({"branchId":"1","tag":"GER","channel":3}),
        true,
        Some(99),
    );
    assert_eq!(reply["window"]["known"], json!([]));
    assert_eq!(reply["window"]["series"], json!([]));
    assert_eq!(reply["channels"].as_array().unwrap().len(), 12);
}

#[test]
fn out_of_range_channels_and_views_are_refused() {
    let invalid = serde_json::from_value(json!({"branchId":"1","tag":"GER","channel":12})).unwrap();
    assert!(AirStatistics::validate(&invalid).is_err());
    let invalid =
        serde_json::from_value(json!({"branchId":"1","tag":"GER","view":"wings"})).unwrap();
    assert!(AirMissions::validate(&invalid).is_err());
}

#[test]
fn reached_wings_use_shared_frame_cache_preserve_handles_order_and_counted_refusals() {
    let fixture = Fixture::new();
    let a = AirWingRow {
        id: u64::MAX,
        name: Recorded::Value(Some(Vec::new())),
        ..wing(u64::MAX, 0, 5)
    };
    let frame = missions(100, vec![wing(2, 4, 1), a], &[2, u64::MAX]);
    let source = source(vec![frame]);
    let params = json!({"branchId":"1","tag":"GER","view":"wings","hour":100,"region":3});
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 99,
            complete: false,
        },
        None,
    );
    let cancelled = AtomicBool::new(false);
    let query = Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    let first = air_missions::query(&query).unwrap();
    let reads = dashboard_base::prepared::query::frames::reads(&source);
    let second = air_missions::query(&query).unwrap();
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&source),
        reads
    );
    assert_eq!(first, second);
    assert_eq!(first["wings"][0]["id"], "2");
    assert_eq!(first["wings"][1]["id"], u64::MAX.to_string());
    assert_eq!(first["wings"][1]["planes"], 0);
    assert_eq!(first["wings"][1]["name"], "");
    assert_eq!(first["wings"][1]["missions"], json!(["0", "2"]));
    assert!(first["wings"][1]["efficiency"].is_null());
    assert!(!first["wings"][1]["refusals"].as_array().unwrap().is_empty());
    assert!(!first["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn cancellation_is_respected_before_any_content_or_page_read() {
    let fixture = Fixture::new();
    let source = source(vec![statistics(100, vec![])]);
    let params = json!({"branchId":"1","tag":"GER","channel":3});
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 100,
            complete: true,
        },
        None,
    );
    let cancelled = AtomicBool::new(true);
    let query = Query {
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
        air_statistics::query(&query).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn tag_reuse_keeps_country_slots_separate() {
    let first = missions(100, vec![wing(1, 2, 1)], &[1]);
    let mut second = missions(101, vec![wing(1, 3, 1)], &[1]);
    *second.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 11,
            canonical_slot: 9,
            tag: b"GER".to_vec(),
            ..Default::default()
        }]));
    *second.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 9,
        }]));
    let fixture = Fixture::new();
    publish::<AirMissionsObserver>(&fixture, &[first.clone(), second.clone()]);
    let reply = answer(
        &fixture,
        vec![first, second],
        json!({"branchId":"1","tag":"GER"}),
        false,
        None,
    );
    assert_eq!(runs(&reply, "GER@7|3|m0"), &json!([[100, 100, "2", true]]));
    assert_eq!(runs(&reply, "GER@9|3|m0"), &json!([[101, 101, "3", true]]));
}

#[test]
fn equal_daily_statistics_carry_the_actual_supplying_read() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let cancelled = AtomicBool::new(false);
    let mut scan = Scan::<AirStatisticsObserver>::default();
    scan.prepare(
        FormatVersion::CURRENT,
        &statistics(100, vec![stat(3, [0; 24])]),
        true,
    )
    .unwrap();
    scan.advance_quiet(101, false).unwrap();
    scan.advance_quiet(102, true).unwrap();
    scan.flush(&mut writer, 1, &cancelled).unwrap();
    writer.commit().unwrap();
    let window = query::open::<AirStatisticsObserver>(&fixture.store, &fixture.key, 1, &cancelled)
        .unwrap()
        .unwrap()
        .window(100, 102, None, |name| name.ends_with("|c0"))
        .unwrap();
    assert_eq!(window.reads, [(100, 101, 100), (102, 102, 102)]);
    assert_eq!(window.series[0].runs, [(100, 102, "0".into(), true)]);
}

#[test]
fn reached_wing_efficiency_override_is_exact_unclamped_and_independent_of_names() {
    let fixture = Fixture::new();
    let mut frame = missions(100, vec![wing(1, 10, 1)], &[1]);
    *frame.tables[TableId::AirSortieModifiers.index()] =
        TableView::Complete(TableRows::AirSortieModifiers(vec![
            replay_data::rows::AirSortieModifierRow {
                scope: replay_data::rows::AirSortieModifierRow::EFFICIENCY_OVERRIDE,
                subject: 7,
                region: 0,
                raw: 125_000,
            },
        ]));
    let reply = answer(
        &fixture,
        vec![frame],
        json!({"branchId":"1","tag":"GER","view":"wings","hour":100,"region":3}),
        false,
        None,
    );
    assert_eq!(reply["wings"][0]["efficiency"], "125000");
    assert!(reply["wings"][0]["name"].is_null());
    assert_eq!(reply["wings"][0]["planes"], 10);
}

#[test]
fn theatre_membership_is_joined_at_one_reached_hour_not_unioned_over_history() {
    let fixture = Fixture::new();
    let mut frame = missions(100, vec![wing(1, 10, 1)], &[1]);
    *frame.tables[TableId::AirTheatres.index()] =
        TableView::Complete(TableRows::AirTheatres(vec![
            replay_data::rows::AirTheatreRow {
                id: 900,
                owner: 11,
                ordinal: 0,
                name: b"Fixture theatre".to_vec(),
            },
        ]));
    *frame.tables[TableId::AirTheatreGroups.index()] =
        TableView::Complete(TableRows::AirTheatreGroups(vec![
            replay_data::rows::AirTheatreGroupRow {
                id: 800,
                theatre: 900,
                ordinal: 0,
                name: Vec::new(),
            },
        ]));
    *frame.tables[TableId::AirTheatreMembers.index()] =
        TableView::Complete(TableRows::AirTheatreMembers(vec![
            replay_data::rows::AirTheatreMemberRow {
                wing: 1,
                group: 800,
                ordinal: 0,
            },
        ]));
    publish::<AirMissionsObserver>(&fixture, std::slice::from_ref(&frame));
    let reply = answer(
        &fixture,
        vec![frame],
        json!({"branchId":"1","tag":"GER","hour":100}),
        false,
        None,
    );
    assert_eq!(reply["theatres"][0]["id"], "900");
    assert_eq!(reply["theatres"][0]["regions"], json!(["3"]));
}

#[test]
fn mod_replace_path_regions_and_localisation_name_both_products_without_changing_numbers() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stack = installed_content_core::test_support::Stack::new("dashboard_air_operations_names");
    let base = stack.base();
    let addon = stack.addon();
    stack.write(
        &base,
        "map/strategicregions/base.txt",
        "strategic_region={id=3 name=fixture_base provinces={1}}",
    );
    stack.write(
        &addon,
        "map/strategicregions/mod.txt",
        "strategic_region={id=3 name=fixture_mod provinces={1 2}}",
    );
    let mut loc = String::from(
        "\u{feff}l_english:\n fixture_mod: \"Authored region\"\n AIRWING_MISSION_TYPE_AIR_SUPERIORITY: \"Authored mission\"\n",
    );
    for channel in 1..=24 {
        loc.push_str(&format!(
            " AIRVIEW_DETAILS_CHANNEL_{channel}: \"Authored channel {channel} $VAL$\"\n"
        ));
    }
    stack.write(
        &addon,
        "localisation/english/replace/names_l_english.yml",
        &loc,
    );
    // The mod's own mission strip: the entry template's button names it.
    stack.write(
        &addon,
        "interface/mapicons.gui",
        "guiTypes={containerWindowType={name=air_mission_mapicon_entry buttonType={name=button quadTextureSprite=GFX_fixture_missions}}}",
    );
    stack.write(
        &addon,
        "interface/fixture.gfx",
        "spriteTypes={spriteType={name=GFX_fixture_missions textureFile=gfx/missions.dds noOfFrames=18}}",
    );
    let mut frame = missions(100, vec![wing(1, 9, 1)], &[1]);
    *frame.tables[TableId::AirStatistics.index()] =
        TableView::Complete(TableRows::AirStatistics(vec![stat(3, [37; 24])]));
    // The statistics are read at hour 100; statistics bind countries at their read.
    frame.metrics.table_ns[TableId::AirStatistics.index()] = 1;
    let mut next = frame.clone();
    next.hour = 101;
    let frames = vec![frame, next];
    let _ = test_fixtures::source_with_content(frames.clone(), &base);
    stack.write(
        &base,
        "user/mod/air.mod",
        &format!(
            "name=\"Fixture air names\" path=\"{}\" replace_path=\"map/strategicregions\"",
            addon.to_string_lossy().replace('\\', "/")
        ),
    );
    stack.write(
        &base,
        "user/dlc_load.json",
        r#"{"enabled_mods":["mod/air.mod"],"disabled_dlcs":[]}"#,
    );
    let (options, identity) = test_fixtures::content_identity(&base);
    let mut records = test_fixtures::records(1, frames.clone());
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    let replay =
        replay::container::Replay::from_records(test_fixtures::CAMPAIGN.into(), &[records])
            .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "mod-air".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let fixture = Fixture::new();
    publish::<AirMissionsObserver>(&fixture, &frames);
    publish::<AirStatisticsObserver>(&fixture, &frames);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 101,
            complete: true,
        },
        None,
    );
    let cancelled = AtomicBool::new(false);
    let params = json!({"branchId":"1","tag":"GER","channel":2});
    let mut query = Query {
        source: &source,
        params: &params,
        content: Some(&options),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    let stats = air_statistics::query(&query).unwrap();
    assert_eq!(stats["regions"][0]["name"], "Authored region");
    assert_eq!(stats["channels"][2]["ours"], "Authored channel 3");
    assert_eq!(stats["channels"][2]["enemy"], "Authored channel 15");
    assert_eq!(stats["channels"][10]["ours"], "Authored channel 8");
    assert_eq!(runs(&stats, "GER@7|3|c2"), &json!([[100, 101, "37", true]]));
    let params = json!({"branchId":"1","tag":"GER"});
    query.params = &params;
    let missions = air_missions::query(&query).unwrap();
    assert_eq!(missions["missions"][0]["name"], "Authored mission");
    // Mission bit 0 is the strip's first frame, through the mod's GUI.
    assert_eq!(
        missions["missions"][0]["icon"],
        json!({"key":"GFX_fixture_missions","frame":0})
    );
    assert_eq!(missions["regions"][0]["name"], "Authored region");
    assert_eq!(
        runs(&missions, "GER@7|3|m0"),
        &json!([[100, 101, "9", true]])
    );
}

#[test]
fn reached_wing_efficiency_matches_viewer_for_sorties_overrides_and_missing_fuel() {
    use replay_data::rows::{
        AirMissionInputRow, AirReferenceWitnessRow, AirSortieBaseCountryRow,
        AirSortieModifierMetadataRow, AirSortieModifierRow, ProvinceRow, VolunteerHostRow,
    };
    use viewer_foundation::map::{
        air_reachability::MissionCountrySources, air_registry_sources::AirRegistrySources,
        aircraft_count::SortieSources,
    };
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stack = installed_content_core::test_support::Stack::new("wing_efficiency_parity");
    let base = stack.base();
    stack.write(
        &base,
        "common/defines/sorties.lua",
        "NDefines = { NAir = { MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL = 0.2 } }",
    );
    let wings: Vec<_> = [(1, 5, 100), (2, 5, 10), (3, 6, 20), (4, 5, 0), (5, 6, 30)]
        .into_iter()
        .map(|(id, air_base, planes)| AirWingRow {
            air_base,
            country_override: if id == 2 { 0 } else { 21 },
            ..wing(id, planes, 1 << 4)
        })
        .collect();
    let mut frame = missions(100, wings, &[1, 2, 3, 4, 5]);
    *frame.tables[TableId::AirMissionInputs.index()] =
        TableView::Complete(TableRows::AirMissionInputs(
            (1..=5)
                .map(|wing| AirMissionInputRow {
                    wing,
                    coverage_raw: 90_000,
                    reassignment_penalty_raw: 10_000,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::AirSortieBaseCountries.index()] =
        TableView::Complete(TableRows::AirSortieBaseCountries(
            [5, 6]
                .into_iter()
                .flat_map(|base| {
                    [
                        AirSortieBaseCountryRow {
                            base,
                            country: 7,
                            capacity_raw: 80_000,
                            supply_raw: 95_000,
                            ..Default::default()
                        },
                        AirSortieBaseCountryRow {
                            base,
                            country: 8,
                            fuel_demand_raw: (base == 5).then_some(1_000),
                            fuel_allocated_raw: (base == 5).then_some(700),
                            ..Default::default()
                        },
                    ]
                })
                .collect(),
        ));
    *frame.tables[TableId::AirSortieSpecialBases.index()] =
        TableView::Complete(TableRows::AirSortieSpecialBases(vec![]));
    *frame.tables[TableId::AirSortieModifierMetadata.index()] =
        TableView::Complete(TableRows::AirSortieModifierMetadata(vec![
            AirSortieModifierMetadataRow::default(),
        ]));
    *frame.tables[TableId::AirReferenceWitnesses.index()] =
        TableView::Complete(TableRows::AirReferenceWitnesses(
            [5, 6]
                .into_iter()
                .map(|handle| AirReferenceWitnessRow {
                    handle,
                    base_kind: Some(0),
                    state: Some(2),
                    province: Some(1),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::VolunteerHosts.index()] =
        TableView::Complete(TableRows::VolunteerHosts(vec![VolunteerHostRow {
            country: 21,
            host: 11,
            ..Default::default()
        }]));
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            state: 2,
            controller: 11,
            ..Default::default()
        }]));
    *frame.tables[TableId::AirSortieModifiers.index()] =
        TableView::Complete(TableRows::AirSortieModifiers(vec![
            AirSortieModifierRow {
                scope: AirSortieModifierRow::WEATHER,
                subject: 0,
                region: 3,
                raw: -5_000,
            },
            AirSortieModifierRow {
                scope: AirSortieModifierRow::COUNTRY,
                subject: 8,
                region: 0,
                raw: 2_000,
            },
            AirSortieModifierRow {
                scope: AirSortieModifierRow::COUNTRY_REGION,
                subject: 8,
                region: 3,
                raw: -10_000,
            },
            AirSortieModifierRow {
                scope: AirSortieModifierRow::EFFICIENCY_OVERRIDE,
                subject: 7,
                region: 0,
                raw: 125_000,
            },
        ]));
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let registry = AirRegistrySources::read(&frame).unwrap().unwrap();
    let countries = MissionCountrySources::read(&frame, &links, &registry).unwrap();
    let sources = SortieSources::read(&frame).unwrap().unwrap();
    let expected: Vec<_> = frame
        .complete::<tables::AirWings>()
        .unwrap()
        .iter()
        .map(|wing| {
            let mission = countries.resolve(wing).unwrap();
            assert_eq!(mission.slot, 7); // Volunteer host supplies capacity, not fuel.
            sources.efficiency(
                wing,
                sources.mission(wing.id).unwrap(),
                &registry.bases[&wing.air_base],
                mission.slot,
                if wing.id == 2 { (11, 7) } else { (21, 8) },
                20_000,
            )
        })
        .collect();
    assert_eq!(expected[0], Ok(40_842));
    assert_eq!(expected[1], Ok(125_000));
    assert_eq!(expected[3], expected[0]); // Zero aircraft do not make efficiency zero.
    let missing = "the effective country's base container lacks its fuel";
    assert_eq!(expected[2], Err(missing));
    assert_eq!(expected[4], Err(missing));
    let (source, context) = test_fixtures::source_with_content(vec![frame], &base);
    let fixture = Fixture::new();
    let params = json!({"branchId":"1","tag":"GER","view":"wings","hour":100,"region":3});
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 100,
            complete: true,
        },
        None,
    );
    let cancelled = AtomicBool::new(false);
    let query = Query {
        source: &source,
        params: &params,
        content: context.content.as_ref(),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    let reply = air_missions::query(&query).unwrap();
    for (wing, expected) in reply["wings"].as_array().unwrap().iter().zip(expected) {
        match expected {
            Ok(value) => assert_eq!(wing["efficiency"], value.to_string()),
            Err(reason) => {
                assert!(wing["efficiency"].is_null());
                assert!(
                    wing["refusals"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|value| value == reason)
                );
            }
        }
    }
    assert_eq!(reply["wings"].as_array().unwrap().len(), 5);
    assert!(reply["warnings"].as_array().unwrap().iter().any(|value| {
        value
            .as_str()
            .unwrap()
            .starts_with(&format!("2 air wing fields are not shown: {missing};"))
    }));
}

mod statistics_read_binding;
