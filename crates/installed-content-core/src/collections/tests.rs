use super::*;

#[test]
fn named_collections_flatten_operators_keep_headers_and_guard_cycles() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut db = Database::default();
    db.read_file(
        b"base={input=game:all_states name=BASE operators={limit={always=yes}}} \
        later={input=collection:base operators={limit={state=7}}} \
        cycle={input=collection:cycle}",
        &KeyNames::default(),
    );
    let parse = |text: &str| {
        crate::parser::parse(text.as_bytes())
            .unwrap()
            .items
            .remove(0)
    };
    let result = compile(
        &parse("input={input=collection:later name=OUTER operators={limit={always=no}}}"),
        Some(&db),
    )
    .unwrap();
    assert_eq!(result.input, Input::States);
    assert_eq!(result.operators.len(), 3);
    assert!(matches!(&result.header, Header::Bound(item) if item.value.scalar()==Some(b"OUTER")));
    let result = compile(&parse("input=collection:later"), Some(&db)).unwrap();
    assert!(matches!(&result.header, Header::Bound(item) if item.value.scalar()==Some(b"BASE")));
    assert!(
        compile(&parse("input=collection:cycle"), Some(&db))
            .unwrap_err()
            .contains("recursion")
    );
    assert_eq!(
        compile(
            &parse("input={input=collection:missing operators={faction_members}}"),
            Some(&db)
        )
        .unwrap()
        .operators
        .len(),
        0
    );
}

#[test]
fn source_and_operator_headers_follow_native_collection_kind() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let parse = |text: &str| {
        crate::parser::parse(text.as_bytes())
            .unwrap()
            .items
            .remove(0)
    };
    for (source, header) in [
        ("game:all_countries", b"COLLECTION_ALL_COUNTRIES".as_slice()),
        (
            "game:all_possible_countries",
            b"COLLECTION_ALL_POSSIBLE_COUNTRIES",
        ),
        ("game:all_states", b"COLLECTION_ALL_STATES"),
    ] {
        assert_eq!(
            compile(&parse(&format!("input={source}")), None)
                .unwrap()
                .header,
            Header::Key(header)
        );
    }
    assert_eq!(
        compile(
            &parse("input={input=game:scope operators={faction_members}}"),
            None
        )
        .unwrap()
        .header,
        Header::Key(b"COLLECTION_OP_FACTION_MEMBERS")
    );
    assert!(
        compile(
            &parse("input={input=game:scope operators={unknown_operator}}"),
            None
        )
        .unwrap_err()
        .contains("unknown_operator")
    );
    let mut db = Database::default();
    db.read_file(
        b"native={input=game:scope operators={owned_states limit={state=7}}}",
        &KeyNames::default(),
    );
    let read = compile(&parse("input=collection:native"), Some(&db)).unwrap();
    assert_eq!(read.input, Input::Scope);
    assert!(matches!(
        read.operators.as_slice(),
        [Operator::OwnedStates, Operator::Limit(_)]
    ));
}

#[test]
fn collection_misreads_are_local_to_their_definition() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut db = Database::default();
    db.read_file(
        b"good={input=game:all_states} bad={input=game:scope operators={unknown_operator}}",
        &KeyNames::default(),
    );
    let bad = crate::parser::parse(b"input=collection:bad")
        .unwrap()
        .items
        .remove(0);
    assert!(
        compile(&bad, Some(&db))
            .unwrap_err()
            .contains("unknown_operator")
    );
    db.read_file(b"tail={input=", &KeyNames::default());
    let input = crate::parser::parse(b"input=collection:good")
        .unwrap()
        .items
        .remove(0);
    assert_eq!(compile(&input, Some(&db)).unwrap().input, Input::States);
    let missing = crate::parser::parse(b"input=collection:missing")
        .unwrap()
        .items
        .remove(0);
    assert_eq!(compile(&missing, Some(&db)).unwrap().input, Input::Empty);
}
