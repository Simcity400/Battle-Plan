use super::*;

#[test]
fn wrapped_definitions_narrow_clamp_and_keep_first_normal_token() {
    let definitions = parse(
        br#"ignored={@hidden=4} opinion_modifiers={
        SYN={value=65546 min_trust=-50 max_trust=50 trade=yes days=1}
        syn={value=-200 trade=no}
        SYN={target=yes value=99}
        toggled={target=yes target=no value=0}
        decimal={value=010}
        invalid={value=bad}}
        opinion_modifiers={last={value=200}}"#,
        [-100, 100],
    )
    .unwrap();
    assert_eq!(definitions.len(), 6);
    let database = Database {
        definitions,
        ..Database::default()
    };
    let first = database.find("sYn").unwrap().unwrap();
    assert_eq!(
        (first.opinion, first.trade, first.name.as_str()),
        (10, true, "SYN")
    );
    assert_eq!(database.find("last").unwrap().unwrap().opinion, 100);
    assert_eq!(database.find("decimal").unwrap().unwrap().opinion, 10);
    assert_eq!(database.find("invalid").unwrap().unwrap().opinion, 0);
    assert_eq!(
        database.find("toggled").unwrap().unwrap().name,
        "toggled_target"
    );
    assert!(database.find("missing").unwrap().is_none());
}

#[test]
fn target_only_definitions_do_not_replace_normal_lookup() {
    let definitions = parse(b"opinion_modifiers={only_target={target=yes value=30} ordinary={value=25 max_trust=10 min_trust=20}}", [-100, 100]).unwrap();
    let database = Database {
        definitions,
        ..Database::default()
    };
    assert!(database.find("only_target").unwrap().is_none());
    assert_eq!(database.find("ordinary").unwrap().unwrap().opinion, 10);
    let error = parse(b"opinion_modifiers={broken={value=}}", [-100, 100])
        .unwrap_err()
        .to_string();
    assert!(error.contains("broken") && error.contains("empty short operand has no stored value"));
}
