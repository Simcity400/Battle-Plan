use super::*;

impl Database {
    /// [`Database::apply`] as a result: its notes, or its refusal.
    fn applied(&mut self, bytes: &[u8]) -> io::Result<Vec<String>> {
        match self.apply(bytes, &KeyNames::default()) {
            (notes, None) => Ok(notes.into_iter().map(|(_, message)| message).collect()),
            (_, Some(refusal)) => Err(invalid(refusal)),
        }
    }
}

/// The group, ordered-list and string-list readers (1424A9730,
/// 1409B9A20, 1401426E0) and the lexer's quoted string (1424A7A90) end
/// at the end of the text, however deep. The game logs nothing for it:
/// 1409B4370 reads keys until token 0x13 and each loop stops there.
#[test]
fn native_name_readers_close_every_block_open_at_eof() {
    let source = br#"FIRST = { ordered = { 1 = { "First %d" } } }
        LAST = { ordered = { 1 = { "Last %d" } }
        # No final group brace in the shipped Brazilian name file.
    "#;
    let names = parse(source, &KeyNames::default()).unwrap();
    assert_eq!(
        names.resolve("FIRST", 1, true).ok().as_deref(),
        Some("First 1")
    );
    assert_eq!(
        names.resolve("LAST", 1, true).ok().as_deref(),
        Some("Last 1")
    );
    for (source, expected) in [
        (br#"G = { ordered = { 1 = { "First" }"#.as_slice(), "First"),
        (br#"G = { ordered = { 1 = { "First""#, "First"),
        (br#"G = { unique = { "First""#, "First"),
    ] {
        let names = parse(source, &KeyNames::default()).unwrap();
        let (order, ordered) = if source.starts_with(b"G = { unique") {
            (0, false)
        } else {
            (1, true)
        };
        assert_eq!(
            names.resolve("G", order, ordered).ok().as_deref(),
            Some(expected)
        );
    }
    // An empty value before `}`: the ordered loop reads nothing.
    let mut names = Database::default();
    names.applied(b"SECOND = { ordered = }").unwrap();
    assert_eq!(
        names.resolve("SECOND", 1, true),
        Err(Unresolved::NoFallback)
    );
}

#[test]
fn selection_uses_exact_ids_and_literal_unordered_names() {
    let names = parse(
        br#"
        INF = {
            ordered = { 1 = { "%d. Infantry" "DESCRIPTION" } 12 = { "%s Army %s / %d" } }
            unordered = { a = { "Guard %d" } b = { "Reserve %s" "DESC" "URL" } }
            fallback_name = "%d. Reserve"
        }
    "#,
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(
        names.resolve("INF", 1, true).ok().as_deref(),
        Some("1. Infantry")
    );
    assert_eq!(
        names.resolve("INF", 12, true).ok().as_deref(),
        Some("XII Army XII / %d")
    );
    assert_eq!(
        names.resolve("INF", 2, true).ok().as_deref(),
        Some("2. Reserve")
    );
    assert_eq!(
        names.resolve("INF", 0, false).ok().as_deref(),
        Some("Guard %d")
    );
    assert_eq!(
        names.resolve("INF", 1, false).ok().as_deref(),
        Some("Reserve %s")
    );
    assert_eq!(
        names.resolve("INF", 2, false).ok().as_deref(),
        Some("3. Reserve")
    );
}

#[test]
fn repeated_groups_merge_entries_and_keep_omitted_fallback() {
    let mut names = parse(
        br#"
        INF = { ordered = { 1 = { "Old %d" } 2 = { "Second %d" } }
                unordered = { a = { "First" } } fallback_name = "Fallback %d" }
        INF = { ordered = { 1 = { "Updated %d" } } unordered = { a = { "Next" } } }
    "#,
        &KeyNames::default(),
    )
    .unwrap();
    names
        .applied(
            br#"INF = { ordered = { 1 = { "Mod %d" } 3 = { "Third %d" } }
                                  unordered = { a = { "Mod" } } }"#,
        )
        .unwrap();
    assert_eq!(names.resolve("INF", 1, true).ok().as_deref(), Some("Mod 1"));
    assert_eq!(
        names.resolve("INF", 2, true).ok().as_deref(),
        Some("Second 2")
    );
    assert_eq!(
        names.resolve("INF", 3, true).ok().as_deref(),
        Some("Third 3")
    );
    assert_eq!(
        names.resolve("INF", 4, true).ok().as_deref(),
        Some("Fallback 4")
    );
    for (order, expected) in ["First", "Next", "Mod"].into_iter().enumerate() {
        assert_eq!(
            names.resolve("INF", order as i32, false).ok().as_deref(),
            Some(expected)
        );
    }
    names.applied(br#"INF = { fallback_name = "" }"#).unwrap();
    assert_eq!(names.resolve("INF", 4, true), Err(Unresolved::NoFallback));
}

#[test]
fn missing_operands_do_not_invent_names_but_authored_empty_names_survive() {
    let names = parse(
        br#"INF = { ordered = { 1 = { "" } 2 = {} }
                                   unordered = { a = {} b = { "" } } }"#,
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(
        names.resolve("UNKNOWN", 1, true),
        Err(Unresolved::GroupAbsent)
    );
    assert_eq!(names.resolve("INF", 2, true), Err(Unresolved::NoFallback));
    assert_eq!(names.resolve("INF", 1, true).ok().as_deref(), Some(""));
    assert_eq!(names.resolve("INF", 0, false).ok().as_deref(), Some(""));
}

#[test]
fn fallback_clamps_native_signed_ordinals_before_formatting() {
    let names = parse(
        br#"INF = { fallback_name = "%d / %d" }"#,
        &KeyNames::default(),
    )
    .unwrap();
    for (order, ordered) in [(-1, true), (-2, false), (i32::MAX, false)] {
        assert_eq!(
            names.resolve("INF", order, ordered).ok().as_deref(),
            Some("1 / 1")
        );
    }
    assert_eq!(
        names.resolve("INF", 0, true),
        Err(Unresolved::UnassignedOrder)
    );
    assert_eq!(roman(3999), "MMMCMXCIX");
    assert_eq!(roman(4004), "MMMMIV");
}

#[test]
fn malformed_name_operands_are_reported() {
    for input in [
        br#"INF = { ordered = { 0 = { "bad" } } }"#.as_slice(),
        br#"INF = { ordered = { -1 = { "bad" } } }"#,
        br#"INF = { ordered = { 1 = "bad" } }"#,
        br#"INF = { unordered = { "bad" } }"#,
        br#"INF = { unordered = { 1 = "bad" } }"#,
        br#"INF = { unordered = { {} } }"#,
        br#"INF = { unknown_field = { fallback_name = "x" } }"#,
        br#"INF = { "anonymous" }"#,
        br#"INF = { fallback_name = {} }"#,
        br#"SHIP = { unique = { { "nested" } } }"#,
        br#"SHIP = { prefix = {} }"#,
    ] {
        assert!(parse(input, &KeyNames::default()).is_err());
    }
}

#[test]
fn ship_unique_names_share_unordered_indices_and_prefix_only_authored_entries() {
    let names = parse(
        br#"SHIP = {
        prefix = "HMS "
        unique = { "Gallant" "Garland" }
        unordered = { x = { "Literal %s" } }
        unique = { "Gipsy" }
        ordered = { 7 = { "%s Flotilla" } }
        fallback_name = "Destroyer %s"
    }"#,
        &KeyNames::default(),
    )
    .unwrap();
    for (order, expected) in ["HMS Gallant", "HMS Garland", "HMS Literal %s", "HMS Gipsy"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            names.resolve("SHIP", order as i32, false).ok().as_deref(),
            Some(expected)
        );
    }
    assert_eq!(
        names.resolve("SHIP", 7, true).ok().as_deref(),
        Some("HMS VII Flotilla")
    );
    assert_eq!(
        names.resolve("SHIP", 4, false).ok().as_deref(),
        Some("Destroyer V")
    );
}

#[test]
fn prefix_finalization_runs_on_each_declaration_and_preserves_absent_prefix() {
    let mut names = parse(
        br#"
        SHIP = { prefix = "HMS " unique = { "Gallant" } }
        SHIP = { unique = { "Garland" } }
    "#,
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(
        names.resolve("SHIP", 0, false).ok().as_deref(),
        Some("HMS HMS Gallant")
    );
    assert_eq!(
        names.resolve("SHIP", 1, false).ok().as_deref(),
        Some("HMS Garland")
    );
    names
        .applied(br#"SHIP = { prefix = "USS " unique = { "Gipsy" } }"#)
        .unwrap();
    assert_eq!(
        names.resolve("SHIP", 0, false).ok().as_deref(),
        Some("USS HMS HMS Gallant")
    );
    assert_eq!(
        names.resolve("SHIP", 1, false).ok().as_deref(),
        Some("USS HMS Garland")
    );
    assert_eq!(
        names.resolve("SHIP", 2, false).ok().as_deref(),
        Some("USS Gipsy")
    );
    names
        .applied(br#"SHIP = { prefix = "" unique = { "Bare" } }"#)
        .unwrap();
    assert_eq!(
        names.resolve("SHIP", 2, false).ok().as_deref(),
        Some("USS Gipsy")
    );
    assert_eq!(
        names.resolve("SHIP", 3, false).ok().as_deref(),
        Some("Bare")
    );
}

/// 1409B4370 folds group names (140BBA1B0, `_stricmp` 14253D9F8): a later
/// declaration spelled differently overrides the same group, and a
/// recorded key in any case finds it.
#[test]
fn group_names_compare_without_case() {
    let mut database = parse(
        b"GER_INF = { ordered = { 1 = { \"First %d\" } } }",
        &KeyNames::default(),
    )
    .unwrap();
    database
        .applied(b"ger_inf = { ordered = { 1 = { \"Erste\" } } }")
        .unwrap();
    for key in ["GER_INF", "ger_inf", "Ger_Inf"] {
        assert_eq!(
            database.resolve(key, 1, true).ok().as_deref(),
            Some("Erste")
        );
    }
}

/// Field keys are folded as the lexer folds them, so a later layer's
/// upper-case field overrides a lower-case group's.
#[test]
fn field_keys_compare_without_case() {
    let mut database = parse(b"G = { fallback_name = \"Old %d\" }", &KeyNames::default()).unwrap();
    database
        .applied(b"g = { FALLBACK_NAME = \"New %d\" ORDERED = { 2 = { \"Two\" } } }")
        .unwrap();
    assert_eq!(
        database.resolve("G", 1, true).ok().as_deref(),
        Some("New 1")
    );
    assert_eq!(database.resolve("G", 2, true).ok().as_deref(), Some("Two"));
}

/// Every field keyword folds (1424A7A90), whatever its case.
#[test]
fn every_field_keyword_compares_without_case() {
    let names = parse(
        br#"G = { UNORDERED = { u = { "U" } } Unique = { "Q" } PREFIX = "P "
                  Ordered = { 1 = { "O" } } Fallback_Name = "F %d" }"#,
        &KeyNames::default(),
    )
    .unwrap();
    for (order, ordered, expected) in [
        (0, false, "P U"),
        (1, false, "P Q"),
        (1, true, "P O"),
        (2, true, "F 2"),
    ] {
        assert_eq!(
            names.resolve("g", order, ordered).ok().as_deref(),
            Some(expected)
        );
    }
}

/// 1424A7A90 gives a quoted key token 0xF, so 1409B9A20 dispatches it to
/// its default branch (1409B9FF0) like any name that is no keyword:
/// RecordError (1424ACF90) skips its block.
#[test]
fn a_quoted_field_key_is_no_keyword() {
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"G = { "fallback_name" = "Quoted %d" FALLBACK_name = "Bare %d" "PREFIX" = "X" }"#,
        )
        .unwrap();
    assert_eq!(names.resolve("G", 3, true).ok().as_deref(), Some("Bare 3"));
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].starts_with(
        "group G: quoted field fallback_name is not a division-name field; the game rejects it and skips its block (1409B9FF0, 1424ACF90)"
    ));
    let notes = names
        .applied(br#"H = { "ordered" = { 1 = { "One" } } fallback_name = "H %d" }"#)
        .unwrap();
    assert!(
        notes[0].contains("quoted field ordered is not a division-name field"),
        "{notes:?}"
    );
    assert_eq!(names.resolve("H", 1, true).ok().as_deref(), Some("H 1"));
}

/// 1409B9A20 reads an ordered index with ParseTokenInt32 (1424B0080 ->
/// 14254AF28 -> 14253C770): a signed decimal prefix of the key's text.
/// Below 1 it leaves the ordered loop with the entry's list unread; the
/// loop's closing read consumes the list's first token, and the group
/// reads what follows as its own fields.
#[test]
fn ordered_indices_take_the_native_decimal_prefix() {
    let names = parse(
        br#"G = { ordered = { "1tail" = { "First" } 02x = { "Second" } 99999999999 = { "Max" } } }"#, &KeyNames::default(),
    )
    .unwrap();
    for (order, expected) in [(1, "First"), (2, "Second"), (i32::MAX, "Max")] {
        assert_eq!(
            names.resolve("G", order, true).ok().as_deref(),
            Some(expected)
        );
    }
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"G = { ordered = { 2 = { "Two" } -3 = { "Negative" } 4 = { "Four" } } fallback_name = "F %d" }"#,
        )
        .unwrap();
    assert!(
        notes[0].contains("field `-3` ends the loop reading it"),
        "{notes:?}"
    );
    assert!(notes[1].ends_with("consumes \"Negative\""), "{notes:?}");
    assert_eq!(names.resolve("G", 2, true).ok().as_deref(), Some("Two"));
    // The entry's `}` ended the group: entry 4 and the fallback were
    // read as groups of their own.
    assert_eq!(names.resolve("G", 4, true), Err(Unresolved::NoFallback));
    assert!(names.groups.contains_key(b"4".as_slice()));
    assert!(
        parse(
            br#"G = { ordered = { tail = { "Zero" } } }"#,
            &KeyNames::default()
        )
        .is_err()
    );
}

/// The readers stop at the end of the file as at `}` (1424A9730,
/// 1409B9A20, 1401426E0), also when it falls inside a comment.
#[test]
fn a_file_ending_inside_a_comment_keeps_its_entries() {
    for source in [
        br#"G = { ordered = { 1 = { "First" } # tail"#.as_slice(),
        br#"G = { ordered = { 1 = { "First"#,
    ] {
        let mut names = Database::default();
        let notes = names.applied(source).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(names.resolve("G", 1, true).ok().as_deref(), Some("First"));
    }
}

/// One reader reads a file (1409B8510): a definition holds from where it
/// is read to the end of that file (1424AC830), a bare reference takes
/// its value (1424AB540), and a quoted one, a name not yet defined, or
/// another spelling of a name stays its own text.
#[test]
fn variables_hold_to_the_end_of_their_file_and_quoted_text_stays_literal() {
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"@top = "Top %d"
            EARLY = { fallback_name = @late }
            @late = "Late %d"
            G = {
                @n = "Division %d"
                fallback_name = @n
                ordered = { @o = "Ordered %d" 1 = { @o "DESC" } 2 = { "@o" } }
                unordered = { @u = "Loose" x = { @u } }
                unique = { @top @missing "@n" }
            }
            H = { fallback_name = @N ordered = { 1 = { @o } } }
            LATE = { fallback_name = @late }
            TOP = { FALLBACK_NAME = @top }"#,
        )
        .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    for (group, order, ordered, expected) in [
        ("EARLY", 1, true, "@late"),
        ("G", 5, true, "Division 5"),
        ("G", 1, true, "Ordered 1"),
        ("G", 2, true, "@o"),
        ("G", 0, false, "Loose"),
        ("G", 1, false, "Top %d"),
        ("G", 2, false, "@missing"),
        ("G", 3, false, "@n"),
        ("H", 3, true, "@N"),
        ("H", 1, true, "Ordered 1"),
        ("LATE", 2, true, "Late 2"),
        ("TOP", 4, true, "Top 4"),
    ] {
        assert_eq!(
            names.resolve(group, order, ordered).ok().as_deref(),
            Some(expected),
            "{group} {order}"
        );
    }
    // The next file's reader starts with no variables.
    names.applied(br#"NEXT = { fallback_name = @n }"#).unwrap();
    assert_eq!(names.resolve("NEXT", 1, true).ok().as_deref(), Some("@n"));
}

/// The fields that only choose a group still share the file's reader:
/// `can_use` is a trigger read by key (1405431C0), so a definition there
/// registers in document order; a string list (1401426E0) and a skipped
/// `type` block (1424ACF90 -> 1424AD270) define nothing.
#[test]
fn unread_group_fields_define_as_their_readers_do() {
    let mut names = Database::default();
    names
        .applied(
            br#"G = { fallback_name = @n can_use = { OR = { @n = "Guard %d" always = yes } } }
            H = { fallback_name = @n division_types = { @l = "x" } type = { @t = "y" } }
            I = { fallback_name = @l } J = { fallback_name = @t }"#,
        )
        .unwrap();
    for (group, expected) in [
        // Read before its definition, the reference keeps its text.
        ("G", "@n"),
        ("H", "Guard 1"),
        ("I", "@l"),
        ("J", "@t"),
    ] {
        assert_eq!(
            names.resolve(group, 1, true).ok().as_deref(),
            Some(expected),
            "{group}"
        );
    }
}

/// `can_use` is a CAndTrigger body (1405431C0), which hands LoadKey a
/// key read without an operator with the stale value slot: `OR {`
/// under `can_use = {` reads its block as a body; `always yes` hands
/// `always` (CSimpleAssignTrigger, 141395A40) the stale `{`, of which it
/// reads nothing more. A key the names cannot tell leaves the rest of
/// the file unknown.
#[test]
fn a_can_use_key_without_an_operator_follows_the_stale_value() {
    let mut names = Database::default();
    let (_, refusal) = names.apply(
        br#"G = { can_use = { OR { always = yes } } unique = { "A" } }"#,
        &KeyNames::default(),
    );
    assert!(refusal.is_none(), "{refusal:?}");
    assert_eq!(names.resolve("G", 0, false).ok().as_deref(), Some("A"));
    let (_, refusal) = names.apply(
        br#"H = { can_use = { always yes } unique = { "B" } }"#,
        &KeyNames::default(),
    );
    assert!(refusal.is_none(), "{refusal:?}");
    assert_eq!(names.resolve("H", 0, false).ok().as_deref(), Some("B"));
    let (_, refusal) = names.apply(
        br#"I = { can_use = { maybe yes } unique = { "C" } }"#,
        &KeyNames::unknown(),
    );
    let refusal = refusal.unwrap();
    assert!(
        refusal.contains("field `maybe`: LoadKey (1405434F0) reads it"),
        "{refusal}"
    );
    assert_eq!(names.resolve("I", 0, false), Err(Unresolved::GroupRefused));
}

/// A repeated definition reads nothing more (1424AC830): the reader
/// takes its `=` as the next key, which consumes the value as its
/// operator, and 1409B9A20 rejects that key; the first value holds. A
/// definition naming a variable takes its value without consuming it,
/// so that token is read next as a repeated definition.
#[test]
fn repeated_and_chained_definitions_follow_the_reader() {
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"@b = "B %d"
            G = { @n = "First %d" @n = "Second %d" fallback_name = @n }
            H = { @a = @b fallback_name = @a }
            Q = { @q = "@b" fallback_name = @q }"#,
        )
        .unwrap();
    assert!(
        notes[0].starts_with("line 2: variable `@n` is already taken"),
        "{notes:?}"
    );
    assert!(
        notes[1].starts_with("line 2: key `=` has no operator"),
        "{notes:?}"
    );
    assert!(
        notes[2].starts_with("line 3: definition `@a` takes `@b`"),
        "{notes:?}"
    );
    assert!(
        notes
            .last()
            .unwrap()
            .starts_with("group G: field = is not a division-name field"),
        "{notes:?}"
    );
    assert!(
        parse(
            br#"G = { @n = "A" @n = "B" fallback_name = @n }"#,
            &KeyNames::default()
        )
        .is_err()
    );
    for (group, expected) in [("G", "First 2"), ("H", "B 2"), ("Q", "@b")] {
        assert_eq!(
            names.resolve(group, 2, true).ok().as_deref(),
            Some(expected),
            "{group}"
        );
    }
}

/// 1409B9A20 reads each `unordered` entry through 1424AC5E0 and then
/// 1401426E0: a keyed entry `x = { "A" "B" }` is named "A", while the
/// anonymous `{ "A" "B" }` spends "A" as the key's operator (reported by
/// 1424AB2A0) and is named "B"; `{ "A" }` then adds no entry.
#[test]
fn unordered_entries_are_read_as_the_game_reads_them() {
    let source =
        br#"G = { unordered = { x = { "A" "B" } { "A" "B" "URL" } { "Alone" } y = { "C" } } }"#;
    let mut names = Database::default();
    let notes = names.applied(source).unwrap();
    assert_eq!(notes.len(), 4, "{notes:?}");
    assert!(
        notes[0].starts_with("line 1: key `{` has no operator: 1424AC5E0 consumes \"A\""),
        "{notes:?}"
    );
    assert!(notes[2].contains("consumes \"Alone\""), "{notes:?}");
    for (order, expected) in [(0, "A"), (1, "B"), (2, "C")] {
        assert_eq!(
            names.resolve("G", order, false).ok().as_deref(),
            Some(expected),
            "{order}"
        );
    }
    assert_eq!(names.resolve("G", 3, false), Err(Unresolved::NoFallback));
    // The strict reader treats the reported token as an error.
    assert!(parse(source, &KeyNames::default()).is_err());
}

/// 1409B9A20's default (1409B9FF0) rejects a field it does not read
/// through RecordError (1424ACF90), which skips a block (1424AD270): the
/// group reads on either way.
#[test]
fn unknown_group_fields_are_rejected_as_the_game_rejects_them() {
    let mut names = Database::default();
    let notes = names
        .applied(br#"G = { colour = "red" fallback_name = "F %d" }"#)
        .unwrap();
    assert_eq!(
        notes,
        [
            "group G: field colour is not a division-name field; the game rejects it and skips its block (1409B9FF0, 1424ACF90)"
        ]
    );
    assert_eq!(names.resolve("G", 2, true).ok().as_deref(), Some("F 2"));
    let notes = names
        .applied(br#"H = { extra = { fallback_name = "X %d" } fallback_name = "F %d" }"#)
        .unwrap();
    assert!(notes[0].starts_with("group H: field extra is not a division-name field"));
    assert_eq!(names.resolve("H", 1, true).ok().as_deref(), Some("F 1"));
    // Fields 1409B9A20 does read are no refusal, in any case.
    assert!(
        parse(
            br#"G = { NAME = "x" type = army can_use = { always = yes } division_types = { "a" }
                  link_numbering_with = { "b" } for_countries = { GER } ship_types = { "c" } }"#,
            &KeyNames::default()
        )
        .is_ok()
    );
}

/// CReader::ReadStringInto (1424ABB50) stores a string holding a line
/// feed as "Unreadable String".
#[test]
fn a_name_holding_a_line_feed_is_unreadable() {
    let names = parse(
        b"G = { ordered = { 1 = { \"Line\nBreak\" } } }",
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(
        names.resolve("G", 1, true).ok().as_deref(),
        Some("Unreadable String")
    );
}

/// A block the port cannot settle (under a trigger key the content's
/// names cannot tell) leaves the rest of the file unknown. Every group the replay read is applied; the group being read
/// and every name a token from the unsettled block on could declare are
/// refused, earlier layers' groups of those names included, and stay
/// refused through later files. Other groups keep their values.
#[test]
fn an_unsettled_file_refuses_every_group_its_rest_could_declare() {
    let mut names = parse(
        br#"SHIP = { prefix = "HMS " unique = { "Gallant" } }
            OTHER = { unique = { "Kept" } }
            KEPT = { unique = { "Earlier" } }"#,
        &KeyNames::default(),
    )
    .unwrap();
    let (notes, refusal) = names.apply(
        br#"SHIP = { unique = { "Garland" } } FIRST = { unique = { "First" } }
            ship = {
                can_use = { OR = { maybe = { @v = yes } } } unique = { "Lost" } }
            OTHER = { unique = { "Unknown" } }"#,
        &KeyNames::unknown(),
    );
    assert!(notes.is_empty(), "{notes:?}");
    let refusal = refusal.unwrap();
    assert!(
        refusal.starts_with("line 3: field `maybe`: LoadKey (1405434F0) reads it"),
        "{refusal}"
    );
    assert!(
        refusal.ends_with(
            "are refused (13): =, @v, can_use, lost, maybe, or, other, ship, unique, unknown, yes, {, }"
        ),
        "{refusal}"
    );
    // The first declaration applied (prefix pass included) before the
    // group that cannot be followed.
    assert_eq!(
        names.groups[b"ship".as_slice()].1.unordered,
        [b"HMS HMS Gallant".to_vec(), b"HMS Garland".to_vec()]
    );
    assert_eq!(
        names.resolve("Ship", 0, false),
        Err(Unresolved::GroupRefused)
    );
    // The rest names OTHER, so its earlier values may be stale.
    assert_eq!(
        names.resolve("OTHER", 0, false),
        Err(Unresolved::GroupRefused)
    );
    for (group, expected) in [("FIRST", "First"), ("KEPT", "Earlier")] {
        assert_eq!(
            names.resolve(group, 0, false).ok().as_deref(),
            Some(expected)
        );
    }
    names
        .applied(br#"SHIP = { unique = { "Later" } } OTHER = { unique = { "Later" } }"#)
        .unwrap();
    for group in ["SHIP", "other"] {
        assert_eq!(
            names.resolve(group, 1, false),
            Err(Unresolved::GroupRefused)
        );
    }
}

/// Text the readers take out of step with its braces is read as they
/// read it (the replay), each place noted: a definition that ends its
/// block, a taken name, a definition without `=` or with a block value,
/// `@(` (the lexer ends a token at `(`), a block where ReadStringInto
/// takes one value, and a value where ReadStringList reads a list.
#[test]
fn out_of_step_forms_are_read_as_the_game_reads_them() {
    for (source, fallback, note) in [
        (
            br#"G = { fallback_name = "x" @n = "y" }"#.as_slice(),
            Some("x"),
            "key `}` has no operator",
        ),
        (
            br#"@n = "a" @n = "b" G = { fallback_name = @n }"#,
            None,
            "variable `@n` is already taken",
        ),
        (
            br#"G = { unordered = { @n { "x" } } }"#,
            None,
            "definition `@n` has no `=`",
        ),
        (
            br#"G = { @n = { "a" } fallback_name = @n }"#,
            Some("{"),
            "definition `@n` keeps only the `{` of its block value",
        ),
        (
            br#"G = { fallback_name = @(1+1) }"#,
            Some("@"),
            "key `(` has no operator",
        ),
        (
            br#"G = { NAME = { "a" } fallback_name = "x" }"#,
            Some("x"),
            "field `NAME` is a block where its reader takes one value",
        ),
        (
            br#"G = { for_countries = GER fallback_name = "x" }"#,
            None,
            "field `for_countries` is `GER` where its reader reads a list",
        ),
    ] {
        let mut names = Database::default();
        let notes = names.applied(source).unwrap();
        assert!(
            notes.iter().any(|found| found.contains(note)),
            "{}: {notes:?}",
            String::from_utf8_lossy(source)
        );
        let expected = match fallback {
            Some(text) => Ok(text.to_owned()),
            None => Err(Unresolved::NoFallback),
        };
        if names.groups.contains_key(b"g".as_slice()) {
            assert_eq!(names.resolve("G", 1, true), expected);
        } else {
            assert!(fallback.is_none());
        }
    }
}

/// The replay's classes through this reader: a quoted key is no keyword
/// (1424A7A90), a bare one folds; a variable holds from its definition
/// to the end of its file only (1409B8510); a block where ReadStringInto
/// takes one value is read as the group's fields; the end of the text
/// closes every loop.
#[test]
fn the_replay_classes_hold_for_name_groups() {
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"@late = "Late %d"
            MIXED = { "unique" = { "Quoted" } UNIQUE = { "Bare" } Fallback_Name = @late }
            BLOCK = { prefix = { fallback_name = "Inner %d" } unique = { "After" } }"#,
        )
        .unwrap();
    assert_eq!(
        names.resolve("mixed", 0, false).ok().as_deref(),
        Some("Bare")
    );
    assert_eq!(
        names.resolve("MIXED", 4, true).ok().as_deref(),
        Some("Late 4")
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("quoted field unique"))
    );
    // `prefix` reads `{`; the block's fallback is the group's own, and
    // its `}` ends the group, so `unique` is a group at the top level.
    assert_eq!(
        names.groups[b"block".as_slice()].1.prefix.as_deref(),
        Some(b"{".as_slice())
    );
    assert_eq!(
        names.resolve("BLOCK", 2, true).ok().as_deref(),
        Some("Inner 2")
    );
    assert!(names.groups.contains_key(b"unique".as_slice()));
    assert!(
        notes
            .iter()
            .any(|note| note.contains("field `prefix` is a block")),
        "{notes:?}"
    );
    names
        .applied(br#"NEXT = { fallback_name = @late } OPEN = { ordered = { 1 = { "Open"#)
        .unwrap();
    assert_eq!(
        names.resolve("NEXT", 1, true).ok().as_deref(),
        Some("@late")
    );
    assert_eq!(names.resolve("OPEN", 1, true).ok().as_deref(), Some("Open"));
}

/// 1409B4370 searches the army, ship and code-name tables, not the
/// railway-gun one: a railway-gun group and a later army group of the
/// same name are two groups, so the army ordinals do not shift. A
/// redeclaration re-reads the group in its first table, whatever its
/// new `type`; a group first declared as a ship or code name is not in
/// the army table divisions take their group from (1409B71F0).
#[test]
fn group_identity_follows_the_typed_tables() {
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"G = { type = railway_gun unique = { "Rail" } }
            G = { unique = { "Army" } }
            R = { TYPE = Railway_Gun unique = { "One" } } R = { type = railway_gun unique = { "Two" } }
            A = { unique = { "First" } } A = { type = ship unique = { "Second" } }
            S = { type = ship unique = { "Ship" } } S = { type = army unique = { "Army" } }
            C = { type = codename unique = { "Code" } }
            L = { type = ship type = army unique = { "Last" } }"#,
        )
        .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    for (group, order, expected) in [
        ("g", 0, Ok("Army".to_owned())),
        ("G", 1, Err(Unresolved::NoFallback)),
        ("R", 0, Err(Unresolved::GroupAbsent)),
        ("A", 1, Ok("Second".to_owned())),
        ("S", 0, Err(Unresolved::NotArmy(Table::Ship))),
        ("C", 0, Err(Unresolved::NotArmy(Table::Codename))),
        ("L", 0, Ok("Last".to_owned())),
    ] {
        assert_eq!(names.resolve(group, order, false), expected, "{group}");
    }
    // Across layers too: a later layer's army group is its own.
    names.applied(br#"R = { unique = { "Army R" } }"#).unwrap();
    assert_eq!(names.resolve("R", 0, false).as_deref(), Ok("Army R"));
    assert!(
        Unresolved::NotArmy(Table::Ship)
            .to_string()
            .contains("first declared with type ship")
    );
}

/// 1409B9A20 compares the value token's id: a quoted `type`, a number and
/// a block are "unexpected name type" (1424ACF90), and the group keeps
/// the type it had.
#[test]
fn an_unexpected_type_keeps_the_type_the_group_had() {
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"Q = { type = "railway_gun" unique = { "Quoted" } }
            N = { type = 3 unique = { "Number" } }
            B = { type = { railway_gun } unique = { "Block" } }"#,
        )
        .unwrap();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(
        notes[0].starts_with(
            "group Q: type \"railway_gun\" is not army, ship, codename or railway_gun; the game records \"unexpected name type\""
        ),
        "{notes:?}"
    );
    assert!(notes[1].starts_with("group N: type `3`"), "{notes:?}");
    assert!(notes[2].starts_with("group B: type a block"), "{notes:?}");
    for (group, expected) in [("Q", "Quoted"), ("N", "Number"), ("B", "Block")] {
        assert_eq!(names.resolve(group, 0, false).as_deref(), Ok(expected));
    }
    // A text ending in `type = ship`: the last bare token's advance
    // fails (1424A7A90), so the value slot holds the end token 0x13
    // with the text `ship`, which is no keyword; the group stays army.
    let mut names = Database::default();
    let notes = names
        .applied(b"E = { unique = { \"End\" } type = ship")
        .unwrap();
    assert!(
        notes[0].starts_with("group E: type the end of the file (text `ship`)"),
        "{notes:?}"
    );
    assert_eq!(names.resolve("E", 0, false).as_deref(), Ok("End"));
    let mut names = Database::default();
    names
        .applied(b"S = { unique = { \"Ship\" } TYPE = Ship\n")
        .unwrap();
    assert_eq!(
        names.resolve("S", 0, false),
        Err(Unresolved::NotArmy(Table::Ship))
    );
}

/// 1409B4370 takes a group's name to its first NUL, and ReadStringInto
/// (1424ABB50, read_string_token 1424B05E0) a name to its first NUL.
#[test]
fn names_and_group_keys_end_at_their_first_nul() {
    let names = parse(
        b"\"G\0tail\" = { unique = { \"Ab\0cd\" } }",
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(names.resolve("g", 0, false).as_deref(), Ok("Ab"));
}

/// Names are stored bytes: only a lookup that selects one that is not
/// UTF-8 is refused, the group's other names still render. (A byte 0xFF
/// ends the text for the lexer, 1424A7A90.)
#[test]
fn a_name_that_is_not_utf8_refuses_only_the_lookups_that_select_it() {
    let names = parse(
        b"G = { unique = { \"\xe9\" \"Fine\" } ordered = { 2 = { \"\xe8 %d\" } } fallback_name = \"F %d\" }\n\
          P = { prefix = \"\xe9\" unique = { \"Any\" } fallback_name = \"Plain %d\" }", &KeyNames::default(),
    )
    .unwrap();
    for (group, order, ordered, expected) in [
        ("G", 0, false, Err(Unresolved::NotText)),
        ("G", 1, false, Ok("Fine".to_owned())),
        ("G", 2, true, Err(Unresolved::NotText)),
        ("G", 3, true, Ok("F 3".to_owned())),
        ("P", 0, false, Err(Unresolved::NotText)),
        // The prefix never reaches the fallback (1409B86A0).
        ("P", 1, false, Ok("Plain 2".to_owned())),
    ] {
        assert_eq!(
            names.resolve(group, order, ordered),
            expected,
            "{group} {order}"
        );
    }
}

/// The capture records an empty key for a null group; a group declared
/// with an empty name makes that key ambiguous.
#[test]
fn an_empty_key_is_a_null_group_unless_one_is_declared_empty() {
    let names = parse(br#"G = { unique = { "N" } }"#, &KeyNames::default()).unwrap();
    assert_eq!(names.resolve("", 0, false), Err(Unresolved::NoGroup));
    let names = parse(br#""" = { unique = { "Empty" } }"#, &KeyNames::default()).unwrap();
    assert_eq!(names.resolve(b"", 0, false), Err(Unresolved::EmptyKey));
}
