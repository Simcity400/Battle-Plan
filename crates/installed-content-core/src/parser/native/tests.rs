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
                    kind: crate::DiagnosticKind::Misread,
                    report: "synthetic reader diagnostic".into(),
                    read: Box::new(Shape::One),
                }
            } else {
                Shape::Reported {
                    kind: crate::DiagnosticKind::Unsupported,
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
            crate::DiagnosticKind::Misread,
            crate::DiagnosticKind::Unsupported
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
                    kind: crate::DiagnosticKind::Unsupported,
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
    assert!(read.item_misreads.iter().all(Vec::is_empty));
    assert_eq!(read.unattributed_misreads.len(), 2);
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

/// Comments start with `!`, `#` or `;`. Operators and brackets are single characters,
/// except `?=`. Quoted escapes decode, and 0xFF reads as the end.
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
            // The 0xFF ends the token read.
            (Kind::End, "".into()),
        ]
    );
    assert_eq!(tokens[3].line, 2);
}

/// The byte 0xFF reads as an end marker and advances, so reading can continue after it.
/// At a token start or after `?`, it returns an end token; for a bare token it keeps
/// the text already read. It ends a comment or quoted string, returning the latter. A
/// block loop ends there and its enclosing reader continues; a file loop stops.
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
    // The `?` remains the lexer's token after that read, so a value read takes it.
    let read = test_read(b"a = { n = ?\xff n = 2 }");
    assert_eq!(
        block(&read.items, "a")[0].value,
        Value::Scalar(b"?".to_vec())
    );
}

/// Retrying after a lone `?` or comment skips white space and returns an end at 0xFF or
/// EOF, keeping `?` or the marker as its token. A token found by the retry replaces it.
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
    // A value read at such a read takes the kept text from the peeked token.
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
    // `a`'s loop ends at the 0xFF; the file loop reads on: `n = 2` is a
    // top-level field (an object given a scalar, ended by `}`), then `b`.
    let read = test_read(b"a = { n = 1 \xff n = 2 } b = { n = 3 }");
    assert_eq!(keys(&read.items), ["a", "n", "b"]);
    assert_eq!(keys(block(&read.items, "a")), ["n"]);
    assert_eq!(keys(block(&read.items, "b")), ["n"]);
    // At the top level the file loop stops at it.
    let read = test_read(b"a = { n = 1 } \xff b = { n = 3 }");
    assert_eq!(keys(&read.items), ["a"]);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Test {
    Top,
    Body,
}

/// A test reader: `obj` reads an object, `list` a list, `n` one value,
/// `trigger` by key; anything else in a body is skipped, at the top an
/// object.
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

/// Each raw `{` is copied into the value slot before its element is read as an object.
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

/// Neither the list nor its element object read guards on `{`: scalar values pull
/// subsequent fields into the object.
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

/// Both closing reads leave the value slot intact; only nonclosing raw entries and
/// fields inside their objects replace it.
#[test]
fn object_lists_keep_the_last_value_slot_through_closing_reads() {
    for (text, expected_kind, expected_text) in [
        (
            b"list = { { n = kept } } ".as_slice(),
            Kind::Bare,
            b"kept".as_slice(),
        ),
        (
            b"list = { { n = kept } { } } ".as_slice(),
            Kind::Open,
            b"{".as_slice(),
        ),
        (b"list = { } ".as_slice(), Kind::Open, b"{".as_slice()),
    ] {
        let mut reader = Reader::new(lex(text, Mode::KeepMark));
        assert_eq!(reader.key_operator_value(), Slots::Fresh);
        reader.object_list((), &mut |(), _| Shape::One);
        assert_eq!(reader.current.kind, Kind::Close);
        assert_eq!(reader.value.kind, expected_kind);
        assert_eq!(reader.value.text, expected_text);
    }
}

/// A block given to a one-value reader: `n` reads `{`, the block's
/// fields are the body's, its `}` ends the body, and the body's own
/// remaining fields are the file's.
#[test]
fn a_block_where_one_value_is_read_shifts_the_rest_up() {
    let read = test_read(b"a = { n = { n = 2 x = 3 } n = 4 } b = { n = 5 }");
    assert_eq!(keys(&read.items), ["a", "n", "b"]);
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "n", "x"]);
    assert_eq!(a[0].value, Value::Scalar(b"{".to_vec()));
    assert_eq!(a[1].value, Value::Scalar(b"2".to_vec()));
    // `n = 4` is read at the top level, whose reader takes it as an
    // object that a's own `}` then ends at once.
    assert!(block(&read.items, "n").is_empty());
    assert_eq!(keys(block(&read.items, "b")), ["n"]);
    assert_eq!(read.misreads.len(), 2, "{:?}", read.misreads);
    assert!(read.misreads[0].what.starts_with("field `n` is a block"));
}

/// A bare token reaching EOF retains its text but fails the read. A peek turns it into
/// an end token, so lists and block loops stop before it, while a field-value read
/// takes its text. A quoted string at EOF succeeds.
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
    // A last key is never read: the loop's peek sees the end.
    let read = test_read(b"a = { n = 1 } stray");
    assert_eq!(keys(&read.items), ["a"]);
    assert!(read.misreads.is_empty(), "{:?}", read.misreads);
}

/// A pair reads one entry, then skips to the block's `}`. A second entry is reported
/// and passed over without defining anything; a value other than `{` is reported.
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
    // Raw skipping defines nothing: `@y` keeps its text.
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

/// A reader that requires `{` reports any other value and skips the block at the
/// current token.
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
    // `obj` consumed `{` as its operator and retained value `1`; reporting skips the
    // block at the current token.
    assert_eq!(keys(a), ["n", "obj", "list", "n"]);
    assert_eq!(a[3].value, Value::Scalar(b"2".to_vec()));
    assert!(
        read.misreads[1].what.contains("it is reported"),
        "{:?}",
        read.misreads
    );
}

/// Lexer mode 1 skips a leading UTF-8 byte-order mark. Mode 0 keeps its three bytes at
/// the start of the first token, preventing a keyword match.
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

/// A failed read keeps the token and quoted flag. A failed peek changes its kind to End
/// but keeps that flag, so a quoted `@x` before 0xFF or EOF remains literal when the
/// block's closing read resolves it.
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

/// Variable-resolving skips write each resolved token back before checking for `{`. A
/// reference that retained only a block brace opens a nested skip, which consumes the
/// next `}`. The outer skip ends at its enclosing block's `}`, and reading continues
/// after it.
#[test]
fn a_skip_resolves_each_token_it_reads() {
    // `@o` retains only `{`; `u` is skipped by the default dispatch. Inside it, `@o`
    // opens a nested skip that also takes `a`'s `}`.
    let read = test_read(b"@o = { a = { u = { @o } } n = 1 } b = { n = 2 }");
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(keys(block(&read.items, "a")), ["u", "n"]);
    assert_eq!(
        block(&read.items, "a")[1].value,
        Value::Scalar(b"1".to_vec())
    );
    // An unknown name stays its own text and opens nothing.
    let read = test_read(b"a = { u = { @none } n = 1 } b = { n = 2 }");
    assert_eq!(keys(&read.items), ["a", "b"]);
    assert_eq!(keys(block(&read.items, "a")), ["u", "n"]);
}

/// A definition at EOF reads no key, so the loop dispatches the key slot as the last
/// read left it, or its initial empty key if none was read.
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

/// A value without a key consumes the next token as its operator and dispatches the key
/// with the previous value. A following `{` is skipped whole for an unknown key.
#[test]
fn a_value_without_a_key_consumes_the_next_token() {
    let read = test_read(b"a = { n = 7 stray n = 1 unknown { n = 9 } n = 2 }");
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "stray", "=", "unknown", "n"]);
    // `stray` takes the stale 7; `=` then consumes `1`.
    assert_eq!(a[1].value, Value::Scalar(b"7".to_vec()));
    assert_eq!(a[4].value, Value::Scalar(b"2".to_vec()));
    assert_eq!(read.misreads.len(), 3, "{:?}", read.misreads);
    assert!(
        read.misreads[0]
            .what
            .starts_with("key `stray` has no operator")
    );
}

/// Definitions hold from where they are read on, at any depth read by
/// key; a taken name leaves its `=` to the next key read; a quoted
/// reference stays text; a definition ending its block takes `}` as the
/// next key.
#[test]
fn definitions_follow_the_readers_consumption() {
    let read = test_read(
        b"@x = 1 a = { n = @x @x = 2 n = \"@x\" trigger = { @y = 3 z = 1 } n = @y } b = { n = @y @z = 4 } c = { n = 5 }",
    );
    let a = block(&read.items, "a");
    assert_eq!(keys(a), ["n", "=", "n", "trigger", "n"]);
    assert_eq!(a[0].value, Value::Scalar(b"1".to_vec()));
    assert_eq!(a[2].value, Value::Scalar(b"@x".to_vec()));
    assert!(a[2].quoted());
    assert_eq!(a[4].value, Value::Scalar(b"3".to_vec()));
    // `@z = 4` ends b's block, so `}` is read as a key consuming `c`,
    // and `=` as a key consuming `{`, whose block is skipped.
    let b = block(&read.items, "b");
    assert_eq!(keys(b), ["n", "}", "="]);
    assert!(
        read.misreads[0]
            .what
            .contains("variable `@x` is already taken")
    );
}

/// An object reader given a scalar reads the following fields up to
/// the enclosing `}`; a list given a scalar the same; a skipped key's
/// block is skipped whole.
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

/// A trigger block holding values without keys is read as a list; one
/// that also holds a definition cannot be settled.
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

/// A known trigger body reads by key. A token without a key consumes the next token as
/// its missing operator, and dispatch sees the resulting slots. A definition is
/// consumed as a definition. Scalars are reported without further reading. Lists with
/// unknown dispatch retain each operand's kind.
#[test]
fn a_known_trigger_body_is_read_by_key_never_as_a_list() {
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
    // `=` is a key too, whose missing operator consumes `1`.
    assert_eq!(inside[2].kind, Kind::Quoted);
    assert!(inside[2].quoted());
    // `@v` was consumed as `x`'s operator, never defined.
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
    // The same text under a reader the port does not know is a list.
    let read = body(b"a = { trigger = { x y = \"5\" } }");
    let list = block(block(&read.items, "a"), "trigger");
    assert_eq!(list[1].kind, Kind::Quoted);
    assert_eq!(list[1].value, Value::Scalar(b"5".to_vec()));
}

/// Two tokens of any kind become pair strings. A third is reported, its block skipped
/// if present, then the pair is skipped to `}`. Fewer than two are reported; nothing
/// defines variables.
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

    // `{` is a string; the `}` after two strings ends the pair.
    let read = pair(b"a = { pair = { { x } n = 2 }");
    assert_eq!(strings(&read), ["{", "x"]);
    assert_eq!(keys(block(&read.items, "a")), ["pair", "n"]);

    // `k = v`: `k` and `=` are the strings; `v` is the third, and a
    // definition's name is only resolved.
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

/// Trigger dispatch receives the previous value when a key has no operator. A `{`
/// consumed as operator beneath a stale `{` is read by key as its block. A stale
/// nonopening value uses the reporting skip; a stale `{` without a consumed `{` depends
/// on dispatch and is unsettled.
#[test]
fn an_untraced_key_without_an_operator_follows_the_stale_value() {
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

    // A one-value trigger reads nothing more of the stale `{`.
    let read = trigger(b"a = { loose = { always yes } n = 3 }");
    assert!(read.unsettled.is_none(), "{:?}", read.unsettled);
    assert_eq!(keys(block(&read.items, "a")), ["loose", "n"]);
    // A key only content could name, read without names.
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

/// Items retain the source token's kind. End tokens keep text but cannot be keywords;
/// readers of the current lexer token keep that token's kind.
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

/// The operator slot retains `<`, `>` or `=`. A key without an operator keeps the
/// previous operator.
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
    // `>=` is `>` and then the value `=`.
    let b = block(&read.items, "b");
    assert_eq!(b[0].operator, Operator::Greater);
    assert_eq!(b[0].value, Value::Scalar(b"=".to_vec()));
}

/// Each field writes its single-character operator into one shared slot. A block item
/// keeps the `=` read with its key rather than the last comparison within it, for both
/// object and [`Shape::ByKey`] reads.
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

/// A reference copies its whole defining token, including its kind. Quoted definitions
/// stay nonkeywords. Chains take the named value, whose token becomes the next key when
/// its name is taken. Names compare exactly; quoted or unknown references retain their
/// own text.
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

/// The GUI readers' lexer (mode 0): comments end a word as well, `?=` is one token, a `?`
/// opening a token is dropped, and `[`, `]`, `.`, `%` and `@` are word bytes.
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

/// `\"` and `\\` keep their second byte, any other backslash stays, a line feed is kept, and
/// the end of the file ends an open string, which is returned.
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

/// The lexer classifies a word by its first byte before any lookup, and lookup reads the
/// text up to its NUL.
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

/// A 0xFF is one end of the stream and reading goes on after it: at a token's start, ending
/// a word (which the lexer keeps), after `?`; it only ends a comment, and ends a string that
/// is still returned. In mode 0 a leading byte-order mark is word bytes; bytes from 0x80 are
/// word bytes, never space.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    // Railway-construction context uses the same mean-time-to-happen reader.
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
