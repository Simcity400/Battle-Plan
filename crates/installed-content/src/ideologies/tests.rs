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
        "installed-content-ideologies-{name}-{}",
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

// 140A38190: a duplicate skips its body, not the rest of the file.
#[cfg(feature = "test-support")]
#[test]
fn duplicate_groups_keep_the_first_file_and_skip_the_later_body() {
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

// 140A38A00 stops this file; 1401886F0 reads the next file.
#[cfg(feature = "test-support")]
#[test]
fn top_level_stop_keeps_the_prefix_and_only_stops_its_file() {
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

// 140A38190 rereads owned groups; 140A38FE0 resets omitted groups.
#[cfg(feature = "test-support")]
#[test]
fn later_blocks_in_the_defining_file_reset_group_contents() {
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

/// CColor::vf3 (142237160): three `%f` components, each divided by 255
/// when above 1.0, the alpha set to 1.0; a fourth component is the alpha
/// as read. No `color` leaves CColor::construct's white (142236FA0).
#[test]
fn groups_keep_colours_and_sub_ideologies() {
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
    // 1 is not above 1.0: full red, not 1/255.
    assert_eq!(color(&ideologies, "red"), Ok([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(color(&ideologies, "plain"), Ok([1.0; 4]));
    assert_eq!(
        ideologies.group_of("liberalism").unwrap().unwrap().token,
        "democratic"
    );
    assert!(ideologies.group_of("unknown").unwrap().is_none());
}

/// A later `color` reads into the same CColor from where the last left
/// it, a component `%f` cannot read keeps its value, a `{`-less form is
/// refused by name when it is a packed number (`%x`, 1424B01A0) and
/// otherwise changes nothing (reported), and a variable resolves first.
#[test]
fn colours_follow_ccolors_reads_in_order() {
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
    // The division follows the attempted store, so a component `%f`
    // cannot read is divided again when above 1.0; a quoted number is
    // no number token (0xF): reported, the colour unchanged.
    let ideologies = parsed(
        b"ideologies = { d = { color = { 510 0 0 } color = { bad 0 0 } } \
              e = { color = \"123\" } f = { color = { \"12\" 0 0 } } }",
    );
    assert_eq!(color(&ideologies, "d"), Ok([2.0 / 255.0, 0.0, 0.0, 1.0]));
    assert_eq!(color(&ideologies, "e"), Ok([1.0; 4]));
    assert_eq!(color(&ideologies, "f"), Ok([12.0 / 255.0, 0.0, 0.0, 1.0]));
}

/// A group whose token is not UTF-8 is a named gap, never dropped.
#[test]
fn a_group_that_is_not_text_stops_the_file() {
    let (read, _, _) = read_file_in(
        b"ideologies = { a = { } \"\xfe\" = { } b = { } }",
        "",
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

/// CColor (142237160) reads a number token or `{`: `rgb` is reported
/// and nothing more read, so the block after it is read as the group's
/// fields and its `}` ends the group.
#[test]
fn a_tagged_colour_is_not_read_by_ccolor() {
    let (read, _, _) = read_file_in(
        b"ideologies = { fascism = { color = rgb { 150 75 0 } types = { nazism = { } } } }",
        "",
        &BTreeMap::new(),
    );
    let (groups, _) = groups(&read.items);
    assert_eq!(groups[0].token, "fascism");
    assert_eq!(groups[0].color, Ok([1.0; 4]));
    assert!(groups[0].types.is_empty());
    assert!(
        read.misreads[0]
            .what
            .contains("where CColor reads a number or `{`"),
        "{:?}",
        read.misreads
    );
}

/// A sub-ideology's other keys go to its CModifier (14117C960 -> +0x40):
/// a block value there is a modifier block the registry decides.
#[test]
fn a_sub_ideologys_other_keys_are_its_modifier() {
    let (read, undecided, _) = read_file_in(
            b"ideologies = { a = { types = { x = { political_power_factor = 0.1 odd = { @v = 1 } } } } }",
            "",
            &BTreeMap::new(),
        );
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
    assert_eq!(undecided[0].name, "odd");
}

/// The replay's classes through this reader: a quoted key is no keyword
/// (1424A7A90) and a bare one folds; a variable holds within its file;
/// a block where ReadStringList or one value is read is read as the
/// reader reads it; the end of the text closes every loop.
#[test]
fn the_replay_classes_hold_for_ideologies() {
    let (read, _, _) = read_file_in(
            b"@blue = 255 ideologies = { a = { \"color\" = { 1 2 3 } COLOR = { 0 0 @blue } Types = { x = { } } } \
              b = { can_be_boosted = { color = { 9 9 9 } } } }",
            "",
            &BTreeMap::new(),
        );
    let (found, _) = groups(&read.items);
    assert_eq!(found[0].color, Ok([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(found[0].types, ["x"]);
    // `can_be_boosted` read `{`; the block's colour is b's own and its
    // `}` ended b.
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
        "",
        &BTreeMap::new(),
    );
    assert!(read.misreads.is_empty());
    assert_eq!(groups(&read.items).0[0].types, ["y"]);
}

/// `140A38A00` numbers each group by its place in its block's list (`140A38190`): a key
/// repeated in the block, or first read by an earlier file, takes no place and keeps its
/// first number; a later block of the same file resets the groups it omits (`140A38FE0`)
/// and numbers its own; a file stopping short refuses all.
#[cfg(feature = "test-support")]
#[test]
fn a_group_frame_is_its_place_in_the_defining_block() {
    let root =
        std::env::temp_dir().join(format!("installed-content-ideology-frames-{}", std::process::id()));
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
    // Two blocks: 140A38FE0 resets anarchism when the second omits it; monarchism is 1.
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
    // A file stopping short could define any group first or shift its place.
    let stopped = Ideologies::from_groups(Vec::new(), Some(("common/ideologies/d.txt", "x")));
    assert!(stopped.frame("neutrality").is_err());
}
