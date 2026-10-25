use super::*;
use crate::rows::{ClockRow, DivisionRow, TableId, TableRows};

fn frame() -> Frame {
    let mut tables = std::array::from_fn(|_| TableUpdate::Unavailable {
        reason: String::new(),
    });
    tables[TableId::Clock.index()] = TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow {
        hour: 3,
        ..Default::default()
    }]));
    tables[TableId::Divisions.index()] = TableUpdate::Snapshot(TableRows::Divisions(
        (1..=128)
            .map(|id| DivisionRow {
                id,
                province: 9,
                ..Default::default()
            })
            .collect(),
    ));
    Frame {
        branch: 1,
        hour: 3,
        keyframe: false,
        metrics: Metrics::default(),
        tables,
    }
}

fn recorded(frame: Frame, columns: bool) -> RecordedFrame {
    if !columns {
        return frame.into();
    }
    let updates: Vec<_> = frame
        .tables
        .into_iter()
        .map(|update| Updates::new(vec![update]).unwrap())
        .collect();
    RecordedFrame::from_columns(
        frame.branch,
        frame.hour,
        frame.keyframe,
        frame.metrics,
        Arc::new(std::array::from_fn(Some)),
        &updates,
        0,
        Arc::new(TableUpdate::Unavailable {
            reason: String::new(),
        }),
    )
    .unwrap()
}

fn bytes(frame: &RecordedFrame) -> usize {
    let mut total = 0;
    frame.allocations(|_, size| total += size);
    total
}

#[test]
fn one_hour_cells_do_not_retain_the_previous_hour_and_transfer_without_copying() {
    let original = frame();
    let cells: Vec<_> = original.tables.iter().cloned().map(Arc::new).collect();
    let weak = Arc::downgrade(&cells[TableId::Divisions.index()]);
    let recorded = RecordedFrame::from_updates(
        original.branch,
        original.hour,
        original.keyframe,
        original.metrics.clone(),
        Arc::new(std::array::from_fn(Some)),
        cells,
        Arc::new(TableUpdate::Unavailable {
            reason: "unselected".into(),
        }),
    )
    .unwrap();
    assert_eq!(weak.strong_count(), 1);
    assert_eq!(recorded.to_owned_frame().unwrap(), original);
    let owned = recorded.into_owned_frame().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(owned, original);
}

#[test]
fn one_hour_slots_reject_duplicate_and_unreachable_cells() {
    let update = Arc::new(TableUpdate::Unavailable {
        reason: "missing".into(),
    });
    let mut slots = [None; TABLE_COUNT];
    slots[0] = Some(0);
    slots[1] = Some(0);
    assert!(
        RecordedFrame::from_updates(
            1,
            1,
            true,
            Metrics::default(),
            Arc::new(slots),
            vec![Arc::clone(&update)],
            Arc::clone(&update)
        )
        .is_err()
    );
    assert!(
        RecordedFrame::from_updates(
            1,
            1,
            true,
            Metrics::default(),
            Arc::new([None; TABLE_COUNT]),
            vec![Arc::clone(&update)],
            update
        )
        .is_err()
    );
}

#[test]
fn retention_releases_unkept_payloads_from_owned_and_shared_backings() {
    for columns in [false, true] {
        for shared in [false, true] {
            let original = frame();
            let mut recorded = recorded(original.clone(), columns);
            let other = shared.then(|| recorded.clone());
            let inline = match (&recorded.tables.0, shared) {
                (Backing::Frame(frame), true) => Some(Arc::downgrade(frame)),
                _ => None,
            };
            recorded.retain_updates(&[true; TABLE_COUNT]).unwrap();
            assert_eq!(recorded.to_owned_frame().unwrap(), original);
            let before = bytes(&recorded);
            let mut kept = [false; TABLE_COUNT];
            kept[TableId::Clock.index()] = true;
            recorded.retain_updates(&kept).unwrap();
            for table in TableId::ALL {
                if table == TableId::Clock {
                    assert_eq!(
                        recorded.tables[table.index()],
                        original.tables[table.index()]
                    );
                } else {
                    assert!(
                        matches!(&recorded.tables[table.index()], TableUpdate::Unavailable { reason } if reason.is_empty()),
                        "{table:?}: change-only payload retained (columns={columns}, shared={shared})"
                    );
                }
            }
            assert!(bytes(&recorded) < before, "discarded payloads remain held");
            if let Some(other) = other {
                assert_eq!(other.to_owned_frame().unwrap(), original);
            }
            if let Some(inline) = inline {
                assert_eq!(
                    inline.strong_count(),
                    0,
                    "the filtered frame still holds the original backing"
                );
            }
            recorded.retain_updates(&[false; TABLE_COUNT]).unwrap();
            assert!(recorded.tables.iter().all(|update| matches!(update,
                TableUpdate::Unavailable { reason } if reason.is_empty()
            )));
        }
    }
}

#[test]
fn owned_conversion_preserves_edited_metadata_for_both_backings_and_owner_counts() {
    for columns in [false, true] {
        for shared in [false, true] {
            let original = frame();
            let mut recorded = recorded(original.clone(), columns);
            let other = shared.then(|| recorded.clone());
            recorded.branch = 7;
            recorded.hour = 21;
            recorded.keyframe = true;
            recorded.metrics.total_ns = 123;
            recorded.metrics.pool_bytes = 456;
            recorded.metrics.dropped_frames = 2;
            recorded.metrics.table_ns[TableId::Clock.index()] = 789;
            let expected = recorded.to_owned_frame().unwrap();
            assert_eq!(
                recorded.into_owned_frame().unwrap(),
                expected,
                "columns={columns}, shared={shared}"
            );
            if let Some(other) = other {
                assert_eq!(other.to_owned_frame().unwrap(), original);
            }
        }
    }
}
