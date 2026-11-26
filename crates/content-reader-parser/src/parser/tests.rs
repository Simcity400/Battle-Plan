use super::*;

#[test]
fn boolean_tokens_compare_exact_c_strings_whatever_the_kind() {
    for (text, expected) in [
        (b"yes".as_slice(), Some(true)),
        (b"no".as_slice(), Some(false)),
        (b"yes\0suffix".as_slice(), Some(true)),
        (b"no\0suffix".as_slice(), Some(false)),
        (b"YES".as_slice(), None),
        (b"No".as_slice(), None),
        (b"yes!".as_slice(), None),
        (b"".as_slice(), None),
    ] {
        assert_eq!(read_bool(text), expected);
    }
    let items = parse(b"a = yes b = \"yes\" c = no d = \"no\"")
        .unwrap()
        .items;
    assert_eq!(
        items
            .iter()
            .map(|item| read_bool(item.value.scalar().unwrap()))
            .collect::<Vec<_>>(),
        [Some(true), Some(true), Some(false), Some(false)]
    );
}

#[test]
fn scalars_follow_the_lexer_past_errors() {
    let words = |text: &[u8]| {
        scalars(text)
            .into_iter()
            .map(|word| String::from_utf8(word).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        words(b"# note\rentity = { name = \"a\\\"b\\\\c\" } } = x\0y # z\n\"open"),
        ["entity", "name", "a\"b\\c", "x", "y", "open"]
    );
    assert!(parse(b"a = \"open").is_err());
    assert!(parse(b"a = x\0y").is_err());
}

#[test]
fn block_lines_follow_the_braces_in_preorder() {
    let (document, lines, repaired) = parse_unclosed_lines(
        b"a = {\n  b = { c = 1 }\n  color = rgb { 1 2 3 }\n}\r\nd = {\n\n  e = {\n}",
    )
    .unwrap();
    assert!(repaired);
    assert_eq!(document.items.len(), 2);
    let spans: Vec<_> = lines
        .iter()
        .map(|lines| (lines.open, lines.first_token, lines.close))
        .collect();
    assert_eq!(
        spans,
        [(1, 2, 4), (2, 2, 2), (3, 3, 3), (5, 7, 8), (7, 8, 8)]
    );
    assert_eq!(
        parse(b"x = { }").unwrap(),
        parse_unclosed_lines(b"x = { }").unwrap().0
    );
}

#[test]
fn block_first_token_lines_skip_whitespace_and_comments() {
    for (input, first_token) in [
        (b"x = {\n value = 1\n}".as_slice(), 2),
        (b"x = {\n\n # comment\n value = 1\n}", 4),
        (b"x = { value = 1 }", 1),
        (b"x = {}", 1),
        (b"x = {\n\n}", 3),
        (b"x = {\r# comment\rvalue = 1\r}", 1),
        (b"x = {\r\n\r\n# comment\r\nvalue = 1\r\n}", 4),
        (b"x = {\n\n", 3),
    ] {
        let (_, lines, _) = parse_unclosed_lines(input).unwrap();
        assert_eq!(lines[0].first_token, first_token, "{input:?}");
    }
}

#[test]
fn document_keeps_save_header_duplicate_keys_and_mixed_lists() {
    let document = parse(
        b"\xef\xbb\xbf# save\r\nHOI4txt\nreplace_path=map\nreplace_path=\"history/states\"\n\
          data={ 42 tag=GER { 7 8 } empty={} # a comment\n 9 }",
    )
    .unwrap();
    assert_eq!(document.items[0].key, None);
    assert_eq!(
        document.items[0].value.scalar(),
        Some(b"HOI4txt".as_slice())
    );
    assert_eq!(
        document.get("replace_path").unwrap().scalar_str(),
        Some("map")
    );
    assert_eq!(
        document
            .get_all(b"replace_path")
            .map(|value| value.scalar().unwrap())
            .collect::<Vec<_>>(),
        vec![b"map".as_slice(), b"history/states".as_slice()]
    );
    let items = document.get("data").unwrap().items().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[0].value.scalar_str(), Some("42"));
    assert_eq!(items[1].key.as_deref(), Some(b"tag".as_slice()));
    assert_eq!(items[1].value.scalar_str(), Some("GER"));
    assert_eq!(items[2].value.items().unwrap().len(), 2);
    assert_eq!(items[3].value.items(), Some([].as_slice()));
    assert_eq!(items[4].value.scalar_str(), Some("9"));
    assert!(document.get("missing").is_none());
}

#[test]
fn color_tags_preserve_case_and_do_not_consume_ordinary_list_elements() {
    let document =
        parse(b"a=HSV { 0.1 0.15 0.4 } b=rgb{1 2 3} c={ rgb plain { 4 } \"rgb\" { 5 } }").unwrap();
    assert!(
        matches!(document.get("a"), Some(Value::Tagged { tag, items }) if tag == b"HSV" && items.len() == 3)
    );
    assert!(
        matches!(document.get("b"), Some(Value::Tagged { tag, items }) if tag == b"rgb" && items.len() == 3)
    );
    let items = document.get("c").unwrap().items().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[0].value.scalar_str(), Some("rgb"));
    assert_eq!(items[1].value.scalar_str(), Some("plain"));
    assert_eq!(items[3].value.scalar_str(), Some("rgb"));
}

#[test]
fn quoted_strings_preserve_non_utf8_formatting_and_literal_unknown_escapes() {
    let document = parse(
        b"\"quoted key\"=\"a\\\"b\\\\c\\n\\q # { = }\" name=\"\x11HSpy\xff\x11!\" empty=\"\"",
    )
    .unwrap();
    assert_eq!(
        document.get("quoted key").unwrap().scalar(),
        Some(b"a\"b\\c\\n\\q # { = }".as_slice())
    );
    assert_eq!(
        document.get("name").unwrap().scalar(),
        Some(b"\x11HSpy\xff\x11!".as_slice())
    );
    assert_eq!(document.get("name").unwrap().scalar_str(), None);
    assert_eq!(document.get("empty").unwrap().scalar_str(), Some(""));
    assert!(Value::Scalar(Vec::new()).items().is_none());
    assert!(Value::Block(Vec::new()).scalar().is_none());
}

#[test]
fn items_record_whether_their_value_is_quoted() {
    let document = parse(b"a = \"@x\" b = @x \"@y\" @y c = { } d = rgb { 1 }").unwrap();
    let quoted: Vec<_> = document.items.iter().map(Item::quoted).collect();
    assert_eq!(quoted, [true, false, true, false, false, false]);
}

#[test]
fn keys_keep_their_token_kind_and_only_bare_keys_are_keywords() {
    let document =
        parse(b"Fallback_NAME = a \"fallback_name\" = b 1x = c -x = d { e } \"@q\" = 1").unwrap();
    let items = &document.items;
    let quoted: Vec<_> = items.iter().map(|item| item.key_quoted).collect();
    assert_eq!(quoted, [false, true, false, false, false, true]);
    assert!(items[0].keyword_is("fallback_name"));
    assert_eq!(items[0].keyword().unwrap().folded(), b"fallback_name");
    assert_eq!(items[0].keyword().unwrap().text(), b"Fallback_NAME");
    for item in &items[1..5] {
        assert_eq!(item.keyword(), None, "{item:?}");
    }
}

#[test]
fn keyword_identity_preserves_non_ascii_and_quoted_text() {
    let document = parse(b"Name=one \"name\"=two N\xc4ME=three n\xe4me=four").unwrap();
    assert!(document.items[0].keyword_is("name"));
    assert!(!document.items[1].keyword_is("name"));
    assert_eq!(document.items[2].keyword().unwrap().folded(), b"n\xc4me");
    assert_eq!(document.items[3].keyword().unwrap().folded(), b"n\xe4me");
    assert_ne!(document.items[2].keyword(), document.items[3].keyword());
}

#[test]
fn comparisons_are_keys_with_their_operator_and_value_token() {
    let document = parse(b"t = { skill>@x phase > 99 a<\"010\" b >= 5 c = d } < 3").unwrap();
    let items = document.get("t").unwrap().items().unwrap();
    let shown: Vec<_> = items
        .iter()
        .map(|item| {
            (
                item.key
                    .as_deref()
                    .map(|key| String::from_utf8_lossy(key).into_owned()),
                item.operator,
                String::from_utf8_lossy(item.value.scalar().unwrap()).into_owned(),
                item.quoted(),
            )
        })
        .collect();
    let keyed = |key: &str, operator, value: &str, quoted| {
        (Some(key.to_owned()), operator, value.to_owned(), quoted)
    };
    assert_eq!(
        shown,
        [
            keyed("skill", Operator::Greater, "@x", false),
            keyed("phase", Operator::Greater, "99", false),
            keyed("a", Operator::Less, "010", true),
            (
                Some("b".to_owned()),
                Operator::Greater,
                "=".to_owned(),
                false
            ),
            (None, Operator::Equals, "5".to_owned(), false),
            keyed("c", Operator::Equals, "d", false),
        ]
    );
    let doubled = parse(b"a == b").unwrap();
    assert_eq!(doubled.items[0].value.scalar(), Some(b"=".as_slice()));
    assert_eq!(doubled.items[1].value.scalar(), Some(b"b".as_slice()));
    assert_eq!(document.items[1].key, None);
    assert_eq!(document.items[1].value.scalar(), Some(b"<".as_slice()));
}

#[test]
fn number_tokens_follow_the_lexer() {
    for (text, quoted, number) in [
        (b"010".as_slice(), false, true),
        (b"1tail", false, true),
        (b"-x", false, true),
        (b".5", false, true),
        (b"1.5.2", false, true),
        (b"010", true, false),
        (b"x1", false, false),
        (b"yes", false, false),
        (b"", false, false),
        (b"\0x", false, true),
        (b".5\0x", false, true),
        (b"x\0", false, false),
    ] {
        assert_eq!(
            number_token(text, quoted),
            number,
            "{}",
            String::from_utf8_lossy(text)
        );
    }
}

#[test]
fn the_end_of_a_script_file_closes_what_it_leaves_open() {
    for source in [
        b"G = { ordered = { 1 = { \"First\" } # tail".as_slice(),
        b"G = { ordered = { 1 = { \"First\" }\n# tail\n",
        b"G = { ordered = { 1 = { \"First",
    ] {
        assert!(parse(source).is_err());
        let (document, closed) = parse_unclosed(source).unwrap();
        assert!(closed, "{}", String::from_utf8_lossy(source));
        let ordered = document.get("G").unwrap().items().unwrap()[0]
            .value
            .items()
            .unwrap();
        assert_eq!(
            ordered[0].value.items().unwrap()[0].value.scalar(),
            Some(b"First".as_slice())
        );
    }
    let (_, closed) = parse_unclosed(b"G = { } # done").unwrap();
    assert!(!closed);
    assert!(parse_unclosed(b"G = { } }").is_err());
    assert!(parse_unclosed(b"G = { a = ").is_err());
}

#[test]
fn numeric_scalars_retain_every_lexical_byte() {
    let scalars = [
        "-9223372036854775808",
        "18446744073709551615",
        "99999999999999999999999999999999999",
        "0.0000100",
        "+00042",
        "1.234567890123456789e-20",
        "1936.1.2.17",
    ];
    let document = parse(scalars.join(" ").as_bytes()).unwrap();
    let values: Vec<_> = document
        .items
        .iter()
        .map(|item| item.value.scalar_str().unwrap())
        .collect();
    assert_eq!(values, scalars);
}

#[test]
fn malformed_documents_report_original_byte_offsets() {
    for (input, offset) in [
        (b"a =".as_slice(), 3),
        (b"a = {".as_slice(), 5),
        (b"a = }".as_slice(), 4),
        (b"}".as_slice(), 0),
        (b"\"unfinished".as_slice(), 0),
        (b"\"unfinished\\".as_slice(), 0),
        (b"a={x=y}}".as_slice(), 7),
        (b"\xef\xbb\xbf}".as_slice(), 3),
        (b"a=\"x\0y\"".as_slice(), 4),
        (b"a=x\0y".as_slice(), 3),
        (b"#x\0".as_slice(), 2),
    ] {
        let error = parse(input).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .starts_with(&format!("Clausewitz text at byte {offset}:")),
            "input={input:?}: {error}"
        );
    }
}

#[test]
fn depth_limit_applies_to_anonymous_and_tagged_blocks() {
    let accepted = format!("{}{}", "{".repeat(MAX_DEPTH), "}".repeat(MAX_DEPTH));
    assert!(parse(accepted.as_bytes()).is_ok());
    let rejected = format!("{}{}", "{".repeat(MAX_DEPTH + 1), "}".repeat(MAX_DEPTH + 1));
    assert!(
        parse(rejected.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("depth limit")
    );
    let tagged = format!(
        "{}{}",
        "HSV {".repeat(MAX_DEPTH + 1),
        "}".repeat(MAX_DEPTH + 1)
    );
    assert!(
        parse(tagged.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("depth limit")
    );
    assert_eq!(
        parse(b"\xef\xbb\xbf # comment only").unwrap(),
        Document::default()
    );
}

#[test]
fn the_save_grammar_reads_an_empty_key_and_an_empty_value_as_written() {
    let text = b"HOI4txt\nassets={\n\t={ alpha beta }\n\tGER={ gamma }\n}\nconveyor={\n\trole=\n\tclosed=yes\n\tlast=\n}\ntail=yes\n";
    let items = parse_save(text).unwrap().items;
    let key = |item: &Item| item.key.clone();
    assert_eq!(items.iter().map(key).collect::<Vec<_>>(), [None, Some(b"assets".to_vec()), Some(b"conveyor".to_vec()), Some(b"tail".to_vec())]);
    let assets = items[1].value.items().unwrap();
    assert_eq!(assets.iter().map(key).collect::<Vec<_>>(), [Some(Vec::new()), Some(b"GER".to_vec())]);
    let teams = assets[0].value.items().unwrap();
    assert_eq!(teams.iter().map(|item| item.value.scalar().unwrap().to_vec()).collect::<Vec<_>>(), [b"alpha".to_vec(), b"beta".to_vec()]);
    let conveyor = items[2].value.items().unwrap();
    assert_eq!(
        conveyor.iter().map(|item| (key(item), item.value.scalar().map(<[u8]>::to_vec))).collect::<Vec<_>>(),
        [(Some(b"role".to_vec()), Some(Vec::new())), (Some(b"closed".to_vec()), Some(b"yes".to_vec())), (Some(b"last".to_vec()), Some(Vec::new()))]
    );
    assert_eq!(items[3].value.scalar(), Some(b"yes".as_slice()));
    let inline = parse_save(b"a=x =0").unwrap().items;
    assert_eq!(
        inline.iter().map(|item| (key(item), item.value.scalar().map(<[u8]>::to_vec))).collect::<Vec<_>>(),
        [(Some(b"a".to_vec()), Some(b"x".to_vec())), (Some(Vec::new()), Some(b"0".to_vec()))]
    );
    let variables = parse_save(b"variables={named=1 =2}").unwrap().items;
    let entries = variables[0].value.items().unwrap();
    assert_eq!(
        entries.iter().map(|item| (key(item), item.value.scalar().map(<[u8]>::to_vec))).collect::<Vec<_>>(),
        [(Some(b"named".to_vec()), Some(b"1".to_vec())), (Some(Vec::new()), Some(b"2".to_vec()))]
    );
    assert!(parse(text).is_err());
    assert!(parse(b"a={ ={ b } }").is_err());
    assert!(parse(b"a={ b= c=d }").is_err());
}
