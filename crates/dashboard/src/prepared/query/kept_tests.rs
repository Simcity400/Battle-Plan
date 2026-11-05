//! The disk-kept reply cache, on a product the native driver prepares (equipment losses): only a
//! complete answer is kept, served by any later session of the recording until the store
//! publishes again, and never beside what its computing drew, fell back on or read outside the
//! recorded coverage.
use super::*;
use crate::native_fixtures::{Recorded, UNIT_HOURS, streams};
use crate::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag};
use replay::{container::Replay, spool::Record};
use replay_data::rows::{EquipmentDestructionCause, EquipmentDestructionRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};

/// The recording's first hour.
const FIRST: i32 = 10;

/// An hour in which the first country loses equipment in land combat, every table the native
/// recording holds read.
fn losses(hour: i32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, 2);
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![
            EquipmentDestructionRow {
                country: 100_000,
                cause: EquipmentDestructionCause::LandCombat,
                variant: 17_u64 << 32
                    | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE),
                amount_raw: 100_001,
                ..Default::default()
            },
        ]));
    for table in STREAMS {
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

// The tables the native recording holds: those equipment losses and the identity history read.
streams!(
    Campaign,
    Countries,
    CountryLinks,
    CombatLogEquipment,
    CombatLogState,
    DivisionCensus,
    EquipmentDestruction,
    LendLeaseEquipment,
    Players,
    SimulationFeatures,
    SliceReads,
    SunkConvoys,
    SunkShips,
    CountryAppearance,
    LexicalTokens,
    VariantDefinitions,
    VariantModules,
    VariantUpgrades,
    VariantChanges,
    CountryCounterInputs,
    CountryExperienceGains,
);

/// A replay of a unit of [`losses`] bound to synthetic content, beside the native recording of
/// its hours, and the content it is bound to.
fn recorded() -> (Recorded, ContentOptions) {
    let recorded = Recorded::new();
    let game = recorded.root().join("disk-content");
    dashboard_base::test_fixtures::snow_content::write(&game);
    // The session a recording of this content starts, which its native capture names.
    let (probe, context) = crate::test_fixtures::source_with_content(vec![losses(FIRST)], &game);
    let mut input = records(1, (FIRST..FIRST + UNIT_HOURS).map(losses).collect());
    input[0] = Record::SessionStart(probe.replay_ref().sessions[0].clone());
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    recorded.record(&replay, &[(1, FIRST, 1)], STREAMS, write_hour);
    (recorded, context.content.unwrap())
}

/// The first country's losses over the branch, drawn `width` points wide: each width another
/// request.
fn request(width: u32) -> serde_json::Value {
    let selection = serde_json::json!({"branchId":"1","tag":String::from_utf8(scale_tag(0)).unwrap(),
        "landSource":"destruction","country":{"slot":0,"countryId":100_000,"originHour":FIRST}});
    serde_json::json!({"selection":selection,"from":FIRST,"to":FIRST + 10,"width":width,
        "reads":[FIRST,FIRST + 10],"headHour":FIRST + 10,"detailHour":FIRST + 10})
}

/// A prepared query's reply data, its answer read under `content`.
fn query_with(
    source: &Source,
    content: &ContentOptions,
    product: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let request = ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::query(source, &request, Some(content), &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

#[test]
fn prepared_answers_replay_only_their_own_diagnostics_in_either_order() {
    use installed_content_core::content::diagnostics::Once;
    thread_local! {
        static SELECTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COMPUTING.with(|hook| hook.set(None));
        }
    }
    fn note(source: &Source, content: &ContentOptions) {
        let index = SELECTED.with(std::cell::Cell::get);
        let entries = source.state::<[Once<()>; 2]>();
        let entries = entries.lock().unwrap();
        entries[index].get_or_init(|| {
            let content = source.content(&content.options).unwrap();
            let _ = content
                .layers
                .read_uncovered(&format!("gfx/prepared-diagnostic-{index}.dds"));
            content
                .layers
                .note_shader_warning(format!("prepared shader {index}"));
            content
                .layers
                .note_overlay_refusal(format!("prepared fallback {index}"));
        });
    }
    engine_executable::executable_tables::initialize_for_tests().expect("supported runtime tables");
    let (recorded, content) = recorded();
    let flag = AtomicBool::new(false);
    crate::prepared::pipeline::run(
        &recorded.source("prepare-diagnostic"),
        Some(&content),
        &flag,
        |_| {},
    )
    .unwrap();
    let _reset = Reset;
    COMPUTING.with(|hook| hook.set(Some(note)));
    let read = |source: &Source, index: usize| {
        SELECTED.with(|selected| selected.set(index));
        let mut params = request(4 + index as u32);
        let selection = params["selection"].take();
        params["family"] = serde_json::json!({"kind":"equipment-losses", "selection":selection});
        params["branchId"] = serde_json::json!("1");
        query_with(source, &content, "series-summary", params)
    };
    let alone: Vec<_> = (0..2)
        .map(|index| read(&recorded.source(&format!("alone-{index}")), index))
        .collect();
    for index in 0..2 {
        let json = alone[index].to_string();
        assert!(json.contains(&format!("prepared shader {index}")));
        assert!(json.contains(&format!("prepared fallback {index}")));
        assert!(json.contains(&format!("gfx/prepared-diagnostic-{index}.dds")));
        assert!(!json.contains(&format!("prepared shader {}", 1 - index)));
    }
    for order in [[0, 1], [1, 0]] {
        let source = recorded.source(&format!("order-{}", order[0]));
        let before = COMPUTED.with(std::cell::Cell::get);
        for _ in 0..3 {
            for index in order {
                assert_eq!(read(&source, index), alone[index]);
            }
        }
        assert_eq!(
            COMPUTED.with(std::cell::Cell::get) - before,
            2,
            "later replies use CachedQuery"
        );
    }
}

#[test]
fn a_complete_prepared_answer_is_kept_on_disk_until_the_store_publishes_again() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content tests");
    assert!(
        dashboard_equipment_losses::products::equipment_losses::columnar::TABLES
            .iter()
            .all(|table| STREAMS.contains(table)),
        "the native recording holds every table equipment losses reads"
    );
    let (recorded, content) = recorded();
    let flag = AtomicBool::new(false);
    let params = request(4);
    let computed = || COMPUTED.with(std::cell::Cell::get);
    // What a session reads ahead once it opens, which a kept answer that used the state
    // templates waits for.
    let read_ahead = |source: &Source| {
        crate::catalogue::catalogue(source, &content)
            .unwrap()
            .read_readers()
    };
    let kept = || {
        crate::reply_cache::settle();
        crate::reply_cache::size(&crate::reply_cache::folder(&content.options))
    };
    let first = recorded.source("disk-first");
    let recording = crate::prepared::driver::native::recording(&first)
        .unwrap()
        .expect("the native recording beside the replay is its own");
    // Only the identity history is prepared: equipment losses still wait for their family.
    crate::prepared::driver::native::measure(
        &first,
        Some(&content),
        recording,
        Some(&["identities"][..]),
        1,
        false,
        &flag,
    )
    .unwrap();
    let before = computed();
    // A reply still waiting for preparation is computed and never kept.
    let cold = query_with(&first, &content, "equipment-losses", params.clone());
    assert_eq!(cold["coverage"]["complete"], false, "{cold}");
    assert_eq!(kept(), 0);
    crate::prepared::pipeline::run(&first, Some(&content), &flag, |_| {}).unwrap();
    let prepared = query_with(&first, &content, "equipment-losses", params.clone());
    assert_eq!(prepared["coverage"]["complete"], true, "{prepared}");
    assert_eq!(computed() - before, 2);
    assert!(kept() > 0, "the complete answer is kept");
    // An answer that used no lazy reader is served from disk as soon as a session opens,
    // before it reads ahead: nothing it reports depends on what has been read.
    let unread = recorded.source("disk-unread");
    assert_eq!(
        query_with(&unread, &content, "equipment-losses", params.clone()),
        prepared
    );
    assert_eq!(
        computed() - before,
        2,
        "answered from disk before reading ahead"
    );
    // Another source of the recording, as a later session opens it, answers from disk.
    let later = recorded.source("disk-later");
    read_ahead(&later);
    assert_eq!(
        query_with(&later, &content, "equipment-losses", params.clone()),
        prepared
    );
    assert_eq!(computed() - before, 2, "answered from disk");
    // Another request's diagnostics do not change a kept answer or its cache policy.
    let other = request(5);
    let noisy = recorded.source("disk-noisy");
    read_ahead(&noisy);
    noisy
        .content(&content.options)
        .unwrap()
        .layers
        .note_shader_warning("drawn earlier".into());
    let served = query_with(&noisy, &content, "equipment-losses", params.clone());
    assert_eq!(computed() - before, 2, "served from disk there");
    assert!(!served.to_string().contains("drawn earlier"), "{served}");
    assert_eq!(served, prepared);
    let size = kept();
    query_with(&noisy, &content, "equipment-losses", other.clone());
    assert_eq!(computed() - before, 3);
    assert!(
        kept() > size,
        "kept independently of another request's diagnostic"
    );
    let quiet = recorded.source("disk-quiet");
    query_with(&quiet, &content, "equipment-losses", other.clone());
    assert_eq!(
        computed() - before,
        3,
        "served independently of other diagnostics"
    );
    assert!(kept() > size, "and kept, as nothing was drawn");
    let disk_other = recorded.source("disk-other");
    read_ahead(&disk_other);
    query_with(&disk_other, &content, "equipment-losses", other);
    assert_eq!(computed() - before, 3, "answered from disk");
    // An answer outside the in-memory cache is kept too, under every page the recording has.
    let identities = serde_json::json!({"branchId":"1"});
    let size = kept();
    let fresh = query_with(&quiet, &content, "country-identities", identities.clone());
    assert_eq!(computed() - before, 4);
    assert!(kept() > size, "kept");
    let reopened = recorded.source("disk-identities");
    read_ahead(&reopened);
    assert_eq!(
        query_with(&reopened, &content, "country-identities", identities),
        fresh
    );
    assert_eq!(computed() - before, 4, "answered from disk");
    // An answer whose computing noted a fallback is not kept: served from disk, it would leave
    // the fallback out of the stack's read warnings, which every reply names.
    let third = request(6);
    let computing =
        |note: Option<fn(&Source, &ContentOptions)>| COMPUTING.with(|hook| hook.set(note));
    let size = kept();
    computing(Some(|source: &Source, content: &ContentOptions| {
        source
            .content(&content.options)
            .unwrap()
            .layers
            .note_overlay_refusal("synthetic fallback".into())
    }));
    query_with(
        &recorded.source("disk-fallback"),
        &content,
        "equipment-losses",
        third.clone(),
    );
    computing(None);
    assert_eq!(computed() - before, 5);
    assert_eq!(kept(), size, "not kept beside a fallback");
    // A text refusal noted while computing is the answer's own, named wherever it is served,
    // and every other reply names only its own: the answer is kept.
    computing(Some(|source: &Source, content: &ContentOptions| {
        crate::catalogue::catalogue(source, content)
            .unwrap()
            .refuse("synthetic reason", "SYNTHETIC")
    }));
    query_with(
        &recorded.source("disk-refused"),
        &content,
        "equipment-losses",
        third.clone(),
    );
    computing(None);
    assert_eq!(computed() - before, 6);
    assert!(kept() > size, "kept beside its own text refusal");
    query_with(
        &recorded.source("disk-plain"),
        &content,
        "equipment-losses",
        third,
    );
    assert_eq!(computed() - before, 6, "answered from disk");
    // Cache eligibility no longer waits for unrelated lazy readers.
    let fourth = request(7);
    computing(Some(|_: &Source, _: &ContentOptions| {
        dashboard_text::catalogue::used(dashboard_text::catalogue::Reader::States)
    }));
    let size = kept();
    query_with(
        &recorded.source("disk-states-unread"),
        &content,
        "equipment-losses",
        fourth.clone(),
    );
    assert_eq!(computed() - before, 7);
    assert!(kept() > size, "kept with its own stable diagnostics");
    let states_read = recorded.source("disk-states-read");
    read_ahead(&states_read);
    query_with(&states_read, &content, "equipment-losses", fourth.clone());
    assert_eq!(computed() - before, 7);
    assert!(kept() > size, "kept once it is");
    query_with(
        &recorded.source("disk-states-fresh"),
        &content,
        "equipment-losses",
        fourth.clone(),
    );
    assert_eq!(
        computed() - before,
        7,
        "served without warming unrelated readers"
    );
    let states_ready = recorded.source("disk-states-ready");
    read_ahead(&states_ready);
    query_with(&states_ready, &content, "equipment-losses", fourth);
    computing(None);
    assert_eq!(computed() - before, 7, "answered from disk where it is");
    // An unrelated uncovered read neither decorates nor prevents caching another answer.
    let outside = recorded.source("disk-outside");
    read_ahead(&outside);
    let outside_read = "gfx/event_pictures/outside_coverage.dds";
    let layers = &outside.content(&content.options).unwrap().layers;
    assert!(layers.read_uncovered(outside_read).is_err());
    let served = query_with(&outside, &content, "equipment-losses", params.clone());
    assert_eq!(computed() - before, 7, "served from disk there");
    assert!(!served.to_string().contains(outside_read), "{served}");
    assert_eq!(served, prepared);
    let size = kept();
    let fifth = request(8);
    query_with(&outside, &content, "equipment-losses", fifth);
    assert_eq!(computed() - before, 8);
    assert!(
        kept() > size,
        "kept independently of unrelated uncovered reads"
    );
    // Preparing again after a Clear publishes new pages: the kept answer is not served.
    crate::prepared::files(&later, Some(&content))
        .unwrap()
        .0
        .clear()
        .unwrap();
    crate::prepared::pipeline::run(&later, Some(&content), &flag, |_| {}).unwrap();
    let again = recorded.source("disk-again");
    assert_eq!(
        query_with(&again, &content, "equipment-losses", params),
        prepared
    );
    assert_eq!(computed() - before, 9, "computed for the new publication");
}
