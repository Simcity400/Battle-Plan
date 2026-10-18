//! Keyframe samples of a branch's blocks after a prepared prefix, for a first
//! view whose values beyond coverage are approximate: each block's keyframe
//! stands for the block's hours after the prefix, without its recorded gaps.
//! A value read at a keyframe is exact at that hour; the hours it stands for
//! are approximate, and the reply names them so.
//!
//! The keyframes are read and decoded ahead on spare cores, each alone, and
//! visited in hour order. A block whose keyframe cannot be read, or falls in a
//! recorded gap, is not sampled: its hours stay pending. Inside
//! [`super::progressive::without_block_decodes`] retained observations answer
//! immediately; a missing or busy sample refuses so the query runs again where
//! decodes are allowed.

use crate::{Source, metadata::check_cancel, products::country_groups::HourSpan};
use replay::container::BlockIndex;
use replay_data::rows::TableId;
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// Spare cores reading keyframes ahead of the visit: half the hardware
/// threads, as the sampled-read loaders use, and at least one.
fn read_ahead() -> usize {
    std::thread::available_parallelism().map_or(1, |threads| (threads.get() / 2).max(1))
}

/// The hours of `block` after `after` through `through` (the block's end
/// when None), without the branch's recorded gaps, in order.
fn block_spans(
    source: &Source,
    block: &BlockIndex,
    after: i32,
    through: Option<i32>,
) -> Vec<HourSpan> {
    let first = i64::from(block.first_hour).max(i64::from(after) + 1);
    let last = through.map_or(block.last_hour, |through| block.last_hour.min(through));
    if first > i64::from(last) {
        return Vec::new();
    }
    crate::prepared::prefix::without_gaps(source, block.branch, first as i32, last)
}

/// Visits the keyframe of each block of `branch` holding recorded hours after
/// `after` (through `through`, or the branch's end when None), in hour order,
/// with `tables` decoded and the hours it stands for. `sample` extracts a
/// content-independent observation, kept per source, table set and result type;
/// a re-ask reuses it even when the prefix cuts further into that block. Use a
/// product's own result type. Source revisions do not inherit these entries.
/// None (an hour its family did not observe) leaves its hours pending.
/// Returns the hours of the keyframes sampled,
/// joined where they meet: the reply's approximate spans.
#[allow(clippy::too_many_arguments)]
pub fn after<T: Send + Sync + 'static>(
    source: &Source,
    product: &'static str,
    branch: u64,
    after: i32,
    through: Option<i32>,
    tables: &[TableId],
    cancelled: &AtomicBool,
    sample: &(impl Fn(&DecodedFrame) -> io::Result<Option<T>> + Sync),
    bytes: &(impl Fn(&T) -> usize + Sync),
    mut visit: impl FnMut(&[HourSpan], &T) -> io::Result<()>,
) -> io::Result<Vec<HourSpan>> {
    blocks_after(
        source,
        product,
        branch,
        after,
        through,
        tables,
        &(),
        None,
        cancelled,
        bytes,
        &|block| {
            let keyframe = source.replay_ref().read_keyframe_recorded(block, tables)?;
            let mut decoder = FrameDecoder::for_tables(tables);
            sample(decoder.advance(&keyframe)?)
        },
        |spans, observation| {
            visit(spans, observation)?;
            Ok(true)
        },
    )
}

/// The same cached, ordered sampling with a family's supplying-read extraction
/// instead of a keyframe alone. `params` names every extraction input beyond
/// the selected tables; `binding` invalidates content-dependent facts. A visitor
/// returning false leaves the block pending without discarding its observation.
#[allow(clippy::too_many_arguments)]
pub fn blocks_after<T: Send + Sync + 'static>(
    source: &Source,
    product: &'static str,
    branch: u64,
    after: i32,
    through: Option<i32>,
    tables: &[TableId],
    params: &(impl serde::Serialize + Sync),
    binding: Option<&store::SourceKey>,
    cancelled: &AtomicBool,
    bytes: &(impl Fn(&T) -> usize + Sync),
    sample: &(impl Fn(&BlockIndex) -> io::Result<Option<T>> + Sync),
    mut visit: impl FnMut(&[HourSpan], &T) -> io::Result<bool>,
) -> io::Result<Vec<HourSpan>> {
    check_cancel(cancelled)?;
    let replay = source.replay_ref();
    let mut blocks: Vec<(&BlockIndex, Vec<HourSpan>)> = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && !replay.gaps.iter().any(|gap| {
                    gap.branch == branch
                        && gap.first_hour <= block.first_hour
                        && block.first_hour <= gap.last_hour
                })
        })
        .map(|block| (block, block_spans(source, block, after, through)))
        .filter(|(_, spans)| !spans.is_empty())
        .collect();
    if blocks.is_empty() {
        return Ok(Vec::new());
    }
    let consumer = replay.section_consumer(
        blocks
            .iter()
            .filter_map(|(block, _)| replay.block_position(block))
            .min()
            .unwrap_or(0),
    );
    let cache = super::observations::scope(source, product, binding);
    let mut selected: Vec<_> = tables.iter().map(|table| table.index()).collect();
    selected.sort_unstable();
    selected.dedup();
    blocks.sort_by_key(|(block, _)| block.first_hour);
    let read = |block: &BlockIndex| -> io::Result<Arc<Option<T>>> {
        if let Some(position) = replay.block_position(block) {
            consumer.reading(position);
        }
        cache.get(
            block,
            &(&selected, params),
            cancelled,
            |observation: &Option<T>| {
                std::mem::size_of::<Option<T>>() + observation.as_ref().map_or(0, bytes)
            },
            || sample(block),
        )
    };
    let spans: std::collections::HashMap<(u64, i32), &[HourSpan]> = blocks
        .iter()
        .map(|(block, spans)| ((block.branch, block.first_hour), spans.as_slice()))
        .collect();
    let mut approximate: Vec<HourSpan> = Vec::new();
    let mut accept = |block: &BlockIndex, observation: &Option<T>| -> io::Result<()> {
        check_cancel(cancelled)?;
        let Some(observation) = observation else {
            return Ok(());
        };
        let spans = spans[&(block.branch, block.first_hour)];
        if visit(spans, observation)? {
            for span in spans {
                crate::products::country_groups::mark_unobserved(
                    &mut approximate,
                    span.first_hour,
                    span.last_hour,
                );
            }
        }
        Ok(())
    };
    if super::progressive::block_decodes_disabled() {
        // Cache-only requests stay on this thread so a miss cannot decode or
        // wait on a worker. The observation cache refuses missing or busy slots.
        for (block, _) in &blocks {
            if let Some(position) = replay.block_position(block) {
                consumer.at(position);
            }
            let observation = read(block)?;
            accept(block, &observation)?;
        }
    } else {
        dashboard_core::blocks::scan_blocks_by(
            blocks.iter().map(|(block, _)| *block).collect(),
            read_ahead(),
            cancelled,
            &read,
            |reads| {
                for (block, observation) in reads {
                    if let Some(position) = replay.block_position(block) {
                        consumer.at(position);
                    }
                    check_cancel(cancelled)?;
                    // An unreadable keyframe samples nothing: its hours stay pending.
                    if let Ok(observation) = observation {
                        accept(block, &observation)?;
                    }
                }
                Ok(())
            },
        )?;
    }
    check_cancel(cancelled)?;
    Ok(approximate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{continuation_records, file_source, replay, source as frame};

    /// Blocks at hours 0..=1, 2..=3 and, after a recorded gap, 10..=11.
    fn source() -> Source {
        let mut recording = replay(vec![frame(1, 0, None), frame(1, 1, None)]);
        for hours in [[2, 3], [10, 11]] {
            recording = recording
                .append(&[continuation_records(
                    &recording,
                    1,
                    hours.iter().map(|&hour| frame(1, hour, None)).collect(),
                )])
                .unwrap();
        }
        let (source, root) = file_source(recording);
        let _ = std::fs::remove_dir_all(root);
        source
    }

    fn span(first_hour: i32, last_hour: i32) -> HourSpan {
        HourSpan {
            first_hour,
            last_hour,
        }
    }

    fn visits(
        source: &Source,
        after: i32,
        through: Option<i32>,
    ) -> (Vec<(i32, Vec<HourSpan>)>, Vec<HourSpan>) {
        let mut seen = Vec::new();
        let approximate = super::after(
            source,
            "test-keyframe",
            1,
            after,
            through,
            &[TableId::Campaign],
            &AtomicBool::new(false),
            &|frame| Ok(Some(frame.hour)),
            &|_: &i32| std::mem::size_of::<i32>(),
            |spans, hour| {
                seen.push((*hour, spans.to_vec()));
                Ok(())
            },
        )
        .unwrap();
        (seen, approximate)
    }

    #[test]
    fn each_block_after_the_prefix_is_its_keyframe_for_its_remaining_hours() {
        let source = source();
        let (seen, approximate) = visits(&source, 1, None);
        assert_eq!(seen, [(2, vec![span(2, 3)]), (10, vec![span(10, 11)])]);
        // The recorded gap between the blocks is not approximate.
        assert_eq!(approximate, [span(2, 3), span(10, 11)]);
        // A prefix ending inside a block samples the block's keyframe for
        // the block's hours after it only.
        let (seen, approximate) = visits(&source, 2, Some(10));
        assert_eq!(seen, [(2, vec![span(3, 3)]), (10, vec![span(10, 10)])]);
        assert_eq!(approximate, [span(3, 3), span(10, 10)]);
        // Re-asking a shorter range reuses the observation, clipped to it.
        let (seen, approximate) = visits(&source, 1, Some(3));
        assert_eq!(seen, [(2, vec![span(2, 3)])]);
        assert_eq!(approximate, [span(2, 3)]);
        // From before the branch, every block; a complete prefix, none.
        assert_eq!(visits(&source, -1, None).0.len(), 3);
        assert_eq!(visits(&source, 11, None), (Vec::new(), Vec::new()));
        let (bytes, bound, extractions) =
            super::super::observations::usage(&source, "test-keyframe");
        assert_eq!(
            extractions, 3,
            "every block is sampled once across all re-asks"
        );
        assert!(bytes <= bound);
    }

    #[test]
    fn declined_and_absent_samples_stay_pending_and_reasks_share_the_extraction() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let source = source();
        let flag = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let read = |block: &BlockIndex| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok((block.first_hour != 10).then_some(block.first_hour))
        };
        for accepted in [false, true] {
            let mut seen = Vec::new();
            let approximate = blocks_after(
                &source,
                "test-declined",
                1,
                1,
                None,
                &[TableId::Campaign],
                &"mod country",
                None,
                &flag,
                &|_: &i32| std::mem::size_of::<i32>(),
                &read,
                |spans, hour| {
                    seen.push((*hour, spans.to_vec()));
                    Ok(accepted)
                },
            )
            .unwrap();
            assert_eq!(seen, [(2, vec![span(2, 3)])]);
            assert_eq!(
                approximate,
                if accepted {
                    vec![span(2, 3)]
                } else {
                    Vec::new()
                }
            );
            let mut pending = crate::prepared::prefix::pending(&source, 1, 1, None);
            pending = crate::prepared::prefix::subtract(&pending, &approximate);
            assert_eq!(
                pending,
                if accepted {
                    vec![span(10, 11)]
                } else {
                    vec![span(2, 3), span(10, 11)]
                }
            );
            assert_eq!(
                calls.load(Ordering::Relaxed),
                2,
                "None and declined observations are reused"
            );
        }
        blocks_after(
            &source,
            "test-declined",
            1,
            1,
            None,
            &[TableId::Campaign],
            &"another mod country",
            None,
            &flag,
            &|_: &i32| std::mem::size_of::<i32>(),
            &read,
            |_, _| Ok(true),
        )
        .unwrap();
        assert_eq!(
            calls.load(Ordering::Relaxed),
            4,
            "extraction parameters separate observations"
        );
        let (bytes, bound, _) = super::super::observations::usage(&source, "test-declined");
        assert!(bytes <= bound);
    }

    #[test]
    fn a_block_starting_in_a_gap_is_never_sampled_even_when_later_hours_are_recorded() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let mut recording = replay(vec![frame(1, 0, None), frame(1, 1, None)]);
        recording.gaps.push(replay_frame::branches::Gap {
            branch: 1,
            first_hour: 0,
            last_hour: 0,
            reason: "synthetic gap".into(),
        });
        let (source, root) = file_source(recording);
        let approximate = blocks_after(
            &source,
            "test-gap",
            1,
            -1,
            None,
            &[TableId::Campaign],
            &(),
            None,
            &AtomicBool::new(false),
            &|_: &i32| std::mem::size_of::<i32>(),
            &|_| -> io::Result<Option<i32>> {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(Some(0))
            },
            |_, _| panic!("a gap-start block must not be visited"),
        )
        .unwrap();
        assert!(approximate.is_empty());
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "a gap-start block must not be extracted"
        );
        assert_eq!(
            crate::prepared::prefix::pending(&source, 1, -1, None),
            [span(1, 1)]
        );
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_keyframe_sample_refuses_without_block_decodes() {
        let source = source();
        let (result, refused) = super::super::progressive::without_block_decodes(|| {
            super::after(
                &source,
                "test-keyframe",
                1,
                1,
                None,
                &[TableId::Campaign],
                &AtomicBool::new(false),
                &|frame| Ok(Some(frame.hour)),
                &|_: &i32| std::mem::size_of::<i32>(),
                |_, _| Ok(()),
            )
        });
        assert!(refused);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        super::after(
            &source,
            "test-keyframe",
            1,
            1,
            None,
            &[TableId::Campaign],
            &AtomicBool::new(false),
            &|frame| Ok(Some(frame.hour)),
            &|_: &i32| std::mem::size_of::<i32>(),
            |_, _| Ok(()),
        )
        .unwrap();
        let (held, refused) = super::super::progressive::without_block_decodes(|| {
            super::after(
                &source,
                "test-keyframe",
                1,
                2,
                None,
                &[TableId::Campaign],
                &AtomicBool::new(false),
                &|_| panic!("retained observations must be reused"),
                &|_: &i32| std::mem::size_of::<i32>(),
                |_, _| Ok(()),
            )
        });
        assert!(!refused);
        assert_eq!(held.unwrap(), [span(3, 3), span(10, 11)]);
        // Nothing to sample decodes nothing, so it answers there too.
        let (result, refused) = super::super::progressive::without_block_decodes(|| {
            super::after(
                &source,
                "test-keyframe",
                1,
                11,
                None,
                &[TableId::Campaign],
                &AtomicBool::new(false),
                &|frame| Ok(Some(frame.hour)),
                &|_: &i32| std::mem::size_of::<i32>(),
                |_, _| Ok(()),
            )
        });
        assert!(!refused);
        assert!(result.unwrap().is_empty());
    }
    #[test]
    fn saved_block_samples_are_visited_in_order_without_decoding_again() {
        let source = source();
        let flag = AtomicBool::new(false);
        after(
            &source,
            "test-saved-order",
            1,
            1,
            None,
            &[TableId::Campaign],
            &flag,
            &|frame| Ok(Some(frame.hour)),
            &|_: &i32| size_of::<i32>(),
            |_, _| Ok(()),
        )
        .unwrap();
        let (result, refused) = super::super::progressive::without_block_decodes(|| {
            let mut seen = Vec::new();
            let spans = after(
                &source,
                "test-saved-order",
                1,
                1,
                None,
                &[TableId::Campaign],
                &flag,
                &|_| panic!("a saved block must not decode again"),
                &|_: &i32| size_of::<i32>(),
                |spans, hour| {
                    seen.push((*hour, spans.to_vec()));
                    Ok(())
                },
            )?;
            io::Result::Ok((seen, spans))
        });
        assert!(!refused);
        let (seen, spans) = result.unwrap();
        assert_eq!(seen, [(2, vec![span(2, 3)]), (10, vec![span(10, 11)])]);
        assert_eq!(spans, [span(2, 3), span(10, 11)]);
    }
}
