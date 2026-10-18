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

/// Each hour's read, the owners it reads and the owners still live.
type HourRead = (Vec<ResearchInputRow>, fn(u32) -> bool, fn(u32) -> bool);

fn hours() -> Vec<HourRead> {
    vec![
        // Equal rows of the read owners.
        (
            vec![row(1, 0, 0), row(1, 1, 0)],
            |owner| owner == 1,
            |_| true,
        ),
        // Changed rows, same counts.
        (vec![row(2, 0, 5)], |owner| owner == 2, |_| true),
        // An owner gains a row, another is read without rows.
        (
            vec![row(1, 0, 6), row(1, 1, 0), row(1, 2, 1)],
            |owner| owner == 1 || owner == 3,
            |_| true,
        ),
        // A new owner, and one no longer live outside the read.
        (vec![row(4, 0, 2)], |owner| owner == 4, |owner| owner != 25),
        // Nothing read: unchanged.
        (vec![], |_| false, |_| true),
        // Several owners at once, counts unchanged.
        (
            vec![row(1, 0, 7), row(1, 1, 1), row(1, 2, 1), row(2, 0, 6)],
            |owner| owner <= 2,
            |_| true,
        ),
    ]
}

/// The update path writes the bytes of the merged complete table and
/// commits its rows inside the baseline both share.
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
    let mut held = Arc::new(start.clone());
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
        assert_eq!(
            update.owners_after(&held).unwrap(),
            crate::sliced::owners(&merged)
        );
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
        // The producer releases its copy so the commit updates in place.
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
    // Rows of an owner the update does not list do not fit.
    let update = SliceUpdate {
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
    // A keyframe, a gap or another branch has no slice baseline.
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
    // While the producer still holds the baseline, commit replaces a copy.
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
