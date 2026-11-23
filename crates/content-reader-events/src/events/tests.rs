use content_reader_key_names::key_name_products::KeyNamesExt as _;
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn cached_event_index_and_trees_replay_their_own_reads_after_prefetch() {
    let (events, root) = option_index_fixture(&[
        (
            "a.txt",
            b"add_namespace=s country_event={id=s.1 title=FIRST option={name=OPTION}}",
        ),
        (
            "b.txt",
            b"add_namespace=s country_event={id=s.2 title=SECOND}",
        ),
    ]);
    let layers = ResolvedLayers::from_folders_with_coverage_policy(&[(&root, &[])], 2).unwrap();
    let _ = layers.report_uncovered_reads();
    let read = |events: &LazyEvents| {
        content_reader_layers::content::diagnostics::observe(|| events.get(b"s.1").unwrap())
    };
    let alone = LazyEvents::new(Arc::new(layers.clone()));
    let (expected, notes) = read(&alone);
    assert!(notes.has_uncovered());
    assert_eq!(read(&alone).1, notes);
    let shared = LazyEvents::new(Arc::new(layers));
    shared.prefetch([b"s.1".as_slice(), b"s.2"], &AtomicBool::new(false));
    let (actual, warm) = read(&shared);
    assert_eq!(*actual, *expected);
    assert_eq!(warm, notes);
    let (_, option_notes) =
        content_reader_layers::content::diagnostics::observe(|| shared.options(b"OPTION").unwrap());
    assert_eq!(option_notes, notes);
    drop(events);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn event_presentation_keeps_description_picture_and_each_option_admission_trigger() {
    content_reader_script::test_game::enter();
    let parsed=parse(b"add_namespace=synthetic country_event={id=synthetic.3 title=TITLE desc=BODY picture=FIRST picture=LAST option={name=CHOICE trigger={always=yes} original_recipient_only=yes custom_effect_tooltip=EFFECT}}", "synthetic.txt", &KeyNames::default());
    let event = parsed.get(b"synthetic.3").unwrap();
    assert_eq!(event.sole_description_key(), Ok(Some(b"BODY".as_slice())));
    assert_eq!(event.picture, b"LAST");
    assert_eq!(event.option_triggers.len(), 1);
    assert_eq!(event.option_triggers[0].len(), 1);
    assert!(event.option_triggers[0][0].keyword_is("always"));
    assert_eq!(event.original_recipient_only, [Some(true)]);
    assert_eq!(event.options[0].len(), 1);
    assert!(event.options[0][0].keyword_is("custom_effect_tooltip"));
}

#[test]
fn any_original_recipient_assignment_gates_the_option_and_a_failed_read_keeps_the_mode() {
    content_reader_script::test_game::enter();
    let parsed = parse(
        b"add_namespace=synthetic country_event={id=synthetic.4 title=TITLE \
          option={name=KEPT original_recipient_only=yes original_recipient_only=invalid} \
          option={name=OTHERS original_recipient_only=no} \
          option={name=REFUSED original_recipient_only=invalid} \
          option={name=OPEN}}",
        "synthetic.txt",
        &KeyNames::default(),
    );
    let event = parsed.get(b"synthetic.4").unwrap();
    assert_eq!(
        event.original_recipient_only,
        [Some(true), Some(false), Some(false), None]
    );
    assert!(event.options.iter().all(Vec::is_empty));
}

#[test]
fn title_candidates_evaluate_all_blocks_and_refuse_only_different_eligible_keys() {
    let event = |titles: &[u8]| Event {
        titles: crate::parser::parse(titles).unwrap().items,
        ..Default::default()
    };
    let settled = |key: &'static [u8]| {
        Ok(TitleKey {
            key: Some(key),
            unsettled: None,
        })
    };
    let bypass = event(b"title={key=ONLY trigger={always=no}}");
    assert_eq!(
        bypass.deterministic_title_key(|_| panic!("sole trigger")),
        settled(b"ONLY")
    );
    let titles = event(b"title={text=FIRST trigger={always=no}} title={text=LAST key=SECOND trigger={always=yes} trigger={always=no}}");
    let mut calls = 0;
    assert_eq!(
        titles.deterministic_title_key(|conditions| {
            calls += 1;
            assert_eq!(conditions.len(), if calls == 1 { 1 } else { 2 });
            Ok(calls == 2)
        }),
        settled(b"SECOND")
    );
    assert_eq!(calls, 2);
    assert_eq!(
        titles.deterministic_title_key(|_| Ok(false)),
        settled(b"FIRST")
    );
    assert!(
        titles
            .deterministic_title_key(|_| Ok(true))
            .unwrap_err()
            .contains("high-resolution seed")
    );
    assert_eq!(
        titles.deterministic_title_key(|_| Err("unrecorded operand".into())),
        Err("unrecorded operand".into())
    );
    let same = event(b"title=SAME title={key=SAME}");
    assert_eq!(same.deterministic_title_key(|_| Ok(true)), settled(b"SAME"));
}

#[test]
fn identical_title_keys_survive_a_trigger_that_cannot_be_decided() {
    let event = Event {
        titles: crate::parser::parse(
            b"title={key=SAME trigger={always=yes}} title={text=SAME trigger={always=no}} title={key=SAME trigger={always=yes}}",
        )
        .unwrap()
        .items,
        ..Default::default()
    };
    let mut calls = 0;
    assert_eq!(
        event.deterministic_title_key(|_| {
            calls += 1;
            if calls == 2 {
                Err("unrecorded operand".into())
            } else {
                Ok(true)
            }
        }),
        Ok(TitleKey {
            key: Some(b"SAME".as_slice()),
            unsettled: Some("unrecorded operand".into()),
        })
    );
    assert_eq!(calls, 2);
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
    assert_eq!(event.sole_title_key(), Ok(None));
    event.titles[1].value = crate::parser::Value::Scalar(b"DIFFERENT".to_vec());
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
fn border_events_keep_immediate_and_option_order_through_effect_reader() {
    content_reader_script::test_game::enter();
    let input = b"add_namespace=synthetic country_event={id=synthetic.1 title=TITLE immediate={custom_effect_tooltip=FIRST} option={name=A custom_effect_tooltip=SECOND} option={name=B hidden_effect={custom_effect_tooltip=HIDDEN}}}";
    let events = parse(input, "events/synthetic.txt", &KeyNames::default());
    let event = &events.entries[b"synthetic.1".as_slice()];
    assert_eq!(event.titles[0].value.scalar(), Some(b"TITLE".as_slice()));
    assert_eq!(event.immediate[0].value.scalar(), Some(b"FIRST".as_slice()));
    assert_eq!(event.options.len(), 2);
    assert_eq!(event.option_names, [b"A".to_vec(), b"B".to_vec()]);
    assert!(event.options[0][0].keyword_is("custom_effect_tooltip"));
    assert!(event.options[1][0].keyword_is("hidden_effect"));
    assert!(event.refused.is_none());
}

#[test]
fn border_event_identifiers_reduce_suffix_and_refuse_namespace_slot_aliases() {
    content_reader_script::test_game::enter();
    assert_eq!(
        identifier_key(b"Synthetic.00001\0tail").unwrap(),
        b"synthetic.1"
    );
    assert_eq!(identifier_key(b"s.A").unwrap(), b"s.17");
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
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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
        assert_eq!(event.titles[0].value.scalar(), Some(b"@ignored".as_slice()));
    }
}

#[test]
fn operative_option_direct_role_uses_character_scope_and_skips_rejected_block() {
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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
            why.contains("needs the scoped-object parser helper"),
            "{why}"
        );
        assert!(why.contains("Deferred effect"), "{why}");
    }
    let events = parse(
        b"add_namespace=s operative_leader_event={id=s.5 title=TITLE option={name=A every_character={promote_character={character={@ignored=LEAKED} ideology=fixture} remove_unit_leader_role=yes}}}",
        "events/synthetic.txt",
        &KeyNames::default(),
    );
    assert!(events.get(b"s.5").unwrap().refused.is_none());
    assert!(events.refused.is_empty(), "{:?}", events.refused);
}

#[test]
fn lazy_events_read_nothing_until_demand_and_cache_only_requested_identifiers() {
    content_reader_script::test_game::enter();
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

fn option_index_fixture(files: &[(&str, &[u8])]) -> (LazyEvents, std::path::PathBuf) {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4-option-index-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("events")).unwrap();
    for (name, bytes) in files {
        std::fs::write(root.join("events").join(name), bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    (LazyEvents::new(Arc::new(layers)), root)
}

#[test]
fn prefetched_events_read_each_file_once_and_look_up_as_single_lookups_do() {
    let (single, root) = option_index_fixture(&[
        (
            "a.txt",
            b"add_namespace=s country_event={id=s.1 title=FIRST} country_event={id=s.2 title=SECOND} country_event={id=s.3 title=THIRD}",
        ),
        ("b.txt", b"add_namespace=t country_event={id=t.1 title=OTHER}"),
    ]);
    let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let traced = {
        let reads = Arc::clone(&reads);
        ResolvedLayers::from_folders(&[(&root, &[])])
            .unwrap()
            .with_read_trace(Arc::new(move |read: content_reader_layers::content::ContentRead| {
                reads.lock().unwrap().push(String::from_utf8(read.path).unwrap());
            }))
    };
    let batched = LazyEvents::new(Arc::new(traced));
    let ids: [&[u8]; 4] = [b"s.1", b"s.3", b"t.1", b"x.9"];
    batched.prefetch(ids, &AtomicBool::new(false));
    let prefetched = reads.lock().unwrap().clone();
    for file in ["events/a.txt", "events/b.txt"] {
        assert_eq!(prefetched.iter().filter(|path| *path == file).count(), 2, "{prefetched:?}");
    }
    for id in ids {
        assert_eq!(format!("{:?}", batched.get(id)), format!("{:?}", single.get(id)));
    }
    assert_eq!(*reads.lock().unwrap(), prefetched);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prefetched_events_read_apart_and_leave_unread_what_a_lookup_refuses_unread() {
    let (single, root) = option_index_fixture(&[
        (
            "a.txt",
            b"add_namespace=s country_event={id=s.0 option={name=Other promote_character={character={ignored}}}} country_event={id=s.1 option={name=Chosen custom_effect_tooltip=ONE}}",
        ),
        ("b.txt", b"country_event={id=orphan.1 title=ORPHAN}"),
    ]);
    let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let traced = {
        let reads = Arc::clone(&reads);
        ResolvedLayers::from_folders(&[(&root, &[])])
            .unwrap()
            .with_read_trace(Arc::new(move |read: content_reader_layers::content::ContentRead| {
                reads.lock().unwrap().push(String::from_utf8(read.path).unwrap());
            }))
    };
    let batched = LazyEvents::new(Arc::new(traced));
    batched.prefetch(std::iter::empty(), &AtomicBool::new(false));
    batched.prefetch([b"s.1".as_slice()], &AtomicBool::new(true));
    assert!(reads.lock().unwrap().is_empty());
    assert!(batched.cache.lock().unwrap().index.is_none());
    let cancel = Arc::new(AtomicBool::new(false));
    let cancelling = {
        let (cancel, reads) = (Arc::clone(&cancel), Arc::clone(&reads));
        ResolvedLayers::from_folders(&[(&root, &[])])
            .unwrap()
            .with_read_trace(Arc::new(move |read: content_reader_layers::content::ContentRead| {
                reads.lock().unwrap().push(String::from_utf8(read.path).unwrap());
                cancel.store(true, Ordering::Release);
            }))
    };
    let cancelled = LazyEvents::new(Arc::new(cancelling));
    cancelled.prefetch([b"s.1".as_slice()], &cancel);
    assert_eq!(reads.lock().unwrap().len(), 1, "the index stopped after its first file");
    assert!(cancelled.cache.lock().unwrap().index.is_none());
    reads.lock().unwrap().clear();
    let ids: [&[u8]; 3] = [b"s.0", b"s.1", b"orphan.1"];
    batched.prefetch(ids, &AtomicBool::new(false));
    let prefetched = reads.lock().unwrap().clone();
    for (file, count) in [("events/a.txt", 2), ("events/b.txt", 1)] {
        assert_eq!(
            prefetched.iter().filter(|path| *path == file).count(),
            count,
            "{prefetched:?}"
        );
    }
    assert_eq!(batched.cache.lock().unwrap().trees.len(), 2);
    for id in ids {
        assert_eq!(format!("{:?}", batched.get(id)), format!("{:?}", single.get(id)));
    }
    assert!(batched.get(b"s.1").is_ok_and(|event| event.refused.is_none()));
    assert_eq!(*reads.lock().unwrap(), prefetched);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_option_index_uses_raw_names_and_registration_order() {
    let (events, root) = option_index_fixture(&[
        ("a.txt", b"add_namespace=s country_event={id=s.9 option={name=Choice custom_effect_tooltip=FIRST} option={name=Choice custom_effect_tooltip=SECOND}} state_event={id=s.1 option={name=Choice custom_effect_tooltip=STATE}} news_event={id=s.2 option={name=Choice custom_effect_tooltip=NEWS}}"),
        ("z.txt", b"country_event={id=s.9 option={name=Choice custom_effect_tooltip=DUPLICATE}} unit_leader_event={id=bare option={name=Choice custom_effect_tooltip=LEADER}} operative_leader_event={id=unregistered.4 option={name=Choice custom_effect_tooltip=OPERATIVE}} country_event={id=s.100000 option={name=Choice custom_effect_tooltip=CROSSES_SLOT}} country_event={option={name=Choice custom_effect_tooltip=NO_ID}}"),
    ]);
    let choices = events.options(b"Choice\0ignored").unwrap();
    assert_eq!(
        choices
            .iter()
            .map(|(event, option)| event.options[*option][0].value.scalar().unwrap())
            .collect::<Vec<_>>(),
        [
            b"FIRST".as_slice(),
            b"SECOND",
            b"NEWS",
            b"DUPLICATE",
            b"LEADER",
            b"OPERATIVE",
            b"CROSSES_SLOT",
            b"NO_ID"
        ]
    );
    assert!(Arc::ptr_eq(&choices[0].0, &choices[1].0));
    assert!(choices.iter().all(|(event, _)| event.refused.is_none()));
    assert!(events.options(b"choice").unwrap().is_empty());
    assert!(
        events
            .get(b"s.9")
            .unwrap()
            .refused
            .as_ref()
            .unwrap()
            .contains("repeated event identifier")
    );
    assert!(
        events
            .get(b"unregistered.4")
            .unwrap_err()
            .contains("not registered")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_option_index_preserves_macro_names_last_name_and_empty_name() {
    let (events, root) = option_index_fixture(&[
        ("a.txt", b"@first=Choice country_event={id=7 option={name=old name=@first @later=Later custom_effect_tooltip=ONE} option={name=@later custom_effect_tooltip=TWO} option={custom_effect_tooltip=EMPTY}}"),
    ]);
    let first = events.options(b"Choice").unwrap();
    let later = events.options(b"Later").unwrap();
    let empty = events.options(b"").unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(later[0].1, 1);
    assert_eq!(empty[0].1, 2);
    assert_eq!(
        first[0].0.option_names,
        [b"Choice".to_vec(), b"Later".to_vec(), Vec::new()]
    );
    assert!(Arc::ptr_eq(&first[0].0, &later[0].0));
    assert!(Arc::ptr_eq(&first[0].0, &empty[0].0));
    assert!(events.options(b"old").unwrap().is_empty());
    assert_eq!(events.cache.lock().unwrap().files.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_option_index_shares_requested_trees_and_keeps_other_files_header_only() {
    let (events, root) = option_index_fixture(&[
        ("a.txt", b"@chosen=Chosen add_namespace=s country_event={id=s.1 option={name=@chosen custom_effect_tooltip=ONE}}"),
        ("unrelated.txt", b"add_namespace=u country_event={id=u.1 option={name=Other hidden_effect={remove_unit_leader_role={character=unsupported}}}}"),
    ]);
    assert!(events.cache.lock().unwrap().index.is_none());
    assert!(events.options(b"Missing").unwrap().is_empty());
    assert!(events.cache.lock().unwrap().files.is_empty());
    let first = events.options(b"Chosen").unwrap();
    assert!(events.cache.lock().unwrap().files.is_empty());
    assert_eq!(events.cache.lock().unwrap().trees.len(), 1);
    let by_id = events.get(b"S.01").unwrap();
    assert!(Arc::ptr_eq(&first[0].0, &by_id));
    std::fs::write(root.join("events/a.txt"), b"invalid replacement").unwrap();
    assert!(Arc::ptr_eq(
        &first[0].0,
        &events.options(b"Chosen").unwrap()[0].0
    ));
    assert!(Arc::ptr_eq(&by_id, &events.get(b"s.1").unwrap()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_option_index_skips_unrelated_unsettled_bodies_in_the_same_static_file() {
    content_reader_script::test_game::enter();
    let bytes = b"add_namespace=s country_event={id=s.0 option={name=Other promote_character={character={ignored}}}} country_event={id=s.1 option={name=Chosen custom_effect_tooltip=ONE}} country_event={id=s.2 option={name=Chosen custom_effect_tooltip=TWO}} country_event={id=s.3 option={name=Later custom_effect_tooltip=THREE}}";
    let (events, root) = option_index_fixture(&[("a.txt", bytes)]);
    let full = read_events(bytes, &KeyNames::default(), None);
    assert!(full.read.unsettled.is_some());
    let chosen = events.options(b"Chosen").unwrap();
    assert_eq!(chosen.len(), 2);
    assert_eq!(
        chosen
            .iter()
            .map(|(event, option)| event.options[*option][0].value.scalar().unwrap())
            .collect::<Vec<_>>(),
        [b"ONE".as_slice(), b"TWO"]
    );
    assert!(chosen.iter().all(|(event, _)| event.refused.is_none()));
    assert!(events.cache.lock().unwrap().files.is_empty());
    assert_eq!(events.cache.lock().unwrap().trees.len(), 2);
    let later = events.get(b"s.3").unwrap();
    assert_eq!(
        later.options[0][0].value.scalar(),
        Some(b"THREE".as_slice())
    );
    assert_eq!(events.cache.lock().unwrap().trees.len(), 3);
    assert!(events.cache.lock().unwrap().files.is_empty());
    std::fs::write(root.join("events/a.txt"), b"invalid replacement").unwrap();
    assert!(Arc::ptr_eq(&chosen[0].0, &events.get(b"s.1").unwrap()));
    assert!(Arc::ptr_eq(&chosen[1].0, &events.get(b"s.2").unwrap()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lazy_events_share_exact_content_and_viewer_layer_owners_and_memos() {
    let content = Arc::new(ResolvedContent {
        layers: ResolvedLayers::default(),
        stats: Default::default(),
        changed: Vec::new(),
        notes: Vec::new(),
    });
    let content_owner = Arc::downgrade(&content);
    let content_names = content.layers.key_names().unwrap();
    let events = LazyEvents::from_content(Arc::clone(&content));
    assert!(std::ptr::eq(&*events.layers, &content.layers));
    assert_eq!(Arc::strong_count(&content), 2);
    assert!(Arc::ptr_eq(
        &content_names,
        &events.layers.key_names().unwrap()
    ));
    assert!(events.cache.lock().unwrap().index.is_none());
    assert!(events.options(b"absent").unwrap().is_empty());
    assert!(events.cache.lock().unwrap().index.is_some());
    drop(content);
    assert!(content_owner.upgrade().is_some());
    drop(events);
    assert!(content_owner.upgrade().is_none());

    let layers = Arc::new(ResolvedLayers::default());
    let viewer_owner = Arc::downgrade(&layers);
    let viewer_names = layers.key_names().unwrap();
    let viewer_events = LazyEvents::new(Arc::clone(&layers));
    assert!(std::ptr::eq(&*viewer_events.layers, layers.as_ref()));
    assert_eq!(Arc::strong_count(&layers), 2);
    assert!(Arc::ptr_eq(
        &viewer_names,
        &viewer_events.layers.key_names().unwrap()
    ));
    assert!(viewer_events.cache.lock().unwrap().index.is_none());
    drop(layers);
    assert!(viewer_owner.upgrade().is_some());
    drop(viewer_events);
    assert!(viewer_owner.upgrade().is_none());
}

#[test]
fn event_lookup_diagnostics_name_the_shared_event_database() {
    content_reader_script::test_game::enter();
    assert_eq!(
        identifier_key(b"42").unwrap_err(),
        "event 42 has no namespace; event lookup cannot resolve it"
    );
    assert_eq!(
        identifier_key(b".1").unwrap_err(),
        "event namespace is empty"
    );
    let events = parse(
        b"add_namespace=s",
        "events/synthetic.txt",
        &KeyNames::default(),
    );
    assert_eq!(
        events.get(b"missing.1").unwrap_err(),
        "event missing.1 namespace is not registered through add_namespace"
    );
    assert_eq!(
        events.get(b"s.1").unwrap_err(),
        "event s.1 is absent from the resolved event database; "
    );
    let lazy = LazyEvents::new(Arc::new(ResolvedLayers::default()));
    assert_eq!(
        lazy.get(b"missing.1").unwrap_err(),
        "event missing.1 namespace is not registered through add_namespace"
    );
}
