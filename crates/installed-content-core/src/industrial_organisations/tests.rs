use super::*;

#[test]
fn display_projection_keeps_name_initial_trait_and_include_overrides() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    database.read_file(b"Base={name=BASE_NAME initial_trait={name=BASE_TRAIT}} Child={include=Base initial_trait={}} Local={include=Base name=LOCAL_NAME initial_trait={name=LOCAL_TRAIT}} Deleted={include=Base delete_included_values={name initial_trait}} TraitDeleted={include=Base initial_trait={delete_included_values={name}}}", &KeyNames::default()).unwrap();
    assert_eq!(database.template(b"BASE").unwrap().token, b"Base");
    assert_eq!(
        database.display_fields(b"Child").unwrap(),
        (b"BASE_NAME".to_vec(), Some(b"BASE_TRAIT".to_vec()))
    );
    assert_eq!(
        database.display_fields(b"Local").unwrap(),
        (b"LOCAL_NAME".to_vec(), Some(b"LOCAL_TRAIT".to_vec()))
    );
    assert_eq!(
        database.display_fields(b"Deleted").unwrap(),
        (Vec::new(), None)
    );
    assert_eq!(
        database.display_fields(b"TraitDeleted").unwrap(),
        (b"BASE_NAME".to_vec(), Some(Vec::new()))
    );
    assert!(database.notes.is_empty(), "{:?}", database.notes);
}

#[test]
fn visibility_projection_keeps_first_definition_and_include_delete_tokens() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut db = Database::default();
    let names = KeyNames::default();
    db.read_file(b"base={visible={always=no}} child={include=base visible={always=yes} initial_trait={irrelevant={nested=yes}} delete_included_values={visible available}} child={visible={always=no}}", &names).unwrap();
    let child = db.template(b"CHILD").unwrap();
    assert_eq!(child.include.as_deref(), Some(b"base".as_slice()));
    assert!(child.delete_visible);
    assert_eq!(child.visible.len(), 1);
    assert_eq!(db.notes.len(), 1);
    assert!(db.notes[0].contains("duplicate definition skipped (count: 1)"));
    db.read_file(b"CHILD={include=different visible={always=no}}", &names)
        .unwrap();
    assert_eq!(
        db.template(b"child").unwrap().include.as_deref(),
        Some(b"base".as_slice())
    );
    assert_eq!(db.notes.len(), 2);
    assert!(db.template(b"unregistered").is_err());
}

#[test]
fn a_misread_template_does_not_drop_the_database() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!("hoi4_mio_projection_{}", std::process::id()));
    let directory = root.join("common/military_industrial_organization/organizations");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("a_good.txt"), b"good={visible={always=yes}}").unwrap();
    std::fs::write(directory.join("b_bad.txt"), b"bad={include={base}}").unwrap();
    let policies = root.join("common/military_industrial_organization/policies");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(
        policies.join("synthetic.txt"),
        b"synthetic_policy={name=POLICY}",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let db = read(&layers, &KeyNames::default()).unwrap();
    assert_eq!(
        db.policies.policy(b"synthetic_policy").unwrap().name,
        b"POLICY"
    );
    assert_eq!(db.template(b"good").unwrap().visible.len(), 1);
    assert!(
        db.template(b"bad")
            .unwrap_err()
            .contains("field `include` is a block")
    );
    std::fs::remove_dir_all(root).unwrap();
}
