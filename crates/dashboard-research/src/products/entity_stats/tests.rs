use super::*;
use dashboard_base::{
    products::at_hour::entry,
    test_fixtures::{CAMPAIGN, records, source},
};
use content_reader_key_names::equipment::StatValue;
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
        ("DESIGNER_MANPOWER", "Manpower: "),
        ("PRODUCTION_MATERIALS_STEEL", "Steel"),
        ("AIRWING_MISSION_TYPE_CAS", "Close Air Support"),
        ("deck", "Flight Deck"),
        ("EQ_MOD_SLOT_main_TITLE", "Main Slot"),
        ("motor", "$missing_name$"),
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
    sheet.equipment(METADATA, &[1_200_000, 0, 50_000], &BTreeMap::new());
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
        unread: Vec::new(),
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
    let fields = content_reader_parser::parser::parse(b"is_attacker = yes").unwrap().items;
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
    let (database, _) = content_reader_key_names::equipment::fixture::load(&[(
        "common/units/equipment/convoys.txt",
        "equipment_modules = {
            gun = { add_stats = { lg_attack = 2 } multiply_stats = { lg_attack = 0.25 } }
            motor = { add_stats = { torpedo_attack = 1 } }
            bad = { add_stats = { lg_attack = unknown_amount } }
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
            hull_6 = { archetype = hull module_slots = inherit default_modules = { main = bad } }
         }",
    )]);
    let metadata = content_reader_script::equipment_stats::stat_metadata();
    let stat = |name: &str| content_reader_script::equipment_stats::stat_index(name.as_bytes()).unwrap();
    let names = localisation();
    let totals = |token: &str| equipment_totals(&database, metadata, &names, token);
    let hull_1 = totals("hull_1").unwrap().stats;
    assert_eq!((hull_1[stat("lg_attack")], hull_1[stat("hg_attack")]), (450_000, 40_000), "the default gun adds 2 light attack, scaled to 3 and multiplied to 4.5");
    assert_eq!(hull_1[stat("torpedo_attack")], 100_000, "the engine slot inherited with the archetype keeps the archetype's default engine");
    assert_eq!(totals("hull_2").unwrap().stats[stat("lg_attack")], 0, "an empty slot adds nothing");
    let hull_4 = totals("hull_4").unwrap().stats;
    assert_eq!((hull_4[stat("lg_attack")], hull_4[stat("torpedo_attack")]), (450_000, 0), "a slot's default comes from the nearest of parent and archetype that names it, and an explicit empty overrides the archetype's");
    let refused = totals("hull_3").err().unwrap();
    assert!(refused.contains("lost_gun") && refused.contains("hull_3"), "{refused}");
    let hull_5 = totals("hull_5").unwrap();
    assert_eq!(hull_5.lineage[0].resources, [(b"steel".to_vec(), Ok(100_000))], "the equipment's own resources keep their names and amounts");
    assert_eq!(hull_5.modules.iter().map(|(source, _)| source.as_str()).collect::<Vec<_>>(), ["Default module Flight Deck in Main Slot", "Default module motor (name unavailable) in engine (name unavailable)"], "a slot names itself by its title, and a name whose text cannot be made is named as unavailable, never blank");
    assert!(hull_5.withheld.is_empty(), "known values are never withheld: {:?}", hull_5.withheld);
    let hull_6 = totals("hull_6").unwrap();
    assert_eq!(hull_6.withheld, BTreeMap::from([(stat("lg_attack"), "Default module bad (name unavailable) in Main Slot: lg_attack = unknown_amount is not a number".to_owned())]), "an unreadable module term withholds the stat it feeds, never a total without it");
}

#[test]
fn a_modules_conditional_stats_and_costs_are_shown_with_their_condition_and_only_unreadable_ones_are_withheld() {
    let localisation = localisation();
    let mut sheet = Sheet::new(&localisation);
    let design = content_reader_key_names::equipment::ModuleDesign {
        mission_terms: vec![(vec![b"cas".to_vec()], StatGroup { add: vec![StatValue { index: 0, value: 600_000 }], ..Default::default() })],
        manpower: Some(Ok(50_000_000)),
        build_resources: vec![(b"steel".to_vec(), Ok(200_000)), (b"chromium".to_vec(), Err("chromium = lots is not a number".into()))],
        ..Default::default()
    };
    module_design(&mut sheet, METADATA, &design, Some("Default module Flight Deck"));
    let shown: Vec<_> = sheet.stats.iter().map(|stat| (shown(&stat.label), stat.source.clone())).collect();
    assert_eq!(
        shown,
        [
            ("Soft Attack: ".to_owned(), Some("Default module Flight Deck · On Close Air Support missions".to_owned())),
            ("Manpower: ".to_owned(), Some("Default module Flight Deck".to_owned())),
            ("Steel".to_owned(), Some("Default module Flight Deck · Production resources".to_owned())),
        ]
    );
    assert!(shown_value(&sheet, 0).contains('6') && shown_value(&sheet, 1) == "500" && shown_value(&sheet, 2) == "2", "{:?}", sheet.stats);
    assert_eq!(sheet.refusals, BTreeMap::from([("Default module Flight Deck · Production resources: chromium = lots is not a number".to_owned(), 1)]));
}

#[test]
fn a_module_stat_with_an_unreadable_term_is_withheld_whole() {
    let localisation = localisation();
    let mut sheet = Sheet::new(&localisation);
    let why = "soft_attack = unknown_amount is not a number";
    let stats = StatGroup { add: vec![StatValue { index: 0, value: 200_000 }], unread: vec![(0, why.to_owned())], ..Default::default() };
    sheet.module(METADATA, &stats);
    assert!(sheet.stats.is_empty(), "a sum missing one of its terms is never shown: {:?}", lines(&sheet));
    assert_eq!(sheet.refusals, BTreeMap::from([(why.to_owned(), 1)]));
}

#[test]
fn an_unreadable_module_multiplier_withholds_only_the_stats_a_module_scales_with_it() {
    let (database, _) = content_reader_key_names::equipment::fixture::load(&[(
        "common/units/equipment/convoys.txt",
        "equipment_modules = { gun = { add_stats = { lg_attack = 2 } } motor = { add_stats = { torpedo_attack = 1 } } }
         equipments = {
            hull = { is_archetype = yes type = convoy lg_attack = 2
                module_add_stats_multiplier = { lg_attack = unknown_amount }
                module_slots = { main = { allowed_module_categories = { any } } } }
            hull_1 = { archetype = hull module_slots = inherit }
            hull_2 = { archetype = hull module_slots = inherit default_modules = { main = gun } }
            hull_3 = { archetype = hull module_slots = inherit default_modules = { main = motor } }
         }",
    )]);
    let metadata = content_reader_script::equipment_stats::stat_metadata();
    let lg_attack = content_reader_script::equipment_stats::stat_index(b"lg_attack").unwrap();
    let names = localisation();
    let totals = |token: &str| equipment_totals(&database, metadata, &names, token).unwrap();
    assert!(totals("hull_1").withheld.is_empty(), "X-S10: with no module adding light attack, the unread multiplier scales nothing, so the known base attack stays: {:?}", totals("hull_1").withheld);
    assert_eq!(totals("hull_1").stats[lg_attack], 200_000);
    assert!(totals("hull_2").withheld.get(&lg_attack).is_some_and(|why| why.contains("unknown_amount")), "a module adding light attack is scaled by it, so the total is withheld");
    assert!(totals("hull_3").withheld.is_empty(), "a module scaling another stat is not");
}

#[test]
fn a_buildings_sheet_shows_its_recorded_maximum_level_and_names_what_it_leaves_out() {
    let localisation = localisation();
    let mut sheet = Sheet::new(&localisation);
    let building = content_reader_key_names::buildings::BuildingType { name: "mod_bunker".into(), province_max: Some(5), local_resources: vec![("steel".into(), 1)], ..Default::default() };
    sheet.building(&building, &content_reader_defines::defines::Defines::new().unwrap());
    assert_eq!(lines(&sheet), ["Maximum level|5"], "O-S2, X-S11: the sheet is the building's recorded facts, never empty");
    assert_eq!(sheet.stats[0].source.as_deref(), Some("Per province"));
    assert!(sheet.refusals.keys().any(|why| why.contains("local resources")), "{:?}", sheet.refusals);
}

fn shown_value(sheet: &Sheet<'_>, at: usize) -> String {
    shown(&sheet.stats[at].value)
}

fn recording() -> Source {
    let frames = (20..=30).map(|hour| source(1, hour, None)).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "entity-stats".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
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

#[test]
fn a_label_whose_text_cannot_be_made_is_withheld_by_name_never_blank() {
    let localisation: Localisation = [("DESIGNER_MANPOWER", "$missing_name$")].into_iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect();
    let mut sheet = Sheet::new(&localisation);
    sheet.costs(Some(&Ok(50_000_000)), &Vec::new());
    assert!(sheet.stats.is_empty(), "a blank label is never shown beside a value: {:?}", lines(&sheet));
    assert!(sheet.refusals.keys().any(|why| why.contains("missing_name")), "{:?}", sheet.refusals);
}

#[test]
fn an_averaged_stat_whose_qualifier_cannot_be_made_is_withheld_whole() {
    let localisation: Localisation = [("STAT_SOFT", "Soft Attack: "), ("STAT_SOFT_DIFF", " $VALUE|=+1$"), ("EQUIPMENT_MODULE_STAT_AVERAGED", "$missing_name$")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let mut sheet = Sheet::new(&localisation);
    sheet.module(METADATA, &StatGroup { average: vec![StatValue { index: 0, value: 100_000 }], ..Default::default() });
    assert!(sheet.stats.is_empty(), "an averaged value is never shown without its qualifier: {:?}", lines(&sheet));
    assert!(sheet.refusals.keys().any(|why| why.contains("missing_name")), "{:?}", sheet.refusals);
}
