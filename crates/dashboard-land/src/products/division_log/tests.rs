use super::*;
use std::collections::HashMap;

fn names() -> Names {
    Names { slots: HashMap::from([(4, vec![("GER".to_owned(), 0, 500)]), (9, vec![("SOV".to_owned(), 100, 500)])]) }
}

fn cells(values: &[(usize, Cell)]) -> Vec<Cell> {
    let mut cells = vec![Cell::Unrecorded; 26];
    for (column, value) in values {
        cells[*column] = value.clone();
    }
    cells
}

fn shown(mapped: &Mapped, name: &str) -> LogCell {
    mapped.cells[COLUMNS.iter().position(|column| *column == name).unwrap()].clone()
}

fn key(kind: u8, hour: i32, division: u64) -> u128 {
    u128::from(kind) << 96 | u128::from(hour as u32) << 64 | u128::from(division)
}

#[test]
fn a_covered_death_is_exact_at_its_province_with_its_killer_and_a_sighting_is_bounded_by_the_last_census() {
    let death = cells(&[
        (column::DISPOSITION, Cell::Number(0)),
        (column::CAUSE, Cell::Number(13)),
        (column::SLOT, Cell::Number(4)),
        (column::NATIVE_HOUR, Cell::Number(205)),
        (column::EXACT, Cell::Number(1)),
        (column::AFTER, Cell::Missing),
        (column::PROVINCE, Cell::Number(77)),
        (column::PLACE_HOUR, Cell::Number(205)),
        (column::KILLER_SLOT, Cell::Number(9)),
        (column::KILLER_DIVISION, Cell::Number(12_345)),
        (column::TEMPLATE_ID, Cell::Number(31)),
        (column::MATCHED_DEATHS, Cell::Number(1)),
        (column::TEMPLATE_LITERAL, Cell::Text("1. Panzer".into())),
        (column::TEMPLATE_COUNTRY, Cell::Number(4)),
    ]);
    let lost = map(206, key(DivisionEventRow::LOST, 206, 9 << 32 | 3), &death, &names());
    assert_eq!(shown(&lost, "division"), LogCell::Text { value: (9u64 << 32 | 3).to_string() });
    assert_eq!(shown(&lost, "status"), LogCell::Text { value: "division_destroyed".into() });
    assert_eq!(shown(&lost, "hour"), LogCell::Number { value: "205".into() });
    assert_eq!(shown(&lost, "by"), LogCell::Number { value: "0".into() });
    assert_eq!(shown(&lost, "tag"), LogCell::Text { value: "GER".into() });
    assert_eq!(shown(&lost, "killerTag"), LogCell::Text { value: "SOV".into() });
    assert_eq!(shown(&lost, "killerDivision"), LogCell::Text { value: "12345".into() });
    assert_eq!(shown(&lost, "placeAtEvent"), LogCell::Number { value: "1".into() });
    assert_eq!(shown(&lost, "template"), LogCell::Text { value: "31".into() });
    assert_eq!(shown(&lost, "afterHour"), LogCell::Missing);
    assert_eq!(lost.template, Some((b"1. Panzer".to_vec(), None, 4)));
    assert!(!lost.ambiguous);
    assert_eq!(shown(&lost, "nameMissing"), LogCell::Text { value: "Division name not recorded at this observation".into() });

    let sighting = cells(&[(column::SLOT, Cell::Number(i64::from(TagHolderRow::NONE))), (column::NATIVE_HOUR, Cell::Number(90)), (column::EXACT, Cell::Number(0)), (column::AFTER, Cell::Number(80))]);
    let created = map(90, key(DivisionEventRow::CREATED, 90, 5 << 32 | 1), &sighting, &names());
    assert_eq!(shown(&created, "by"), LogCell::Number { value: "1".into() });
    assert_eq!(shown(&created, "afterHour"), LogCell::Number { value: "80".into() });
    assert_eq!(shown(&created, "tag"), LogCell::Missing, "an unresolved owner names no country, never a guess");
    assert_eq!(shown(&created, "placeAtEvent"), LogCell::Number { value: "1".into() });
    assert_eq!(shown(&created, "killerTag"), LogCell::Missing);

    let unmatched = cells(&[(column::CAUSE, Cell::Number(13)), (column::EXACT, Cell::Number(0)), (column::MATCHED_DEATHS, Cell::Number(2)), (column::KILLER_SLOT, Cell::Missing)]);
    let ambiguous = map(300, key(DivisionEventRow::LOST, 300, 7 << 32 | 1), &unmatched, &names());
    assert!(ambiguous.ambiguous);
    assert_eq!(shown(&ambiguous, "placeAtEvent"), LogCell::Number { value: "0".into() }, "a death not dated is placed where the division was last seen");
    assert_eq!(shown(&ambiguous, "killerTag"), LogCell::Missing);
}

fn templates_frame(hour: i32) -> replay_frame::frame::DecodedFrame {
    use replay_data::rows::{DivisionEquipmentDeletionRow, DivisionRow, DivisionTemplateIconRow, PacketHour, TableRows, TemplateDisplayName, TemplateNameValue};
    use replay_frame::frame::TableView;
    let template = |id: u64, literal: &[u8]| {
        let name = TemplateDisplayName { sample_hour: PacketHour::Host, value: TemplateNameValue::Changed { literal: literal.to_vec(), fallback_key: None } };
        DivisionTemplateIconRow { template: id << 32 | 3, country: 9001, display_name: Recorded::Value(Some(name)), ..Default::default() }
    };
    let division = |id: u64, template: u64| DivisionRow { id: id << 32 | 9, roster_owner: 9001, logical_owner: 9001, template: template << 32 | 3, hit_points: 100_000, ..Default::default() };
    let mut divisions = vec![division(1, 1)];
    if (1_100..1_500).contains(&hour) {
        divisions.push(division(2, 2));
    }
    if hour >= 1_900 {
        divisions.push(division(3, 1));
    }
    if hour >= 2_500 {
        divisions.push(division(4, 2));
    }
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] = TableView::Complete(TableRows::DivisionEquipmentDeletions(Vec::<DivisionEquipmentDeletionRow>::new()));
    *frame.tables[TableId::DivisionTemplateIcons.index()] = TableView::Complete(TableRows::DivisionTemplateIcons(vec![template(1, b"1. Infanterie"), template(2, b"2. Panzer")]));
    frame
}

#[test]
fn a_runs_templates_are_named_by_their_first_row_in_the_log() {
    let frames = (1_000..=2_600).map(templates_frame).collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), dashboard_base::test_fixtures::CAMPAIGN.into(), &[dashboard_base::test_fixtures::records(1, frames)]).unwrap();
    let seen = run_templates(replay.recording(), 0).unwrap();
    let named = |id: u32, hour: i32, literal: &[u8]| SeenTemplate { id, hour, literal: literal.to_vec(), key: None, country: Some(9001) };
    assert_eq!(seen, vec![named(1, 1_900, b"1. Infanterie"), named(2, 1_100, b"2. Panzer")]);
}

#[test]
fn hours_not_read_are_the_stretch_less_its_read_runs() {
    assert_eq!(unread(&[(10, 19), (25, 30)], 5, 40), vec![(5, 9), (20, 24), (31, 40)]);
    assert_eq!(unread(&[(0, 50)], 5, 40), Vec::<(i32, i32)>::new());
    assert_eq!(unread(&[], 5, 6), vec![(5, 6)]);
}
