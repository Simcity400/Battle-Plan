use super::*;
use crate::playhead_host::views::tests::{Put, hourly_steps, open, record_with, record_with_settings, write_rows};
use replay_columnar::{state::Cell, unit::Settings};

#[test]
fn a_bad_key_is_refused_on_restore_and_on_advance() {
    let good = 1u128 << 32;
    let damaged = 1u128 << 72;
    let units = record_with("wide variable key", 2, |store, at| {
        store.status(TABLE.index(), Status::Complete, true, Some(at as Hour), true).unwrap();
        if at == 0 {
            write_rows(store, TABLE, &[(good, vec![Put::Cell(Cell::I64(4))])]);
        } else {
            write_rows(store, TABLE, &[(good, vec![Put::Cell(Cell::I64(4))]), (damaged, vec![Put::Cell(Cell::I64(5))])]);
        }
    });
    let entry = open(units, hourly_steps()).unwrap();
    let source = ScriptValues::new(&entry, TableWritten::Written);
    let (decoded, _) = source.snapshot(1, 1, None, &mut Vec::new());
    let TableView::Unavailable { reason } = decoded.held() else { panic!("the decoder accepted the wide key") };
    assert!(reason.contains("row key exceeds native width"));
    let refused = |key| matches!(source.read(1, Some(key)), KeyedNumber::Unavailable(why) if why.contains("ScriptVariableValues") && why.contains("row key exceeds native width"));
    assert!(refused(good));
    assert_eq!(source.read(0, Some(good)), KeyedNumber::Value(Some(4)));
    assert!(refused(good));
}

#[test]
fn reading_forward_drops_each_passed_part() {
    let units = record_with_settings("variable column parts", 64, Settings { cap: 64, ..Settings::at(3) }, |store, at| {
        store.status(TABLE.index(), Status::Complete, true, Some(at as Hour), true).unwrap();
        write_rows(store, TABLE, &(0..32).map(|key| (key, vec![Put::Cell(Cell::I64((at as i64) * 1237 + key as i64))])).collect::<Vec<_>>());
    });
    let entry = open(units, hourly_steps()).unwrap();
    let block = entry.pages().table_entry(0, 0, TABLE.index()).unwrap();
    let through = Hour::from(block.parts[1].first);
    let source = ScriptValues::new(&entry, TableWritten::Written);
    assert_eq!(source.read(0, Some(0)), KeyedNumber::Value(Some(0)));
    assert_eq!(source.read(through, Some(0)), KeyedNumber::Value(Some(i64::from(through) * 1237)));
    let mut retired = Vec::new();
    lock(&source.lane).as_mut().unwrap().as_mut().unwrap().retire_into(&mut retired);
    assert!(retired.is_empty(), "the source kept passed parts after a completed lookup");
}
