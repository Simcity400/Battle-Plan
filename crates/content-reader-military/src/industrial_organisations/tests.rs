use super::*;

#[test]
fn display_projection_keeps_name_initial_trait_and_include_overrides() {
    content_reader_script::test_game::enter();
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
fn display_fields_follow_the_one_pass_include_order() {
    content_reader_script::test_game::enter();
    let mut database = Database::default();
    let names = KeyNames::default();
    database
        .read_file(
            b"a={include=b} b={include=c} c={name=C_NAME initial_trait={name=C_TRAIT}} d={include=b} e={include=missing name=E_NAME}",
            &names,
        )
        .unwrap();
    let inherited = (b"C_NAME".to_vec(), Some(b"C_TRAIT".to_vec()));
    assert_eq!(database.display_fields(b"b").unwrap(), inherited);
    assert_eq!(database.display_fields(b"a").unwrap(), (Vec::new(), None));
    assert_eq!(database.display_fields(b"D").unwrap(), inherited);
    assert_eq!(
        database.display_fields(b"e").unwrap(),
        (b"E_NAME".to_vec(), None)
    );
    database.read_file(b"self={include=SELF}", &names).unwrap();
    assert!(
        database
            .display_fields(b"self")
            .unwrap_err()
            .contains("includes itself")
    );
}

#[test]
fn a_stray_closing_brace_ends_the_organisations_file() {
    content_reader_script::test_game::enter();
    let mut database = Database::default();
    database
        .read_file(b"kept={name=KEPT} } after={name=AFTER}", &KeyNames::default())
        .unwrap();
    assert_eq!(database.template(b"kept").unwrap().name, b"KEPT");
    assert!(database.template(b"after").is_err());
}

#[test]
fn visibility_projection_keeps_first_definition_and_include_delete_tokens() {
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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
