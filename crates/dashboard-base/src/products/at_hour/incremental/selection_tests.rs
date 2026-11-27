use super::*;
use crate::test_fixtures::{CAMPAIGN, played, records, source};
use dashboard_follow::panel::{AtHour as Panel, ProductRead};
use replay::campaign::Replay;
use replay_data::rows::{CountryRow, tables};

#[test]
fn a_changed_selection_has_the_same_state_as_a_whole_read() {
    compare_changed_selection_with_whole_read(false);
    compare_changed_selection_with_whole_read(true);
}

fn compare_changed_selection_with_whole_read(with_validity: bool) {
    const FIRST: i32 = 100;
    const CHECKPOINT: i32 = FIRST + replay_columnar::unit::BLOCK as i32 - 1;
    const LAST: i32 = CHECKPOINT + 3;
    let table = TableId::Countries;
    let frames = (FIRST..=LAST).map(|hour| {
        let mut frame = source(1, hour, Some(1));
        let mut rows = frame.complete::<tables::Countries>().unwrap().to_vec();
        rows.extend((9002..=9008).map(|id| CountryRow {
            id, canonical_slot: if id == 9003 && hour >= FIRST + 6 { id + 100 } else { id },
            tag: format!("X{:02}", id - 9000).into_bytes(),
            political_power_raw: Some(match id {
                9003 if (FIRST + 6..=FIRST + 7).contains(&hour) => i64::from(FIRST + 5),
                9003 => i64::from(hour),
                _ => i64::from(id),
            }),
            ..Default::default()
        }));
        *frame.tables[table.index()] = match hour {
            h if h == CHECKPOINT + 1 => TableView::Unavailable { reason: "country sample withheld".into() },
            h if h == CHECKPOINT + 2 => TableView::Truncated { rows: TableRows::Countries(rows), missing: 4 },
            _ => TableView::Complete(TableRows::Countries(rows)),
        };
        frame.metrics.table_ns[table.index()] = 1;
        frame
    }).collect();
    let replay = Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap());
    let source = Source::from_replay(crate::SourceKey { source_id: "selection".into(), revision: "1".into() }, "selection.battleplan".into(), replay.clone(), &false.into()).unwrap();
    let pool = Pool::of(&source).unwrap();
    let run = pool.pages.run_of(0).unwrap();
    let mode = capture_mode(&pool.pages).unwrap();
    let fields = field_columns(table, &["political_power_raw"]);
    let cold = if with_validity { field_columns(table, &["political_power_raw", "canonical_slot"]) } else { fields.clone() };
    let state = Mutex::new(([(Projections::new(), not_selected()), (Projections::new(), not_selected())], FIRST));
    let advance: ProductReader = Arc::new(move |hour, _, _, step| {
        let keys = match hour {
            h if h == FIRST + 1 => Keys::These(vec![9002, 9003, 9004, 9005, 9007]),
            h if h == FIRST + 2 => Keys::These(vec![9003, 9004, 9005, 9007, 9999]),
            h if h == FIRST + 3 => Keys::These(vec![]),
            h if h == FIRST + 7 => Keys::These(vec![9002, 9003, 9004, 9005, 9007]),
            h if h == CHECKPOINT => Keys::Ranges(vec![(9002, 9004), (9006, 9008)]),
            h if h == CHECKPOINT + 1 => Keys::Range(9004, 9007),
            h if h == CHECKPOINT + 2 => Keys::These(vec![9001, 9002]),
            _ => Keys::Range(9002, 9006),
        };
        let read = Read { table, keys };
        let mut state = state.lock().unwrap();
        let (paths, previous_from) = &mut *state;
        let outputs = std::array::from_fn::<_, 2, _>(|i| {
            let (projections, view) = &mut paths[i];
            if i == 1 && let Some(Ok(projection)) = projections.get_mut(&table)
                && projection.selection.as_deref() != Some(&read.keys) {
                projection.at = None;
            }
            let mut rows = 0;
            let mut edits = Edits::default();
            let mut input = Input { pool: &pool, run, hour, mode, count: &mut rows, step, placed: None, gap: false, exact: false };
            input.read(projections, &read, view, || (Some(fields.clone()), Some(cold.clone())), true, &mut edits).unwrap();
            assert_eq!(projections[&table].as_ref().unwrap().validity != 0, with_validity);
            let from = if input.gap || !edits.tables.is_empty() || !edits.replaced.is_empty() { hour } else { *previous_from };
            let at = projections[&table].as_ref().ok().and_then(|p| p.at);
            (from, at, view.sampled_at(), view.keys().clone(), view.held().clone(), edits.tables, edits.replaced, rows)
        });
        let [mut updated, mut rebuilt] = outputs;
        let rows = std::mem::take(&mut updated.7);
        let whole_rows = std::mem::take(&mut rebuilt.7);
        assert_eq!(updated, rebuilt, "projection state at hour {hour}");
        if !with_validity && (hour == FIRST + 1 || hour == CHECKPOINT) {
            assert!(rows < whole_rows, "hour {hour} copies {rows} compared with {whole_rows}");
        }
        if with_validity || (FIRST + 2..=FIRST + 3).contains(&hour) || (CHECKPOINT + 1..=LAST).contains(&hour) {
            assert_eq!(rows, whole_rows, "hour {hour}");
        }
        *previous_from = updated.0;
        ProductRead { value: Arc::new(Ok(Value::Null)), rows, fresh: true, pending: false, live: None,
            lanes: paths[0].0.iter().filter_map(declared).collect(), from: updated.0 }
    });
    let panel = Panel { read: Arc::new(|_, _| Ok(Value::Null)), cold: Arc::new(|_, _| Ok(Value::Null)), advance };
    let played = played(&replay, &panel, FIRST, LAST);
    assert!(played.iter().any(|read| read.hour == CHECKPOINT));
}
