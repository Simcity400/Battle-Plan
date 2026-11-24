use super::{TrainingNeed, equipment_fill};

fn need(archetype: &str, need_raw: i64, held_raw: i64) -> TrainingNeed {
    TrainingNeed { archetype: archetype.to_owned(), name: None, need_raw: need_raw.to_string(), held_raw: held_raw.to_string() }
}

#[test]
fn fill_counts_each_type_up_to_its_need() {
    let needs = [need("infantry_equipment", 1_000, 1_500), need("artillery_equipment", 1_000, 500)];
    assert_eq!(equipment_fill(&needs), Some(75_000));
}

#[test]
fn fill_ignores_types_not_needed_and_negative_holdings() {
    let needs = [need("infantry_equipment", 400, -20), need("support_equipment", 0, 900)];
    assert_eq!(equipment_fill(&needs), Some(0));
}

#[test]
fn fill_is_unknown_when_nothing_is_needed() {
    assert_eq!(equipment_fill(&[need("infantry_equipment", 0, 10)]), None);
    assert_eq!(equipment_fill(&[]), None);
}
