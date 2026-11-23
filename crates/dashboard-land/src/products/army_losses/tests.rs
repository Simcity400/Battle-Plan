use super::*;
use replay_data::rows::Recorded;

fn month(group: u32, kind: u8, date: i32) -> CombatLogMonthRow {
    CombatLogMonthRow { country: 9, group, kind, date, ..CombatLogMonthRow::default() }
}

fn line(group: u32, kind: u8, date: i32, variant: u32, amount: i64) -> CombatLogEquipmentRow {
    CombatLogEquipmentRow { country: 9, group, kind, month: 0, variant, date, amount_raw: amount }
}

fn dropped_line(group: u32, kind: u8, date: i32, variant: u32, amount: i64) -> CombatLogDroppedRow {
    CombatLogDroppedRow { country: 9, group, kind, month: 0, variant, date: Recorded::Value(date), amount: Recorded::Value(amount) }
}

#[test]
fn a_month_reads_its_listed_rows_then_the_rows_the_game_dropped_then_says_why_it_is_unknown() {
    let listed = [line(1, 0, 300, 7, 500), line(1, 0, 200, 7, 900)];
    let kept_rows = [dropped_line(1, 0, 100, 7, 400), dropped_line(1, 0, 101, 8, 1)];
    let kept: Vec<&CombatLogDroppedRow> = kept_rows.iter().collect();
    let dates = [50, 100, 200, 300];
    assert_eq!(witness(&month(1, 0, 300), Some(400), &listed, &kept, &dates), Witness::Listed(vec![&listed[0]]));
    assert_eq!(witness(&month(1, 0, 100), Some(400), &listed, &kept, &dates), Witness::Dropped(vec![&kept_rows[0]]), "a dropped row of another entry of the month does not witness it");
    assert_eq!(witness(&month(1, 0, 50), Some(400), &listed, &kept, &dates), Witness::Unknown(UNWITNESSED), "an older month nothing kept is unknown, never empty");
    assert_eq!(witness(&month(1, 0, 450), Some(400), &listed, &kept, &[450]), Witness::Unknown(AFTER_READ));
    assert_eq!(witness(&month(1, 0, 300), None, &[], &kept, &dates), Witness::Unknown(NO_READ_YET));
    assert_eq!(witness(&month(2, 0, 300), Some(400), &listed, &kept, &[200, 300]), Witness::Empty, "a month among its array's last two that the read listed nothing of has none");
}

#[test]
fn a_months_witness_reads_its_own_arrays_run_of_the_hours_rows() {
    let mut rows = vec![line(1, 0, 300, 7, 500), line(1, 1, 300, 7, 2), line(2, 0, 300, 7, 3), line(1, 0, 200, 8, 900)];
    rows.sort_by_key(|row| (row.country, row.group, row.kind, row.month, row.variant));
    let other = CombatLogEquipmentRow { country: 10, ..line(1, 0, 300, 7, 4) };
    rows.push(other);
    let run = array_rows(&rows, (9, 1, 0));
    assert_eq!(run.iter().map(|row| (row.variant, row.amount_raw)).collect::<Vec<_>>(), [(7, 500), (8, 900)]);
    assert!(array_rows(&rows, (9, 3, 0)).is_empty());
    assert_eq!(witness(&month(1, 0, 300), Some(400), run, &[], &[200, 300]), Witness::Listed(vec![&run[0]]));
    assert_eq!(witness(&month(1, 0, 300), Some(400), array_rows(&rows, (9, 1, 0)), &[], &[200, 300]), witness(&month(1, 0, 300), Some(400), &rows, &[], &[200, 300]), "the array's run witnesses a month as all the hour's rows do");
}

#[test]
fn a_view_holds_the_measure_drawn_or_every_part_of_one_month() {
    use super::{ArmyLossesParams, LossView};
    let equipment = LossView::Equipment { ledgers: vec![1, 2, 3] };
    assert!(LossView::Manpower.manpower(5) && !LossView::Manpower.ledger(0, 5));
    assert!(!equipment.manpower(5) && equipment.ledger(2, 5) && !equipment.ledger(0, 5) && !equipment.ledger(super::ENEMY_LEDGER, 5));
    let month = LossView::Month { month: 5 };
    assert!(month.manpower(5) && month.ledger(0, 5) && month.ledger(super::ENEMY_LEDGER, 5));
    assert!(!month.manpower(6) && !month.ledger(0, 6));
    let asked = |json: serde_json::Value| serde_json::from_value::<ArmyLossesParams>(json).unwrap().view;
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER" })), LossView::Manpower);
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER", "view": { "kind": "equipment", "ledgers": [1, 2, 3] } })), equipment);
    assert_eq!(asked(serde_json::json!({ "branchId": "1", "country": "GER", "view": { "kind": "month", "month": 5 } })), month);
}
