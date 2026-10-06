use super::*;

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
