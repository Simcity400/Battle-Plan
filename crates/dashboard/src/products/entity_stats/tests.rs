use super::*;
use crate::{
    products::at_hour::entry,
    test_fixtures::{CAMPAIGN, records, source},
};
use content_reader_core::equipment::StatValue;
use replay::campaign::Replay;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const METADATA: &Metadata = &[("soft_attack", "STAT_SOFT", true), ("build_cost_ic", "STAT_COST", false), ("reliability", "STAT_RELIABILITY", true)];

fn localisation() -> Localisation {
    [
        ("STAT_SOFT", "Soft Attack: "),
        ("STAT_SOFT_VALUE", "$VALUE|H1$"),
        ("STAT_SOFT_DIFF", " $VALUE|=+1$"),
        ("STAT_COST", "Production Cost: "),
        ("STAT_COST_DIFF", " $VALUE|=-2$"),
        ("STAT_RELIABILITY", "Reliability: "),
        ("STAT_MODIFIER_DIFF", "$VALUE|=+0%$"),
        ("STAT_MODIFIER_DIFF_NEG", "$VALUE|=-0%$"),
        ("EQUIPMENT_MODULE_STAT_AVERAGED", " (averaged)"),
        ("TACTICS_DAMAGE_ATTACKER", "Attacker tactic damage:"),
        ("TACTICS_DAMAGE_DEFENDER", "Defender tactic damage:"),
        ("TACTICS_COMBATWIDTH", "Tactics combat width:"),
        ("TACTICS_MOVEMENTSPEED_ATTACKER", "Tactics movement:"),
        ("TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER", "Attacker organization damage modifier:"),
        ("TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER", "Defender organization damage modifier:"),
        ("TACTIC_LIST_VALUE_GOOD", "$VAL|+=0%$"),
        ("TACTIC_LIST_VALUE_BAD", "$VAL|-=0%$"),
        ("TACTIC_LIST_VALUE_NEUTRAL", "$VAL|=Y0%$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

fn shown(runs: &[GameRun]) -> String {
    runs.iter()
        .map(|run| match run {
            GameRun::Text { text, tone: Some(tone) } => format!("[{tone}]{text}"),
            GameRun::Text { text, tone: None } => text.clone(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn lines(sheet: &Sheet<'_>) -> Vec<String> {
    sheet.stats.iter().map(|stat| format!("{}|{}", shown(&stat.label), shown(&stat.value))).collect()
}

#[test]
fn equipment_shows_every_held_stat_in_the_game_format_and_refuses_unformatted_ones_by_name() {
    let localisation = localisation();
    let mut sheet = Sheet::new(&localisation);
    sheet.equipment(METADATA, &[1_200_000, 0, 50_000]);
    assert_eq!(lines(&sheet), ["Soft Attack: |[H]12.0"]);
    assert_eq!(sheet.refusals, BTreeMap::from([("STAT_RELIABILITY has no value format".to_owned(), 1)]));
}

#[test]
fn module_stats_sign_their_changes_like_the_game() {
    let localisation = localisation();
    let mut sheet = Sheet::new(&localisation);
    let stats = StatGroup {
        add: vec![StatValue { index: 0, value: 200_000 }, StatValue { index: 1, value: 150_000 }],
        average: vec![StatValue { index: 0, value: -100_000 }],
        multiply: vec![StatValue { index: 2, value: -10_000 }],
    };
    sheet.module(METADATA, &stats);
    let shown = lines(&sheet);
    assert_eq!(shown.len(), 4, "{shown:?}");
    assert!(shown[0].starts_with("Soft Attack: | ") && shown[0].contains("2.0"), "{shown:?}");
    assert!(shown[1].starts_with("Production Cost: | ") && shown[1].contains("1.50"), "{shown:?}");
    assert!(shown[2].ends_with(" (averaged)") && shown[2].contains("-1.0"), "{shown:?}");
    assert!(shown[3].starts_with("Reliability: |") && shown[3].contains("-10%"), "{shown:?}");
    assert!(sheet.refusals.is_empty(), "{:?}", sheet.refusals);
}

#[test]
fn tactics_show_all_six_effects_favoured_by_side() {
    let localisation = localisation();
    let mut sheet = Sheet::new(&localisation);
    let fields = content_reader_core::parser::parse(b"is_attacker = yes").unwrap().items;
    let tactic = Tactic { id: 1, token: b"mod_tactic".to_vec(), fields, phase: 0, display_phase: 0, effects: [10_000, 0, 25_000, 0, -10_000, 0], unproven: None };
    let mut warnings = Vec::new();
    sheet.tactic(&tactic, &mut warnings);
    let shown = lines(&sheet);
    assert_eq!(shown.len(), 6, "{shown:?}");
    assert!(shown[0].starts_with("Attacker tactic damage:|") && shown[0].contains("25%"), "{shown:?}");
    assert!(shown[1].starts_with("Defender tactic damage:|") && shown[1].contains("-10%"), "{shown:?}");
    assert!(shown[2].starts_with("Tactics combat width:|") && shown[2].contains("10%"), "{shown:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

fn recording() -> Source {
    let frames = (20..=30).map(|hour| source(1, hour, None)).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(crate::SourceKey { source_id: "entity-stats".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn stats_without_the_recorded_content_are_withheld_by_name() {
    let request = json!({ "branchId": "1", "kind": "equipment", "token": "infantry_equipment_1", "hour": 25 });
    let text = (entry::<EntityStatsAt>().registered.load)(&recording(), "lease", request, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
    let reply: Value = serde_json::from_str::<Value>(&text).unwrap()["data"].take();
    assert_eq!(reply["stats"], json!([]));
    assert_eq!(reply["kind"], "equipment");
    assert!(reply["unavailable"].as_str().unwrap().contains("recorded content"), "{reply}");
}

#[test]
fn a_request_without_a_token_is_refused() {
    let params = EntityStatsParams { branch_id: "1".into(), kind: EntityKind::Tactic, token: String::new() };
    assert_eq!(EntityStatsAt::validate(&params), Err("Entity stats need a content token"));
    assert!(EntityStatsAt::tables(&params).is_empty(), "a stats sheet is recorded content alone, so playback never reads or recomputes it");
}
