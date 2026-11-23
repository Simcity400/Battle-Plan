use super::*;
use replay_data::rows::TableRows;
use replay_frame::frame::TableView;

const NONE: u32 = 0x11;
const DOCTRINE: u32 = 0x22;

fn tokens() -> TokenNames<'static> {
    [
        (41, b"synthetic_unit".as_slice()),
        (61, b"synthetic_category".as_slice()),
        (62, b"synthetic_recipient".as_slice()),
        (NONE, b"none".as_slice()),
        (DOCTRINE, b"synthetic_doctrine".as_slice()),
    ]
    .into_iter()
    .collect()
}

fn stat(name: &[u8], value_raw: i64) -> UnitBonusStat {
    UnitBonusStat {
        selector: content_reader_script::equipment_stats::stat_index(name).unwrap() as u32,
        value_raw,
    }
}

fn rows() -> Vec<CountryUnitBonusRow> {
    content_reader_script::test_game::enter();
    vec![
        CountryUnitBonusRow {
            country: 102,
            ordinal: 0,
            source_kind: 2,
            source_id: NONE,
            source_number: -3,
            label: b"named".to_vec(),
            targets: vec![
                RecordedTarget {
                    category: false,
                    token: 41,
                    combat_width_raw: -50_000,
                    stats: vec![stat(b"max_strength", 25_000)],
                    grants: vec![RecordedGrant {
                        category_token: 62,
                        add: 1,
                        display_as_percentage: 0,
                        stats: vec![stat(b"breakthrough", 10_000), stat(b"breakthrough", 0)],
                    }],
                },
                RecordedTarget {
                    category: true,
                    token: 61,
                    combat_width_raw: 0,
                    stats: vec![stat(b"hard_attack", 7)],
                    grants: Vec::new(),
                },
            ],
        },
        CountryUnitBonusRow {
            country: 102,
            ordinal: 1,
            source_kind: 5,
            source_id: DOCTRINE,
            source_number: 1,
            ..Default::default()
        },
    ]
}

fn saved(list: &str) -> Vec<UnitBonus> {
    let text = format!("countries={{GER={{deployment={{unit_modifiers={{{list}}}}}}}}}");
    let document = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
    content_reader_save::production::parse(&document.items)
        .unwrap()
        .unit_bonuses
        .remove(b"GER".as_slice())
        .unwrap()
        .unwrap()
}

const LIST: &str = r#"
    { synthetic_unit={ max_strength=0.25 combat_width=-0.5 need_equipment={ synthetic_archetype=1.5 }
        battalion_mult={ category=synthetic_recipient add=yes display_as_percentage=no breakthrough=0.1 }
        night={ attack=0.1 } synthetic_terrain={ movement=0.2 } priority=2 sprite=synthetic_sprite }
      synthetic_category={ hard_attack=0.00007 }
      type=effect number=-3 localization_key="named" }
    { type=rewards number=1 id=synthetic_doctrine }"#;

fn run(rows: Vec<CountryUnitBonusRow>, list: &str, hour: i32) -> Report {
    content_reader_script::test_game::enter();
    let (mut frame, mut save, _, _) = crate::fixtures::fixture();
    save.production
        .unit_bonuses
        .insert(b"GER".to_vec(), Ok(saved(list)));
    frame.hour = hour;
    *frame.tables[TableId::CountryUnitBonuses.index()] =
        TableView::Complete(TableRows::CountryUnitBonuses(rows));
    let countries = crate::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare(&frame, &save, &countries, &tokens(), &mut report).unwrap();
    report
}

const SLOT: i32 = 24 * 1_000 + 22;

#[test]
fn the_saved_list_matches_its_recorded_entries_targets_grants_and_sources() {
    let report = run(rows(), LIST, SLOT);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryUnitBonuses"], 2);
}

#[test]
fn a_differing_stat_an_extra_entry_and_source_omissions_are_named_differences() {
    let mut changed = rows();
    changed[0].targets[1].stats[0].value_raw = 8;
    let report = run(changed, LIST, SLOT);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("[0].target[1].stats"));

    let mut extra = rows();
    extra.push(CountryUnitBonusRow {
        ordinal: 2,
        ..extra[1].clone()
    });
    let report = run(extra, LIST, SLOT);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("unit_modifiers.entries"));

    let mut sources = rows();
    sources[0].source_id = DOCTRINE;
    sources[0].label.clear();
    let report = run(sources, LIST, SLOT);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.differences[0].contains("[0].id"));
    assert!(report.differences[1].contains("[0].localization_key"));
}

#[test]
fn a_country_without_saved_modifiers_has_no_rows() {
    let report = run(Vec::new(), "", SLOT);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let report = run(rows(), "", SLOT);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("unit_modifiers.entries"));
}

#[test]
fn rows_read_at_another_hour_are_not_compared() {
    let report = run(Vec::new(), LIST, SLOT + 1);
    assert!(report.differences.is_empty());
    assert!(!report.compared_tables.contains_key("CountryUnitBonuses"));
}
