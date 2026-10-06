use super::*;

#[test]
fn decision_projection_preserves_native_names_seed_and_highlight_inputs() {
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
fn empty_highlight_target_retains_presence_without_refusing_membership_lookup() {
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
fn content_stack_uses_effective_files_and_native_first_definitions() {
    let root = std::env::temp_dir().join(format!(
        "effect-content-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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
