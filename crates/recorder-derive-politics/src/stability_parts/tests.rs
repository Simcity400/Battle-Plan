use super::*;
use inputs::Template;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::{CharacterRow, CountryRow, LexicalTokenRow, SliceReadRow};
use std::collections::BTreeMap;

const STABILITY: u8 = StabilityPartRow::STABILITY;
const WAR_SUPPORT: u8 = StabilityPartRow::WAR_SUPPORT;

fn inputs_row(country: u32, base: i64, factor: Option<i64>) -> CountryStabilityInputRow {
    let mut keys: Vec<u32> = (0..10).collect();
    if factor.is_none() {
        keys[0] = u32::MAX;
    }
    CountryStabilityInputRow {
        country,
        stability_raw: base,
        war_support_raw: 20_000,
        max_stability: 50_000,
        max_war_support: 100_000,
        stability_factor: factor,
        party_popularity_stability_factor: Some(0),
        war_stability_factor: Some(0),
        offensive_war_stability_factor: Some(0),
        defensive_war_stability_factor: Some(0),
        war_support_factor: Some(0),
        stability_weekly: Some(0),
        war_support_weekly: Some(0),
        keys,
        ..Default::default()
    }
}

fn line(country: u32, at: u32, key: u32, kind: u8, identity: &[u8], identity_id: u32, value: i64, text: &[u8]) -> CountryStabilityLineRow {
    CountryStabilityLineRow { country, line: at, key, kind, identity: identity.to_vec(), identity_id, secondary: 0, value, text: text.to_vec() }
}

fn lines() -> Vec<CountryStabilityLineRow> {
    vec![
        line(1, 0, 0, CountryStabilityLineRow::IDEA, b"a", 0, 10_000, b""),
        line(1, 1, 0, CountryStabilityLineRow::ADVISOR, b"adv", 7, 4_000, b""),
        line(1, 2, 0, CountryStabilityLineRow::UNPAIRED, b"", 0, 3_000, b"Mystery"),
        line(1, 3, 0, CountryStabilityLineRow::IDEA, b"a", 0, 5_000, b""),
        line(1, 4, 0, CountryStabilityLineRow::STATIC, b"a", 0, 0, b""),
        line(1, 5, 8, CountryStabilityLineRow::IDEA, b"a", 0, 1_000, b""),
    ]
}

fn spelled(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(|| "?".to_owned(), |bytes| String::from_utf8_lossy(bytes).into_owned())
}

fn label(source: &Source) -> String {
    let naming = &source.naming;
    let template = match &naming.template {
        None => "?".to_owned(),
        Some(None) => "-".to_owned(),
        Some(Some(template)) => {
            format!("{}@{}:{}:{}", spelled(template.tag.as_deref()), String::from_utf8_lossy(&template.file), template.line, template.generated)
        }
    };
    format!(
        "{}:{}:{}:{}:{}:{}:{template}",
        source.kind,
        String::from_utf8_lossy(&source.identity),
        source.identity_id,
        source.secondary,
        spelled(naming.script.as_deref()),
        spelled(naming.saved.as_deref())
    )
}

fn labelled(parts: Option<Vec<(Source, i64)>>) -> Option<Vec<(String, i64)>> {
    parts.map(|parts| parts.iter().map(|(source, value)| (label(source), *value)).collect())
}

fn adviser(id: u32) -> Naming {
    match id {
        7 => Naming {
            script: Some(b"ADV_CHAR".as_slice().into()),
            saved: Some(b"Adviser Name".as_slice().into()),
            template: Some(Some(Template { tag: Some(b"TPL".as_slice().into()), file: b"x.txt".as_slice().into(), line: 12, generated: 0 })),
        },
        _ => Naming::unknown(),
    }
}

#[test]
fn a_reads_parts_add_up_to_its_displayed_value_one_part_per_source() {
    let row = inputs_row(1, 30_000, Some(25_000));
    let stability = labelled(parts(&row, STABILITY, &lines(), &mut adviser)).unwrap();
    let expected = [
        ("12::0:0:::-", 50_000),
        ("9:base:0:0:::-", 30_000),
        ("1:a:0:0:::-", 15_000),
        ("4:adv:7:0:ADV_CHAR:Adviser Name:TPL@x.txt:12:0", 4_000),
        ("0:Mystery:0:0:::-", 3_000),
        ("10::0:0:::-", 3_000),
        ("11::0:0:::-", -5_000),
    ];
    assert_eq!(stability, expected.map(|(label, value)| (label.to_owned(), value)));
    assert_eq!(stability[1..].iter().map(|(_, value)| value).sum::<i64>(), stability[0].1, "the parts add up to the total");
    let war_support = labelled(parts(&row, WAR_SUPPORT, &lines(), &mut adviser)).unwrap();
    assert_eq!(war_support, [("12::0:0:::-".to_owned(), 20_000), ("9:base:0:0:::-".to_owned(), 20_000)], "lines of another key are not its parts");
    let unnamed = labelled(parts(&row, STABILITY, &lines(), &mut |_| Naming::unknown())).unwrap();
    assert_eq!(unnamed[3], ("4:adv:7:0:?:?:?".to_owned(), 4_000), "an unknown character is not named, never guessed");
}

#[test]
fn a_measure_whose_term_is_not_recorded_has_no_parts_never_zero() {
    let row = inputs_row(1, 30_000, None);
    assert_eq!(parts(&row, STABILITY, &lines(), &mut adviser), None);
    assert!(parts(&row, WAR_SUPPORT, &lines(), &mut adviser).is_some(), "the other measure stands on its own terms");
}

#[test]
fn unpaired_labels_shown_alike_are_one_part_and_one_plain_text_cannot_show_keeps_its_bytes() {
    let row = inputs_row(1, 30_000, Some(6_000));
    let lines = [
        line(1, 0, 0, CountryStabilityLineRow::UNPAIRED, b"", 0, 1_000, b"\x11GMystery\x11!"),
        line(1, 1, 0, CountryStabilityLineRow::UNPAIRED, b"", 0, 2_000, b" Mystery"),
        line(1, 2, 0, CountryStabilityLineRow::UNPAIRED, b"", 0, 3_000, b"Odd\x14"),
    ];
    let parts = labelled(parts(&row, STABILITY, &lines, &mut adviser)).unwrap();
    assert_eq!(parts[2..], [("0:Mystery:0:0:::-".to_owned(), 3_000), ("13:Odd\u{14}:0:0:::-".to_owned(), 3_000)]);
}

const TEXTS: &[&[u8]] =&[b"", b"not read", b"a", b"adv", b"Mystery", b"Adviser Name", b"ADV_CHAR", b"Renamed", b"TPL", b"x.txt"];

const TABLES: [TableId; 6] = [
    TableId::Countries,
    TableId::CountryStabilityInputs,
    TableId::CountryStabilityLines,
    TableId::SliceReads,
    TableId::Characters,
    TableId::LexicalTokens,
];

fn read(store: &mut Store, table: TableId, owner: u32, hour: i32) {
    let row = SliceReadRow { table: table.index() as u32, owner, sample_hour: hour };
    testing::put(store, TableId::SliceReads, &row, TEXTS).unwrap();
}

fn character(store: &mut Store, token: &[u8]) {
    let row = CharacterRow {
        id: 7,
        country: 1,
        name_token_id: 3,
        token: token.to_vec(),
        template_present: 1,
        template_tag: 4,
        template_file: b"x.txt".to_vec(),
        template_line: 12,
        ..Default::default()
    };
    testing::put(store, TableId::Characters, &row, TEXTS).unwrap();
}

fn world(store: &mut Store, hour: i32) {
    for table in TABLES {
        store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    }
    for (id, slot) in [(1, 10), (2, 20)] {
        testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
        read(store, TableId::CountryStabilityInputs, id, hour);
    }
    testing::put(store, TableId::CountryStabilityInputs, &inputs_row(1, 30_000, Some(25_000)), TEXTS).unwrap();
    testing::put(store, TableId::CountryStabilityInputs, &inputs_row(2, 40_000, Some(0)), TEXTS).unwrap();
    for row in lines() {
        testing::put(store, TableId::CountryStabilityLines, &row, TEXTS).unwrap();
    }
    read(store, TableId::CountryStabilityLines, 1, hour);
    character(store, b"Adviser Name");
    testing::put(store, TableId::LexicalTokens, &LexicalTokenRow { token: 3, literal: b"ADV_CHAR".to_vec() }, TEXTS).unwrap();
    testing::put(store, TableId::LexicalTokens, &LexicalTokenRow { token: 4, literal: b"TPL".to_vec() }, TEXTS).unwrap();
}

fn edit(store: &mut Store, hour: i32) {
    match hour {
        0 => world(store, hour),
        1 => {
            testing::put(store, TableId::CountryStabilityInputs, &inputs_row(1, 32_000, Some(25_000)), TEXTS).unwrap();
            read(store, TableId::CountryStabilityInputs, 1, hour);
            read(store, TableId::CountryStabilityLines, 1, hour);
        }
        2 => character(store, b"Renamed"),
        3 => {
            testing::remove(store, TableId::CountryStabilityLines, 1 << 64 | 3 << 32).unwrap();
            read(store, TableId::CountryStabilityInputs, 1, hour);
            read(store, TableId::CountryStabilityLines, 1, hour);
        }
        4 => {
            read(store, TableId::CountryStabilityInputs, 2, hour);
            read(store, TableId::CountryStabilityLines, 2, hour - 1);
        }
        5 => testing::remove(store, TableId::Countries, 2).unwrap(),
        6 => store.status(TableId::CountryStabilityLines.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap(),
        7 => store.status(TableId::CountryStabilityLines.index(), Status::Complete, true, Some(hour), false).unwrap(),
        _ => {}
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    edit(store, hour);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(StabilityParts::default())]);
    writers.start_run();
    writers
}

fn named(store: &Store, country: u32, measure: u8) -> BTreeMap<String, Option<i64>> {
    let keys = store.view(TableId::StabilityPartKeys.index());
    let sources: HashMap<u32, String> = keys
        .rows()
        .map(|slot| {
            let (handle, source) = dictionary_row(&keys, slot).unwrap();
            (handle, label(&source))
        })
        .collect();
    let parts = store.view(TableId::StabilityParts.index());
    parts
        .rows()
        .filter_map(|slot| {
            let key = parts.key(slot)?;
            ((key >> 40) as u32 == country && (key >> 32) as u8 == measure).then(|| (sources[&(key as u32)].clone(), parts.number(0, slot)))
        })
        .collect()
}

fn expect(parts: &[(&str, Option<i64>)]) -> BTreeMap<String, Option<i64>> {
    parts.iter().map(|(label, value)| ((*label).to_owned(), *value)).collect()
}

#[test]
fn parts_move_at_reads_alone_and_unknown_or_gone_countries_hold_no_parts() {
    let (mut store, mut writers) = (testing::store(), writers());
    run(&mut store, &mut writers, 0);
    assert_eq!(named(&store, 2, STABILITY), expect(&[("12::0:0:::-", Some(40_000)), ("9:base:0:0:::-", Some(40_000))]));
    run(&mut store, &mut writers, 1);
    let read = expect(&[
        ("12::0:0:::-", Some(50_000)),
        ("9:base:0:0:::-", Some(32_000)),
        ("1:a:0:0:::-", Some(15_000)),
        ("4:adv:7:0:ADV_CHAR:Adviser Name:TPL@x.txt:12:0", Some(4_000)),
        ("0:Mystery:0:0:::-", Some(3_000)),
        ("10::0:0:::-", Some(3_000)),
        ("11::0:0:::-", Some(-7_000)),
    ]);
    assert_eq!(named(&store, 1, STABILITY), read);
    run(&mut store, &mut writers, 2);
    assert_eq!(named(&store, 1, STABILITY), read, "a character renamed between reads changes no part");
    run(&mut store, &mut writers, 3);
    let reread = expect(&[
        ("12::0:0:::-", Some(50_000)),
        ("9:base:0:0:::-", Some(32_000)),
        ("1:a:0:0:::-", Some(10_000)),
        ("4:adv:7:0:ADV_CHAR:Renamed:TPL@x.txt:12:0", Some(4_000)),
        ("0:Mystery:0:0:::-", Some(3_000)),
        ("10::0:0:::-", Some(8_000)),
        ("11::0:0:::-", Some(-7_000)),
    ]);
    assert_eq!(named(&store, 1, STABILITY), reread, "the next read names the character as it is, a part of its own");
    assert_eq!(store.view(TableId::StabilityPartKeys.index()).rows().count(), 8, "the dictionary keeps the old name's source");
    run(&mut store, &mut writers, 4);
    for measure in MEASURES {
        assert_eq!(named(&store, 2, measure), expect(&[("12::0:0:::-", None)]), "lines read apart from the inputs make the parts unknown");
    }
    run(&mut store, &mut writers, 5);
    assert!(named(&store, 2, STABILITY).is_empty() && named(&store, 2, WAR_SUPPORT).is_empty(), "a gone country holds no parts");
    run(&mut store, &mut writers, 6);
    for table in [TableId::StabilityParts, TableId::StabilityPartKeys] {
        assert!(matches!(store.view(table.index()).status(), Status::Unavailable { .. }), "{table:?} without its lines");
    }
    run(&mut store, &mut writers, 7);
    assert_eq!(store.view(TableId::StabilityParts.index()).status(), Status::Complete);
    assert_eq!(named(&store, 1, STABILITY), reread, "lines back, every country is read again as it stands");
}

#[test]
fn an_hour_recomputes_only_the_countries_its_changes_reach() {
    let mut store = testing::store();
    let mut writer = StabilityParts::default();
    let mut owners = Vec::new();
    for at in 0..=2 {
        store.begin_hour(at, if at == 0 { TEXTS } else { &[] }).unwrap();
        edit(&mut store, at);
        store.settle();
        for table in TABLES.into_iter().chain([TableId::StabilityParts, TableId::StabilityPartKeys]) {
            store.index(table.index());
        }
        let keys = testing::logging(&store, writer.tables(), |rows| {
            writer.hour(&Hour::new(&store, 1, at, at == 0, at == 0, &[]), rows)?;
            Ok(rows.keys(TableId::StabilityParts).0)
        }).unwrap();
        let mut set: Vec<u32> = keys.iter().map(|key| (key >> 40) as u32).collect();
        set.dedup();
        owners.push(set);
        store.end_hour(&mut Discard).unwrap();
    }
    assert_eq!(owners, [vec![1, 2], vec![1], vec![]], "country 1's read reaches it alone; a renamed character reaches no country");
}

#[test]
fn a_read_dated_anew_with_nothing_changed_sets_no_part() {
    let edit = |store: &mut Store, at: i32| {
        if at == 2 {
            for id in [1, 2] {
                read(store, TableId::CountryStabilityInputs, id, at);
            }
            read(store, TableId::CountryStabilityLines, 1, at);
        } else {
            edit(store, at);
        }
    };
    let mut store = testing::store();
    let mut writer = StabilityParts::default();
    for at in 0..=2 {
        store.begin_hour(at, if at == 0 { TEXTS } else { &[] }).unwrap();
        edit(&mut store, at);
        store.settle();
        for table in TABLES.into_iter().chain([TableId::StabilityParts, TableId::StabilityPartKeys]) {
            store.index(table.index());
        }
        testing::logging(&store, writer.tables(), |rows| {
            let hour = Hour::new(&store, 1, at, at == 0, at == 0, &[]);
            if at == 2 {
                assert!(writer.reread_alike(&hour, 2), "country 2 has no line");
                assert!(!writer.reread_alike(&hour, 1), "country 1's adviser line names a character");
            }
            writer.hour(&hour, rows)
        }).unwrap();
        store.end_hour(&mut Discard).unwrap();
    }
    let (mut store, mut writers) = (testing::store(), Writers::new(vec![Box::new(testing::noting::Noting(StabilityParts::default()))]));
    writers.start_run();
    testing::noting::take();
    for at in 0..=2 {
        store.begin_hour(at, if at == 0 { TEXTS } else { &[] }).unwrap();
        edit(&mut store, at);
        store.settle();
        writers.hour(1, at, at == 0, &mut store).unwrap();
        store.end_hour(&mut Discard).unwrap();
    }
    let noted = testing::noting::take();
    assert!(noted[2].iter().all(|(table, ..)| *table != TableId::StabilityParts), "{noted:?}");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    for at in 0..=7 {
        run(&mut unbroken, &mut writing, at);
    }
    let (mut stopped, mut before) = (testing::store(), writers());
    for at in 0..=2 {
        run(&mut stopped, &mut before, at);
    }
    let saved = before.save().unwrap();
    let mut resumed = testing::resumed(&stopped, TEXTS);
    let mut after = writers();
    after.load(&saved, &mut resumed).unwrap();
    after.prime(&resumed, 1, 2);
    for at in 3..=7 {
        run(&mut resumed, &mut after, at);
    }
    for table in [TableId::StabilityParts, TableId::StabilityPartKeys] {
        assert_eq!(testing::rows(&resumed, table), testing::rows(&unbroken, table), "{table:?}");
    }
}

#[test]
fn its_state_is_its_tables_so_a_resume_reads_them_back() {
    let mut writer = StabilityParts::default();
    let mut blob = Bytes::new();
    writer.save(&mut blob).unwrap();
    assert!(blob.bytes().is_empty());
    writer.load(&[]).unwrap();
    assert!(writer.rebuild);
    assert!(writer.load(&[1]).is_err());

    let (mut saved, mut before) = (testing::store(), writers());
    run(&mut saved, &mut before, 0);
    let mut store = testing::resumed_holding(&saved);
    writers().load(&before.save().unwrap(), &mut store).unwrap();
    let removed: Vec<_> = testing::rows(&store, TableId::StabilityParts)
        .into_iter()
        .filter_map(|(key, _)| (key >> 40 == 2).then_some(key))
        .collect();
    assert!(!removed.is_empty(), "the save holds the departed country's parts");
    store.begin_hour(1, &[]).unwrap();
    testing::remove(&mut store, TableId::Countries, 2).unwrap();
    store.settle();
    let (deletes, new_handles) = testing::logging(&store, writer.tables(), |rows| {
        writer.hour(&Hour::new(&store, 1, 1, false, false, &[]), rows)?;
        Ok((rows.keys(TableId::StabilityParts).1, rows.keys(TableId::StabilityPartKeys).0))
    }).unwrap();
    assert_eq!(deletes, removed, "the rebuild deletes every saved part for the departed country");
    assert!(new_handles.is_empty(), "the rebuild reuses the saved dictionary");
    store.end_hour(&mut Discard).unwrap();
}
