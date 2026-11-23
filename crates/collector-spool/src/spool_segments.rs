use collector_workers::spool_phases::{HELPER, Phase, PhaseObserver, Steps};
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

#[cfg(not(test))]
const SEGMENT_ROWS: usize = 64;
#[cfg(test)]
const SEGMENT_ROWS: usize = 1;

fn weight(encoder: &FrameEncoder, table: TableId, input: &TableInput) -> usize {
    match input {
        TableInput::Sampled(TableView::Complete(rows) | TableView::Truncated { rows, .. }) => {
            rows.len()
        }
        TableInput::SharedComplete(rows) => {
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
        TableInput::SliceUpdate { update, .. }
        | TableInput::UniformOptionalI32 { update, .. } => update.rows().len(),
        TableInput::ComparedEqual(_)
        | TableInput::Sampled(TableView::Unavailable { .. } | TableView::NotRecorded { .. }) => 0,
    }
}

pub(crate) fn encode_keeping(
    encoder: &FrameEncoder,
    source: &FrameInput,
    target: (BranchId, Hour, bool),
    kept: impl Fn(TableId) -> bool,
    helpers: collector_workers::spool_helper::Helpers<'_>,
) -> [Option<TableSegment>; TABLE_COUNT] {
    encode_keeping_with_steps(encoder, source, target, kept, helpers, &HELPER)
}

#[cfg(test)]
fn encode_with_steps(
    encoder: &FrameEncoder,
    source: &FrameInput,
    target: (BranchId, Hour, bool),
    helpers: collector_workers::spool_helper::Helpers<'_>,
    steps: &Steps<impl PhaseObserver + Sync>,
) -> [Option<TableSegment>; TABLE_COUNT] {
    encode_keeping_with_steps(encoder, source, target, |_| true, helpers, steps)
}

fn encode_keeping_with_steps(
    encoder: &FrameEncoder,
    source: &FrameInput,
    (branch, hour, force): (BranchId, Hour, bool),
    kept: impl Fn(TableId) -> bool,
    helpers: collector_workers::spool_helper::Helpers<'_>,
    steps: &Steps<impl PhaseObserver + Sync>,
) -> [Option<TableSegment>; TABLE_COUNT] {
    let context = encoder.frame_context(branch, hour, force);
    let mut order = [(0_usize, 0_usize); TABLE_COUNT];
    let mut count = 0;
    for table in TableId::ALL {
        if !kept(table) {
            continue;
        }
        let rows = weight(encoder, table, &source.tables[table.index()]);
        if rows >= SEGMENT_ROWS {
            order[count] = (rows, table.index());
            count += 1;
        }
    }
    let order = &mut order[..count];
    if order.len() < 2 {
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
        if collector_memory::terminal::failed() {
            break;
        }
        let table = TableId::ALL[index];
        if let Ok(segment) = encoder.encode_segment(&context, table, &source.tables[index]) {
            *segments[index]
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(segment);
        }
    };
    let allocation_context = collector_memory::terminal::context();
    let job = || {
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Encode);
        collector_memory::terminal::set_context(allocation_context);
        let _span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { steps.span(Phase::Encode) };
        work();
    };
    // SAFETY: The spool owns this conversion round and uses no helper in a competing round.
    unsafe { helpers.join_each(&job, work) };
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

    #[test]
    fn segments_encoded_on_the_helpers_assemble_the_frame_one_thread_encodes() {
        let pool = [
            // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
            unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
            // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
            unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
        ];
        let mut whole = FrameEncoder::new();
        let mut split = FrameEncoder::new();
        let steps = Steps::counted();
        for hour in 0..6 {
            let input = frame(hour, 10 + hour as u32 / 2);
            let force = hour == 4;
            let expected = whole.encode_bytes(input.clone(), force).unwrap();
            let helpers = collector_workers::spool_helper::Helpers {
                pool: &pool[..if hour % 2 == 0 { 2 } else { 1 }],
                backlogged: false,
            };
            let before = steps.entries(Phase::Encode);
            let mut segments = encode_with_steps(&split, &input, (1, hour, force), helpers, &steps);
            assert_eq!(steps.entries(Phase::Encode) - before, helpers.pool.len());
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
        assert!(steps.entries(Phase::Encode) > 0);
    }
}
