use super::*;

#[test]
fn folder_names_keep_assignment_and_suffix_lookup() {
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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

#[test]
fn doctrine_presentation_follows_layer_overrides_replacements_and_reused_objects() {
    content_reader_script::test_game::enter();
    let stack = content_reader_layers::test_support::Stack::new("doctrine_presentation");
    let (base, addon) = (stack.base(), stack.addon());
    for (path, text) in [
        (
            "common/doctrines/folders/a.txt",
            "Mod_Folder={name=BASE_NAME tab_gfx=GFX_Base_Tab} Other={name=OTHER}",
        ),
        (
            "common/doctrines/tracks/a.txt",
            "Mod_Track={name=BASE_TRACK icon=GFX_Base_Track}",
        ),
        (
            "common/doctrines/grand_doctrines/hidden.txt",
            "Hidden={name=HIDDEN folder=Mod_Folder tracks={Mod_Track}}",
        ),
        (
            "common/doctrines/subdoctrines/hidden.txt",
            "Hidden_Sub={track=Mod_Track}",
        ),
    ] {
        stack.write(&base, path, text);
    }
    for (path, text) in [
        (
            "common/doctrines/folders/z.txt",
            "MOD_FOLDER={tab_gfx=GFX_Mod_Tab} New_Folder={name=NEW}",
        ),
        (
            "common/doctrines/tracks/a.txt",
            "Mod_Track={name=MOD_TRACK icon=GFX_Mod_Track}",
        ),
        (
            "common/doctrines/grand_doctrines/a.txt",
            "Mod_Grand={name=GRAND folder=mod_folder icon=GFX_Mod_Grand tracks={Mod_Track}} Second={name=SECOND folder=MOD_FOLDER icon=GFX_Second tracks={Mod_Track}} Elsewhere={folder=Mod_Folder}",
        ),
        (
            "common/doctrines/grand_doctrines/z.txt",
            "MOD_GRAND={icon=GFX_Override} SECOND={name=LAST} ELSEWHERE={folder=Other}",
        ),
        (
            "common/doctrines/subdoctrines/mod/a.txt",
            "@mod_icon=GFX_Mod_Sub Mixed_Sub={name=SUB track=Mod_Track icon=@mod_icon rewards={Tier={mastery=1.25}}}",
        ),
        (
            "common/doctrines/subdoctrines/z.txt",
            "MIXED_SUB={name=LAST_SUB}",
        ),
    ] {
        stack.write(&addon, path, text);
    }
    let layers = stack.layers(&[
        "common/doctrines/grand_doctrines",
        "common/doctrines/subdoctrines",
    ]);
    let database = read(&layers).unwrap();
    let token = Token::c_str;
    assert_eq!(
        database.folder_order,
        [token(b"Mod_Folder"), token(b"Other"), token(b"New_Folder")]
    );
    let folder = database
        .folder_presentation(b"MOD_FOLDER")
        .unwrap()
        .unwrap();
    assert_eq!(folder.token, b"Mod_Folder");
    assert_eq!(folder.icon, b"GFX_Mod_Tab");
    assert_eq!(
        database.folder_exact(b"mod_folder").unwrap().unwrap().name,
        b"BASE_NAME"
    );
    let grand = database.grand_in_folder(b"mOD_fOLDER").unwrap();
    assert_eq!(
        grand
            .iter()
            .map(|entry| entry.token.as_slice())
            .collect::<Vec<_>>(),
        [b"Mod_Grand".as_slice(), b"Second"]
    );
    assert_eq!(grand[0].icon, b"GFX_Override");
    assert_eq!(grand[1].icon, b"GFX_Second");
    assert!(database.name(0, b"Hidden").unwrap().is_none());
    assert!(database.presentation(1, b"Hidden_Sub").unwrap().is_none());
    let sub = database.presentation(1, b"MIXED_SUB").unwrap().unwrap();
    assert_eq!(sub.token, b"Mixed_Sub");
    assert_eq!(sub.icon, b"GFX_Mod_Sub");
    assert_eq!(
        database
            .presentation(2, b"mod_track")
            .unwrap()
            .unwrap()
            .icon,
        b"GFX_Mod_Track"
    );
    assert_eq!(database.name(0, b"second").unwrap().unwrap().value, b"LAST");
    assert_eq!(
        database.name(1, b"mixed_sub").unwrap().unwrap().value,
        b"LAST_SUB"
    );
    assert_eq!(
        database.name(2, b"mod_track").unwrap().unwrap().value,
        b"MOD_TRACK"
    );
    assert_eq!(
        database.tactic_sources.grand_order,
        [
            token(b"Mod_Grand"),
            token(b"Second"),
            token(b"Elsewhere"),
            token(b"MOD_GRAND"),
            token(b"SECOND"),
            token(b"ELSEWHERE")
        ]
    );
    let source = &database.tactic_sources.sub[&token(b"mixed_sub")];
    assert_eq!(source.tracks, [token(b"Mod_Track")]);
    assert_eq!(source.rewards[0].token, b"Tier");
    assert_eq!(source.rewards[0].mastery, 125_000);
}
