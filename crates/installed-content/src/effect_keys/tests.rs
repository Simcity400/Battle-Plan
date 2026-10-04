use super::*;
use crate::parser::{
    Item, Value,
    native::{self, Read},
};

pub(super) fn read(text: &[u8], names: &KeyNames) -> Read {
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(names),
        &mut |(), field| {
            if field.keyword_is("body") {
                Shape::Effect
            } else {
                Shape::One
            }
        },
    )
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
    for (token, _) in REGISTERED_EFFECTS {
        assert!(class(token).is_some());
    }
    assert_eq!(class(u32::MAX), None);
}

// Invalid effect + RecordError 1405345D0 -> 1424ACF90 -> 1424AD270.
#[test]
fn invalid_effect_skips_without_defining_names() {
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
    let names = KeyNames {
        countries: [b"ABC".to_vec()].into(),
        ..Default::default()
    };
    let read = read(b"body = { ABC = { invalid_effect = { @hidden = 7 } @seen = 3 add_stability = 0 } } final = @seen hidden = @hidden", &names);
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"3");
    assert_eq!(scalar(&read.items, "hidden"), b"@hidden");
}

// CScriptedEffect vtable 14299F7C8, slot 3 = 141385570.
#[test]
fn a_scripted_effect_reads_one_value_and_no_block() {
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
                .any(|note| note.what.contains("140534280"))
        );
    }
}

fn read_effect_nonblock(text: &[u8]) -> Read {
    read(text, &KeyNames::default())
}

#[test]
fn readscript_default_body_dispatches_each_effect() {
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
        // The one-value reader leaves the child's `}` to end `body`;
        // ReadScript and the error skip consume it inside that body.
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
    // Both class tokens have zero reach. every_active_scientist shares the
    // ported every_character key-reader address but keeps its named refusal.
    let read = read(b"body = { ai_message = { @seen = 1 unknown = @seen } ai_message = { unknown = 2 } every_active_scientist = { @flag = 2 invalid = { @hidden = 9 marker = 1 } } } final = @flag hidden = @hidden", &KeyNames::default());
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(scalar(&read.items, "final"), b"2");
    assert_eq!(scalar(&read.items, "hidden"), b"9");
    let gaps: Vec<_> = read
        .misreads
        .iter()
        .filter(|note| note.count.is_some())
        .collect();
    assert_eq!(gaps.len(), 2);
    assert!(gaps[0].what.contains("`ai_message`"));
    assert!(gaps[0].what.contains("0x140534280"));
    assert!(gaps[0].what.contains("0x1403AE870"));
    assert_eq!(gaps[0].count, Some(2));
    assert!(gaps[1].what.contains("`every_active_scientist`"));
    assert!(gaps[1].what.contains("0x140534280"));
    assert!(gaps[1].what.contains("0x1402F6D70"));
    assert_eq!(gaps[1].count, Some(1));
}
