//! A frame's larger tables encoded on the spool thread and its conversion
//! helpers at once.
//!
//! The encoder borrows its baselines shared while it encodes one table's
//! update ([`FrameEncoder::encode_segment`]), so each thread takes the next
//! of the frame's tables with rows to compare or write, largest first, and
//! the spool thread then assembles the frame in table order from those
//! segments, encoding the rest itself ([`FrameEncoder::prepare_bytes_with`]).
//! The bytes are those one thread writes. A table whose segment failed is
//! encoded again in place, which reports its error.

use crate::spool_phases::{HELPER, Phase};
use replay_data::rows::TABLE_COUNT;
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::{
    frame::FrameEncoder, frame::FrameInput, frame::TableInput, frame::TableSegment,
    frame::TableView,
};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicUsize, Ordering},
};

/// Tables with fewer rows encode in place: a segment costs an allocation and
/// a copy of its bytes. Tests segment every table with rows, so the small
/// synthetic spool campaigns assemble their frames from segments too.
#[cfg(not(test))]
const SEGMENT_ROWS: usize = 64;
#[cfg(test)]
const SEGMENT_ROWS: usize = 1;

/// The rows a table's input asks the encoder to compare or write.
fn weight(encoder: &FrameEncoder, table: TableId, input: &TableInput) -> usize {
    match input {
        TableInput::Sampled(TableView::Complete(rows) | TableView::Truncated { rows, .. }) => {
            rows.len()
        }
        TableInput::SharedComplete(rows) => {
            // The committed allocation itself encodes as unchanged.
            if encoder
                .shared_baseline(table)
                .is_some_and(|baseline| std::sync::Arc::ptr_eq(baseline, rows))
            {
                0
            } else {
                rows.len()
            }
        }
        TableInput::ComparedChanges { rows, .. } => rows.len(),
        TableInput::SliceUpdate { update, .. } => update.rows().len(),
        TableInput::ComparedEqual(_) | TableInput::Sampled(TableView::Unavailable { .. }) => 0,
    }
}

/// Encode `source`'s larger tables for the frame at its branch and hour on
/// this thread and `helpers`; the frame takes the returned segments.
pub(crate) fn encode(
    encoder: &FrameEncoder,
    source: &FrameInput,
    (branch, hour, force): (BranchId, Hour, bool),
    helpers: crate::spool_helper::Helpers<'_>,
) -> [Option<TableSegment>; TABLE_COUNT] {
    let context = encoder.frame_context(branch, hour, force);
    let mut order = [(0_usize, 0_usize); TABLE_COUNT];
    let mut count = 0;
    for table in TableId::ALL {
        let rows = weight(encoder, table, &source.tables[table.index()]);
        if rows >= SEGMENT_ROWS {
            order[count] = (rows, table.index());
            count += 1;
        }
    }
    let order = &mut order[..count];
    if order.len() < 2 {
        // Nothing to share: the frame encodes every table itself.
        return std::array::from_fn(|_| None);
    }
    order.sort_unstable_by(|a, b| b.cmp(a));
    let segments: [Mutex<Option<TableSegment>>; TABLE_COUNT] =
        std::array::from_fn(|_| Mutex::new(None));
    let next = AtomicUsize::new(0);
    let order = &*order;
    let work = || loop {
        let slot = next.fetch_add(1, Ordering::Relaxed);
        let Some(&(_, index)) = order.get(slot) else {
            break;
        };
        if crate::allocation::failed() {
            break;
        }
        let table = TableId::ALL[index];
        if let Ok(segment) = encoder.encode_segment(&context, table, &source.tables[index]) {
            *segments[index]
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(segment);
        }
    };
    let allocation_context = crate::allocation::context();
    let job = || {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Encode);
        crate::allocation::set_context(allocation_context);
        let _span = HELPER.span(Phase::Encode);
        work();
    };
    helpers.join_each(&job, work);
    segments.map(|segment| segment.into_inner().unwrap_or_else(PoisonError::into_inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::StateRow, rows::TableRows};
    use replay_frame::frame::Metrics;

    fn frame(hour: Hour, owner: u32) -> FrameInput {
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
        for table in [TableId::States, TableId::Provinces, TableId::Countries] {
            frame.tables[table.index()] = TableView::Complete(match table {
                TableId::States => TableRows::States(
                    (0..200)
                        .map(|id| StateRow {
                            id,
                            owner: owner + id % 3,
                            controller: owner,
                            ..Default::default()
                        })
                        .collect(),
                ),
                TableId::Provinces => TableRows::Provinces(
                    (0..500)
                        .map(|id| replay_data::rows::ProvinceRow {
                            id,
                            owner: owner + id % 7,
                            ..Default::default()
                        })
                        .collect(),
                ),
                _ => TableRows::Countries(
                    (1..=3)
                        .map(|id| replay_data::rows::CountryRow {
                            id,
                            canonical_slot: id,
                            tag: b"TAG".to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ),
            })
            .into();
        }
        frame
    }

    /// Frames assembled from segments encoded on the helpers are those one
    /// thread encodes, and the helpers took tables with enough rows.
    #[test]
    fn segments_encoded_on_the_helpers_assemble_the_frame_one_thread_encodes() {
        let first = crate::spool_helper::SpoolHelper::start().unwrap();
        let second = crate::spool_helper::SpoolHelper::start().unwrap();
        let mut whole = FrameEncoder::new();
        let mut split = FrameEncoder::new();
        let helper_encodes = || HELPER.phase(Phase::Encode).0;
        let before = helper_encodes();
        for hour in 0..6 {
            let input = frame(hour, 10 + hour as u32 / 2);
            let force = hour == 4;
            let expected = whole.encode_bytes(input.clone(), force).unwrap();
            let helpers = crate::spool_helper::Helpers {
                first: &first,
                second: (hour % 2 == 0).then_some(&second),
                backlogged: false,
            };
            let mut segments = encode(&split, &input, (1, hour, force), helpers);
            for table in [TableId::Provinces, TableId::States, TableId::Countries] {
                assert!(segments[table.index()].is_some(), "{table:?}");
            }
            assert!(segments[TableId::Clock.index()].is_none());
            let mut prepared = split
                .prepare_bytes_with(input, force, &mut segments)
                .unwrap();
            let actual = prepared.take_frame();
            split.commit(prepared).unwrap();
            assert_eq!(
                actual.decode().unwrap(),
                expected.decode().unwrap(),
                "hour {hour}"
            );
        }
        assert!(helper_encodes() > before);
    }
}
