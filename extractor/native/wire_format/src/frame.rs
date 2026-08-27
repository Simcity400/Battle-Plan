//! Frames, blocks and the two time chains.
//!
//! A block carries frames for exactly one `(domain, branch)` on two independent
//! chains — state and event. `kind` says which, so a reader that cannot decode a
//! frame still knows which chain its `tick_delta` advances, which is the whole
//! point of the reserved-kind assignment.

use crate::schema::Schema;
use crate::varint::{read_varint, write_varint, Result, WireError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    State,
    Event,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Kind {
    Baseline = 0,
    Delta = 1,
    Unchanged = 2,
    Events = 3,
    /// frame-tag contract assigns reserved kinds to a part in advance.
    ReservedState4 = 4,
    ReservedState5 = 5,
    ReservedEvent6 = 6,
    ReservedEvent7 = 7,
}

crate::wire_coded::wire_coded_kinds! {
    Kind: u64,
    code = pub code,
    Ok = from_code_inner(code) -> crate::Result<Self> {
        Err(crate::WireError::Corrupt("kind is three bits"))
    };

    Baseline = 0,
    Delta = 1,
    Unchanged = 2,
    Events = 3,
    ReservedState4 = 4,
    ReservedState5 = 5,
    ReservedEvent6 = 6,
    ReservedEvent7 = 7,
}

impl Kind {
    pub fn from_code(code: u64) -> Result<Self> {
        Self::from_code_inner(code)
    }

    pub fn part(self) -> Part {
        match self {
            Kind::Baseline | Kind::Delta | Kind::Unchanged | Kind::ReservedState4
            | Kind::ReservedState5 => Part::State,
            Kind::Events | Kind::ReservedEvent6 | Kind::ReservedEvent7 => Part::Event,
        }
    }

    pub fn is_reserved(self) -> bool {
        matches!(
            self,
            Kind::ReservedState4 | Kind::ReservedState5 | Kind::ReservedEvent6 | Kind::ReservedEvent7
        )
    }

    /// frame-run contract: only `BASELINE` and `DELTA` terminate an `UNCHANGED` run. `EVENTS`
    /// belongs to the other chain and never does.
    pub fn terminates_run(self) -> bool {
        matches!(self, Kind::Baseline | Kind::Delta)
    }
}

/// frame-tag contract `tag = (tick_delta << 3) | kind`.
pub fn pack_tag(tick_delta: u64, kind: Kind) -> Result<u64> {
    if tick_delta > u64::MAX >> 3 {
        return Err(WireError::Corrupt("tick_delta does not fit alongside kind"));
    }
    Ok((tick_delta << 3) | kind.code())
}

pub fn unpack_tag(tag: u64) -> Result<(u64, Kind)> {
    Ok((tag >> 3, Kind::from_code(tag & 7)?))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockHeader {
    pub domain: u32,
    pub branch: u32,
    pub schema_hash: u64,
    pub first_tick: u64,
}

impl BlockHeader {
    pub fn encode(&self, out: &mut Vec<u8>) {
        write_varint(out, self.domain as u64);
        write_varint(out, self.branch as u64);
        out.extend_from_slice(&self.schema_hash.to_le_bytes());
        write_varint(out, self.first_tick);
    }

    pub fn decode(buf: &[u8], pos: usize) -> Result<(Self, usize)> {
        let (domain, pos) = read_varint(buf, pos)?;
        let (branch, pos) = read_varint(buf, pos)?;
        // Both are varints on the wire and `u32` in the struct. Truncating an
        // over-range value aliases it onto a valid identity — a forged block
        // claiming domain 2^32 would be read as domain 0 — so it is rejected.
        if domain > u32::MAX as u64 || branch > u32::MAX as u64 {
            return Err(WireError::Corrupt("block domain or branch exceeds u32"));
        }
        if pos + 8 > buf.len() {
            return Err(WireError::Truncated);
        }
        let schema_hash = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        let (first_tick, pos) = read_varint(buf, pos + 8)?;
        Ok((
            Self {
                domain: domain as u32,
                branch: branch as u32,
                schema_hash,
                first_tick,
            },
            pos,
        ))
    }
}

/// One frame as the writer sees it: a kind, its absolute tick, and its body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub kind: Kind,
    pub tick: u64,
    /// For `UNCHANGED`, the run length in **sample ticks** (frame-run contract). Ignored otherwise.
    pub run_len: u64,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn state(kind: Kind, tick: u64, body: Vec<u8>) -> Self {
        Self { kind, tick, run_len: 1, body }
    }

    pub fn unchanged(tick: u64, run_len: u64) -> Self {
        Self { kind: Kind::Unchanged, tick, run_len, body: Vec::new() }
    }

    pub fn events(tick: u64, body: Vec<u8>) -> Self {
        Self { kind: Kind::Events, tick, run_len: 1, body }
    }

    /// frame-run contract: an `UNCHANGED` run covers `N` consecutive sample ticks, so its last
    /// covered tick is `t + (N-1)*stride`. Every other kind covers one tick.
    ///
    /// `None` if that endpoint does not fit a `u64`. Both operands come off the
    /// wire as varints, so the product is attacker-chosen; unchecked it panics
    /// under this workspace's `overflow-checks` and wraps without them, and a
    /// wrapped endpoint makes every later tick in the block wrong.
    pub fn last_covered_tick_checked(&self, stride: u64) -> Option<u64> {
        match self.kind {
            Kind::Unchanged => self
                .run_len
                .saturating_sub(1)
                .checked_mul(stride)
                .and_then(|span| self.tick.checked_add(span)),
            _ => Some(self.tick),
        }
    }
}

/// Why a block does not satisfy frame contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockError {
    /// frame-run contract: a `DELTA` or `UNCHANGED` before this block's first `BASELINE`.
    DeltaBeforeBaseline { tick: u64 },
    /// frame-order contract: frames must be nondecreasing by their own tick.
    TickWentBackwards { part: Part, from: u64, to: u64 },
    /// frame-order contract: the block's opening frame carries `tick_delta = 0`.
    FirstFrameNotAtFirstTick { first_tick: u64, tick: u64 },
    /// tick-grid contract: every frame lands on a multiple of the domain's sample stride.
    OffGrid { tick: u64, stride: u64 },
    /// frame-run contract: runs are maximal, so two `UNCHANGED` frames cannot be time-adjacent.
    AdjacentUnchangedRuns { first: u64, second: u64 },
    /// frame-run contract: a run of zero ticks is not a run.
    EmptyRun { tick: u64 },
    /// frame-order contract: at equal ticks the state frame precedes the `EVENTS` frame.
    EventBeforeStateAtSameTick { tick: u64 },
    /// frame-order contract: one state observation and one event batch per tick.
    DuplicateObservation { part: Part, tick: u64 },
    /// A tick or run endpoint that does not fit a `u64`.
    TickOverflow { tick: u64 },
    /// A `tick_delta` too large to pack alongside its kind (frame-tag contract).
    TickDeltaTooLarge { delta: u64 },
    /// block-header contract: `first_tick` is the first frame's tick, so a block has one.
    EmptyBlock,
}

/// frame contract's invariants, in one place so the writer and the reader cannot enforce
/// different rules.
///
/// A block that only the writer validates is a block a hostile or corrupt stream
/// can forge: off-grid ticks, a `DELTA` with no `BASELINE`, two observations at
/// one tick. The reader holds everything needed to reject all of them, so it does.
struct BlockValidator {
    first_tick: u64,
    stride: u64,
    started: bool,
    seen_baseline: bool,
    last_state: Option<u64>,
    last_event: Option<u64>,
    last_state_kind: Option<Kind>,
    last_frame: Option<(u64, Part)>,
}

impl BlockValidator {
    fn new(first_tick: u64, stride: u64) -> Self {
        Self {
            first_tick,
            stride: stride.max(1),
            started: false,
            seen_baseline: false,
            last_state: None,
            last_event: None,
            last_state_kind: None,
            last_frame: None,
        }
    }

    /// Check `frame` against every frame contract rule, then advance.
    ///
    /// **Every check runs before any mutation.** A rejected frame that had
    /// already moved `seen_baseline` or a chain cursor would leave the validator
    /// describing a block that was never written — accept `EVENTS@0`, reject
    /// `BASELINE@0`, accept `DELTA@1`, and the result is a delta with no baseline.
    fn accept(&mut self, frame: &Frame) -> core::result::Result<(), BlockError> {
        // tick-grid contract: a domain samples on ticks that are multiples of its stride —
        // absolutely, not merely congruent to whatever first_tick happened to be.
        if frame.tick % self.stride != 0 {
            return Err(BlockError::OffGrid { tick: frame.tick, stride: self.stride });
        }
        if !self.started && frame.tick != self.first_tick {
            return Err(BlockError::FirstFrameNotAtFirstTick {
                first_tick: self.first_tick,
                tick: frame.tick,
            });
        }
        if matches!(frame.kind, Kind::Unchanged) && frame.run_len == 0 {
            return Err(BlockError::EmptyRun { tick: frame.tick });
        }
        let last_covered = frame
            .last_covered_tick_checked(self.stride)
            .ok_or(BlockError::TickOverflow { tick: frame.tick })?;

        let part = frame.kind.part();
        match part {
            Part::State => {
                if matches!(frame.kind, Kind::Delta | Kind::Unchanged) && !self.seen_baseline {
                    return Err(BlockError::DeltaBeforeBaseline { tick: frame.tick });
                }
                if let Some(last) = self.last_state {
                    if frame.tick <= last {
                        return if frame.tick == last {
                            Err(BlockError::DuplicateObservation { part, tick: frame.tick })
                        } else {
                            Err(BlockError::TickWentBackwards { part, from: last, to: frame.tick })
                        };
                    }
                    // Maximal runs: an UNCHANGED immediately following another with
                    // no gap in between would be two encodings of one run.
                    if matches!(frame.kind, Kind::Unchanged)
                        && matches!(self.last_state_kind, Some(Kind::Unchanged))
                        && Some(frame.tick) == last.checked_add(self.stride)
                    {
                        return Err(BlockError::AdjacentUnchangedRuns {
                            first: last,
                            second: frame.tick,
                        });
                    }
                }
            }
            Part::Event => {
                if let Some(last) = self.last_event {
                    if frame.tick <= last {
                        return if frame.tick == last {
                            Err(BlockError::DuplicateObservation { part, tick: frame.tick })
                        } else {
                            Err(BlockError::TickWentBackwards { part, from: last, to: frame.tick })
                        };
                    }
                }
            }
        }

        // frame-order contract byte order: nondecreasing by own tick, state first at equal ticks.
        if let Some((previous_tick, previous_part)) = self.last_frame {
            if frame.tick < previous_tick {
                return Err(BlockError::TickWentBackwards {
                    part,
                    from: previous_tick,
                    to: frame.tick,
                });
            }
            if frame.tick == previous_tick
                && matches!(part, Part::State)
                && matches!(previous_part, Part::Event)
            {
                return Err(BlockError::EventBeforeStateAtSameTick { tick: frame.tick });
            }
        }

        // Validation is complete; commit.
        match part {
            Part::State => {
                if matches!(frame.kind, Kind::Baseline) {
                    self.seen_baseline = true;
                }
                self.last_state = Some(last_covered);
                self.last_state_kind = Some(frame.kind);
            }
            Part::Event => self.last_event = Some(frame.tick),
        }
        self.last_frame = Some((frame.tick, part));
        self.started = true;
        Ok(())
    }
}

/// Writes a block, enforcing frame contract's invariants rather than trusting the caller.
pub struct BlockWriter {
    header: BlockHeader,
    stride: u64,
    frames: Vec<Frame>,
    validator: BlockValidator,
}

impl BlockWriter {
    pub fn new(header: BlockHeader, stride: u64) -> Self {
        let validator = BlockValidator::new(header.first_tick, stride);
        Self { header, stride: stride.max(1), frames: Vec::new(), validator }
    }

    pub fn push(&mut self, frame: Frame) -> core::result::Result<(), BlockError> {
        self.validator.accept(&frame)?;
        self.frames.push(frame);
        Ok(())
    }

    pub fn finish(&self) -> core::result::Result<Vec<u8>, BlockError> {
        if self.frames.is_empty() {
            return Err(BlockError::EmptyBlock);
        }
        let mut out = Vec::new();
        self.header.encode(&mut out);
        let mut last_state = self.header.first_tick;
        let mut last_event = self.header.first_tick;
        let mut state_started = false;
        let mut event_started = false;

        for frame in &self.frames {
            let (base, started) = match frame.kind.part() {
                Part::State => (last_state, state_started),
                Part::Event => (last_event, event_started),
            };
            // Each part's first frame measures from first_tick; the opening frame
            // of the block therefore carries 0.
            let delta = if started { frame.tick - base } else { frame.tick - self.header.first_tick };
            let tag = pack_tag(delta, frame.kind)
                .map_err(|_| BlockError::TickDeltaTooLarge { delta })?;

            let mut body = Vec::new();
            write_varint(&mut body, tag);
            if matches!(frame.kind, Kind::Unchanged) {
                write_varint(&mut body, frame.run_len);
            } else {
                body.extend_from_slice(&frame.body);
            }
            write_varint(&mut out, body.len() as u64);
            out.extend_from_slice(&body);

            match frame.kind.part() {
                Part::State => {
                    // `push` already proved this endpoint fits.
                    last_state = frame
                        .last_covered_tick_checked(self.stride)
                        .ok_or(BlockError::TickOverflow { tick: frame.tick })?;
                    state_started = true;
                }
                Part::Event => {
                    last_event = frame.tick;
                    event_started = true;
                }
            }
        }
        Ok(out)
    }
}

/// Reads a block back, reconstructing both chains and enforcing the same frame contract
/// invariants the writer does — see `BlockValidator`. A block that decodes here
/// is one `BlockWriter` could have produced.
pub fn read_block(buf: &[u8], stride: u64) -> Result<(BlockHeader, Vec<Frame>)> {
    let stride = stride.max(1);
    let (header, mut pos) = BlockHeader::decode(buf, 0)?;
    let mut frames = Vec::new();
    let mut last_state = header.first_tick;
    let mut last_event = header.first_tick;
    let mut state_started = false;
    let mut event_started = false;
    let mut validator = BlockValidator::new(header.first_tick, stride);

    while pos < buf.len() {
        let (len, after_len) = read_varint(buf, pos)?;
        let end = after_len
            .checked_add(len as usize)
            .ok_or(WireError::Corrupt("frame length overflows"))?;
        if end > buf.len() {
            return Err(WireError::Truncated);
        }
        // Every varint a frame owns must terminate inside that frame, or a short
        // `len` lets a corrupt stream read a neighbour's bytes.
        let frame_slice = &buf[after_len..end];
        let (tag, tag_len) = read_varint(frame_slice, 0)?;
        let after_tag = after_len + tag_len;
        let (delta, kind) = unpack_tag(tag)?;

        let base = match kind.part() {
            Part::State => (last_state, state_started),
            Part::Event => (last_event, event_started),
        };
        // `delta` is an untrusted varint up to 2^61; the base is whatever the
        // stream has built up. Unchecked, this panics under `overflow-checks`
        // and wraps without them.
        let tick = if base.1 { base.0 } else { header.first_tick }
            .checked_add(delta)
            .ok_or(WireError::Corrupt("frame tick overflows"))?;

        let mut frame = Frame { kind, tick, run_len: 1, body: Vec::new() };
        if matches!(kind, Kind::Unchanged) {
            let (run_len, consumed) = read_varint(frame_slice, tag_len)?;
            if run_len == 0 {
                return Err(WireError::Corrupt("UNCHANGED run length must be at least 1"));
            }
            // Trailing bytes inside an UNCHANGED frame would be a second encoding
            // of the same run.
            if after_len + consumed != end {
                return Err(WireError::Corrupt("UNCHANGED frame carries trailing bytes"));
            }
            frame.run_len = run_len;
        } else if !kind.is_reserved() {
            frame.body = buf[after_tag..end].to_vec();
        }

        validator
            .accept(&frame)
            .map_err(|_| WireError::Corrupt("block violates a frame contract invariant"))?;

        match kind.part() {
            Part::State => {
                last_state = frame
                    .last_covered_tick_checked(stride)
                    .ok_or(WireError::Corrupt("UNCHANGED run endpoint overflows"))?;
                state_started = true;
            }
            Part::Event => {
                last_event = frame.tick;
                event_started = true;
            }
        }
        frames.push(frame);
        pos = end;
    }
    // block-header contract: `first_tick` is the tick of the block's first frame, so a header with
    // no frames names a tick nothing occupies.
    if frames.is_empty() {
        return Err(WireError::Corrupt("block carries no frames"));
    }
    Ok((header, frames))
}

/// A domain's sample stride on the container's grid, tick-grid contract. Takes a `TickGrid`
/// rather than a raw exponent so the shift cannot be handed an unrepresentable
/// one.
pub fn stride_for(schema: &Schema, grid: crate::TickGrid) -> u64 {
    grid.sample_stride(schema.cadence_log2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> BlockHeader {
        BlockHeader { domain: 20, branch: 0, schema_hash: 0xDEAD_BEEF_CAFE_F00D, first_tick: 0 }
    }

    #[test]
    fn tag_packing_is_collision_free_for_every_kind() {
        // zigzag(-1)|kind=0 both yielded 1.
        let mut seen = Vec::new();
        for delta in 0..4u64 {
            for code in 0..8u64 {
                let kind = Kind::from_code(code).unwrap();
                let tag = pack_tag(delta, kind).unwrap();
                assert!(!seen.contains(&tag), "tag {tag} collides");
                seen.push(tag);
                assert_eq!(unpack_tag(tag).unwrap(), (delta, kind));
            }
        }
    }

    #[test]
    fn one_byte_tag_holds_delta_up_to_fifteen() {
        // tick-grid contract's byte arithmetic: a one-byte varint holds 127, and the low three
        // bits are the kind, so tick_delta <= 15.
        let mut out = Vec::new();
        write_varint(&mut out, pack_tag(15, Kind::Baseline).unwrap());
        assert_eq!(out.len(), 1);
        out.clear();
        write_varint(&mut out, pack_tag(16, Kind::Baseline).unwrap());
        assert_eq!(out.len(), 2, "16 needs a second tag byte");
    }

    #[test]
    fn reserved_kinds_are_assigned_to_a_part() {
        assert_eq!(Kind::ReservedState4.part(), Part::State);
        assert_eq!(Kind::ReservedState5.part(), Part::State);
        assert_eq!(Kind::ReservedEvent6.part(), Part::Event);
        assert_eq!(Kind::ReservedEvent7.part(), Part::Event);
    }

    #[test]
    fn a_skipped_reserved_frame_keeps_both_chains() {
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame { kind: Kind::ReservedState4, tick: 1, run_len: 1, body: vec![9, 9] })
            .unwrap();
        w.push(Frame::state(Kind::Delta, 2, vec![2])).unwrap();
        let bytes = w.finish().unwrap();
        let (_, frames) = read_block(&bytes, 1).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2].tick, 2, "the frame after an unreadable one still lands at 2");
    }

    #[test]
    fn the_two_chains_can_start_at_different_ticks() {
        // block with a late first event unframeable.
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame::events(500, vec![7])).unwrap();
        let bytes = w.finish().unwrap();
        let (h, frames) = read_block(&bytes, 1).unwrap();
        assert_eq!(h.first_tick, 0);
        assert_eq!(frames[0].tick, 0);
        assert_eq!(frames[1].tick, 500, "the event chain measures from first_tick");
    }

    #[test]
    fn an_unchanged_run_spans_ticks_that_carry_events() {
        // part-independence contract: the parts are independent, so an event does not break a run.
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame::unchanged(1, 10)).unwrap();
        w.push(Frame::events(5, vec![7])).unwrap();
        let bytes = w.finish().unwrap();
        let (_, frames) = read_block(&bytes, 1).unwrap();
        assert_eq!(frames[1].run_len, 10);
        assert_eq!(frames[1].last_covered_tick_checked(1).unwrap(), 10);
        assert_eq!(frames[2].tick, 5, "the event sits inside the run's span");
    }

    #[test]
    fn run_length_is_counted_in_sample_ticks() {
        // frame-run contract with a stride of 8: 3 samples cover ticks 0, 8, 16.
        let f = Frame::unchanged(0, 3);
        assert_eq!(f.last_covered_tick_checked(8).unwrap(), 16);
        // The next frame's delta measures from 16, not from 2.
        let mut w = BlockWriter::new(
            BlockHeader { first_tick: 0, ..header() },
            8,
        );
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame::unchanged(8, 3)).unwrap(); // covers 8, 16, 24
        w.push(Frame::state(Kind::Delta, 32, vec![2])).unwrap();
        let bytes = w.finish().unwrap();
        let (_, frames) = read_block(&bytes, 8).unwrap();
        assert_eq!(frames[1].last_covered_tick_checked(8).unwrap(), 24);
        assert_eq!(frames[2].tick, 32);
    }

    #[test]
    fn delta_before_baseline_is_rejected() {
        let mut w = BlockWriter::new(header(), 1);
        assert_eq!(
            w.push(Frame::state(Kind::Delta, 0, vec![1])),
            Err(BlockError::DeltaBeforeBaseline { tick: 0 })
        );
        let mut w2 = BlockWriter::new(header(), 1);
        assert_eq!(
            w2.push(Frame::unchanged(0, 2)),
            Err(BlockError::DeltaBeforeBaseline { tick: 0 })
        );
    }

    #[test]
    fn time_adjacent_unchanged_runs_are_rejected() {
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame::unchanged(1, 2)).unwrap(); // covers 1, 2
        assert_eq!(
            w.push(Frame::unchanged(3, 2)),
            Err(BlockError::AdjacentUnchangedRuns { first: 2, second: 3 }),
            "runs are maximal, so these must be one run"
        );
    }

    #[test]
    fn runs_separated_by_a_gap_are_legal() {
        // Reduced cadence: a gap terminates the run, and the next observation is a
        // BASELINE (frame-run contract), so two runs are separated by that baseline in time.
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame::unchanged(1, 2)).unwrap();
        w.push(Frame::state(Kind::Baseline, 5, vec![1])).unwrap();
        w.push(Frame::unchanged(6, 2)).unwrap();
        assert_eq!(w.finish().unwrap().is_empty(), false);
    }

    #[test]
    fn one_observation_per_tick_per_part() {
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        assert_eq!(
            w.push(Frame::state(Kind::Delta, 0, vec![2])),
            Err(BlockError::DuplicateObservation { part: Part::State, tick: 0 })
        );
    }

    #[test]
    fn state_precedes_events_at_the_same_tick() {
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, vec![1])).unwrap();
        w.push(Frame::events(4, vec![7])).unwrap();
        assert_eq!(
            w.push(Frame::state(Kind::Delta, 4, vec![2])),
            Err(BlockError::EventBeforeStateAtSameTick { tick: 4 })
        );
    }

    #[test]
    fn header_roundtrips() {
        let h = header();
        let mut out = Vec::new();
        h.encode(&mut out);
        let (back, pos) = BlockHeader::decode(&out, 0).unwrap();
        assert_eq!(back, h);
        assert_eq!(pos, out.len());
    }

    #[test]
    fn a_year_static_domain_costs_four_bytes_of_frame() {
        // frame-run contract's worked figure, at ticks_per_hour_log2 = 0 so the tag is one byte.
        let mut w = BlockWriter::new(header(), 1);
        w.push(Frame::state(Kind::Baseline, 0, Vec::new())).unwrap();
        let baseline_only = w.finish().unwrap().len();
        w.push(Frame::unchanged(1, 8759)).unwrap();
        let with_run = w.finish().unwrap().len();
        assert_eq!(with_run - baseline_only, 4, "len + tag + 2-byte run length");
    }
}
