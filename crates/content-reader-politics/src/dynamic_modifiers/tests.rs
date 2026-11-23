use super::*;

#[test]
fn constant_and_scoped_values_keep_line_order() {
    content_reader_script::test_game::enter();
    let definitions = parse(
        br#"@base=0.2 SYN={icon=other enable={always=yes} remove_trigger={always=no}
            political_power_gain=@base stability_factor=var:power
            stability_factor=ROOT.power custom_modifier_tooltip=SYN_NOTE
            hidden_modifier={war_support_factor=0.5}
            added_modifier={political_power_gain=1}}
            SYN={political_power_gain=-0.12500} syn={political_power_gain=2}"#,
        &KeyNames::default(),
    )
    .unwrap();
    let first = &definitions[0];
    assert_eq!(first.name, "SYN");
    assert_eq!(first.icon, b"other");
    assert!(definitions[1].icon.is_empty());
    assert_eq!(first.modifiers.values[0].value, 20_000);
    assert_eq!(
        first.scoped_modifiers,
        [
            ("stability_factor".into(), b"var:power".to_vec()),
            ("stability_factor".into(), b"ROOT.power".to_vec()),
        ]
    );
    assert_eq!(first.modifiers.tooltips(), ["SYN_NOTE"]);
    assert!(first.modifiers.hidden.contains("war_support_factor"));
    assert_eq!(first.modifiers.children[0].values[0].value, 100_000);
    let mut database = Database::default();
    for definition in definitions {
        database
            .definitions
            .insert(definition.name.clone(), definition);
    }
    assert_eq!(
        database.find("SYN").unwrap().unwrap().modifiers.values[0].value,
        -12_500
    );
    assert_eq!(
        database.find("syn").unwrap().unwrap().modifiers.values[0].value,
        200_000
    );
    assert!(database.find("missing").unwrap().is_none());
}

#[test]
fn unknown_scoped_keys_skip_their_blocks_and_do_not_define_variables() {
    content_reader_script::test_game::enter();
    let definitions = parse(
        b"SYN={unregistered={@hidden=9} political_power_gain=@hidden}",
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(
        definitions[0].scoped_modifiers,
        [("political_power_gain".into(), b"@hidden".to_vec())]
    );
    let mut names = KeyNames::default();
    names.effects.modifiers_gap = Some("synthetic registry suffix".into());
    let error = parse(b"SYN={unknown=var:power}", &names)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown") && error.contains("registry"));
}
