pub(super) use crate::test_game::read_effect_body as read;
use super::*;
use content_reader_parser::parser::{Item, Value};
use crate::native::Read;

#[test]
fn class_metadata_covers_the_reader_registry_and_aliases() {
    let (first, second) = (0x7_0000_0010, 0x7_0000_0020);
    crate::test_game::with(|game| {
        game.effect("synthetic_class_first", READ_VALUE, 0, first)
            .effect("synthetic_class_second", READ_VALUE, 0, second)
    });
    assert!(
        classes::CLASS_IDENTITIES
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0)
    );
    assert_eq!(classes::CLASS_IDENTITIES.len(), EFFECT_CLASSES.len());
    for ((token, _), (class_token, name, vtable)) in REGISTERED_EFFECTS
        .iter()
        .zip(classes::CLASS_IDENTITIES.iter().copied())
    {
        assert_eq!(*token, class_token);
        assert!(!name.is_empty());
        assert_ne!(vtable, 0);
    }
    assert_eq!(native_class(b"SYNTHETIC_CLASS_FIRST").unwrap().vtable, first);
    assert_eq!(native_class(b"synthetic_class_second").unwrap().vtable, second);
    assert_eq!(
        class(registered(b"synthetic_class_first").unwrap()),
        class(registered(b"synthetic_class_second").unwrap())
    );
    let conditional = native_class(b"if").unwrap();
    assert_eq!(native_class(b"else_if"), Some(conditional));
    assert_eq!(native_class(b"else"), Some(conditional));
    assert_eq!(native_class(b"synthetic_unknown"), None);
}


pub(super) fn block<'a>(items: &'a [Item], key: &str) -> &'a [Item] {
    items
        .iter()
        .find(|item| item.keyword_is(key))
        .unwrap()
        .value
        .items()
        .unwrap()
}

pub(super) fn scalar<'a>(items: &'a [Item], key: &str) -> &'a [u8] {
    items
        .iter()
        .find(|item| item.keyword_is(key))
        .unwrap_or_else(|| panic!("missing {key} in {items:?}"))
        .value
        .scalar()
        .unwrap()
}

#[test]
fn registry_is_sorted_complete_and_names_fold() {
    crate::test_game::enter();
    assert!(EFFECT_CLASSES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    let names: std::collections::BTreeSet<_> =
        REGISTERED_EFFECTS.iter().map(|(_, name)| name).collect();
    assert_eq!(names.len(), EFFECT_CLASSES.len());
    assert_eq!(
        class(registered(b"HIDDEN_EFFECT").unwrap()).unwrap().1,
        READ_EFFECT
    );
    assert_eq!(
        class(registered(b"add_stability").unwrap()).unwrap().0,
        READ_VALUE
    );
    assert!(
        REGISTERED_EFFECTS
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0)
    );
    for &(token, _) in REGISTERED_EFFECTS.iter() {
        assert!(class(token).is_some());
    }
    assert_eq!(class(u32::MAX), None);
}

#[test]
fn description_conditional_adjacency_uses_added_children() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names.countries.insert(b"SYN".to_vec());
    names.scripted_effects.insert(b"authored".to_vec());
    let parsed = read(
        b"body = { if = { limit = { always = yes } add_stability = 1 }
        invalid_effect = { @ignored = 2 } else = { add_political_power = 1 }
        SYN = { army_experience = 3 } authored = yes }",
        &names,
    );
    let fields = block(&parsed.items, "body");
    assert!(adds_child(&fields[0], &names).unwrap());
    assert!(!adds_child(&fields[1], &names).unwrap());
    assert!(!adds_child(&fields[2], &names).unwrap());
    assert!(adds_child(&fields[3], &names).unwrap());
    assert!(adds_child(&fields[4], &names).unwrap());
}
#[test]
fn invalid_effect_skips_without_defining_names() {
    crate::test_game::enter();
    let read = read(
        b"body = { invalid_effect = { @hidden = 7 } } final = @hidden",
        &KeyNames::default(),
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"@hidden");
    assert!(block(block(&read.items, "body"), "invalid_effect").is_empty());
}

#[test]
fn a_scope_body_uses_effect_dispatch() {
    crate::test_game::enter();
    let names = KeyNames {
        countries: [b"ABC".to_vec()].into(),
        ..Default::default()
    };
    let read = read(b"body = { ABC = { invalid_effect = { @hidden = 7 } @seen = 3 add_stability = 0 } } final = @seen hidden = @hidden", &names);
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"3");
    assert_eq!(scalar(&read.items, "hidden"), b"@hidden");
}
#[test]
fn a_scripted_effect_reads_one_value_and_no_block() {
    crate::test_game::enter();
    let names = KeyNames {
        scripted_effects: [b"my_effect".to_vec()].into(),
        ..Default::default()
    };
    let read = read(
        b"body = { My_Effect = { @seen = 7 add_stability = 1 } final = @seen",
        &names,
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"7");
    assert_eq!(scalar(block(&read.items, "body"), "my_effect"), b"{");
}

#[test]
fn else_and_else_if_read_the_conditional_before_attachment() {
    crate::test_game::enter();
    for key in ["else", "else_if"] {
        let text = format!("body = {{ {key} = {{ @seen = 7 add_stability = 0 }} }} final = @seen");
        let read = read(text.as_bytes(), &KeyNames::default());
        assert_eq!(scalar(&read.items, "final"), b"7");
        let gaps: Vec<_> = read
            .misreads
            .iter()
            .filter(|note| note.count.is_some())
            .collect();
        assert!(gaps.is_empty(), "{gaps:?}");
        let text = format!("body = {{ {key} = no }} final = 2");
        assert!(
            read_effect_nonblock(text.as_bytes())
                .misreads
                .iter()
                .any(|note| note.what.contains("block"))
        );
    }
}

fn read_effect_nonblock(text: &[u8]) -> Read {
    read(text, &KeyNames::default())
}

#[test]
fn readscript_default_body_dispatches_each_effect() {
    crate::test_game::enter();
    for key in ["hidden_effect", "effect_tooltip"] {
        let text = format!(
            "body = {{ {key} = {{ invalid_effect = {{ @hidden = 9 }} @seen = 2 add_stability = 0 }} }} final = @seen hidden = @hidden"
        );
        let read = read(text.as_bytes(), &KeyNames::default());
        assert!(read.unsettled.is_none(), "{read:?}");
        assert_eq!(scalar(&read.items, "final"), b"2");
        assert_eq!(scalar(&read.items, "hidden"), b"@hidden");
        assert!(read.misreads.iter().all(|note| note.count.is_none()));
    }
}

#[test]
fn generic_value_reader_does_not_consume_a_body() {
    crate::test_game::enter();
    let read = read(
        b"body = { add_stability = { @seen = 7 add_stability = 0 } final = @seen",
        &KeyNames::default(),
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"7");
    assert_eq!(scalar(block(&read.items, "body"), "add_stability"), b"{");
}

#[test]
fn conditional_limit_keeps_scope_and_scripted_trigger_names() {
    crate::test_game::enter();
    let names = KeyNames {
        countries: [b"ABC".to_vec()].into(),
        scripted: [b"my_check".to_vec()].into(),
        ..Default::default()
    };
    let read = read(b"body = { if = { limit = { ABC = { my_check = yes invalid_check = { @hidden = 9 } @seen = 3 always = yes } } } } final = @seen hidden = @hidden", &names);
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"3");
    assert_eq!(scalar(&read.items, "hidden"), b"@hidden");
    let scope = block(
        block(block(block(&read.items, "body"), "if"), "limit"),
        "ABC",
    );
    assert_eq!(scalar(scope, "my_check"), b"yes");
}

#[test]
fn missing_operator_uses_stale_value_and_current_error_skip() {
    crate::test_game::enter();
    for (key, expected) in [
        ("hidden_effect", b"7".as_slice()),
        ("ABC", b"7"),
        ("invalid_effect", b"@seen"),
        ("add_stability", b"7"),
    ] {
        let names = KeyNames {
            countries: [b"ABC".to_vec()].into(),
            ..Default::default()
        };
        let text = format!("body = {{ {key} {{ @seen = 7 add_stability = 0 }} final = @seen");
        let read = read(text.as_bytes(), &names);
        assert!(read.unsettled.is_none(), "{key}: {read:?}");
        let items = if key == "add_stability" {
            &read.items
        } else {
            block(&read.items, "body")
        };
        assert_eq!(scalar(items, "final"), expected, "{key}");
    }
}

#[test]
fn coded_effect_precedes_scope_and_scripted_name() {
    crate::test_game::enter();
    let names = KeyNames {
        countries: [b"hidden_effect".to_vec()].into(),
        scripted_effects: [b"hidden_effect".to_vec()].into(),
        ..Default::default()
    };
    let read = read(
        b"body = { hidden_effect = { invalid_effect = { @hidden = 1 } } } final = @hidden",
        &names,
    );
    assert_eq!(scalar(&read.items, "final"), b"@hidden");
    assert!(matches!(
        block(&read.items, "body")[0].value,
        Value::Block(_)
    ));
}

#[test]
fn deferred_readers_keep_by_key_bodies_and_count_named_gaps() {
    crate::test_game::enter();
    let read = read(b"body = { ai_message = { @seen = 1 unknown = @seen } ai_message = { unknown = 2 } every_active_scientist = { @flag = 2 invalid = { @hidden = 9 marker = 1 } } } final = @flag hidden = @hidden", &KeyNames::default());
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"2");
    assert_eq!(scalar(&read.items, "hidden"), b"@hidden");
    let body = block(&read.items, "body");
    assert_eq!(scalar(body[0].value.items().unwrap(), "unknown"), b"1");
    assert!(block(block(body, "every_active_scientist"), "invalid").is_empty());
    let gaps: Vec<_> = read
        .misreads
        .iter()
        .filter(|note| note.count.is_some())
        .collect();
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].what.contains("`ai_message`"));
    assert!(gaps[0].what.contains("unsupported"));
    assert!(gaps[0].what.contains("by-key reading retained"));
    assert_eq!(gaps[0].count, Some(2));
}

#[test]
fn description_projection_follows_own_readers_without_a_global_skip_list() {
    crate::test_game::enter();
    let document = content_reader_parser::parser::parse(br#"
        if = { limit = { always = yes } array = yes original_tag = ABC prioritize = 1 }
        else = { limit = { always = no } array = yes }
        hidden_effect = { array = yes original_tag = ABC include_invisible = yes prioritize = 1 }
        synthetic_scripted = { array = yes original_tag = ABC include_invisible = yes prioritize = 1 }
        ABC = { array = yes original_tag = ABC include_invisible = yes prioritize = 1 }
        create_dynamic_country = { original_tag = ABC copy_tag = DEF array = yes prioritize = 1 }
        random_army_leader = { include_invisible = yes limit = { always = yes } array = yes }
        add_popularity = { ideology = synthetic popularity = 0.2 hidden_effect = { add_stability = 1 } }
        complete_national_focus = { focus = synthetic use_side_message = yes originator_name = KEY hidden_effect = { add_stability = 1 } }
        add_stability = { add_war_support = 1 }
        set_character_flag = { flag = synthetic value = 1 days = 3 }
        set_unit_leader_flag = { flag = synthetic value = 1 days = 3 }
    "#).unwrap();
    let expected: &[&[&str]] = &[
        &["array", "original_tag", "prioritize"],
        &["array"],
        &["array", "original_tag", "include_invisible", "prioritize"],
        &["array", "original_tag", "include_invisible", "prioritize"],
        &["array", "original_tag", "include_invisible", "prioritize"],
        &["array", "prioritize"],
        &["array"],
        &["hidden_effect"],
        &["hidden_effect"],
        &[],
        &[],
        &[],
    ];
    for (item, expected) in document.items.iter().zip(expected) {
        let projected = description_fields(item).unwrap();
        let keys: Vec<_> = projected
            .iter()
            .map(|item| item.key.as_deref().unwrap())
            .collect();
        let expected: Vec<_> = expected.iter().map(|key| key.as_bytes()).collect();
        assert_eq!(keys, expected, "{:?}", item.key);
    }
    assert_eq!(
        description_fields(&document.items[10]).unwrap(),
        description_fields(&document.items[11]).unwrap()
    );
}

#[test]
fn random_list_and_member_projection_share_the_own_key_metadata() {
    crate::test_game::enter();
    let document = content_reader_parser::parser::parse(br#"
        random_list = {
            seed = 4 log = no
            10 = { modifier = { factor = 2 always = yes } array = yes hidden_effect = { add_stability = 1 } }
            20 = { modifier = { add = 1 always = yes } original_tag = ABC }
        }
    "#).unwrap();
    let item = &document.items[0];
    let members = random_list_members(item).unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].key.as_deref(), Some(b"10".as_slice()));
    assert_eq!(members[1].key.as_deref(), Some(b"20".as_slice()));
    assert!(description_fields(item).unwrap().is_empty());
    let first = random_member_fields(&members[0]);
    assert_eq!(first.len(), 2);
    assert!(first[0].keyword_is("array"));
    assert!(first[1].keyword_is("hidden_effect"));
    assert!(random_member_fields(&members[1])[0].keyword_is("original_tag"));
}

#[test]
fn event_random_blocks_project_as_children_but_delay_and_tooltip_stay_own() {
    crate::test_game::enter();
    for key in [
        "country_event",
        "state_event",
        "news_event",
        "unit_leader_event",
    ] {
        let text = format!(
            "{key} = {{ id = synthetic.1 tooltip = SYN_TOOLTIP random = 3 random = {{ chance = 100 add_stability = 1 }} }}"
        );
        let items = read_body(text.as_bytes(), &KeyNames::default()).unwrap();
        let projected = description_fields(&items[0]).unwrap();
        assert_eq!(projected.len(), 1, "{key}");
        assert!(projected[0].keyword_is("random"));
        assert!(projected[0].value.items().is_some());
    }
}

#[test]
fn focus_projection_keeps_the_originator_and_invalid_tooltip_boundary() {
    crate::test_game::enter();
    let document = content_reader_parser::parser::parse(
        br#"
        complete_national_focus = {
            focus = SYN_FOCUS use_side_message = yes
            originator_name = { localization_key = SYN_ORIGINATOR }
            tooltip = SYN_INVALID hidden_effect = { add_stability = 1 }
        }
    "#,
    )
    .unwrap();
    let projected = description_fields(&document.items[0]).unwrap();
    assert_eq!(projected.len(), 2);
    assert!(projected[0].keyword_is("tooltip"));
    assert!(projected[1].keyword_is("hidden_effect"));
    let names = KeyNames::default();
    let parsed = read_body(
        b"complete_national_focus = { focus = SYN_FOCUS tooltip = SYN_INVALID hidden_effect = { add_stability = 1 } }",
        &names,
    ).unwrap();
    let children = description_fields(&parsed[0]).unwrap();
    assert_eq!(children.len(), 2);
    assert!(!adds_child(&children[0], &names).unwrap());
    assert!(adds_child(&children[1], &names).unwrap());

    let parsed = read(
        b"body = { complete_national_focus = { focus = SYN_FOCUS tooltip = { @hidden = 9 } hidden_effect = { @seen = 3 add_stability = @seen } } } hidden = @hidden final = @seen",
        &names,
    );
    assert!(parsed.unsettled.is_none(), "{parsed:?}");
    assert!(parsed.misreads.is_empty(), "{parsed:?}");
    assert_eq!(scalar(&parsed.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&parsed.items, "final"), b"3");
}

#[test]
fn active_scientist_projection_reuses_its_character_reader() {
    crate::test_game::enter();
    let names = KeyNames::default();
    for key in ["every_scientist", "every_active_scientist"] {
        let text = format!(
            "{key} = {{ tooltip = SYN_TOOLTIP limit = {{ always = yes }} visible_when_empty = yes display_individual_scopes = yes random_select_amount = 3 add_stability = 1 }}"
        );
        let items = read_body(text.as_bytes(), &names).unwrap();
        let children = description_fields(&items[0]).unwrap();
        assert_eq!(children.len(), 1, "{key}");
        assert!(children[0].keyword_is("add_stability"), "{key}");
        for own in [
            "tooltip",
            "limit",
            "visible_when_empty",
            "display_individual_scopes",
            "random_select_amount",
        ] {
            assert!(
                is_own_field(registered(key.as_bytes()).unwrap(), own.as_bytes()),
                "{key}.{own}"
            );
        }
    }
}

#[test]
fn technology_scalar_effects_keep_their_operand_and_no_child_array() {
    crate::test_game::enter();
    let names = KeyNames::default();
    let items = read_body(
        b"@source=BBB @enabled=yes inherit_technology=@source set_faction_research_unlocked=@enabled",
        &names,
    ).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].value.scalar(), Some(b"BBB".as_slice()));
    assert_eq!(items[1].value_keyword().as_deref(), Some("yes"));
    for item in &items {
        assert!(description_fields(item).unwrap().is_empty());
        assert!(adds_child(item, &names).unwrap());
    }
}
