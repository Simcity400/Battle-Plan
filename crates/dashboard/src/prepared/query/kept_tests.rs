//! A kept prepared answer, on a product the native driver prepares (equipment losses): it replays
//! only its own diagnostics, whichever answers ran before it.
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
