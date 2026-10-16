use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn tooltip_projection_retains_effects_modifiers_and_real_balance_category_binding() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "effect-decision-tooltips-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("common/decisions/categories")).unwrap();
    std::fs::create_dir_all(root.join("common/bop")).unwrap();
    std::fs::write(
        root.join("common/decisions/categories/a.txt"),
        b"SYN_CATEGORY={name=IGNORED power_balance=IGNORED} SYN_OTHER={}",
    )
    .unwrap();
    std::fs::write(
        root.join("common/bop/a.txt"),
        b"SYN_BALANCE={decision_category=SYN_CATEGORY}",
    )
    .unwrap();
    std::fs::write(root.join("common/decisions/a.txt"), b"SYN_CATEGORY={SYN_DECISION={name=SYN_NAME complete_effect={custom_effect_tooltip=SELECT} remove_effect={custom_effect_tooltip=REMOVE} days_remove=var:duration modifier={political_power_gain=1}}} SYN_OTHER={OTHER={}}").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let database = read(&layers).unwrap();
    assert_eq!(
        database.category(b"SYN_CATEGORY").unwrap(),
        Some(b"SYN_CATEGORY".as_slice())
    );
    let decision = database.decision(b"SYN_DECISION").unwrap().unwrap();
    assert!(decision.power_balance);
    assert_eq!(
        decision.complete_effect[0].value.scalar(),
        Some(b"SELECT".as_slice())
    );
    assert_eq!(
        decision.remove_effect[0].value.scalar(),
        Some(b"REMOVE".as_slice())
    );
    assert_eq!(decision.days_remove, b"var:duration");
    assert!(decision.modifier[0].keyword_is("political_power_gain"));
    assert!(!database.decision(b"OTHER").unwrap().unwrap().power_balance);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn decision_projection_preserves_names_seed_and_highlight_inputs() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    append(
        &mut database,
        br#"@days = 5 category = {
        sample = { name = ALT fixed_random_seed = yes fixed_random_seed = invalid
            days_mission_timeout = @days fire_only_once = yes
            highlight_states = { highlight_state_targets = {
                state = var:one state = var:ROOT.two_@FROM
                target_array = candidate_states targets = { 7 9 }
                target_root_trigger = { always = yes } target_trigger = { impassable = no }
            } highlight_states_trigger = { is_core_of = ROOT } }
        }
        default = { name = "" fire_only_once = yes }
        sample = { name = DUPLICATE }
    }"#,
        "synthetic/decisions.txt",
        &[b"category".to_vec()].into(),
        &KeyNames::default(),
    );
    let sample = database.decision(b"SAMPLE").unwrap().unwrap();
    assert_eq!(sample.name, b"ALT");
    assert!(sample.mission && sample.fixed_random_seed);
    assert!(sample.has_state_targets);
    assert_eq!(
        sample.target_states,
        [
            b"var:one".to_vec(),
            b"var:ROOT.two_@FROM".to_vec(),
            b"7".to_vec(),
            b"9".to_vec()
        ]
    );
    assert_eq!(sample.target_arrays, [b"candidate_states".to_vec()]);
    assert!(sample.highlight_conditions[0].keyword_is("is_core_of"));
    assert!(sample.target_trigger[0].keyword_is("impassable"));
    assert!(sample.target_root_trigger[0].keyword_is("always"));
    assert_eq!(sample.seed_component, seed_component(b"sample"));
    assert_eq!(sample.seed_component, -1_763_474_777);
    assert_eq!(
        seed_component(b"Sample\0ignored"),
        seed_component(b"SAMPLE")
    );
    let default = database.decision(b"default").unwrap().unwrap();
    assert_eq!(default.name, b"default");
    assert!(!default.mission && !default.fixed_random_seed);
    assert!(!default.has_state_targets);
    assert!(
        database
            .notes
            .iter()
            .any(|note| note.contains("duplicate decision"))
    );
}

#[test]
fn tooltip_projection_preserves_targeted_modifier_boundaries_and_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    append(&mut database, b"category={sample={targeted_modifier={tag=BBB political_power_gain=0.1} targeted_modifier={tag=ROOT political_power_gain=0.2}}}",
        "synthetic/decisions.txt", &[b"category".to_vec()].into(), &KeyNames::default());
    let decision = database.decision(b"sample").unwrap().unwrap();
    // Walk each static modifier separately rather than one merged body.
    assert_eq!(decision.targeted_modifier.len(), 2);
    assert_eq!(
        decision.targeted_modifier[0][0].value.scalar(),
        Some(b"BBB".as_slice())
    );
    assert_eq!(
        decision.targeted_modifier[1][0].value.scalar(),
        Some(b"ROOT".as_slice())
    );
    assert_eq!(
        crate::modifiers::targeted(&decision.targeted_modifier[0])
            .modifiers
            .values[0]
            .value,
        10_000
    );
    assert_eq!(
        crate::modifiers::targeted(&decision.targeted_modifier[1])
            .modifiers
            .values[0]
            .value,
        20_000
    );
}

#[test]
fn empty_highlight_target_retains_presence_without_refusing_membership_lookup() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    append(&mut database, br#"category = {
        empty = { highlight_states = { highlight_state_targets = {} } }
        trigger = { highlight_states = { highlight_states_trigger = {} } }
        unported = { highlight_states = { highlight_state_targets = { country = ROOT state_target = FROM } } }
    }"#, "synthetic/decisions.txt", &[b"category".to_vec()].into(), &KeyNames::default());
    assert!(
        database
            .decision(b"empty")
            .unwrap()
            .unwrap()
            .has_state_targets
    );
    let trigger = database.decision(b"trigger").unwrap().unwrap();
    assert!(!trigger.has_state_targets && trigger.highlight_conditions.is_empty());
    let refusal = database
        .decision(b"unported")
        .unwrap()
        .unwrap()
        .refused
        .as_deref()
        .unwrap();
    assert!(refusal.contains("country") && refusal.contains("state_target"));
}

#[test]
fn content_stack_uses_effective_files_and_first_definitions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "effect-content-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let base = root.join("base");
    let overlay = root.join("overlay");
    for folder in [&base, &overlay] {
        std::fs::create_dir_all(folder.join("common/decisions/categories")).unwrap();
        std::fs::create_dir_all(folder.join("common/doctrines/folders")).unwrap();
    }
    std::fs::write(base.join("common/decisions/categories/a.txt"), b"cat={}").unwrap();
    std::fs::write(base.join("common/decisions/a.txt"), b"cat={x={name=BASE}}").unwrap();
    std::fs::write(
        overlay.join("common/decisions/a.txt"),
        b"cat={x={name=OVERLAY}}",
    )
    .unwrap();
    std::fs::write(
        overlay.join("common/decisions/b.txt"),
        b"cat={x={name=IGNORED}} missing={y={name=NO}}",
    )
    .unwrap();
    std::fs::write(
        base.join("common/doctrines/folders/a.txt"),
        b"land={name=BASE}",
    )
    .unwrap();
    std::fs::write(
        overlay.join("common/doctrines/folders/b.txt"),
        b"land={name=OVERLAY}",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
    let database = read(&layers).unwrap();
    assert_eq!(database.decision(b"x").unwrap().unwrap().name, b"OVERLAY");
    assert!(database.decision(b"y").unwrap().is_none());
    assert!(
        database
            .notes
            .iter()
            .any(|note| note.contains("not registered"))
    );
    let folders = crate::doctrines::read(&layers).unwrap();
    assert_eq!(
        folders.folder(b"land_doctrine").unwrap().unwrap().name,
        b"OVERLAY"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn decision_icons_use_triggered_text_occurrence_and_unknown_rules() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    append(
        &mut database,
        br#"category={
        single={icon={text=ONLY trigger={always=no}}}
        many={icon={text=FIRST trigger={always=no}} icon={text=SECOND trigger={always=yes}}}
    }"#,
        "synthetic/icons.txt",
        &[b"category".to_vec()].into(),
        &KeyNames::default(),
    );
    let one = database.decision(b"single").unwrap().unwrap();
    assert_eq!(
        one.select_icon(|_| panic!("one icon bypasses its trigger"))
            .unwrap(),
        Some("ONLY")
    );
    let many = database.decision(b"many").unwrap().unwrap();
    assert_eq!(
        many.select_icon(|trigger| Ok(trigger[0].value.scalar() == Some(b"yes".as_slice())))
            .unwrap(),
        Some("SECOND")
    );
    assert_eq!(many.select_icon(|_| Ok(false)).unwrap(), Some("FIRST"));
    assert_eq!(
        many.select_icon(|_| Err("unknown native operand".into()))
            .unwrap_err(),
        "unknown native operand"
    );
    assert_eq!(
        many.sprite_candidates(Some("SECOND")).unwrap(),
        [
            "GFX_decision_many",
            "GFX_decision_SECOND",
            "SECOND",
            "GFX_decision_unknown"
        ]
    );
}
