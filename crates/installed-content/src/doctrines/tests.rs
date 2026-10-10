use super::*;

#[test]
fn folder_names_keep_assignment_and_suffix_lookup() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    let names = KeyNames::default();
    append(
        &mut database,
        br#"@name = FIRST land = { name = @name }
        LAND = { color_frame = 3 } land_doctrine = { name = EXACT }
        ignored = { "name" = QUOTED }"#,
        "synthetic/folders.txt",
        &names,
    );
    assert_eq!(database.folder(b"LAND").unwrap().unwrap().name, b"FIRST");
    assert_eq!(
        database.folder(b"land_doctrine").unwrap().unwrap().name,
        b"EXACT"
    );
    assert!(
        database
            .folder(b"ignored")
            .unwrap()
            .unwrap()
            .name
            .is_empty()
    );
    database.entries.remove(b"land_doctrine".as_slice());
    assert_eq!(
        database.folder(b"land_doctrine").unwrap().unwrap().name,
        b"FIRST"
    );
    assert!(database.folder_exact(b"land_doctrine").unwrap().is_none());
    assert!(database.folder(b"LAND_DOCTRINE").unwrap().is_none());
    append(
        &mut database,
        b"land = { name = LAST }",
        "synthetic/overlay.txt",
        &names,
    );
    assert_eq!(database.folder(b"land").unwrap().unwrap().name, b"LAST");
}

#[test]
fn bound_folder_names_refuse_the_reached_formatter_only() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = Database::default();
    append(&mut database, b"a = { name = { localization_key = TITLE value = VALUE } } b = { name = OK } c = { name = formatter|TOKEN }", "synthetic/folders.txt", &KeyNames::default());
    assert!(database.folder(b"a").unwrap_err().contains("bound name"));
    assert_eq!(database.folder(b"b").unwrap().unwrap().name, b"OK");
    assert!(
        database
            .folder(b"c")
            .unwrap_err()
            .contains("formatted name")
    );
}
