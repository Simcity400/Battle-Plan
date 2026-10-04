use super::*;

#[test]
fn counted_reports_keep_their_source_kind() {
    let read = read(
        b"warning=yes unsupported=yes",
        Mode::KeepMark,
        Top::File(()),
        None,
        &mut |(), field| {
            if field.keyword_is("warning") {
                Shape::Reported {
                    kind: crate::DiagnosticKind::Misread,
                    report: "refused is only wording".into(),
                    read: Box::new(Shape::One),
                }
            } else {
                Shape::Reported {
                    kind: crate::DiagnosticKind::Unsupported,
                    report: "no severity word".into(),
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

/// 1424A7A90: comments on `!`, `#` and `;`, one-character operators and
/// brackets, `?=`, quoted escapes, and 0xFF read as the end.
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
            // The 0xFF: read_token returns 0.
            (Kind::End, "".into()),
        ]
    );
    assert_eq!(tokens[3].line, 2);
}

/// CMemoryFile::ReadNextCharacter (1424D92C0) reads a byte 0xFF as -1,
/// as it reads the end of the text, and always advances; read_token
/// (1424A7A90, CMP AL,0xFF at 1424A7B10) returns `c != -1`. So a 0xFF is
/// one end and reading goes on after it: read_token returns 0 where a
/// token starts, after `?`, and for a bare token it ends, whose text it
/// keeps; it ends a comment, and a quoted string, which is returned. A
/// block's loop (1424A9730) ends at it and the enclosing reader reads
/// on; a file loop (140AD6030) stops there.
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
    // The `?` is the lexer's token after that read: a value read there
    // takes it (1424AC5E0 copies the token it peeked).
    let read = test_read(b"a = { n = ?\xff n = 2 }");
    assert_eq!(
        block(&read.items, "a")[0].value,
        Value::Scalar(b"?".to_vec())
    );
}

/// The retry after a lone `?` or a comment skips white space and
/// returns 0 at a -1, a 0xFF or the end, keeping the `?` or the marker
/// as the token (1424A7CDE-1424A7D80, 1424A7C53-1424A7C86, 1424A7B32);
/// a token it finds replaces it.
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
    // A value read at such a read takes the kept text (1424AC5E0 copies
    // the token it peeked).
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

/// 14144E640 copies each raw `{` into the value slot, then
/// 1424ABB40 reads that element as a CPersistent object.
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

/// Neither 14144E640's list nor the element's CPersistent read guards
/// on `{`: scalar values pull the following fields into the object.
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

/// Both closing reads leave the reader's value slot: only nonclosing
/// raw entries and fields within their objects replace it (14144E640).
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

/// 1424A7A90 returns false for a bare token that runs into the end of
/// the text, its text kept. A peek turns it into the end (0x13): a list
/// (1401426E0) or a block loop stops before it, while 1424AC5E0 takes
/// it, text and all, as a value. A quoted string at the end succeeds.
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

/// 140C89020's pair: one entry, then SkipBlock (1424AD210) to the
/// block's `}`; a second entry is reported and passed over, defining
/// nothing; a value that is no `{` is reported.
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
    // SkipBlock defines nothing: `@y` keeps its text.
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

/// A reader that tests for `{` and otherwise reports (RecordError
/// 1424ACF90) skips a block the reporting stands on.
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
    // `obj` consumed `{` as its operator (1424AC5E0) and kept the stale
    // value `1`; the report skips the block it stands on.
    assert_eq!(keys(a), ["n", "obj", "list", "n"]);
    assert_eq!(a[3].value, Value::Scalar(b"2".to_vec()));
    assert!(
        read.misreads[1].what.contains("it is reported"),
        "{:?}",
        read.misreads
    );
}

/// lexer's mode 1 (characters, 1406B0120; names, 140A5C200); in mode 0
/// (1406B3690, 140A34320 and most loaders) its three bytes start the
/// first token, which is then no keyword.
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

/// A read that returns 0 keeps the lexer's token and its quoted flag
/// (+0x1C, 1424A7A90); a peek that returns 0 gives it the id 0x13 and
/// leaves the flag, and resolve_variable_token (1424AB540) leaves a
/// quoted token: a quoted `@x` before a 0xFF or the end is no reference
/// when CPersistent::Read's end (1424A9730) resolves the token it stops
/// at.
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

/// SkipBlockResolvingVariables (1424AD270) resolves each token it reads
/// through 1424AB540 and writes it back before testing it for `{`: a
/// reference to a definition that kept only a block's brace opens a
/// nested skip, which takes the `}` after it, so the skip ends at the
/// enclosing block's `}` and its reader reads on after that.
#[test]
fn a_skip_resolves_each_token_it_reads() {
    // `@o` keeps only `{` (1424AC830); `u` is skipped (test_read's
    // default); inside it `@o` is `{`, so the skip also takes `a`'s `}`.
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

/// A text ending in a definition: 1424AC5E0 reads no key, and the loop
/// dispatches the key slot as the last read left it (1424A9730), the
/// empty key 1424A9D30 sets when no key was read yet.
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

/// A value without a key: 1424AC5E0 consumes the next token as its
/// operator and dispatches the key with the previous value; a `{` after
/// it is skipped whole for a key the reader does not know.
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
    assert!(
        read.unsettled
            .unwrap()
            .what
            .starts_with("variable @y is in a block read by a trigger")
    );
    assert_eq!(keys(&read.items), ["a"]);
}

/// A known trigger body (CTrigger::LoadBody 1405431C0) is never read as
/// a list: a token without a key is a key whose missing operator
/// consumes the next token, LoadKey takes every key with the slots
/// 1424AC5E0 left, and a definition there is consumed, not a list's
/// unsettled one. A scalar is reported and nothing is read. The list a
/// reader the port does not know reads keeps each operand's kind.
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
            .contains("\"Non assign trigger is not enclosed in {}\" is reported"),
        "{:?}",
        read.misreads
    );
    // The same text under a reader the port does not know is a list.
    let read = body(b"a = { trigger = { x y = \"5\" } }");
    let list = block(block(&read.items, "a"), "trigger");
    assert_eq!(list[1].kind, Kind::Quoted);
    assert_eq!(list[1].value, Value::Scalar(b"5".to_vec()));
}

/// std_pair_parser (140C88A60): two tokens of any kind are the strings;
/// a third is reported, a block it opens skipped, and SkipBlock passes
/// to the pair's `}`; fewer than two are reported; nothing defines.
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

/// LoadBody (1405431C0) hands LoadKey a key read without an operator
/// with the value slot the last read left: a `{` consumed as the
/// operator under a stale `{` is read by key as its block; a stale
/// value that opens no block is RecordError's skip; a stale `{` with
/// no `{` consumed depends on the key's reader, so it is unsettled.
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
            .starts_with("field `maybe`: LoadKey (1405434F0) reads it"),
        "{unsettled}"
    );
}

/// The value slot's token kind reaches the item: the end of the text a
/// failed advance left (0x13) keeps its text but is no keyword, and a
/// reader of the current token (140606100) takes that token's kind.
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

/// 1424AC5E0 keeps the operator it read (+0x78), `<` 0x1D4 and `>`
/// 0x1D3 beside `=`; a key read without one keeps the slot's last.
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

/// 1424A7A90 makes `>` and `<` tokens of their own, and 1424AC5E0 writes
/// each field's operator into the one slot (+0x78). A block's item keeps
/// the `=` read with its key, not the last comparison inside it, whether
/// the block is read as an object or by key unread ([`Shape::ByKey`]).
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

/// 1424AC830 stores the whole defining token and 1424AB540 copies it
/// back, so a reference keeps its definition's kind (a quoted one is no
/// keyword); a chain takes the value it names, whose token is then read
/// as the next key, a taken name; names compare exactly (`@N` is not
/// `@n`); a quoted token or an unknown name stays its own text.
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

/// `read_token` numbers a word by its first byte before any lookup, and the lookup reads the
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

    // CRailwayConstructionContext::vf4 (140436D30) uses the same MTTH.
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
