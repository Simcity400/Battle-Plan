//! Synthetic recording fixtures and grouped-table decode work for consumer tests.
use super::{BlockIndex, Replay, TableId};
use crate::frame::{Frame, FrameDecoder};
use std::{cell::RefCell, io};

/// Write a synthetic one-block recording with the supplied frames. The ordinary
/// current block writer preserves grammar and checksums without admitting the
/// campaign identities, so consumers can exercise a malformed intermediate hour.
pub fn with_block_frames(replay: &Replay, frames: &[Frame]) -> io::Result<Replay> {
    if replay.index.len() != 1 {
        return Err(super::invalid("fixture requires a one-block recording"));
    }
    with_independent_block_frames(replay, &[frames])
}

/// Rewrite synthetic blocks without cross-block anchors, keeping their current framing.
pub fn with_independent_block_frames(replay: &Replay, blocks: &[&[Frame]]) -> io::Result<Replay> {
    let cores = super::pack::Cores::new(1);
    let mut bytes = replay.bytes[..replay.header_end].to_vec();
    let mut index = Vec::new();
    for frames in blocks {
        let first = frames
            .first()
            .ok_or_else(|| super::invalid("fixture requires at least one frame"))?;
        let mut block = super::grouped::BlockWriter::new(first)?;
        let mut anchors = super::grouped::Anchors::default();
        let mut decoder = FrameDecoder::new();
        for frame in *frames {
            let decoded = decoder.advance(frame)?;
            let mut payload = Vec::new();
            let sizes = frame.encode_with_sizes(&mut payload)?;
            block.push(
                frame,
                decoded,
                &payload,
                &sizes,
                &mut anchors,
                super::MAX_BLOCK_RAW_BYTES,
                &cores,
            )?;
        }
        super::append_blocks(&mut bytes, &mut index, &[block.compress(&cores)?])?;
    }
    super::finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &replay.controls,
        &replay.recovery_notes,
        &index,
    )?;
    Replay::open(bytes)
}

/// Add a trailing table control byte; shorter prefixes stay valid but whole reads refuse.
pub fn with_trailing_table_control(replay: &Replay, table: TableId) -> io::Result<Replay> {
    if replay.index.len() != 1 {
        return Err(super::invalid("fixture requires a one-block recording"));
    }
    let (mut index, block) = super::grouped::trailing_control(replay, table)?;
    let mut bytes = replay.bytes[..replay.header_end].to_vec();
    index.offset = bytes.len() as u64;
    bytes.extend_from_slice(&block);
    super::finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &replay.controls,
        &replay.recovery_notes,
        &[index],
    )?;
    Replay::open(bytes)
}

type Decodes = Vec<(u64, i32, TableId, usize)>;

thread_local! {
    static DECODES: RefCell<Option<Decodes>> = const { RefCell::new(None) };
}

pub(super) fn decoded(block: &BlockIndex, table: TableId, hours: usize) {
    DECODES.with(|counts| {
        if let Some(counts) = &mut *counts.borrow_mut() {
            counts.push((block.branch, block.first_hour, table, hours));
        }
    });
}

/// Each grouped table decoded by `run`, in order, as (branch, block's first
/// hour, table, hours decoded). Keyframe and later entries form one table decode; anchor
/// reads are part of it. Other threads and sections the decoder skips are excluded.
pub fn table_decodes<T>(run: impl FnOnce() -> T) -> (T, Decodes) {
    struct Reset(Option<Decodes>);
    impl Drop for Reset {
        fn drop(&mut self) {
            DECODES.with(|counts| *counts.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(DECODES.with(|counts| counts.replace(Some(Vec::new()))));
    let result = run();
    let counts = DECODES.with(|counts| counts.take().unwrap());
    (result, counts)
}
