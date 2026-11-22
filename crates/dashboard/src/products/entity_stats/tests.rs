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

#[test]
fn equipment_totals_apply_each_slots_inherited_default_module_before_reliability_scaling_and_name_what_they_withhold() {
    let (database, _) = content_reader_core::equipment::fixture::load(&[(
        "common/units/equipment/convoys.txt",
        "equipment_modules = {
            gun = { add_stats = { lg_attack = 2 } multiply_stats = { lg_attack = 0.25 } }
            motor = { add_stats = { torpedo_attack = 1 } }
            deck = { manpower = 500 build_cost_resources = { steel = 2 } mission_type_stats = { limit = { cas } add_stats = { lg_attack = 6 } } }
         }
         equipments = {
            hull = { is_archetype = yes type = convoy reliability = 0.8
                scaled_by_reliability = { hg_attack = 0.5 }
                module_add_stats_multiplier = { lg_attack = 0.5 }
                module_multiply_stats_multiplier = { lg_attack = 1.0 }
                module_slots = { main = { allowed_module_categories = { any } } engine = { allowed_module_categories = { any } } }
                default_modules = { engine = motor } }
            hull_1 = { archetype = hull module_slots = inherit default_modules = { main = gun } }
            hull_2 = { archetype = hull module_slots = inherit default_modules = { main = empty } }
            hull_3 = { archetype = hull module_slots = inherit default_modules = { main = lost_gun } }
            hull_4 = { archetype = hull parent = hull_1 module_slots = inherit default_modules = { engine = empty } }
            hull_5 = { archetype = hull module_slots = inherit default_modules = { main = deck } resources = { steel = 1 } }
         }",
    )]);
    let metadata = content_reader_core::equipment::stat_metadata();
    let stat = |name: &str| content_reader_core::equipment::stat_index(name.as_bytes()).unwrap();
    let totals = |token: &str| equipment_totals(&database, metadata, token);
    let hull_1 = totals("hull_1").unwrap().stats;
    assert_eq!((hull_1[stat("lg_attack")], hull_1[stat("hg_attack")]), (450_000, 40_000), "the default gun adds 2 light attack, scaled to 3 and multiplied to 4.5");
    assert_eq!(hull_1[stat("torpedo_attack")], 100_000, "the engine slot inherited with the archetype keeps the archetype's default engine");
    assert_eq!(totals("hull_2").unwrap().stats[stat("lg_attack")], 0, "an empty slot adds nothing");
    let hull_4 = totals("hull_4").unwrap().stats;
    assert_eq!((hull_4[stat("lg_attack")], hull_4[stat("torpedo_attack")]), (450_000, 0), "a slot's default comes from the nearest of parent and archetype that names it, and an explicit empty overrides the archetype's");
    let refused = totals("hull_3").err().unwrap();
    assert!(refused.contains("lost_gun") && refused.contains("hull_3"), "{refused}");
    let withheld = totals("hull_5").unwrap().withheld;
    for named in ["resources of hull_5", "Default module deck in slot main: manpower 500 ", "resource cost steel 2 ", "lg_attack 6 (added) applies only on cas missions"] {
        assert!(withheld.iter().any(|why| why.contains(named)), "{named} is named as withheld: {withheld:?}");
    }
    assert!(totals("hull_1").unwrap().withheld.is_empty(), "nothing is withheld from equipment whose stats all come from stat blocks");
}

#[test]
fn a_modules_stats_outside_its_stat_block_are_named_as_withheld() {
    let design = content_reader_core::equipment::ModuleDesign {
        mission_terms: vec![(vec![b"cas".to_vec()], StatGroup { add: vec![StatValue { index: 0, value: 600_000 }], ..Default::default() }), (Vec::new(), StatGroup { multiply: vec![StatValue { index: 2, value: 10_000 }], ..Default::default() })],
        manpower: Some(50_000_000),
        build_resources: vec![(b"steel".to_vec(), 200_000)],
        ..Default::default()
    };
    assert_eq!(
        off_sheet(METADATA, &design),
        [
            "soft_attack 6 (added) applies only on cas missions under mission_type_stats; mission-conditional stats are not shown",
            "reliability 0.1 (multiplied) applies only under mission_type_stats; mission-conditional stats are not shown",
            "manpower 500 is outside the stat block and is not shown",
            "resource cost steel 2 is outside the stat block and is not shown",
        ]
    );
    assert!(off_sheet(METADATA, &content_reader_core::equipment::ModuleDesign::default()).is_empty());
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
