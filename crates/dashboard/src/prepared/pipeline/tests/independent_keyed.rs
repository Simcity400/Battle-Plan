//! Keyed families prepared on their own (`dashboard_keyed`): registered outside
//! the hourly walk, they answer their product queries cold with an empty
//! window and every recorded hour pending, and once prepared with the whole
//! branch's runs, as the air missions family shows.
use super::*;
use crate::test_fixtures::world_frame;
use replay_data::rows::{
    AirRegionalMissionRow, AirWingRow, CountryOccupationReserveManpowerRow, OccupationSettingsRow,
};

fn missions(hour: i32, planes: i32) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    let wing = AirWingRow {
        id: 40,
        owner: 9001,
        planes,
        mission_mask: 0b101,
        mission_region: Some(3),
        ..Default::default()
    };
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![wing]));
    *frame.tables[TableId::AirRegionalMissions.index()] =
        TableView::Complete(TableRows::AirRegionalMissions(vec![
            AirRegionalMissionRow {
                wing: 40,
                ordinal: 0,
                active_order: None,
            },
        ]));
    frame.metrics.table_ns[TableId::AirRegionalMissions.index()] = 1;
    frame
}

fn query(source: &Source, product: &str, params: serde_json::Value) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

/// A prepared query's reply data, its answer read under `content`.
fn query_with(
    source: &Source,
    content: &ContentOptions,
    product: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, Some(content), &AtomicBool::new(false))
            .unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

#[test]
fn a_complete_prepared_answer_is_kept_on_disk_until_the_store_publishes_again() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content tests");
    let frames: Vec<_> = (10..=20).map(|hour| missions(hour, 24)).collect();
    let placeholder = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut fixture = Fixture::new(&placeholder);
    let game = fixture.root.join("disk-content");
    dashboard_base::test_fixtures::snow_content::write(&game);
    let (bound, context) = crate::test_fixtures::source_with_content(frames, &game);
    fixture.replace(&Replay::open(bound.replay_ref().bytes().to_vec()).unwrap());
    let content = context.content.unwrap();
    let flag = AtomicBool::new(false);
    let params = serde_json::json!({"branchId":"1","tag":"GER","view":"history"});
    let computed = || super::super::super::query::COMPUTED.with(std::cell::Cell::get);
    // What a session reads ahead once it opens, which a kept answer waits for.
    let read_ahead =
        |source: &Source| crate::catalogue::catalogue(source, &content).unwrap().read_readers();
    let kept = || {
        crate::reply_cache::settle();
        crate::reply_cache::size(&crate::reply_cache::folder(&content.options))
    };
    let first = fixture.source("disk-first");
    let before = computed();
    // A reply still waiting for preparation is computed and never kept.
    let cold = query_with(&first, &content, "air-missions", params.clone());
    assert_eq!(cold["coverage"]["complete"], false);
    assert_eq!(kept(), 0);
    run(&first, Some(&content), &flag, |_| {}).unwrap();
    let prepared = query_with(&first, &content, "air-missions", params.clone());
    assert_eq!(prepared["coverage"]["complete"], true, "{prepared}");
    assert_eq!(computed() - before, 2);
    assert!(kept() > 0, "the complete answer is kept");
    // A session whose catalogue has not read all it reads lazily computes the answer: serving it
    // would leave that unread, and other replies' warnings without it.
    let unread = fixture.source("disk-unread");
    assert_eq!(
        query_with(&unread, &content, "air-missions", params.clone()),
        prepared
    );
    assert_eq!(computed() - before, 3, "computed before the session reads ahead");
    // Another source of the recording, as a later session opens it, answers from disk.
    let later = fixture.source("disk-later");
    read_ahead(&later);
    assert_eq!(
        query_with(&later, &content, "air-missions", params.clone()),
        prepared
    );
    assert_eq!(computed() - before, 3, "answered from disk");
    // A kept answer is served whatever the session holds: a diagnostic drawn earlier there joins
    // the reply as it is served, as it would a fresh one. An answer computed while the content
    // holds a drawing diagnostic is not kept, as memoised art it took may have drawn it.
    let other = serde_json::json!({"branchId":"1","tag":"ITA","view":"history"});
    let noisy = fixture.source("disk-noisy");
    read_ahead(&noisy);
    noisy
        .content(&content.options)
        .unwrap()
        .layers
        .note_shader_warning("drawn earlier".into());
    let served = query_with(&noisy, &content, "air-missions", params.clone());
    assert_eq!(computed() - before, 3, "served from disk there");
    assert!(served.to_string().contains("drawn earlier"), "{served}");
    let size = kept();
    query_with(&noisy, &content, "air-missions", other.clone());
    assert_eq!(computed() - before, 4);
    assert_eq!(kept(), size, "not kept beside a drawing diagnostic");
    let quiet = fixture.source("disk-quiet");
    query_with(&quiet, &content, "air-missions", other.clone());
    assert_eq!(computed() - before, 5, "computed where none was kept");
    assert!(kept() > size, "and kept, as nothing was drawn");
    let disk_other = fixture.source("disk-other");
    read_ahead(&disk_other);
    query_with(&disk_other, &content, "air-missions", other);
    assert_eq!(computed() - before, 5, "answered from disk");
    // An answer outside the in-memory cache is kept too, under every page the recording has.
    let identities = serde_json::json!({"branchId":"1"});
    let size = kept();
    let fresh = query_with(&quiet, &content, "country-identities", identities.clone());
    assert_eq!(computed() - before, 6);
    assert!(kept() > size, "kept");
    let reopened = fixture.source("disk-identities");
    read_ahead(&reopened);
    assert_eq!(
        query_with(&reopened, &content, "country-identities", identities),
        fresh
    );
    assert_eq!(computed() - before, 6, "answered from disk");
    // An answer whose computing noted a fallback is not kept: served from disk, it would leave
    // the fallback out of the stack's read warnings, which every reply names.
    let third = serde_json::json!({"branchId":"1","tag":"GER","view":"history","firstHour":12,"lastHour":18});
    let computing = |note: Option<fn(&Source, &ContentOptions)>| {
        super::super::super::query::COMPUTING.with(|hook| hook.set(note))
    };
    let size = kept();
    computing(Some(|source: &Source, content: &ContentOptions| {
        source
            .content(&content.options)
            .unwrap()
            .layers
            .note_overlay_refusal("synthetic fallback".into())
    }));
    query_with(&fixture.source("disk-fallback"), &content, "air-missions", third.clone());
    computing(None);
    assert_eq!(computed() - before, 7);
    assert_eq!(kept(), size, "not kept beside a fallback");
    // A text refusal noted while computing is the answer's own, named wherever it is served,
    // and every other reply names only its own: the answer is kept.
    computing(Some(|source: &Source, content: &ContentOptions| {
        crate::catalogue::catalogue(source, content)
            .unwrap()
            .refuse("synthetic reason", "SYNTHETIC")
    }));
    query_with(&fixture.source("disk-refused"), &content, "air-missions", third.clone());
    computing(None);
    assert_eq!(computed() - before, 8);
    assert!(kept() > size, "kept beside its own text refusal");
    let plain = fixture.source("disk-plain");
    read_ahead(&plain);
    query_with(&plain, &content, "air-missions", third);
    assert_eq!(computed() - before, 8, "answered from disk");
    // Preparing again after a Clear publishes new pages: the kept answer is not served.
    super::super::super::files(&later, Some(&content))
        .unwrap()
        .0
        .clear()
        .unwrap();
    run(&later, Some(&content), &flag, |_| {}).unwrap();
    let again = fixture.source("disk-again");
    assert_eq!(
        query_with(&again, &content, "air-missions", params),
        prepared
    );
    assert_eq!(computed() - before, 9, "computed for the new publication");
    super::super::super::files(&again, Some(&content))
        .unwrap()
        .0
        .clear()
        .unwrap();
}

#[test]
fn keyed_families_are_prepared_on_their_own_and_answer_cold_and_prepared() {
    for name in [
        "air-missions",
        "air-statistics",
        "regional-weather",
        "minefields",
        "naval-supremacy",
        "doctrine-mastery",
        "occupation-reserve",
        "intel-network-coverage",
        "naval-repairs",
        "naval-refits",
    ] {
        assert!(independent::registered(name), "{name} is registered");
        assert!(
            !independent::hourly(name),
            "{name} stays out of the hourly walk"
        );
    }
    let frames: Vec<_> = (10..=20)
        .map(|hour| missions(hour, if hour < 15 { 24 } else { 12 }))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("keyed-air-missions");
    let params = serde_json::json!({"branchId":"1","tag":"GER","view":"history"});
    let cold = query(&source, "air-missions", params.clone());
    assert_eq!(cold["window"]["series"], serde_json::json!([]));
    assert_eq!(cold["coverage"]["complete"], false);
    assert_eq!(
        cold["pending"],
        serde_json::json!([{"firstHour":10,"lastHour":20}])
    );
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    let prepared = query(&source, "air-missions", params);
    assert_eq!(prepared["coverage"]["complete"], true, "{prepared}");
    assert_eq!(prepared["pending"], serde_json::json!([]));
    let series = prepared["window"]["series"].as_array().unwrap();
    let runs = |name: &str| {
        series
            .iter()
            .find(|item| item["name"] == name)
            .map(|item| item["runs"].clone())
            .unwrap_or_default()
    };
    // Two missions on one wing: its planes in each, once in wings.
    assert_eq!(
        runs("GER@7|3|m0"),
        serde_json::json!([[10, 14, "24", true], [15, 20, "12", false]])
    );
    assert_eq!(runs("GER@7|3|m2"), runs("GER@7|3|m0"));
    assert_eq!(
        runs("GER@7|3|wings"),
        serde_json::json!([[10, 20, "1", true]])
    );
    assert_eq!(
        prepared["window"]["known"],
        serde_json::json!([{"firstHour":10,"lastHour":20}])
    );
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// GER holds ITA's 500 in reserve; ITA becomes RSI at hour 130, between the
/// occupation walk's daily reads at 123 and 147. As recorded, that walk's read
/// is timed on OccupationSettings alone.
fn reserve(hour: i32) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    if hour >= 130
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"RSI".to_vec();
    }
    *frame.tables[TableId::OccupationSettings.index()] =
        TableView::Complete(TableRows::OccupationSettings(
            [21, 9001]
                .map(|country| OccupationSettingsRow {
                    country,
                    ..Default::default()
                })
                .into(),
        ));
    *frame.tables[TableId::CountryOccupationReserveManpower.index()] =
        TableView::Complete(TableRows::CountryOccupationReserveManpower(vec![
            CountryOccupationReserveManpowerRow {
                country: 9001,
                ordinal: 0,
                source_country: 21,
                amount: 500,
            },
        ]));
    frame.metrics.table_ns[TableId::OccupationSettings.index()] = u64::from(hour % 24 == 3);
    frame
}

#[test]
fn occupation_reserve_waits_for_the_next_walk_read_after_an_identity_change_in_blocks_as_hourly() {
    let frames: Vec<_> = (123..=160).map(reserve).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut fixture = Fixture::new(&replay);
    for hourly in [false, true] {
        use crate::prepared::prepass;
        let source = fixture.source(&format!("keyed-occupation-reserve-{hourly}"));
        let prepare = || run(&source, None, &AtomicBool::new(false), |_| {});
        // A refused pre-pass worker leaves every block to hourly observation.
        prepass::with_test_prepass(1, None, || {
            prepass::with_test_refused_workers(usize::from(hourly), prepare)
        })
        .unwrap();
        let reply = query(
            &source,
            "occupation-reserve-manpower",
            serde_json::json!({"branchId":"1"}),
        );
        assert_eq!(
            reply["window"]["known"],
            serde_json::json!([{"firstHour":123,"lastHour":129},{"firstHour":147,"lastHour":160}]),
            "hourly {hourly}: {reply}"
        );
        assert_eq!(
            reply["window"]["reads"],
            serde_json::json!([[123, 129, 123], [147, 160, 147]])
        );
        let runs = |name: &str| {
            reply["window"]["series"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == name)
                .map(|item| item["runs"].clone())
                .unwrap_or_default()
        };
        assert_eq!(
            runs("GER@7|0|amount"),
            serde_json::json!([[123, 129, "500", true], [147, 160, "500", true]])
        );
        assert_eq!(
            runs("ITA@8|holder"),
            serde_json::json!([[123, 129, "1", true]])
        );
        assert_eq!(
            runs("RSI@8|holder"),
            serde_json::json!([[147, 160, "1", true]])
        );
        super::super::super::files(&source, None)
            .unwrap()
            .0
            .clear()
            .unwrap();
    }
}
