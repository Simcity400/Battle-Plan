use super::*;

#[test]
fn visibility_projection_keeps_first_definition_and_include_delete_tokens() {
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
    let root = std::env::temp_dir().join(format!("hoi4_mio_projection_{}", std::process::id()));
    let directory = root.join("common/military_industrial_organization/organizations");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("a_good.txt"), b"good={visible={always=yes}}").unwrap();
    std::fs::write(directory.join("b_bad.txt"), b"bad={include={base}}").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let db = read(&layers, &KeyNames::default()).unwrap();
    assert_eq!(db.template(b"good").unwrap().visible.len(), 1);
    assert!(
        db.template(b"bad")
            .unwrap_err()
            .contains("field `include` is a block")
    );
    std::fs::remove_dir_all(root).unwrap();
}
