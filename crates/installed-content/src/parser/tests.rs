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

/// The word scan reads what the parser's lexer reads, past any error.
#[test]
fn scalars_follow_the_lexer_past_errors() {
    let words = |text: &[u8]| {
        scalars(text)
            .into_iter()
            .map(|word| String::from_utf8(word).unwrap())
            .collect::<Vec<_>>()
    };
    // A bare CR ends a comment; escapes are decoded; a structural error
    // and a NUL byte do not stop the scan; an unterminated quote runs
    // to the end.
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
        .map(|lines| (lines.open, lines.close))
        .collect();
    // a, b, the tagged colour, d, then e; the supplied brace closes d.
    assert_eq!(spans, [(1, 4), (2, 2), (3, 3), (5, 8), (7, 8)]);
    assert_eq!(
        parse(b"x = { }").unwrap(),
        parse_unclosed_lines(b"x = { }").unwrap().0
    );
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

/// 1424A7A90 gives a quoted key token 0xF without the keyword lookup and
/// folds a bare one; a key starting with a digit or `-` is never looked up.
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

/// The content readers ask [`Item::keyword`] whether a key is a keyword
/// (1424A7A90: a quoted key is token 0xF, a bare one folds). A raw key
/// comparison in their code outside the tests fails here unless it is
/// allowed below with the address that makes it a text compare.
#[test]
fn content_readers_ask_the_keyword_helper() {
    const READERS: [(&str, &str); 10] = [
        ("division_names.rs", include_str!("../division_names.rs")),
        ("ideas.rs", include_str!("../ideas.rs")),
        ("leader_traits.rs", include_str!("../leader_traits.rs")),
        ("modifiers.rs", include_str!("../modifiers.rs")),
        ("terrain.rs", include_str!("../terrain.rs")),
        ("script_files.rs", include_str!("../script_files.rs")),
        ("characters.rs", include_str!("../characters.rs")),
        ("ideologies.rs", include_str!("../ideologies.rs")),
        (
            "modifier_definitions.rs",
            include_str!("../modifier_definitions.rs"),
        ),
        (
            "scripted_localisation.rs",
            include_str!("../scripted_localisation.rs"),
        ),
    ];
    const RAW: [&str; 6] = [
        "key.as_deref() ==",
        "key.as_deref() !=",
        "eq_ignore_ascii_case(",
        "to_ascii_lowercase(",
        ".get(b\"",
        "get_all(",
    ];
    // (file, line text) with why it compares text, not a keyword.
    const ALLOWED: &[(&str, &str)] = &[
        // A group name: 1409B4370 finds it by the folded hash 140BBA1B0
        // and `_stricmp` (14253D9F8).
        ("division_names.rs", "key.to_ascii_lowercase()"),
        // A trait name: 140710200 compares its hash and `_stricmp`
        // (14253D9F8), selecting the first match in load order.
        ("leader_traits.rs", "let name = token.to_ascii_lowercase();"),
        (
            "leader_traits.rs",
            "let name = found.token.to_ascii_lowercase();",
        ),
        // An idea group's name as the token registry keys it
        // (add_dynamic_token 1424A65A0 -> 1424A5EE0 folds A-Z).
        (
            "ideas.rs",
            "(!matches!(name.first(), Some(b'0'..=b'9' | b'-'))).then(|| name.to_ascii_lowercase())",
        ),
        // An idea a repeated group names again: 140FC0E90's `_stricmp`.
        (
            "ideas.rs",
            "self.ideas[*index].token.eq_ignore_ascii_case(&idea.token)",
        ),
        // A modifier definition's name in the token registry (1405FF750
        // -> 1424A65A0, folded, StringView_CompareNoCase 1424B6750).
        (
            "modifier_definitions.rs",
            "self.by_name.get(&key.as_bytes().to_ascii_lowercase())",
        ),
        (
            "modifier_definitions.rs",
            "let name = definition.token.as_bytes().to_ascii_lowercase();",
        ),
        // A category name: FindTerrainByHashedName (1403BCA00).
        (
            "terrain.rs",
            ".position(|category| category.name.eq_ignore_ascii_case(name))",
        ),
        // A file path's extension, not a key.
        ("script_files.rs", "let lower = path.to_ascii_lowercase();"),
        ("characters.rs", "let lower = path.to_ascii_lowercase();"),
        // A character token CCharacterTemplateDatabase interns
        // (add_dynamic_token 1424A6500, folded).
        (
            "characters.rs",
            "parser::c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase()",
        ),
        // A recorded template's tag, a dynamic token (1424A65A0 ->
        // 1424A5EE0 folds A-Z).
        (
            "characters.rs",
            ".filter(|(tag, _)| tag.eq_ignore_ascii_case(&built.tag))",
        ),
        // The modifier a key names (1406042A0): the lexer folds a bare
        // key (1424A7A90), FindTokenNoCase a quoted one (1424A7060).
        ("modifiers.rs", "name: key.to_ascii_lowercase(),"),
        // Its registry lookup: FindTokenNoCase 14022EC40 folds ASCII
        // before comparing, for built-in and scripted names alike.
        (
            "modifiers.rs",
            ".any(|name| name.as_bytes().eq_ignore_ascii_case(key))",
        ),
        (
            "modifiers.rs",
            "if effects.modifiers.contains(&key.to_ascii_lowercase()) {",
        ),
        // Stored dynamic token (140A38190 -> 1424A6500).
        (
            "ideologies.rs",
            "let key = crate::parser::c_string(&field.key.text).to_ascii_lowercase();",
        ),
        // An ideology group's number by its token, a dynamic token
        // (140A38190 -> 1424A6500 -> 1424A65A0 folds A-Z).
        ("ideologies.rs", ".get(&token.to_ascii_lowercase())"),
    ];
    let mut raw = Vec::new();
    for (file, source) in READERS {
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        for line in code.lines().map(str::trim) {
            if !line.starts_with("//")
                && RAW.iter().any(|pattern| line.contains(pattern))
                && !ALLOWED.contains(&(file, line))
            {
                raw.push(format!("{file}: {line}"));
            }
        }
    }
    assert!(raw.is_empty(), "raw key comparisons: {raw:#?}");
}

/// 1424A7A90 makes `<`, `>` and `=` single-character tokens that end a
/// bare token, and 1424AC5E0 accepts each as the operator.
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
            // `>=` is `>` and then `=`: the game reads `=` as the value.
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
    // A stray operator stays a value without a key.
    assert_eq!(document.items[1].key, None);
    assert_eq!(document.items[1].value.scalar(), Some(b"<".as_slice()));
}

/// 1424A7A90 gives the number id 0xC to a bare token starting with a
/// digit or `-`, or made of digits, `-` and `.` up to its first NUL, so
/// one starting with a NUL; text with no byte is the id-0 slot.
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
        // The loop stops at the NUL: an empty C string, and `.5`.
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

/// The readers stop at the end-of-file token as at `}` (1424A9730,
/// 1409B9A20, 1401426E0), wherever the end falls.
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
