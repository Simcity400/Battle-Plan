use super::*;

fn parsed(text: &[u8]) -> Ideologies {
    Ideologies {
        groups: parse(text),
        ..Ideologies::default()
    }
}

fn color(ideologies: &Ideologies, token: &str) -> Result<[f32; 4], String> {
    ideologies.get(token).unwrap().unwrap().color.clone()
}

#[cfg(feature = "test-support")]
fn fixture(name: &str, files: &[(&str, &str)]) -> (Ideologies, Vec<String>) {
    let root = std::env::temp_dir().join(format!(
        "content-reader-ideologies-{name}-{}",
        std::process::id()
    ));
    let directory = root.join("common/ideologies");
    std::fs::create_dir_all(&directory).unwrap();
    for (file, source) in files {
        std::fs::write(directory.join(file), source).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (ideologies, warnings) = read(&layers).unwrap();
    drop(layers);
    std::fs::remove_dir_all(root).unwrap();
    let messages = warnings.into_iter().map(|(_, message)| message).collect();
    (ideologies, messages)
}

#[cfg(feature = "test-support")]
#[test]
fn duplicate_groups_keep_the_first_file_and_skip_the_later_body() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (db, warnings) = fixture(
        "duplicates",
        &[
            (
                "b.txt",
                "ideologies = { A = { types = { lost = { odd = { x = 1 } } } color = { 0 1 0 } } b = { types = { second = { } } } }",
            ),
            (
                "a.txt",
                "ideologies = { a = { types = { first = { } } color = { 1 0 0 } } a = { types = { also_lost = { } } } }",
            ),
        ],
    );
    assert_eq!(
        db.groups()
            .0
            .iter()
            .map(|group| group.token.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(db.get("a").unwrap().unwrap().types, ["first"]);
    assert_eq!(color(&db, "a"), Ok([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(db.group_of("second").unwrap().unwrap().token, "b");
    let refusals: Vec<_> = warnings
        .iter()
        .filter(|warning| warning.contains("Duplicate database id"))
        .collect();
    assert_eq!(refusals.len(), 2, "{warnings:?}");
    assert!(
        refusals
            .iter()
            .all(|warning| warning.contains("`a`") && warning.contains("common/ideologies/a.txt"))
    );
}

#[cfg(feature = "test-support")]
#[test]
fn top_level_stop_keeps_the_prefix_and_only_stops_its_file() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (db, warnings) = fixture(
        "top-stop",
        &[
            (
                "a.txt",
                "ideologies = { a = { } } unexpected = { } ideologies = { ignored = { } }",
            ),
            ("b.txt", "ideologies = { b = { } }"),
        ],
    );
    assert_eq!(
        db.groups()
            .0
            .iter()
            .map(|group| group.token.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning.contains("file reader stops here"))
            .count(),
        1
    );
}

#[cfg(feature = "test-support")]
#[test]
fn later_blocks_in_the_defining_file_reset_group_contents() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (db, warnings) = fixture(
        "same-file",
        &[(
            "a.txt",
            "ideologies = { a = { color = { 1 0 0 } types = { old = { } } } b = { color = { 1 0 0 } types = { omitted = { } } } } ideologies = { A = { types = { new = { } } } }",
        )],
    );
    assert_eq!(db.get("a").unwrap().unwrap().types, ["new"]);
    assert_eq!(color(&db, "a"), Ok(WHITE));
    assert_eq!(color(&db, "b"), Ok(WHITE));
    assert!(db.get("b").unwrap().unwrap().types.is_empty());
    assert_eq!(db.frame("b"), Ok(Some(u32::MAX)));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn groups_keep_colours_and_sub_ideologies() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let ideologies = parsed(
            b"ideologies = { democratic = { types = { conservatism = { } liberalism = { } } color = { 0 0 255 } rules = { a = yes } } \
              fascism = { types = { nazism = { } } color = { 150 75 0 128 } } odd = { color = { 0.1 0.5 0.5 } } \
              red = { color = { 1 0 0 } } plain = { } }",
        );
    assert_eq!(color(&ideologies, "democratic"), Ok([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(
        color(&ideologies, "fascism"),
        Ok([150.0 / 255.0, 75.0 / 255.0, 0.0, 128.0])
    );
    assert_eq!(color(&ideologies, "odd"), Ok([0.1, 0.5, 0.5, 1.0]));
    assert_eq!(color(&ideologies, "red"), Ok([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(color(&ideologies, "plain"), Ok([1.0; 4]));
    assert_eq!(
        ideologies.group_of("liberalism").unwrap().unwrap().token,
        "democratic"
    );
    assert!(ideologies.group_of("unknown").unwrap().is_none());
}

#[test]
fn groups_and_sub_ideologies_are_found_whatever_their_case() {
    let ideologies = parsed(b"ideologies = { Democratic = { types = { Mod_Subtype = { } } } }");
    for sub_ideology in ["Mod_Subtype", "mod_subtype", "MOD_SUBTYPE"] {
        assert_eq!(
            ideologies.group_of(sub_ideology).unwrap().unwrap().token,
            "democratic"
        );
    }
    let group = ideologies.get("DEMOCRATIC").unwrap().unwrap();
    assert_eq!(group.types, ["Mod_Subtype"]);
    assert!(ideologies.get("democratic").unwrap().is_some());
    assert!(ideologies.group_of("mod_subtype_other").unwrap().is_none());
}

#[test]
fn colours_follow_component_reads_in_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let ideologies = parsed(
            b"@r = 2 ideologies = { a = { color = { 0 0 255 } COLOR = { @r x 0.5 } } b = { color = 16711680 } \
              c = { color = { 1tail 0 0 } } }",
        );
    assert_eq!(color(&ideologies, "a"), Ok([2.0 / 255.0, 0.0, 0.5, 1.0]));
    assert!(
        color(&ideologies, "b")
            .unwrap_err()
            .contains("the packed colour 16711680")
    );
    assert_eq!(color(&ideologies, "c"), Ok([1.0, 0.0, 0.0, 1.0]));
    let ideologies = parsed(
        b"ideologies = { d = { color = { 510 0 0 } color = { bad 0 0 } } \
              e = { color = \"123\" } f = { color = { \"12\" 0 0 } } }",
    );
    assert_eq!(color(&ideologies, "d"), Ok([2.0 / 255.0, 0.0, 0.0, 1.0]));
    assert_eq!(color(&ideologies, "e"), Ok([1.0; 4]));
    assert_eq!(color(&ideologies, "f"), Ok([12.0 / 255.0, 0.0, 0.0, 1.0]));
}

#[test]
fn a_later_complete_colour_replaces_a_packed_one() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let ideologies = parsed(
        b"ideologies = { a = { color = 16711680 color = { 0 255 0 } } \
              b = { color = 16711680 color = { 0 0 x } } c = { color = { 0 0 0 } color = 16711680 } }",
    );
    assert_eq!(color(&ideologies, "a"), Ok([0.0, 1.0, 0.0, 1.0]));
    for token in ["b", "c"] {
        assert!(
            color(&ideologies, token)
                .unwrap_err()
                .contains("the packed colour 16711680"),
            "{token}"
        );
    }
}

#[test]
fn a_short_colour_reads_its_groups_closing_brace() {
    let ideologies = parsed(b"ideologies = { b = { color = { 0 0 } } c = { } }");
    assert_eq!(color(&ideologies, "b"), Ok([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(ideologies.get("c"), Ok(None));
}

#[test]
fn a_group_that_is_not_text_stops_the_file() {
    let (read, _, _) = read_file_in(
        b"ideologies = { a = { } \"\xfe\" = { } b = { } }",
        &"".into(),
        &BTreeMap::new(),
    );
    let (found, cause) = groups(&read.items);
    assert_eq!(found.len(), 1);
    assert_eq!(
        cause.as_deref(),
        Some(
            "an ideology group's token is the stored text `\\xfe` is not UTF-8, which the port's text cannot hold"
        )
    );
}

#[test]
fn a_tagged_colour_leaves_its_block_to_the_group_reader() {
    let (read, _, _) = read_file_in(
        b"ideologies = { fascism = { color = rgb { 150 75 0 } types = { nazism = { } } } }",
        &"".into(),
        &BTreeMap::new(),
    );
    let (groups, _) = groups(&read.items);
    assert_eq!(groups[0].token, "fascism");
    assert_eq!(groups[0].color, Ok([1.0; 4]));
    assert!(groups[0].types.is_empty());
    assert!(
        read.misreads[0]
            .what
            .contains("where the colour reader reads a number or `{`"),
        "{:?}",
        read.misreads
    );
}

#[test]
fn a_sub_ideologys_other_keys_are_its_modifier() {
    let (read, undecided, _) = read_file_in(
            b"ideologies = { a = { types = { x = { political_power_factor = 0.1 odd = { @v = 1 } } } } }",
            &"".into(),
            &BTreeMap::new(),
        );
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
    assert_eq!(undecided[0].name, "odd");
}

#[test]
fn the_replay_classes_hold_for_ideologies() {
    let (read, _, _) = read_file_in(
            b"@blue = 255 ideologies = { a = { \"color\" = { 1 2 3 } COLOR = { 0 0 @blue } Types = { x = { } } } \
              b = { can_be_boosted = { color = { 9 9 9 } } } }",
            &"".into(),
            &BTreeMap::new(),
        );
    let (found, _) = groups(&read.items);
    assert_eq!(found[0].color, Ok([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(found[0].types, ["x"]);
    assert_eq!(
        found[1].color,
        Ok([9.0 / 255.0, 9.0 / 255.0, 9.0 / 255.0, 1.0])
    );
    assert!(
        read.misreads[0]
            .what
            .contains("field `can_be_boosted` is a block")
    );
    let (read, _, _) = read_file_in(
        b"ideologies = { c = { types = { y = { } ",
        &"".into(),
        &BTreeMap::new(),
    );
    assert!(read.misreads.is_empty());
    assert_eq!(groups(&read.items).0[0].types, ["y"]);
}

#[cfg(feature = "test-support")]
#[test]
fn a_group_frame_is_its_place_in_the_defining_block() {
    let root =
        std::env::temp_dir().join(format!("content-reader-ideology-frames-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "common/ideologies/a.txt",
        "ideologies = { neutrality = { } Fascism = { } fascism = { } communism = { } }",
    );
    write(
        "common/ideologies/b.txt",
        "ideologies = { COMMUNISM = { } democratic = { } }",
    );
    write(
        "common/ideologies/c.txt",
        "ideologies = { anarchism = { } } ideologies = { monarchism = { } } \
             ideologies = { monarchism = { } syndicalism = { } }",
    );
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (ideologies, _) = read(&layers).unwrap();
    assert_eq!(ideologies.frame("neutrality"), Ok(Some(1)));
    assert_eq!(ideologies.frame("fascism"), Ok(Some(2)));
    assert_eq!(ideologies.frame("communism"), Ok(Some(3)));
    assert_eq!(ideologies.frame("democratic"), Ok(Some(1)));
    assert_eq!(ideologies.frame("anarchism"), Ok(Some(0xFFFF_FFFF)));
    assert_eq!(ideologies.frame("monarchism"), Ok(Some(1)));
    assert_eq!(ideologies.frame("syndicalism"), Ok(Some(2)));
    assert_eq!(ideologies.frame("absent"), Ok(None));

    let _ = std::fs::remove_dir_all(&root);
    let stopped = Ideologies::from_groups(Vec::new(), Some(("common/ideologies/d.txt", "x")));
    assert!(stopped.frame("neutrality").is_err());
}

#[test]
fn count_names_preserve_sub_ideology_identity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let db = parsed(b"ideologies={synthetic_group={types={synthetic_type={}}}}");
    let names = crate::trigger_keys::CountTriggerNames::new(
        &crate::buildings::Buildings::default(),
        &db,
        &crate::resources::Resources::default(),
    );
    assert_eq!(names.is_ideology_type(b"SYNTHETIC_TYPE"), Ok(true));
    assert_eq!(names.is_ideology_type(b"synthetic_group"), Ok(false));
    assert_eq!(names.is_ideology(b"synthetic_group"), Ok(true));
}
