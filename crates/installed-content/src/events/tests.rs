use super::*;

#[test]
fn title_candidates_evaluate_all_blocks_and_refuse_only_different_eligible_keys() {
    let event = |titles: &[u8]| Event {
        titles: crate::parser::parse(titles).unwrap().items,
        ..Default::default()
    };
    let bypass = event(b"title={key=ONLY trigger={always=no}}");
    assert_eq!(
        bypass.deterministic_title_key(|_| panic!("sole trigger")),
        Ok(Some(b"ONLY".as_slice()))
    );
    let titles = event(b"title={text=FIRST trigger={always=no}} title={text=LAST key=SECOND trigger={always=yes} trigger={always=no}}");
    let mut calls = 0;
    assert_eq!(
        titles.deterministic_title_key(|conditions| {
            calls += 1;
            assert_eq!(conditions.len(), if calls == 1 { 1 } else { 2 });
            Ok(calls == 2)
        }),
        Ok(Some(b"SECOND".as_slice()))
    );
    assert_eq!(calls, 2);
    assert_eq!(
        titles.deterministic_title_key(|_| Ok(false)),
        Ok(Some(b"FIRST".as_slice()))
    );
    assert!(
        titles
            .deterministic_title_key(|_| Ok(true))
            .unwrap_err()
            .contains("high-resolution seed")
    );
    let same = event(b"title=SAME title={key=SAME}");
    assert_eq!(
        same.deterministic_title_key(|_| Ok(true)),
        Ok(Some(b"SAME".as_slice()))
    );
}

#[test]
fn sole_event_title_preserves_c_string_last_field_and_trigger_bypass() {
    let mut event = Event {
        titles: crate::parser::parse(b"title={text=FIRST text=LAST trigger={always=no}}")
            .unwrap()
            .items,
        ..Default::default()
    };
    let crate::parser::Value::Block(fields) = &mut event.titles[0].value else {
        unreachable!()
    };
    fields[1].value = crate::parser::Value::Scalar(b"LAST\0ignored".to_vec());
    assert_eq!(event.sole_title_key(), Ok(Some(b"LAST".as_slice())));
    event.titles = crate::parser::parse(b"title=EMPTY").unwrap().items;
    event.titles[0].value = crate::parser::Value::Scalar(b"\0ignored".to_vec());
    assert_eq!(event.sole_title_key(), Ok(None));
    event.titles.push(event.titles[0].clone());
    assert!(
        event
            .sole_title_key()
            .unwrap_err()
            .contains("high-resolution seed")
    );
    event.titles.clear();
    assert_eq!(event.sole_title_key(), Ok(None));
}
#[test]
fn border_events_keep_immediate_and_option_order_through_native_effect_reader() {
    let input = b"add_namespace=synthetic country_event={id=synthetic.1 title=TITLE immediate={custom_effect_tooltip=FIRST} option={name=A custom_effect_tooltip=SECOND} option={name=B hidden_effect={custom_effect_tooltip=HIDDEN}}}";
    let events = parse(input, "events/synthetic.txt", &KeyNames::default());
    let event = &events.entries[b"synthetic.1".as_slice()];
    assert_eq!(event.titles[0].value.scalar(), Some(b"TITLE".as_slice()));
    assert_eq!(event.immediate[0].value.scalar(), Some(b"FIRST".as_slice()));
    assert_eq!(event.options.len(), 2);
    assert!(event.options[0][0].keyword_is("custom_effect_tooltip"));
    assert!(event.options[1][0].keyword_is("hidden_effect"));
    assert!(event.refused.is_none());
}

#[test]
fn border_event_identifiers_reduce_suffix_and_refuse_namespace_slot_aliases() {
    assert_eq!(
        identifier_key(b"Synthetic.00001\0tail").unwrap(),
        b"synthetic.1"
    );
    assert_eq!(identifier_key(b"s.A").unwrap(), b"s.17"); // Native does not validate digits.
    assert_eq!(identifier_key(b"s.").unwrap(), b"s.0");
    assert!(
        identifier_key(b"s.100000")
            .unwrap_err()
            .contains("namespace slot")
    );
    assert!(identifier_key(b"42").is_err());
    let events = parse(
        b"add_namespace=S country_event={id=s.01} country_event={id=S.1}",
        "synthetic.txt",
        &KeyNames::default(),
    );
    assert_eq!(events.entries.len(), 1);
    assert!(
        events
            .get(b"s.001")
            .unwrap()
            .refused
            .as_ref()
            .unwrap()
            .contains("repeated event identifier")
    );
    assert!(
        events
            .get(b"other.1")
            .unwrap_err()
            .contains("not registered")
    );
}

#[test]
fn border_event_loader_skips_bom_before_namespace_and_title_reads_last_text() {
    let events = parse(
        b"\xef\xbb\xbfadd_namespace=s country_event={id=s.1 title={text=FIRST key=LAST}}",
        "synthetic.txt",
        &KeyNames::default(),
    );
    assert_eq!(
        events.get(b"s.1").unwrap().titles[0]
            .value
            .items()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        events.get(b"s.1").unwrap().sole_title_key(),
        Ok(Some(b"LAST".as_slice()))
    );
}

#[test]
fn operative_event_uses_common_reader_and_preserves_titles_and_effect_order() {
    let events = parse(
        b"add_namespace=s operative_leader_event={id=s.5 title=FIRST title={text=LAST trigger={always=no}} desc=DESCRIPTION is_triggered_only=yes major=no immediate={custom_effect_tooltip=ONE custom_effect_tooltip=TWO} option={name=A custom_effect_tooltip=THREE custom_effect_tooltip=FOUR} option={name=B hidden_effect={custom_effect_tooltip=FIVE}}}",
        "events/synthetic.txt",
        &KeyNames::default(),
    );
    let event = events.get(b"s.5").unwrap();
    assert!(event.refused.is_none(), "{:?}", event.refused);
    assert!(events.refused.is_empty(), "{:?}", events.refused);
    assert_eq!(event.titles.len(), 2);
    assert_eq!(event.titles[0].value.scalar(), Some(b"FIRST".as_slice()));
    let title = event.titles[1].value.items().unwrap();
    assert_eq!(title.len(), 2);
    assert!(title[0].keyword_is("text"));
    assert_eq!(title[0].value.scalar(), Some(b"LAST".as_slice()));
    assert!(title[1].keyword_is("trigger"));
    let trigger = title[1].value.items().unwrap();
    assert_eq!(trigger.len(), 1);
    assert!(trigger[0].keyword_is("always"));
    assert_eq!(trigger[0].value.scalar(), Some(b"no".as_slice()));
    assert!(
        event
            .immediate
            .iter()
            .all(|item| item.keyword_is("custom_effect_tooltip"))
    );
    assert_eq!(
        event
            .immediate
            .iter()
            .map(|item| item.value.scalar())
            .collect::<Vec<_>>(),
        [Some(b"ONE".as_slice()), Some(b"TWO".as_slice())]
    );
    assert_eq!(event.options.len(), 2);
    assert!(
        event.options[0]
            .iter()
            .all(|item| item.keyword_is("custom_effect_tooltip"))
    );
    assert_eq!(
        event.options[0]
            .iter()
            .map(|item| item.value.scalar())
            .collect::<Vec<_>>(),
        [Some(b"THREE".as_slice()), Some(b"FOUR".as_slice())]
    );
    assert_eq!(event.options[1].len(), 1);
    assert!(event.options[1][0].keyword_is("hidden_effect"));
    let hidden = event.options[1][0].value.items().unwrap();
    assert_eq!(hidden.len(), 1);
    assert!(hidden[0].keyword_is("custom_effect_tooltip"));
    assert_eq!(hidden[0].value.scalar(), Some(b"FIVE".as_slice()));
}

#[test]
fn operative_event_immediate_and_after_forward_character_parse_scope() {
    for block in ["immediate", "after"] {
        let input = format!(
            "add_namespace=s operative_leader_event={{id=s.5 {block}={{promote_character={{character={{@ignored=LEAKED}} ideology=fixture}} remove_unit_leader_role=yes}} title=@ignored}}"
        );
        let events = parse(
            input.as_bytes(),
            "events/synthetic.txt",
            &KeyNames::default(),
        );
        let event = events.get(b"s.5").unwrap();
        assert!(event.refused.is_none(), "{block}: {:?}", event.refused);
        assert!(events.refused.is_empty(), "{block}: {:?}", events.refused);
        // 1403AA3C0 skips character's block in mask 8, so its variable
        // never enters the common CReader variable dictionary.
        assert_eq!(event.titles[0].value.scalar(), Some(b"@ignored".as_slice()));
    }
}

#[test]
fn operative_option_direct_role_uses_character_scope_and_skips_rejected_block() {
    let events = parse(
        b"add_namespace=s operative_leader_event={id=s.5 option={name=A remove_unit_leader_role={character=fixture @ignored=LEAKED}} title=@ignored}",
        "events/synthetic.txt",
        &KeyNames::default(),
    );
    let event = events.get(b"s.5").unwrap();
    assert!(event.refused.is_none(), "{:?}", event.refused);
    assert_eq!(event.titles[0].value.scalar(), Some(b"@ignored".as_slice()));
    assert_eq!(events.refused.len(), 1);
    assert!(events.refused[0].contains("requires exact yes"));
    assert!(!events.refused[0].contains("Deferred effect"));
}

#[test]
fn operative_option_refuses_only_nested_readers_missing_inherited_scope() {
    for effect in [
        "promote_character={character={@ignored=LEAKED} ideology=fixture}",
        "hidden_effect={remove_unit_leader_role={character=fixture}}",
    ] {
        let input = format!(
            "add_namespace=s operative_leader_event={{id=s.5 title=TITLE option={{name=A {effect}}}}}"
        );
        let events = parse(
            input.as_bytes(),
            "events/synthetic.txt",
            &KeyNames::default(),
        );
        let event = events.get(b"s.5").unwrap();
        let why = event.refused.as_ref().unwrap();
        assert!(
            why.contains("COperativeLeaderEventEffect needs the scoped-object parser helper"),
            "{why}"
        );
        assert!(why.contains("Deferred effect"), "{why}");
    }
    let events = parse(
        b"add_namespace=s operative_leader_event={id=s.5 title=TITLE option={name=A every_character={promote_character={character={@ignored=LEAKED} ideology=fixture} remove_unit_leader_role=yes}}}",
        "events/synthetic.txt",
        &KeyNames::default(),
    );
    // This class's authoritative ScopePolicy sets mask 8 independently.
    assert!(events.get(b"s.5").unwrap().refused.is_none());
    assert!(events.refused.is_empty(), "{:?}", events.refused);
}

#[test]
fn lazy_events_read_nothing_until_demand_and_cache_only_requested_identifiers() {
    let root = std::env::temp_dir().join(format!("hoi4-lazy-events-{}", std::process::id()));
    std::fs::create_dir_all(root.join("events")).unwrap();
    std::fs::write(root.join("events/a.txt"), b"add_namespace=s country_event={id=s.1 title=FIRST immediate={@next=s.2 custom_effect_tooltip=TEXT}} country_event={id=@next title=SECOND}").unwrap();
    std::fs::write(
        root.join("events/unrelated.txt"),
        b"add_namespace=u country_event={id=u.1 immediate={custom_effect_tooltip=UNREAD}} ",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert_eq!(
        layers.directory_files("events", ".txt").unwrap(),
        ["events/a.txt", "events/unrelated.txt"]
    );
    let parsed = parse(
        &layers.read("events/a.txt").unwrap(),
        "events/a.txt",
        &KeyNames::default(),
    );
    assert!(parsed.namespaces.contains(b"s".as_slice()));
    assert!(parsed.get(b"s.2").is_ok());
    let events = LazyEvents::new(Arc::new(layers));
    // Construction performs no event/header I/O.
    assert!(events.cache.lock().unwrap().index.is_none());
    let first = events.get(b"S.01").unwrap();
    assert!(Arc::ptr_eq(&first, &events.get(b"s.1").unwrap()));
    assert_eq!(events.cache.lock().unwrap().events.len(), 1);
    assert_eq!(first.titles[0].value.scalar(), Some(b"FIRST".as_slice()));
    assert!(first.refused.is_none());
    assert_eq!(
        events.get(b"s.2").unwrap().titles[0].value.scalar(),
        Some(b"SECOND".as_slice())
    );
    assert_eq!(events.cache.lock().unwrap().events.len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}
