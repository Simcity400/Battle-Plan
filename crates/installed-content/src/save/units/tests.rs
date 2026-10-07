use super::*;

#[test]
fn railway_gun_names_keep_presence_and_unescaped_bytes() {
    let parse = |suffix: &[u8]| {
        let mut text = b"id={id=7 type=74} logical_country=AAA strength=0 ".to_vec();
        text.extend_from_slice(suffix);
        let document = crate::parser::parse(&text)?;
        parse_railway_gun(b"AAA", &document.items)
    };
    assert_eq!(parse(b"").unwrap().name, None);
    assert_eq!(parse(b"name=\"\"").unwrap().name, Some(Vec::new()));
    assert_eq!(
        parse(b"name=\"\xff\\\\\\\"G\"").unwrap().name,
        Some(vec![0xff, b'\\', b'"', b'G'])
    );
    for text in [b"name=A name=B".as_slice(), b"name={bad=x}".as_slice()] {
        assert!(parse(text).is_err());
    }
}

#[test]
fn template_fallback_key_is_saved_independently_of_explicit_name() {
    let parse = |fields: &str| {
        let document = crate::parser::parse(
            format!("division_templates={{division_template={{id={{id=3 type=52}} country=AAA template_counter=0 ingame_set_template_counter=no {fields}}}}}").as_bytes(),
        )?;
        parse_division_templates(&document.items)
    };
    let id = (3_u64 << 32) | 52;
    let rows = parse("name=Explicit localization_key=FALLBACK").unwrap();
    assert_eq!(rows[&id].name, b"Explicit");
    assert_eq!(rows[&id].name_key, b"FALLBACK");
    assert!(parse("").unwrap()[&id].name_key.is_empty());
    for text in [
        "localization_key={bad=x}",
        "localization_key=A localization_key=B",
    ] {
        assert!(parse(text).is_err());
    }
}
