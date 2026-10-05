use super::*;
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
        b"\xef\xbb\xbfadd_namespace=s country_event={id=s.1 title={text=FIRST text=LAST}}",
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
}

#[test]
fn operative_event_is_named_and_refused_instead_of_disappearing() {
    let events = parse(
        b"add_namespace=s operative_leader_event={id=s.1 title=TITLE immediate={custom_effect_tooltip=TEXT}}",
        "events/synthetic.txt",
        &KeyNames::default(),
    );
    assert!(
        events
            .get(b"s.1")
            .unwrap()
            .refused
            .as_ref()
            .unwrap()
            .contains("operative_leader_event")
    );
    assert_eq!(events.refused.len(), 1);
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
