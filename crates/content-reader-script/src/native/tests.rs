use super::*;

#[test]
fn counted_reports_keep_their_source_kind() {
    let read = read(
        b"warning=yes unsupported=yes\nwarning=no unsupported=no",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field| {
            if field.keyword_is("warning") {
                Shape::Reported {
                    kind: content_reader_parser::DiagnosticKind::Misread,
                    report: "synthetic reader diagnostic".into(),
                    read: Box::new(Shape::One),
                }
            } else {
                Shape::Reported {
                    kind: content_reader_parser::DiagnosticKind::Unsupported,
                    report: "synthetic reader diagnostic".into(),
                    read: Box::new(Shape::One),
                }
            }
        },
    );
    assert_eq!(
        read.misreads
            .iter()
            .map(|note| note.kind)
            .collect::<Vec<_>>(),
        [
            content_reader_parser::DiagnosticKind::Misread,
            content_reader_parser::DiagnosticKind::Unsupported
        ]
    );
    assert!(
        read.misreads
            .iter()
            .all(|note| note.count == Some(2) && note.line == 1)
    );
    assert_eq!(read.item_misreads.len(), 4);
    assert!(
        read.item_misreads
            .iter()
            .all(|notes| notes.len() == 1 && notes[0].count == Some(1))
    );
}

#[test]
fn states_file_diagnostics_keep_ownership_of_repeated_reports_on_one_line() {
    let read = read(
        b"state = { deferred = yes deferred = yes } state = { deferred = yes } state = { }",
        Mode::KeepMark,
        Top::File(false),
        None,
        &mut |nested, _| {
            if nested {
                Shape::Reported {
                    kind: content_reader_parser::DiagnosticKind::Unsupported,
                    report: "synthetic deferred reader".into(),
                    read: Box::new(Shape::One),
                }
            } else {
                Shape::Object(true)
            }
        },
    );
    assert_eq!(read.items.len(), 3);
    assert_eq!(read.misreads.len(), 1);
    assert_eq!(read.misreads[0].count, Some(3));
    assert_eq!(read.item_misreads.len(), 3);
    assert_eq!(read.item_misreads[0][0].count, Some(2));
    assert_eq!(read.item_misreads[1][0].count, Some(1));
    assert!(read.item_misreads[2].is_empty());
    assert!(read.unattributed_misreads.is_empty());
}

#[test]
fn states_file_diagnostics_from_discarded_items_remain_unattributed() {
    let read = read(
        b"discard tail state = { }",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field| {
            if field.keyword_is("discard") {
                Shape::Discard
            } else {
                Shape::Object(())
            }
        },
    );
    assert_eq!(read.items.len(), 1);
    assert_eq!(read.item_misreads, [Vec::<Misread>::new()]);
    assert_eq!(read.unattributed_misreads.len(), 1);
    assert!(
        read.unattributed_misreads[0]
            .what
            .contains("has no operator")
    );
}

#[test]
fn states_direct_persistent_diagnostics_keep_item_alignment() {
    let read = read(
        b"bad tail clean = yes nested = { clean = yes bad tail } } after = no",
        Mode::KeepMark,
        Top::Persistent(false),
        None,
        &mut |nested, field| {
            if !nested && field.keyword_is("nested") {
                Shape::Object(true)
            } else {
                Shape::One
            }
        },
    );
    assert_eq!(read.items.len(), 3);
    assert_eq!(read.item_misreads.len(), read.items.len());
    assert_eq!(read.item_misreads[0].len(), 1);
    assert!(read.item_misreads[1].is_empty());
    assert_eq!(read.item_misreads[2].len(), 1);
    assert!(read.unattributed_misreads.is_empty());
    assert!(
        read.items
            .iter()
            .zip(&read.item_misreads)
            .any(|(item, _)| item.keyword_is("nested"))
    );
    assert_eq!(
        read.items[2].value.items().unwrap().len(),
        2,
        "nested objects must not create extra top-level diagnostic entries"
    );
}

#[test]
fn states_nested_variable_diagnostics_are_file_scoped() {
    let read = read(
        b"state = { @a = 1 @a = 2 name = @a } state = { name = @a }",
        Mode::KeepMark,
        Top::File(false),
        None,
        &mut |nested, _| {
            if nested {
                Shape::One
            } else {
                Shape::Object(true)
            }
        },
    );
    assert_eq!(read.items.len(), 2);
    assert!(read.item_misreads[1].is_empty());
    assert_eq!(read.unattributed_misreads.len(), 1);
    assert!(
        read.unattributed_misreads[0]
            .what
            .contains("variable `@a` is already taken")
    );
    for item in read.items {
        assert_eq!(
            item.value
                .items()
                .unwrap()
                .iter()
                .find(|field| field.keyword_is("name"))
                .unwrap()
                .value
                .scalar(),
            Some(b"1".as_slice())
        );
    }
}

fn texts(tokens: &[Token]) -> Vec<(Kind, String)> {
    tokens
        .iter()
        .map(|token| {
            (
                token.kind,
                String::from_utf8_lossy(&token.text).into_owned(),
            )
        })
        .collect()
}

#[test]
fn the_lexer_splits_as_the_game_does() {
    let lexed = lex(
        b"a=b!c\nd<5 e>=f (g,h) \"x\\\"y\\\\z\\n\" ?= ?q; tail\n{}\xff",
        Mode::KeepMark,
    );
    let tokens = lexed.tokens;
    assert_eq!(
        texts(&tokens),
        [
            (Kind::Bare, "a".into()),
            (Kind::Equals, "=".into()),
            (Kind::Bare, "b".into()),
            (Kind::Bare, "d".into()),
            (Kind::Less, "<".into()),
            (Kind::Bare, "5".into()),
            (Kind::Bare, "e".into()),
            (Kind::Greater, ">".into()),
            (Kind::Equals, "=".into()),
            (Kind::Bare, "f".into()),
            (Kind::Other, "(".into()),
            (Kind::Bare, "g".into()),
            (Kind::Other, ",".into()),
            (Kind::Bare, "h".into()),
            (Kind::Other, ")".into()),
            (Kind::Quoted, "x\"y\\z\\n".into()),
            (Kind::Other, "?=".into()),
            (Kind::Bare, "q".into()),
            (Kind::Open, "{".into()),
            (Kind::Close, "}".into()),
            (Kind::End, "".into()),
        ]
    );
    assert_eq!(tokens[3].line, 2);
}

#[test]
fn a_byte_0xff_is_one_end_and_reading_goes_on_after_it() {
    let tokens = |bytes: &[u8]| texts(&lex(bytes, Mode::KeepMark).tokens);
    let token = |kind, text: &str| (kind, text.to_owned());
    assert_eq!(
        tokens(b"a \xff b = c\n"),
        [
            token(Kind::Bare, "a"),
            token(Kind::End, ""),
            token(Kind::Bare, "b"),
            token(Kind::Equals, "="),
            token(Kind::Bare, "c"),
        ]
    );
    assert_eq!(
        tokens(b"ab\xffc\n"),
        [token(Kind::End, "ab"), token(Kind::Bare, "c")]
    );
    assert_eq!(tokens(b"# x \xff y\n"), [token(Kind::Bare, "y")]);
    assert_eq!(
        tokens(b"\"a\xffb\" c\n"),
        [
            token(Kind::Quoted, "a"),
            token(Kind::Bare, "b"),
            token(Kind::Quoted, " c\n"),
        ]
    );
    assert_eq!(
        tokens(b"?\xffa\n"),
        [token(Kind::End, "?"), token(Kind::Bare, "a")]
    );
    let read = test_read(b"a = { n = ?\xff n = 2 }");
    assert_eq!(
        block(&read.items, "a")[0].value,
        Value::Scalar(b"?".to_vec())
    );
}

#[test]
fn a_failed_retry_keeps_the_question_mark_or_comment_marker() {
    let tokens = |bytes: &[u8]| texts(&lex(bytes, Mode::KeepMark).tokens);
    let token = |kind, text: &str| (kind, text.to_owned());
    assert_eq!(
        tokens(b"? \n\xff a"),
        [token(Kind::End, "?"), token(Kind::End, "a")]
    );
    assert_eq!(
        tokens(b"x ?  "),
        [token(Kind::Bare, "x"), token(Kind::End, "?")]
    );
    for marker in ["!", "#", ";"] {
        let bytes = [b"x ".as_slice(), marker.as_bytes(), b"\xff\xff y\n"].concat();
        assert_eq!(
            tokens(&bytes),
            [
                token(Kind::Bare, "x"),
                token(Kind::End, marker),
                token(Kind::Bare, "y"),
            ],
            "{marker}"
        );
    }
    assert_eq!(
        tokens(b"x # tail\n"),
        [token(Kind::Bare, "x"), token(Kind::End, "#")]
    );
    assert_eq!(tokens(b"? # c\n y"), [token(Kind::End, "y")]);
    for (text, value) in [
        (&b"a = { n = ? \xff n = 2 }"[..], "?"),
        (b"a = { n = ?\n", "?"),
        (b"a = { n = #\xff\xff n = 2 }", "#"),
        (b"a = { n = ; c\n", ";"),
    ] {
        let read = test_read(text);
        assert_eq!(
            block(&read.items, "a")[0].value,
            Value::Scalar(value.as_bytes().to_vec()),
            "{}",
            String::from_utf8_lossy(text)
        );
    }
    let read = test_read(b"a = { n = 1 \xff n = 2 } b = { n = 3 }");
    assert_eq!(keys(&read.items), ["a", "n", "b"]);
    assert_eq!(keys(block(&read.items, "a")), ["n"]);
    assert_eq!(keys(block(&read.items, "b")), ["n"]);
    let read = test_read(b"a = { n = 1 } \xff b = { n = 3 }");
    assert_eq!(keys(&read.items), ["a"]);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Test {
    Top,
    Body,
}

fn test_read(text: &[u8]) -> Read {
    read(
        text,
        Mode::KeepMark,
        Top::File(Test::Top),
        None,
        &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
            (Test::Top, _) => Shape::Object(Test::Body),
            (_, Some(b"obj")) => Shape::Object(Test::Body),
            (_, Some(b"list")) => Shape::List,
            (_, Some(b"n")) => Shape::One,
            (_, Some(b"trigger")) => Shape::ByKey,
            _ => Shape::Skip,
        },
    )
}

fn keys(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|item| String::from_utf8_lossy(item.key.as_deref().unwrap_or(b"-")).into_owned())
        .collect()
}

fn block<'a>(items: &'a [Item], key: &str) -> &'a [Item] {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key.as_bytes()))
        .and_then(|item| item.value.items())
        .unwrap()
}

fn object_list_read(text: &[u8]) -> Read {
    read(
        text,
        Mode::KeepMark,
        Top::File(false),
        None,
        &mut |inside, field: &Field<'_>| {
            if !inside && field.keyword_is("list") {
                Shape::ObjectList(true)
            } else {
                Shape::One
            }
        },
    )
}

#[test]
fn object_lists_read_each_raw_entry_as_an_object() {
    let result =
        object_list_read(b"list = { { @inside = 3 n = @inside } { n = 5 } } final = @inside");
    assert!(result.unsettled.is_none(), "{result:?}");
    let entries = block(&result.items, "list");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|item| item.key.is_none()));
    assert_eq!(entries[0].kind, Kind::Open);
    assert_eq!(
        entries[0].value.items().unwrap()[0].value,
        Value::Scalar(b"3".to_vec())
    );
    assert_eq!(
        entries[1].value.items().unwrap()[0].value,
        Value::Scalar(b"5".to_vec())
    );
    assert_eq!(result.items[1].value, Value::Scalar(b"3".to_vec()));
}

#[test]
fn object_lists_read_scalar_outer_values_and_scalar_entries_unconditionally() {
    for (text, entry_kind) in [
        (b"list = scalar { n = 3 } } tail = 9".as_slice(), Kind::Open),
        (b"list = { scalar n = 3 } } tail = 9".as_slice(), Kind::Bare),
    ] {
        let result = object_list_read(text);
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(keys(&result.items), ["list", "tail"]);
        let entries = block(&result.items, "list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, entry_kind);
        assert_eq!(
            entries[0].value.items().unwrap()[0].value,
            Value::Scalar(b"3".to_vec())
        );
        assert_eq!(result.items[1].value, Value::Scalar(b"9".to_vec()));
    }
}

#[test]
fn a_block_where_one_value_is_read_shifts_the_rest_up() {
    let read = test_read(b"a = { n = { n = 2 x = 3 } n = 4 } b = { n = 5 }");
    assert_eq!(keys(&read.items), ["a", "n", "b"]);
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "n", "x"]);
    assert_eq!(a[0].value, Value::Scalar(b"{".to_vec()));
    assert_eq!(a[1].value, Value::Scalar(b"2".to_vec()));
    assert!(block(&read.items, "n").is_empty());
    assert_eq!(keys(block(&read.items, "b")), ["n"]);
    assert_eq!(read.misreads.len(), 2, "{:?}", read.misreads);
    assert!(read.misreads[0].what.starts_with("field `n` is a block"));
}

#[test]
fn a_last_bare_token_fails_its_advance_but_keeps_its_text() {
    let last = |bytes: &[u8]| {
        lex(bytes, Mode::KeepMark)
            .tokens
            .last()
            .map(|token| token.kind)
    };
    assert_eq!(last(b"a = { list = { x Final"), Some(Kind::End));
    assert_eq!(last(b"a = { list = { x Final\n"), Some(Kind::Bare));
    assert_eq!(last(b"a = \"Final"), Some(Kind::Quoted));
    let read = test_read(b"a = { list = { x Final");
    let list = block(block(&read.items, "a"), "list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].value, Value::Scalar(b"x".to_vec()));
    let read = test_read(b"a = { list = { x Final\n");
    assert_eq!(block(block(&read.items, "a"), "list").len(), 2);
    let read = test_read(b"a = { n = Final");
    assert_eq!(
        block(&read.items, "a")[0].value,
        Value::Scalar(b"Final".to_vec())
    );
    let read = test_read(b"a = { n = 1 } stray");
    assert_eq!(keys(&read.items), ["a"]);
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
}

#[test]
fn a_pair_reads_one_entry_and_skips_the_rest_of_its_block() {
    let pair = |text: &[u8]| {
        read(
            text,
            Mode::KeepMark,
            Top::File(Test::Top),
            None,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"pair")) => Shape::Pair(Test::Body),
                (_, Some(b"obj")) => Shape::Object(Test::Body),
                _ => Shape::One,
            },
        )
    };
    let read = pair(b"a = { pair = { x = 1 } n = 2 } b = { n = 3 }");
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["pair", "n"]);
    assert_eq!(keys(block(a, "pair")), ["x"]);
    let read = pair(b"a = { pair = { x = 1 @y = 2 obj = { z = 1 } w } n = @y } b = { n = 3 }");
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["pair", "n"]);
    assert_eq!(a[1].value, Value::Scalar(b"@y".to_vec()));
    assert!(
        read.misreads[0].what.contains("more than one entry"),
        "{:?}",
        read.misreads
    );
    let read = pair(b"a = { pair = 5 n = 2 }");
    assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);
    assert!(
        read.misreads[0]
            .what
            .contains("where its reader reads `{ key = value }`")
    );
}

#[test]
fn an_object_or_list_given_a_value_is_reported() {
    let read = read(
        b"a = { n = 1 obj { m = 1 } list = 5 n = 2 }",
        Mode::KeepMark,
        Top::File(Test::Top),
        None,
        &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
            (Test::Top, _) => Shape::Object(Test::Body),
            (_, Some(b"obj")) => Shape::ObjectOrSkip(Test::Body),
            (_, Some(b"list")) => Shape::ListOrSkip,
            _ => Shape::One,
        },
    );
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "obj", "list", "n"]);
    assert_eq!(a[3].value, Value::Scalar(b"2".to_vec()));
    assert!(
        read.misreads[1].what.contains("it is reported"),
        "{:?}",
        read.misreads
    );
}

#[test]
fn a_byte_order_mark_is_skipped_only_in_mode_one() {
    let text = b"\xef\xbb\xbfa = { n = 1 } b = { n = 2 }";
    assert_eq!(lex(text, Mode::SkipMark).tokens[0].text, b"a");
    assert_eq!(lex(text, Mode::KeepMark).tokens[0].text, b"\xef\xbb\xbfa");
    let finds_a = |mode| {
        let mut found = false;
        read(
            text,
            mode,
            Top::File(Test::Top),
            None,
            &mut |_, field: &Field<'_>| {
                found |= field.keyword_is("a");
                Shape::Skip
            },
        );
        found
    };
    assert!(finds_a(Mode::SkipMark));
    assert!(!finds_a(Mode::KeepMark));
}

#[test]
fn a_failed_peek_keeps_a_quoted_tokens_flag() {
    for text in [&b"@x = 1 k = \"@x\"\xff"[..], b"@x = 1 k = \"@x\""] {
        let lexed = lex(text, Mode::KeepMark);
        let mut reader = Reader::new(lexed);
        reader.key_operator_value();
        assert_eq!(reader.value.text, b"@x");
        assert_eq!(reader.peek().kind, Kind::End);
        assert!(reader.current.quoted);
        reader.advance();
        let current = reader.resolve(reader.current());
        assert_eq!(current.text, b"@x", "{text:?}");
        assert!(current.quoted);
    }
}

#[test]
fn a_skip_resolves_each_token_it_reads() {
    let read = test_read(b"@o = { a = { u = { @o } } n = 1 } b = { n = 2 }");
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(keys(block(&read.items, "a")), ["u", "n"]);
    assert_eq!(
        block(&read.items, "a")[1].value,
        Value::Scalar(b"1".to_vec())
    );
    let read = test_read(b"a = { u = { @none } n = 1 } b = { n = 2 }");
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(keys(block(&read.items, "a")), ["u", "n"]);
}

#[test]
fn a_text_ending_in_a_definition_dispatches_the_key_slot() {
    let read = test_read(b"@x = 1");
    assert_eq!(keys(&read.items), [""]);
    assert!(
        read.misreads[0]
            .what
            .starts_with("the text ends after a definition")
    );
    let read = test_read(b"a = { n = 2 } @y = 3");
    assert_eq!(keys(&read.items), ["a", "n"]);
}

#[test]
fn a_value_without_a_key_consumes_the_next_token() {
    let read = test_read(b"a = { n = 7 stray n = 1 unknown { n = 9 } n = 2 }");
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "stray", "=", "unknown", "n"]);
    assert_eq!(a[1].value, Value::Scalar(b"7".to_vec()));
    assert_eq!(a[4].value, Value::Scalar(b"2".to_vec()));
    assert_eq!(read.misreads.len(), 3, "{:?}", read.misreads);
    assert!(
        read.misreads[0]
            .what
            .starts_with("key `stray` has no operator")
    );
}

#[test]
fn definitions_follow_the_readers_consumption() {
    let read = test_read(
        b"@x = 1 a = { n = @x @x = 2 n = \"@x\" obj = { @y = 3 z = 1 } n = @y } b = { n = @y @z = 4 } c = { n = 5 }",
    );
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "=", "n", "obj", "n"]);
    assert_eq!(a[0].value, Value::Scalar(b"1".to_vec()));
    assert_eq!(a[2].value, Value::Scalar(b"@x".to_vec()));
    assert!(a[2].quoted());
    assert_eq!(a[4].value, Value::Scalar(b"3".to_vec()));
    let b = block(&read.items, "b");
    assert_eq!(keys(b), ["n", "}", "="]);
    assert!(
        read.misreads[0]
            .what
            .contains("variable `@x` is already taken")
    );
}

#[test]
fn a_scalar_where_a_block_is_read_pulls_the_rest_in() {
    let read = test_read(b"a = { obj = 1 n = 2 } b = { n = 3 }");
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["obj", "b"]);
    assert_eq!(keys(block(a, "obj")), ["n"]);
    assert_eq!(read.misreads.len(), 1, "{:?}", read.misreads);
    let read = test_read(b"b = { list = x y } c = { n = 1 }");
    let b = block(&read.items, "b");
    assert_eq!(keys(b), ["list", "c"]);
    assert_eq!(block(b, "list")[0].value, Value::Scalar(b"y".to_vec()));
    let read = test_read(b"c = { other = { n = 3 { } } n = 4 }");
    assert_eq!(keys(block(&read.items, "c")), ["other", "n"]);
    assert!(read.misreads.is_empty());
}

#[test]
fn by_key_blocks_with_values_without_keys() {
    let read = test_read(b"a = { trigger = { x y = 1 } n = 2 } b = { n = 3 }");
    assert!(read.unsettled.is_none());
    assert_eq!(keys(block(&read.items, "a")), ["trigger", "n"]);
    let read = test_read(b"a = { trigger = { x @y = 1 } n = 2 } b = { n = 3 }");
    assert!(read.unsettled.unwrap().what.starts_with(
        "variable @y is in a trigger or effect block that also holds anonymous values"
    ));
    assert_eq!(keys(&read.items), ["a"]);
}

#[test]
fn a_known_trigger_body_is_read_by_key_never_as_a_list() {
    crate::test_game::enter();
    let body = |text: &[u8]| {
        read(
            text,
            Mode::KeepMark,
            Top::File(Test::Top),
            None,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"body")) => Shape::Body(Test::Body),
                (_, Some(b"loose")) => Shape::Trigger,
                (_, Some(b"trigger")) => Shape::ByKey,
                _ => Shape::One,
            },
        )
    };
    let read =
        body(b"a = { body = { x @v = 1 n = \"5\" } loose = { y = 2 } n = @v } b = { n = 3 }");
    assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["body", "loose", "n"]);
    let inside = block(a, "body");
    assert_eq!(keys(inside), ["x", "=", "n"]);
    assert_eq!(inside[2].kind, Kind::Quoted);
    assert!(inside[2].quoted());
    assert_eq!(a[2].value, Value::Scalar(b"@v".to_vec()));
    assert!(read.misreads[0].what.starts_with("key `x` has no operator"));
    let read = body(b"a = { body = yes n = 1 }");
    assert_eq!(keys(block(&read.items, "a")), ["body", "n"]);
    assert!(
        read.misreads[0]
            .what
            .contains(
                "field `body` is `yes` where its trigger reads a block: the missing block is reported and nothing is read"
            ),
        "{:?}",
        read.misreads
    );
    let read = body(b"a = { trigger = { x y = \"5\" } }");
    let list = block(block(&read.items, "a"), "trigger");
    assert_eq!(list[1].kind, Kind::Quoted);
    assert_eq!(list[1].value, Value::Scalar(b"5".to_vec()));
}

#[test]
fn a_string_pair_reads_two_tokens_and_skips_to_its_end() {
    let pair = |text: &[u8]| {
        read(
            text,
            Mode::KeepMark,
            Top::File(Test::Top),
            None,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"pair")) => Shape::Strings,
                _ => Shape::One,
            },
        )
    };
    let strings = |read: &Read| {
        block(block(&read.items, "a"), "pair")
            .iter()
            .map(|item| String::from_utf8_lossy(item.value.scalar().unwrap()).into_owned())
            .collect::<Vec<_>>()
    };
    let read = pair(b"a = { pair = { \"x y\" 5 } n = 1 }");
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
    assert_eq!(strings(&read), ["x y", "5"]);
    assert_eq!(block(block(&read.items, "a"), "pair")[0].kind, Kind::Quoted);
    assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);

    let read = pair(b"a = { pair = { x y { n = 9 } extra { z } } n = 1 }");
    assert_eq!(strings(&read), ["x", "y"]);
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["pair", "n"]);
    assert_eq!(a[1].value, Value::Scalar(b"1".to_vec()));
    assert!(
        read.misreads[0]
            .what
            .contains("holds `{` after its two strings")
    );

    let read = pair(b"a = { pair = { { x } n = 2 }");
    assert_eq!(strings(&read), ["{", "x"]);
    assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);

    let read = pair(b"a = { pair = { @v = 1 } n = @v }");
    assert_eq!(strings(&read), ["@v", "="]);
    assert_eq!(
        block(&read.items, "a")[1].value,
        Value::Scalar(b"@v".to_vec())
    );

    let read = pair(b"a = { pair = { x } n = 1 }");
    assert_eq!(strings(&read), ["x"]);
    assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);
    assert!(read.misreads[0].what.contains("holds 1 string(s)"));

    let read = pair(b"a = { pair = x n = 1 }");
    assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);
    assert!(read.misreads[0].what.contains("reads `{ string string }`"));
}

#[test]
fn an_untraced_key_without_an_operator_follows_the_stale_value() {
    crate::test_game::enter();
    let trigger = |text: &[u8]| {
        read(
            text,
            Mode::KeepMark,
            Top::File(Test::Top),
            None,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"loose")) => Shape::Trigger,
                _ => Shape::One,
            },
        )
    };
    let read = trigger(b"a = { loose = { Or { x = 1 } y = 2 } n = 3 }");
    assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["loose", "n"]);
    let loose = block(a, "loose");
    assert_eq!(keys(loose), ["Or", "y"]);
    assert_eq!(keys(block(loose, "Or")), ["x"]);
    assert!(
        read.misreads[0]
            .what
            .starts_with("key `Or` has no operator")
    );

    let read = trigger(b"a = { loose = { y = 2 Or { x = 1 } z = 1 } n = 3 }");
    assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
    let loose = block(block(&read.items, "a"), "loose");
    assert_eq!(keys(loose), ["y", "Or", "z"]);
    assert_eq!(loose[1].value, Value::Block(Vec::new()));

    let read = trigger(b"a = { loose = { always yes } n = 3 }");
    assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
    assert_eq!(keys(block(&read.items, "a")), ["loose", "n"]);
    let read = trigger(b"a = { loose = { maybe yes } n = 3 }");
    let unsettled = read.unsettled.unwrap();
    assert_eq!(unsettled.line, 1);
    assert!(
        unsettled
            .what
            .starts_with("field `maybe`: trigger dispatch reads it"),
        "{unsettled}"
    );
}

#[test]
fn items_keep_the_value_tokens_kind() {
    let kinds = |text: &[u8], name: &str| {
        let read = read(
            text,
            Mode::KeepMark,
            Top::File(Test::Top),
            None,
            &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
                (Test::Top, _) => Shape::Object(Test::Body),
                (_, Some(b"current")) => Shape::Current,
                _ => Shape::One,
            },
        );
        block(&read.items, name)
            .iter()
            .map(|item| (item.kind, item.value_keyword()))
            .collect::<Vec<_>>()
    };
    let text = b"a = { type = Ship type = \"ship\" type = 3 current = } b = { type = ship";
    assert_eq!(
        kinds(text, "a"),
        [
            (Kind::Bare, Some("ship".to_owned())),
            (Kind::Quoted, None),
            (Kind::Bare, None),
            (Kind::Close, None),
        ]
    );
    assert_eq!(kinds(text, "b"), [(Kind::End, None)]);
    let result = read(
        text,
        Mode::KeepMark,
        Top::File(Test::Top),
        None,
        &mut |schema, _| match schema {
            Test::Top => Shape::Object(Test::Body),
            Test::Body => Shape::One,
        },
    );
    assert_eq!(
        result.items[1].value.items().unwrap()[0].value,
        Value::Scalar(b"ship".to_vec())
    );
}

#[test]
fn items_keep_the_operator_slot() {
    let read = test_read(b"a = { n < 5 stray n n > 3 n = 1 } b = { n >= 2 }");
    let a = block(&read.items, "a");
    let operators: Vec<_> = a.iter().map(|item| item.operator).collect();
    assert_eq!(keys(a), ["n", "stray", "n", "n"]);
    assert_eq!(
        operators,
        [
            Operator::Less,
            Operator::Less,
            Operator::Greater,
            Operator::Equals
        ]
    );
    let b = block(&read.items, "b");
    assert_eq!(b[0].operator, Operator::Greater);
    assert_eq!(b[0].value, Value::Scalar(b"=".to_vec()));
}

#[test]
fn a_block_keeps_its_own_operator_after_comparisons_inside() {
    let read = read(
        b"t = { Or = { phase > 99 } n = 1 AND = { x < 2 obj = { y > 3 } } trigger = { OR = { phase > 99 } } }",
        Mode::KeepMark,
        Top::File(Test::Top), None,
        &mut |schema, field: &Field<'_>| match (schema, field.keyword().as_deref()) {
            (Test::Top, _) | (_, Some(b"obj")) => Shape::Object(Test::Body),
            (_, Some(b"or" | b"and")) => Shape::Object(Test::Body),
            (_, Some(b"trigger")) => Shape::ByKey,
            _ => Shape::One,
        },
    );
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
    let t = block(&read.items, "t");
    let operators = |items: &[Item]| -> Vec<_> {
        items
            .iter()
            .map(|item| (keys(std::slice::from_ref(item)).remove(0), item.operator))
            .collect()
    };
    let keyed = |key: &str, operator| (key.to_owned(), operator);
    assert_eq!(
        operators(t),
        [
            keyed("Or", Operator::Equals),
            keyed("n", Operator::Equals),
            keyed("AND", Operator::Equals),
            keyed("trigger", Operator::Equals),
        ]
    );
    assert_eq!(
        operators(block(t, "Or")),
        [keyed("phase", Operator::Greater)]
    );
    let and = block(t, "AND");
    assert_eq!(
        operators(and),
        [keyed("x", Operator::Less), keyed("obj", Operator::Equals)]
    );
    assert_eq!(
        operators(block(and, "obj")),
        [keyed("y", Operator::Greater)]
    );
    let trigger = block(t, "trigger");
    assert_eq!(operators(trigger), [keyed("OR", Operator::Equals)]);
    assert_eq!(
        operators(block(trigger, "OR")),
        [keyed("phase", Operator::Greater)]
    );
}

#[test]
fn a_reference_keeps_its_definitions_token_kind() {
    let read = test_read(
        b"@quoted = \"center\" @bare = center @chain = @quoted @N = 3 \
          a = { n = @quoted n = @bare n = @chain n = \"@bare\" n = @none n = @n }",
    );
    let a = block(&read.items, "a");
    let values: Vec<_> = a
        .iter()
        .map(|item| (item.value.scalar().unwrap().to_vec(), item.quoted()))
        .collect();
    assert_eq!(
        values,
        [
            (b"center".to_vec(), true),
            (b"center".to_vec(), false),
            (b"center".to_vec(), true),
            (b"@bare".to_vec(), true),
            (b"@none".to_vec(), false),
            (b"@n".to_vec(), false),
        ]
    );
    assert!(
        read.misreads[0]
            .what
            .contains("definition `@chain` takes `@quoted`")
    );
    assert!(
        read.misreads[1]
            .what
            .contains("variable `@quoted` is already taken")
    );
}

fn gui_tokens(bytes: &[u8]) -> Vec<Token> {
    lex(bytes, Mode::KeepMark).tokens
}

fn shown_gui(bytes: &[u8]) -> Vec<String> {
    gui_tokens(bytes)
        .iter()
        .map(|token| {
            let text = String::from_utf8_lossy(&token.text);
            match token.kind {
                Kind::End => "the end of the file".into(),
                Kind::Quoted => format!("\"{text}\""),
                Kind::Bare => text.into_owned(),
                _ => format!("'{text}'"),
            }
        })
        .collect()
}

#[test]
fn comments_and_punctuation_split_words_as_the_game_splits_them() {
    assert_eq!(
        shown_gui(b"a = 0; b = 1\nc = x!y z\nd = \"q;#!\" # e = 2\nf=rgb{1}g<(h),i?=j\n"),
        [
            "a", "'='", "0", "c", "'='", "x", "d", "'='", "\"q;#!\"", "f", "'='", "rgb", "'{'",
            "1", "'}'", "g", "'<'", "'('", "h", "')'", "','", "i?", "'='", "j",
        ]
    );
    assert_eq!(shown_gui(b"?x ?= y\n"), ["x", "'?='", "y"]);
    assert_eq!(shown_gui(b"@a[1] 50%% -1.5\n"), ["@a[1]", "50%%", "-1.5"]);
}

#[test]
fn quoted_strings_keep_the_game_escapes() {
    let tokens = gui_tokens(b"\"x\\\"y\\\\\" \"a\\nb\" \"\\\\\" \"line\nfeed\" \"open");
    let texts: Vec<_> = tokens.iter().map(|token| token.text.clone()).collect();
    assert_eq!(
        texts,
        [
            b"x\"y\\".to_vec(),
            b"a\\nb".to_vec(),
            b"\\".to_vec(),
            b"line\nfeed".to_vec(),
            b"open".to_vec(),
        ]
    );
    assert!(tokens.iter().all(Token::quoted));
}

#[test]
fn number_and_keyword_tokens_follow_the_lexer() {
    let tokens = gui_tokens(b"12 -x .5 x \"5\" X 3dposition \0q\n");
    let numbers: Vec<_> = tokens[..8].iter().map(Token::is_number).collect();
    assert_eq!(numbers, [true, true, true, false, false, false, true, true]);
    assert!(tokens[5].is_keyword("x") && !tokens[4].is_keyword("5"));
    assert!(!tokens[6].is_keyword("3dposition"));
    let nul = gui_tokens(b"x\0yz = 1\n");
    assert!(nul[0].is_keyword("x"));
    assert_eq!(nul[0].text, b"x\0yz");
}

#[test]
fn keyword_dispatch_stops_at_the_token_nul() {
    let mut seen = Vec::new();
    let read = read(
        b"Buildings suffix = { cost = Land x }  lead = 1",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field| {
            seen.push((field.keyword(), field.value_keyword()));
            if field.keyword_is("buildings") {
                Shape::Body(())
            } else {
                Shape::One
            }
        },
    );
    assert_eq!(read.unsettled, None);
    assert_eq!(
        seen,
        [
            (Some(b"buildings".to_vec()), None),
            (Some(b"cost".to_vec()), Some(b"land".to_vec())),
            (None, None),
        ]
    );
    let root = &read.items[0];
    assert!(root.keyword_is("buildings"));
    assert_eq!(root.keyword().unwrap().text(), b"Buildings");
    let cost = &root.value.items().unwrap()[0];
    assert_eq!(cost.value_keyword().as_deref(), Some("land"));
    assert_eq!(read.items[1].keyword(), None);
}

#[test]
fn a_byte_0xff_is_one_end_of_the_stream() {
    let kinds = |bytes: &[u8]| -> Vec<(Kind, Vec<u8>)> {
        gui_tokens(bytes)
            .into_iter()
            .map(|token| (token.kind, token.text))
            .collect()
    };
    assert_eq!(
        kinds(b"a \xff b"),
        [
            (Kind::Bare, b"a".to_vec()),
            (Kind::End, Vec::new()),
            (Kind::End, b"b".to_vec()),
        ]
    );
    assert_eq!(
        kinds(b"ab\xffc\n"),
        [(Kind::End, b"ab".to_vec()), (Kind::Bare, b"c".to_vec())]
    );
    assert_eq!(kinds(b"# x \xff y\n"), [(Kind::Bare, b"y".to_vec())]);
    assert_eq!(
        kinds(b"\"a\xffb\" c\n"),
        [
            (Kind::Quoted, b"a".to_vec()),
            (Kind::Bare, b"b".to_vec()),
            (Kind::Quoted, b" c\n".to_vec()),
        ]
    );
    assert_eq!(
        kinds(b"?\xffa\n"),
        [(Kind::End, b"?".to_vec()), (Kind::Bare, b"a".to_vec())]
    );
    assert_eq!(kinds(b"\xef\xbb\xbfa = b\n")[0].1, b"\xef\xbb\xbfa");
    assert_eq!(
        shown_gui("a\u{a0}b \u{e9}\n".as_bytes()),
        ["a\u{a0}b", "\u{e9}"]
    );
}

#[test]
fn modifiers_keep_scoped_operands_before_trigger_dispatch() {
    crate::test_game::enter();
    let names = KeyNames::default();
    let read = read(
        b"modifier = { factor = x add = 3 always = yes }",
        Mode::KeepMark,
        Top::File(()),
        Some(&names),
        &mut |(), field| mean_time_to_happen(field),
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    let modifier = read.items[0].value.items().unwrap();
    assert!(modifier[0].keyword_is("factor"));
    assert_eq!(modifier[0].value.scalar(), Some(b"x".as_slice()));
    assert!(modifier[1].keyword_is("add"));
    assert_eq!(modifier[1].value.scalar(), Some(b"3".as_slice()));

    let railway = crate::triggers::read(
        b"can_build_railway = { controller_priority = { modifier = { factor = x add = 3 always = yes } } }",
        &names,
    );
    assert!(railway.unsettled.is_none(), "{railway:?}");
    let priority = railway.items[0].value.items().unwrap()[0]
        .value
        .items()
        .unwrap();
    assert_eq!(priority[0].value.items(), Some(modifier));
}

#[test]
fn keyed_definitions_under_unknown_dispatch_refuse_dependent_suffix() {
    let direct = test_read(
        b"a = { n = 1 } b = { trigger = { @y = 3 z = 1 } } c = { n = @y } d = { n = 4 }",
    );
    let unsettled = direct.unsettled.as_ref().expect("dependent suffix refused");
    assert!(
        unsettled.what.contains("@y") && unsettled.what.contains("trigger"),
        "{unsettled}"
    );
    assert_eq!(keys(&direct.items), ["a", "b", "c"]);
    assert_eq!(block(&direct.items, "a")[0].value, Value::Scalar(b"1".to_vec()));
    assert!(block(&direct.items, "c").is_empty());

    let nested = test_read(
        b"a = { n = 1 } b = { trigger = { nested = { @y = 3 z = 1 } } } c = { n = @y }",
    );
    let unsettled = nested.unsettled.as_ref().expect("nested dependent suffix refused");
    assert!(
        unsettled.what.contains("@y") && unsettled.what.contains("nested"),
        "{unsettled}"
    );
    assert_eq!(keys(&nested.items), ["a", "b", "c"]);
    assert!(block(&nested.items, "c").is_empty());

    let redefined = test_read(
        b"a = { n = 1 } b = { trigger = { @y = 3 z = 1 } } @y = 5 c = { n = @y }",
    );
    let unsettled = redefined.unsettled.as_ref().expect("redefinition refused");
    assert!(
        unsettled.what.contains("@y") && unsettled.what.contains("trigger"),
        "{unsettled}"
    );
    assert_eq!(keys(&redefined.items), ["a", "b"]);

    let local_only = test_read(
        b"a = { n = 1 } b = { trigger = { @y = 3 z = @y } } c = { n = 2 }",
    );
    assert!(local_only.unsettled.is_none(), "{local_only:?}");
    assert_eq!(keys(&local_only.items), ["a", "b", "c"]);
    let trigger = block(block(&local_only.items, "b"), "trigger");
    assert_eq!(trigger[0].value, Value::Scalar(b"3".to_vec()));
    assert_eq!(block(&local_only.items, "c")[0].value, Value::Scalar(b"2".to_vec()));

    let observed = test_read(
        b"a = { obj = { @y = 3 z = 1 } skipped = { @z = 9 } n = @y } b = { n = @z }",
    );
    assert!(observed.unsettled.is_none(), "{observed:?}");
    assert_eq!(block(&observed.items, "a")[2].value, Value::Scalar(b"3".to_vec()));
    assert_eq!(block(&observed.items, "b")[0].value, Value::Scalar(b"@z".to_vec()));
}

#[test]
fn trailing_definition_before_close_under_unknown_dispatch_refuses_without_swallowing_brace() {
    let read = test_read(b"a = { n = 1 } b = { trigger = { @x = 3 } n = @x } c = { n = 4 }");
    let unsettled = read.unsettled.as_ref().expect("trailing definition refused");
    assert!(
        unsettled.what.contains("@x") && unsettled.what.contains("trigger"),
        "{unsettled}"
    );
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(block(&read.items, "a")[0].value, Value::Scalar(b"1".to_vec()));
    assert!(block(block(&read.items, "b"), "trigger").is_empty());
}

#[test]
fn mixed_list_with_nested_definition_under_unknown_dispatch_refuses() {
    let read = test_read(
        b"a = { n = 1 } b = { trigger = { nested = { @x = 3 z = 1 } loose } n = @x } c = { n = 4 }",
    );
    let unsettled = read
        .unsettled
        .as_ref()
        .expect("mixed list nested definition refused");
    assert!(unsettled.what.contains("@x"), "{unsettled}");
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(block(&read.items, "a")[0].value, Value::Scalar(b"1".to_vec()));
}

#[test]
fn direct_resolution_readers_stop_before_storing_ambiguous_values() {
    let raw_list = read(
        b"opaque = { @t = support marker = yes } type = { @t infantry } tail = 1",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"opaque") => Shape::ByKey,
            Some(b"type") => Shape::RawListFirst,
            _ => Shape::One,
        },
    );
    assert!(raw_list.unsettled.is_some(), "{raw_list:?}");
    assert_eq!(keys(&raw_list.items), ["opaque", "type"]);
    assert!(block(&raw_list.items, "type").is_empty());

    let pair = read(
        b"opaque = { @s = first marker = yes } pair = { @s second } tail = 1",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"opaque") => Shape::ByKey,
            Some(b"pair") => Shape::Strings,
            _ => Shape::One,
        },
    );
    assert!(pair.unsettled.is_some(), "{pair:?}");
    assert_eq!(keys(&pair.items), ["opaque", "pair"]);
    assert!(block(&pair.items, "pair").is_empty());

    let color = read(
        b"opaque = { @c = 10 marker = yes } col = { @c 20 30 } tail = 1",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"opaque") => Shape::ByKey,
            Some(b"col") => Shape::Color,
            _ => Shape::One,
        },
    );
    assert!(color.unsettled.is_some(), "{color:?}");
    assert_eq!(keys(&color.items), ["opaque", "col"]);
    assert!(block(&color.items, "col").is_empty());
}

#[test]
fn valueless_definition_before_close_under_unknown_dispatch_refuses_without_swallowing_brace() {
    let read = test_read(b"a = { trigger = { @x = } n = @x } c = { n = 4 }");
    let unsettled = read.unsettled.as_ref().expect("valueless definition refused");
    assert!(
        unsettled.what.contains("@x") && unsettled.what.contains("trigger"),
        "{unsettled}"
    );
    assert_eq!(keys(&read.items), ["a"]);
    assert_eq!(keys(block(&read.items, "a")), ["trigger"]);
    assert!(block(block(&read.items, "a"), "trigger").is_empty());
}

#[test]
fn mixed_list_with_quoted_nested_definition_under_unknown_dispatch_refuses() {
    let read = test_read(
        br#"a = { n = 1 } b = { trigger = { nested = { "@x" = 3 marker = 1 } loose } n = @x } c = { n = 4 }"#,
    );
    let unsettled = read
        .unsettled
        .as_ref()
        .expect("quoted nested definition refused");
    assert!(unsettled.what.contains("@x"), "{unsettled}");
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(block(&read.items, "a")[0].value, Value::Scalar(b"1".to_vec()));
}

#[test]
fn known_dispatch_definition_at_eof_preserves_quoted_token_kind() {
    let read = read(
        br#"k = old @x = "tail""#,
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), _| Shape::Current,
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(read.items.len(), 2);
    assert_eq!(read.items[1].kind, Kind::Quoted);
    assert_eq!(read.items[1].value, Value::Scalar(b"tail".to_vec()));
}

#[test]
fn equipment_limit_and_math_arguments_preserve_proved_partial_children_on_refusal() {
    crate::test_game::enter();
    let names = KeyNames::default();
    let equip = read(
        b"opaque = { @x = flag marker = yes } enable_equipments = { limit = { always = yes has_country_flag = @x } tank_1 } tail = 1",
        Mode::KeepMark,
        Top::File(()),
        Some(&names),
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"opaque") => Shape::ByKey,
            Some(b"enable_equipments") => Shape::TechnologyEquipment,
            _ => Shape::One,
        },
    );
    assert!(equip.unsettled.is_some(), "{equip:?}");
    assert_eq!(keys(&equip.items), ["opaque", "enable_equipments"]);
    let limit_items = block(block(&equip.items, "enable_equipments"), "limit");
    assert_eq!(keys(limit_items), ["always"]);
    assert_eq!(limit_items[0].value, Value::Scalar(b"yes".to_vec()));

    let math = read(
        b"opaque = { @x = 5 marker = yes } expr = { value = 1 clamp = { min = { value = 2 add = @x } max = 9 } } tail = 1",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"opaque") => Shape::ByKey,
            Some(b"expr") => Shape::Expression,
            _ => Shape::One,
        },
    );
    assert!(math.unsettled.is_some(), "{math:?}");
    assert_eq!(keys(&math.items), ["opaque", "expr"]);
    let clamp = block(block(&math.items, "expr"), "clamp");
    assert_eq!(keys(clamp), ["min"]);
    let min = block(clamp, "min");
    assert_eq!(keys(min), ["value"]);
    assert_eq!(min[0].value, Value::Scalar(b"2".to_vec()));
}

#[test]
fn duplicate_valueless_definition_before_close_under_unknown_dispatch_refuses_without_absorbing_sibling() {
    let read = read(
        br#"@x = 1 a = { trigger = { @x = } n = @x } b = { n = 4 }"#,
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"a" | b"b") => Shape::Object(()),
            Some(b"trigger") => Shape::ByKey,
            _ => Shape::One,
        },
    );
    assert!(read.unsettled.is_some(), "{read:?}");
    assert_eq!(keys(&read.items), ["a"]);
    assert_eq!(keys(block(&read.items, "a")), ["trigger"]);
}

#[test]
fn balanced_valueless_operator_before_close_refuses_without_swallowing_siblings() {
    let read = read(
        br#"a = { trigger = { loose k = } @x = 7 n = @x } b = { n = @x }"#,
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"a" | b"b") => Shape::Object(()),
            Some(b"trigger") => Shape::ByKey,
            _ => Shape::One,
        },
    );
    assert!(read.unsettled.is_some(), "{read:?}");
    assert_eq!(keys(&read.items), ["a"]);
    assert_eq!(keys(block(&read.items, "a")), ["trigger"]);
    let trigger = block(block(&read.items, "a"), "trigger");
    assert_eq!(trigger.len(), 1, "{trigger:?}");
    assert_eq!(trigger[0].value, Value::Scalar(b"loose".to_vec()));
}

#[test]
fn balanced_final_variable_reference_at_eof_refuses_unknown_dispatch_definition() {
    let read = read(
        br#"a = { @v = 7 marker = yes } b = { loose k = @v"#,
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), _field: &Field<'_>| Shape::ByKey,
    );
    let unsettled = read.unsettled.as_ref().expect("unsettled refusal");
    assert!(unsettled.what.contains("@v"), "{unsettled:?}");
    assert!(unsettled.what.contains("`a`"), "{unsettled:?}");
    assert_eq!(keys(&read.items), ["a", "b"]);
    let b = block(&read.items, "b");
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0].value, Value::Scalar(b"loose".to_vec()));
}

#[test]
fn balanced_variable_resolving_to_open_brace_refuses_without_shifting_boundary() {
    let read = read(
        br#"@o = { a = { trigger = { loose k = @o } @x = 7 n = @x } b = { n = @x } }"#,
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"a" | b"b") => Shape::Object(()),
            Some(b"trigger") => Shape::ByKey,
            _ => Shape::One,
        },
    );
    assert!(read.unsettled.is_some(), "{read:?}");
    assert_eq!(keys(&read.items), ["a"]);
    assert_eq!(keys(block(&read.items, "a")), ["trigger"]);
    let trigger = block(block(&read.items, "a"), "trigger");
    assert_eq!(trigger.len(), 1, "{trigger:?}");
    assert_eq!(trigger[0].value, Value::Scalar(b"loose".to_vec()));
}

#[test]
fn balanced_embedded_end_marker_refuses_instead_of_continuing_past_scan() {
    let read = read(
        b"opaque = { one two k = 1\xff @x = 7 marker = yes } probe = @x ",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field: &Field<'_>| match field.keyword().as_deref() {
            Some(b"opaque") => Shape::ByKey,
            _ => Shape::One,
        },
    );
    assert!(read.unsettled.is_some(), "{read:?}");
    assert_eq!(keys(&read.items), ["opaque"]);
    let opaque = block(&read.items, "opaque");
    assert_eq!(opaque.len(), 2, "{opaque:?}");
    assert_eq!(opaque[0].value, Value::Scalar(b"one".to_vec()));
    assert_eq!(opaque[1].value, Value::Scalar(b"two".to_vec()));
}

#[test]
fn eof_variable_copy_warning_keeps_retained_source_name() {
    let read = read(
        b"@source = 7 @copy = @source",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), _| Shape::One,
    );
    assert!(
        read.misreads
            .iter()
            .any(|note| note.what.contains("@source")),
        "{:?}",
        read.misreads
    );
}

#[test]
fn recover_declared_names_explores_mixed_unknown_dispatch_and_merges_states() {
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    enum Node {
        Top,
        Decl,
        Inner,
    }
    let dispatch = |node: Node, field: &Field<'_>| match node {
        Node::Top => match field.keyword().as_deref() {
            Some(b"a" | b"b" | b"c" | b"d" | b"e" | b"f" | b"g" | b"entity") => {
                Shape::Object(Node::Decl)
            }
            _ => Shape::Skip,
        },
        Node::Decl => match field.keyword().as_deref() {
            Some(b"opaque" | b"atlas_size") => Shape::ByKey,
            Some(b"game_data") => Shape::Object(Node::Inner),
            Some(b"name") => Shape::One,
            _ => Shape::Skip,
        },
        Node::Inner => match field.keyword().as_deref() {
            Some(b"atlas_size") => Shape::ByKey,
            _ => Shape::Skip,
        },
    };
    let recover = |bytes: &[u8]| {
        recover_declared_names(
            bytes,
            Mode::KeepMark,
            Node::Top,
            |node| matches!(node, Node::Decl).then_some(NamePolicy::LastKeyword),
            |node, field| RecoveryShape::Exact(dispatch(node, field)),
        )
    };
    let to_strings = |explored: ExploredNames| -> (std::collections::BTreeSet<String>, bool) {
        (
            explored
                .names
                .into_iter()
                .map(|raw| String::from_utf8(raw).unwrap())
                .collect(),
            explored.exhausted,
        )
    };

    let redef_only = b"a = { opaque = { @v = one x = 1 } } @v = two b = { name = old name = @v }";
    assert!(
        read(
            redef_only,
            Mode::KeepMark,
            Top::File(Node::Top),
            None,
            &mut |s, f| dispatch(s, f)
        )
        .unsettled
        .is_some()
    );
    let (names, exhausted) = to_strings(recover(redef_only));
    assert!(!exhausted);
    assert_eq!(
        names,
        ["one", "two"].into_iter().map(str::to_owned).collect()
    );

    let multi_redef = b"a = { opaque = { @v = one x = 1 } } b = { opaque = { @v = two @v = ignored x = 1 } } @v = three @v = also_ignored @copy = @v c = { name = @copy }";
    assert!(
        read(
            multi_redef,
            Mode::KeepMark,
            Top::File(Node::Top),
            None,
            &mut |s, f| dispatch(s, f)
        )
        .unsettled
        .is_some()
    );
    let (names, exhausted) = to_strings(recover(multi_redef));
    assert!(!exhausted);
    assert_eq!(
        names,
        ["one", "two", "three"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );

    for (bytes, expected) in [
        (
            b"@id = wanted entity = { game_data = { atlas_size = { loose k = 1\xff x = 1 } } name = @id }"
                .as_slice(),
            "wanted",
        ),
        (
            b"@id = wanted_keyed entity = { game_data = { atlas_size = { a = 1 \xff x = 1 } } name = @id }"
                .as_slice(),
            "wanted_keyed",
        ),
        (
            b"@id = wanted_after_def entity = { game_data = { atlas_size = { @x = 1 \xff } } name = @id }"
                .as_slice(),
            "wanted_after_def",
        ),
    ] {
        assert!(
            read(
                bytes,
                Mode::KeepMark,
                Top::File(Node::Top),
                None,
                &mut |s, f| dispatch(s, f)
            )
            .unsettled
            .is_some()
        );
        let (names, exhausted) = to_strings(recover(bytes));
        assert!(!exhausted);
        assert_eq!(names, [expected.to_owned()].into_iter().collect());
    }

    let copy_revisit = b"entity = { game_data = { atlas_size = { @v = wanted x = 1 } } } @copy = @v entity = { name = @copy }";
    let (names, exhausted) = to_strings(recover(copy_revisit));
    assert!(!exhausted);
    assert_eq!(names, ["wanted".to_owned()].into_iter().collect());

    let brace_value = b"@o = { a = { opaque = { loose k = @o } name = \"after_brace_var\" } }";
    let (names, exhausted) = to_strings(recover(brace_value));
    assert!(!exhausted);
    assert_eq!(names, ["after_brace_var".to_owned()].into_iter().collect());

    let mut converging = String::from("entity = { name = \"merged\"\n");
    for i in 0..64 {
        converging.push_str(&format!(
            "game_data = {{ atlas_size = {{ @dead_{i} = {i} x = 1 y = 2 }} }}\n"
        ));
    }
    converging.push_str("}\n");
    let (names, exhausted) = to_strings(recover(converging.as_bytes()));
    assert!(!exhausted, "converging blocks should merge without exhausting state budget");
    assert_eq!(names, ["merged".to_owned()].into_iter().collect());

    let mut exploding = String::from("entity = { name = \"overflow_target\"\n");
    for i in 0..24 {
        exploding.push_str(&format!(
            "game_data = {{ atlas_size = {{ @live_{i} = {i} x = 1 }} }}\n"
        ));
    }
    for i in 0..24 {
        exploding.push_str(&format!("ref_{i} = @live_{i}\n"));
    }
    exploding.push_str("}\nentity = { name = ;tail");
    let (names, exhausted) = to_strings(recover(exploding.as_bytes()));
    assert!(exhausted, "exponential live variables should hit budget and fail closed");
    assert!(names.contains("overflow_target"));
    assert!(names.contains(";"), "punctuation End token must be preserved on fallback: {names:?}");

    let mut linear = String::from("entity = {\n");
    for _ in 0..70_000 {
        linear.push_str("x = 1\n");
    }
    linear.push_str("name = linear_tail\n}\n");
    let (names, exhausted) = to_strings(recover(linear.as_bytes()));
    assert!(!exhausted, "long deterministic file must not exhaust ambiguity budget");
    assert_eq!(names, ["linear_tail".to_owned()].into_iter().collect());
}
