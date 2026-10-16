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

/// Group, ordered-list, string-list and quoted-string reads stop at the
/// end of the text, however deep. This records no error: the file loop
/// reads keys to the end token and every nested loop stops there.
#[test]
fn name_readers_close_every_block_open_at_eof() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = br#"FIRST = { ordered = { 1 = { "First %d" } } }
        LAST = { ordered = { 1 = { "Last %d" } }
        # The final group brace is deliberately omitted.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
fn fallback_clamps_signed_ordinals_before_formatting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Group names fold ASCII case: a later spelling overrides the same
/// group, and a recorded key in any case finds it.
#[test]
fn group_names_compare_without_case() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Every field keyword folds ASCII case.
#[test]
fn every_field_keyword_compares_without_case() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// A quoted key is not a keyword and reaches the unknown-field branch,
/// which reports it and skips its block.
#[test]
fn a_quoted_field_key_is_no_keyword() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut names = Database::default();
    let notes = names
        .applied(
            br#"G = { "fallback_name" = "Quoted %d" FALLBACK_name = "Bare %d" "PREFIX" = "X" }"#,
        )
        .unwrap();
    assert_eq!(names.resolve("G", 3, true).ok().as_deref(), Some("Bare 3"));
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].starts_with(
        "group G: quoted field fallback_name is not a division-name field; the game rejects it and skips its block"
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

/// Read an ordered index from the signed decimal prefix of its key.
/// Below 1, leave the ordered loop with the list unread. The loop's
/// closing read consumes the list's first token, and the group reads
/// what follows as its own fields.
#[test]
fn ordered_indices_take_the_decimal_prefix() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Readers stop at the end of the file as at `}`, including when that
/// end falls inside a comment.
#[test]
fn a_file_ending_inside_a_comment_keeps_its_entries() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// One reader handles a file: definitions hold to its end. Bare
/// references take their values; quoted references, names not yet defined
/// and other spellings remain their own text.
#[test]
fn variables_hold_to_the_end_of_their_file_and_quoted_text_stays_literal() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Fields used only for group selection share the file's reader.
/// Definitions in `can_use` triggers register in document order, while
/// string lists and skipped `type` blocks define nothing.
#[test]
fn unread_group_fields_define_as_their_readers_do() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// `can_use` reads a conjunction. A key without an operator uses the
/// stale value slot: `OR {` reads that block as a body, while `always yes`
/// uses the stale `{` without consuming more. Unknown key dispatch leaves
/// the remainder of the file unsettled.
#[test]
fn a_can_use_key_without_an_operator_follows_the_stale_value() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        refusal.contains("field `maybe`: trigger dispatch reads it"),
        "{refusal}"
    );
    assert_eq!(names.resolve("I", 0, false), Err(Unresolved::GroupRefused));
}

/// A repeated definition reads nothing more: its `=` becomes the next
/// key, consumes the value as its operator and is rejected as a group
/// field. The first value holds. A chained variable definition takes its
/// value without consuming it, so the token becomes a repeated definition.
#[test]
fn repeated_and_chained_definitions_follow_the_reader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// A keyed unordered entry `x = { "A" "B" }` is named "A". An anonymous
/// `{ "A" "B" }` consumes "A" as its reported key operator and is named
/// "B"; `{ "A" }` adds no entry.
#[test]
fn unordered_entries_are_read_as_the_game_reads_them() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source =
        br#"G = { unordered = { x = { "A" "B" } { "A" "B" "URL" } { "Alone" } y = { "C" } } }"#;
    let mut names = Database::default();
    let notes = names.applied(source).unwrap();
    assert_eq!(notes.len(), 4, "{notes:?}");
    assert!(
        notes[0].starts_with("line 1: key `{` has no operator: the reader consumes \"A\""),
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

/// Unknown fields are reported and their blocks skipped; the group
/// continues reading either way.
#[test]
fn unknown_group_fields_are_rejected_as_the_game_rejects_them() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut names = Database::default();
    let notes = names
        .applied(br#"G = { colour = "red" fallback_name = "F %d" }"#)
        .unwrap();
    assert_eq!(
        notes,
        [
            "group G: field colour is not a division-name field; the game rejects it and skips its block"
        ]
    );
    assert_eq!(names.resolve("G", 2, true).ok().as_deref(), Some("F 2"));
    let notes = names
        .applied(br#"H = { extra = { fallback_name = "X %d" } fallback_name = "F %d" }"#)
        .unwrap();
    assert!(notes[0].starts_with("group H: field extra is not a division-name field"));
    assert_eq!(names.resolve("H", 1, true).ok().as_deref(), Some("F 1"));
    // Supported fields cause no refusal, in any ASCII case.
    assert!(
        parse(
            br#"G = { NAME = "x" type = army can_use = { always = yes } division_types = { "a" }
                  link_numbering_with = { "b" } for_countries = { GER } ship_types = { "c" } }"#,
            &KeyNames::default()
        )
        .is_ok()
    );
}

/// String fields containing a line feed store the unreadable-string
/// placeholder instead of their authored text.
#[test]
fn a_name_holding_a_line_feed_is_unreadable() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        refusal.starts_with("line 3: field `maybe`: trigger dispatch reads it"),
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

/// Reads that consume text out of step with its braces retain that
/// consumption and report each place: a definition ending its block, a
/// taken name, definitions without `=` or with block values, `@(` ending
/// at `(`, blocks used as single values and values used as lists.
#[test]
fn out_of_step_forms_are_read_as_the_game_reads_them() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Quoted keys are not keywords; bare keys fold. Variables hold from
/// their definition through the file's end. Blocks used as single text
/// values become the group's fields, and end-of-text closes every loop.
#[test]
fn the_replay_classes_hold_for_name_groups() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Lookup searches army, ship and code-name tables, skipping railway
/// guns. A railway-gun group and later army group of the same name are
/// separate, leaving army ordinals unchanged. A redeclaration re-reads
/// the first table without moving the group; ship and code-name groups
/// remain outside the army table used by divisions.
#[test]
fn group_identity_follows_the_typed_tables() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Quoted type values, numbers and blocks report an invalid name type
/// and leave the group's existing type unchanged.
#[test]
fn an_unexpected_type_keeps_the_type_the_group_had() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            "group Q: type \"railway_gun\" is not army, ship, codename or railway_gun; the game logs the invalid value"
        ),
        "{notes:?}"
    );
    assert!(notes[1].starts_with("group N: type `3`"), "{notes:?}");
    assert!(notes[2].starts_with("group B: type a block"), "{notes:?}");
    for (group, expected) in [("Q", "Quoted"), ("N", "Number"), ("B", "Block")] {
        assert_eq!(names.resolve(group, 0, false).as_deref(), Ok(expected));
    }
    // When text ends with `type = ship`, advancing the last bare token
    // fails. Its retained `ship` text has the end-token kind rather than a
    // keyword kind, so the group stays army.
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

/// Group keys and stored names both end at their first NUL.
#[test]
fn names_and_group_keys_end_at_their_first_nul() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let names = parse(
        b"\"G\0tail\" = { unique = { \"Ab\0cd\" } }",
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(names.resolve("g", 0, false).as_deref(), Ok("Ab"));
}

/// Names are stored bytes: refuse only a lookup selecting non-UTF-8
/// text, leaving other entries usable. The lexer ends text at byte 0xFF.
#[test]
fn a_name_that_is_not_utf8_refuses_only_the_lookups_that_select_it() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // The prefix never reaches the fallback.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let names = parse(br#"G = { unique = { "N" } }"#, &KeyNames::default()).unwrap();
    assert_eq!(names.resolve("", 0, false), Err(Unresolved::NoGroup));
    let names = parse(br#""" = { unique = { "Empty" } }"#, &KeyNames::default()).unwrap();
    assert_eq!(names.resolve(b"", 0, false), Err(Unresolved::EmptyKey));
}
