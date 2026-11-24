use crate::rows::TableRows;
use crate::sliced::SliceUpdate;
use crate::{
    frame::{FrameEncoder, FrameInput, Metrics, TableInput, TableView},
    rows::{ResearchInputRow, TableId},
};
use std::sync::Arc;

fn row(country: u32, index: u32, progress: i64) -> ResearchInputRow {
    ResearchInputRow {
        country,
        index,
        progress_raw: progress,
        ..Default::default()
    }
}

fn frame(hour: i32, input: TableInput) -> FrameInput {
    let mut frame = FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture table".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    frame.tables[TableId::ResearchInputs.index()] = input;
    frame
}

type HourRead = (Vec<ResearchInputRow>, fn(u32) -> bool, fn(u32) -> bool);

fn hours() -> Vec<HourRead> {
    vec![
        (
            vec![row(1, 0, 0), row(1, 1, 0)],
            |owner| owner == 1,
            |_| true,
        ),
        (vec![row(2, 0, 5)], |owner| owner == 2, |_| true),
        (
            vec![row(1, 0, 6), row(1, 1, 0), row(1, 2, 1)],
            |owner| owner == 1 || owner == 3,
            |_| true,
        ),
        (vec![row(4, 0, 2)], |owner| owner == 4, |owner| owner != 25),
        (vec![], |_| false, |_| true),
        (
            vec![row(1, 0, 7), row(1, 1, 1), row(1, 2, 1), row(2, 0, 6)],
            |owner| owner <= 2,
            |_| true,
        ),
    ]
}

#[test]
fn slice_updates_encode_and_commit_the_merged_complete_table() {
    let start = TableRows::ResearchInputs(vec![
        row(1, 0, 0),
        row(1, 1, 0),
        row(2, 0, 0),
        row(3, 0, 0),
        row(25, 0, 0),
    ]);
    let mut merged_encoder = FrameEncoder::new();
    let mut update_encoder = FrameEncoder::new();
    let mut held = Arc::new(start);
    let first = frame(0, TableInput::SharedComplete(Arc::clone(&held)));
    assert_eq!(
        merged_encoder
            .encode_bytes(first.clone(), false)
            .unwrap()
            .bytes,
        update_encoder.encode_bytes(first, false).unwrap().bytes
    );
    for (step, (fresh, read, live)) in hours().into_iter().enumerate() {
        let hour = step as i32 + 1;
        let fresh = TableRows::ResearchInputs(fresh);
        let merged =
            Arc::new(crate::sliced::merge_slice(&held, fresh.clone(), read, live).unwrap());
        let update = SliceUpdate::new(&held, fresh, read, live).unwrap();
        assert_eq!(update.merged(&held).unwrap(), *merged);
        let expected = merged_encoder
            .encode_bytes(
                frame(hour, TableInput::SharedComplete(Arc::clone(&merged))),
                false,
            )
            .unwrap();
        let (token, baseline) = update_encoder
            .slice_baseline(TableId::ResearchInputs, 1, hour, false)
            .unwrap();
        assert!(Arc::ptr_eq(baseline, &held));
        let before = Arc::as_ptr(&held);
        drop(held);
        let input = TableInput::SliceUpdate {
            token,
            update: Arc::new(update),
        };
        let actual = update_encoder
            .encode_bytes(frame(hour, input), false)
            .unwrap();
        assert_eq!(actual.bytes, expected.bytes, "hour {hour}");
        held = Arc::clone(
            update_encoder
                .shared_baseline(TableId::ResearchInputs)
                .unwrap(),
        );
        assert_eq!(*held, *merged, "hour {hour}");
        assert_eq!(
            Arc::as_ptr(&held),
            before,
            "hour {hour}: copied the baseline"
        );
    }
}

#[test]
fn a_shared_or_stale_baseline_and_a_foreign_row_are_refused_or_copied() {
    let start = Arc::new(TableRows::ResearchInputs(vec![row(1, 0, 0), row(2, 0, 0)]));
    let mut encoder = FrameEncoder::new();
    encoder
        .encode_bytes(
            frame(0, TableInput::SharedComplete(Arc::clone(&start))),
            false,
        )
        .unwrap();
    let update = SliceUpdate {
        ranges: Vec::new(),
        keys: Vec::new(),
        owners: vec![1],
        rows: TableRows::ResearchInputs(vec![row(2, 0, 9)]),
    };
    assert!(!update.fits(&start));
    let (token, _) = encoder
        .slice_baseline(TableId::ResearchInputs, 1, 1, false)
        .unwrap();
    let refused = TableInput::SliceUpdate {
        token,
        update: Arc::new(update),
    };
    assert!(encoder.prepare_bytes(frame(1, refused), false).is_err());
    assert!(
        encoder
            .slice_baseline(TableId::ResearchInputs, 1, 2, false)
            .is_none()
    );
    assert!(
        encoder
            .slice_baseline(TableId::ResearchInputs, 2, 1, false)
            .is_none()
    );
    assert!(
        encoder
            .slice_baseline(TableId::ResearchInputs, 1, 1, true)
            .is_none()
    );
    let update = SliceUpdate::new(
        &start,
        TableRows::ResearchInputs(vec![row(1, 0, 3)]),
        |o| o == 1,
        |_| true,
    )
    .unwrap();
    let input = TableInput::SliceUpdate {
        token,
        update: Arc::new(update),
    };
    encoder.encode_bytes(frame(1, input), false).unwrap();
    let committed = encoder.shared_baseline(TableId::ResearchInputs).unwrap();
    assert!(!Arc::ptr_eq(committed, &start));
    assert_eq!(
        **committed,
        TableRows::ResearchInputs(vec![row(1, 0, 3), row(2, 0, 0)])
    );
    assert_eq!(
        *start,
        TableRows::ResearchInputs(vec![row(1, 0, 0), row(2, 0, 0)])
    );
}

#[test]
fn certified_key_ranges_encode_insert_edit_and_delete_exactly_like_complete_history() {
    use crate::rows::{DivisionHistoryRow, RowCodec};
    let row = |division, ordinal, amount| {
        Arc::new(DivisionHistoryRow {
            division,
            list: 1,
            ordinal,
            available: 1,
            amount,
            ..Default::default()
        })
    };
    let mut current = vec![row(1, 0, 2), row(1, 1, 3), row(2, 0, 4)];
    let input = |hour, table| {
        let mut frame = frame(
            hour,
            TableView::Unavailable {
                reason: "unused".into(),
            }
            .into(),
        );
        frame.tables[TableId::DivisionHistory.index()] = table;
        frame
    };
    let mut sparse = FrameEncoder::new();
    let mut full = FrameEncoder::new();
    let start = input(
        0,
        TableInput::SharedComplete(Arc::new(TableRows::DivisionHistory(current.clone()))),
    );
    assert_eq!(
        sparse.encode_input(start.clone(), false).unwrap(),
        full.encode_input(start, false).unwrap()
    );
    for hour in 1..=3 {
        let (key, replacement) = match hour {
            1 => {
                let new = row(1, 2, 5);
                (new.key(), Some(new))
            }
            2 => {
                let new = row(1, 1, 9);
                (new.key(), Some(new))
            }
            _ => (row(2, 0, 0).key(), None),
        };
        current.retain(|row| row.key() != key);
        if let Some(new) = &replacement {
            current.push(new.clone());
        }
        current.sort_by_key(RowCodec::key);
        let (token, _) = sparse
            .slice_baseline(TableId::DivisionHistory, 1, hour, false)
            .unwrap();
        #[expect(
            clippy::single_range_in_vec_init,
            reason = "This is one certified key interval, not a vector of the individual keys."
        )]
        let update = SliceUpdate {
            owners: Vec::new(),
            ranges: vec![key..key + 1],
            keys: Vec::new(),
            rows: TableRows::DivisionHistory(replacement.into_iter().collect()),
        };
        let actual = sparse
            .encode_input(
                input(
                    hour,
                    TableInput::SliceUpdate {
                        token,
                        update: Arc::new(update),
                    },
                ),
                false,
            )
            .unwrap();
        let expected = full
            .encode_input(
                input(
                    hour,
                    TableInput::SharedComplete(Arc::new(TableRows::DivisionHistory(
                        current.clone(),
                    ))),
                ),
                false,
            )
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            sparse
                .committed::<crate::rows::tables::DivisionHistory>()
                .unwrap()
                .1,
            &current
        );
    }
}

#[test]
fn exact_key_updates_match_full_frames_for_changes_additions_and_removals() {
    let mut baseline =
        TableRows::ResearchInputs((0..5000).map(|index| row(5000, index, 0)).collect());
    let mut full = FrameEncoder::new();
    let mut keyed = FrameEncoder::new();
    let first = frame(0, TableView::Complete(baseline.clone()).into());
    let shared = frame(0, TableInput::SharedComplete(Arc::new(baseline.clone())));
    assert_eq!(
        full.encode_bytes(first, false).unwrap().bytes,
        keyed.encode_bytes(shared, false).unwrap().bytes
    );
    for (hour, key, fresh) in [
        (1, row(5000, 17, 0).key(), vec![row(5000, 17, 77)]),
        (2, row(5000, 5000, 0).key(), vec![row(5000, 5000, 1)]),
        (3, row(5000, 18, 0).key(), vec![]),
        (4, row(5000, 18, 0).key(), vec![]),
    ] {
        let update = SliceUpdate::keyed(vec![key], TableRows::ResearchInputs(fresh));
        assert!(update.fits(&baseline));
        update.clone().apply(&mut baseline).unwrap();
        let expected = full
            .encode_bytes(
                frame(hour, TableView::Complete(baseline.clone()).into()),
                false,
            )
            .unwrap();
        let (token, _) = keyed
            .slice_baseline(TableId::ResearchInputs, 1, hour, false)
            .expect("exact key updates retain their shared baseline");
        let actual = keyed
            .encode_bytes(
                frame(
                    hour,
                    TableInput::SliceUpdate {
                        token,
                        update: Arc::new(update),
                    },
                ),
                false,
            )
            .unwrap();
        assert_eq!(actual.bytes, expected.bytes);
        assert_eq!(
            **keyed.shared_baseline(TableId::ResearchInputs).unwrap(),
            baseline
        );
    }
    let key = row(5000, 17, 0).key();
    for keys in [vec![key, key], vec![key + 1, key]] {
        let update = SliceUpdate::keyed(keys, TableRows::ResearchInputs(vec![row(5000, 17, 9)]));
        assert!(!update.fits(&baseline));
        assert!(
            update
                .encode_changes(&baseline, &mut Vec::new(), &mut Vec::new())
                .is_err()
        );
        assert!(update.apply(&mut baseline).is_err());
    }
    let foreign = SliceUpdate::keyed(vec![key], TableRows::ResearchInputs(vec![row(5000, 19, 9)]));
    assert!(!foreign.fits(&baseline));
}

#[test]
fn script_variable_key_updates_match_full_frames() {
    use crate::rows::{Recorded, ScriptVariableEntry, ScriptVariablesRow};
    let scope = |id: u32, names: &[&str]| ScriptVariablesRow {
        scope_kind: 1,
        scope_id: id,
        entries: names
            .iter()
            .map(|name| ScriptVariableEntry {
                name: name.as_bytes().to_vec(),
                value_raw: i64::from(id),
            })
            .collect(),
        seed: Recorded::Value(Some(id)),
    };
    let frame = |hour: i32, input: TableInput| {
        let unread = TableView::Unavailable {
            reason: "fixture table".into(),
        };
        let mut frame = frame(hour, unread.into());
        frame.tables[TableId::ScriptVariables.index()] = input;
        frame
    };
    let mut baseline =
        TableRows::ScriptVariables((1..=4).map(|id| scope(id, &["a", "b"])).collect());
    let mut full = FrameEncoder::new();
    let mut keyed = FrameEncoder::new();
    assert_eq!(
        full.encode_bytes(
            frame(0, TableView::Complete(baseline.clone()).into()),
            false
        )
        .unwrap()
        .bytes,
        keyed
            .encode_bytes(
                frame(0, TableInput::SharedComplete(Arc::new(baseline.clone()))),
                false
            )
            .unwrap()
            .bytes
    );
    let key = |id: u32| scope(id, &[]).key();
    let hours = [
        (vec![key(2)], vec![scope(2, &["a", "b", "c"])]),
        (vec![key(9)], vec![scope(9, &["z"])]),
        (vec![key(3)], vec![]),
        (
            vec![key(1), key(3), key(4), key(7)],
            vec![scope(1, &[]), scope(3, &["c"]), scope(7, &["a"])],
        ),
    ];
    for (hour, (keys, rows)) in hours.into_iter().enumerate() {
        let hour = hour as i32 + 1;
        let update = SliceUpdate::keyed(keys, TableRows::ScriptVariables(rows));
        assert!(update.fits(&baseline));
        update.clone().apply(&mut baseline).unwrap();
        let expected = full
            .encode_bytes(
                frame(hour, TableView::Complete(baseline.clone()).into()),
                false,
            )
            .unwrap();
        let (token, _) = keyed
            .slice_baseline(TableId::ScriptVariables, 1, hour, false)
            .expect("keyed updates keep the shared baseline");
        let actual = keyed
            .encode_bytes(
                frame(
                    hour,
                    TableInput::SliceUpdate {
                        token,
                        update: Arc::new(update),
                    },
                ),
                false,
            )
            .unwrap();
        assert_eq!(actual.bytes, expected.bytes, "hour {hour}");
        assert_eq!(
            **keyed.shared_baseline(TableId::ScriptVariables).unwrap(),
            baseline,
            "hour {hour}"
        );
    }
}
