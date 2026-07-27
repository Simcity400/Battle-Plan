//! Reference implementation of `extractor/documents/WIRE_FORMAT.md`.
//!
//! The specification is the authority; this crate exists to make it executable,
//! and its golden vectors are what turn "two implementers produce identical bytes"
//! from a claim into a gate. Where code and document disagree, the document wins
//! and the code is wrong.
//!
//! Section numbers in comments refer to that document.
//!
//! **Scope.** §0–§6 and §10–§11 are implemented, and §7 for **keyed-set state
//! bodies only** — the ordered shapes of §7.3 are rejected rather than encoded
//! on both the write and the read side.
//! §8 gap normalisation and §9 event bodies and the event frontier have no code
//! here at all. Nothing in this crate should be read as evidence that any of
//! those three work.

pub mod bitmap;
pub mod body;
pub mod columnar;
pub mod d10;
pub mod d11;
pub mod d13;
pub mod d16;
pub mod d17;
pub mod d20;
pub mod d22;
pub mod d23;
pub mod d24;
pub mod d26;
pub mod d27;
pub mod d28;
pub mod d29;
pub mod d31;
pub mod d32;
pub mod d34;
pub mod d35;
pub mod d36;
pub mod d38;
pub mod d39;
pub mod d40;
pub mod d49;
pub mod d50;
pub mod d56;
pub mod d61;
pub mod d62;
pub mod d63;
pub mod d67;
pub mod d68;
pub mod d70;
pub mod d83;
pub mod d92;
pub mod d94;
pub mod derived;
pub mod frame;
pub mod record;
pub mod schema;
pub mod update;
pub mod value;
pub mod variant_upgrades;
pub mod varint;

pub use bitmap::Bitmap;
pub use schema::{
    Collection, Derived, EventPart, Field, Kind, Quant, Schema, SchemaError, Shape,
};
pub use frame::{BlockError, BlockHeader, BlockWriter, Frame, Part};
pub use record::Record;
pub use value::Value;
pub use varint::{Result, WireError};

/// §1.1. The container declares this once; every timestamp in the format is a
/// tick index from campaign hour 0, and a domain's cadence says only how often it
/// samples — not what unit time is measured in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickGrid {
    ticks_per_hour_log2: u8,
}

impl TickGrid {
    pub const HOURLY: Self = Self { ticks_per_hour_log2: 0 };

    /// §1.1 counts ticks in a `u64`, so the grid exponent must be representable.
    pub const MAX_LOG2: u8 = 63;

    /// The field is private and this is the only constructor, so an out-of-range
    /// grid cannot exist. `Schema::validate` rejecting 64 was not enough on its
    /// own: these helpers shift by whatever they are handed, and a caller that
    /// built the grid directly panicked before any schema was validated.
    pub fn new(ticks_per_hour_log2: u8) -> Result<Self> {
        if ticks_per_hour_log2 > Self::MAX_LOG2 {
            return Err(WireError::Corrupt("ticks_per_hour_log2 exceeds 63"));
        }
        Ok(Self { ticks_per_hour_log2 })
    }

    pub fn log2(self) -> u8 {
        self.ticks_per_hour_log2
    }

    pub fn ticks_per_hour(self) -> u64 {
        1u64 << self.ticks_per_hour_log2
    }

    /// §1.1: a domain samples on ticks that are multiples of its stride.
    pub fn sample_stride(self, cadence_log2: u8) -> u64 {
        1u64 << (self.ticks_per_hour_log2.saturating_sub(cadence_log2))
    }

    /// The frame hour a `derived` field's shift resolves to, §6. A shift past the
    /// grid is not a legal declaration (`Schema::validate`), but this is public,
    /// so it saturates to zero rather than shifting a `u64` by 64 or more.
    pub fn shifted_clock(self, tick: u64, shift: u8) -> u64 {
        if shift > Self::MAX_LOG2 {
            0
        } else {
            tick >> shift
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_domain_on_an_hourly_grid_has_a_stride_of_one() {
        assert_eq!(TickGrid::HOURLY.sample_stride(0), 1);
    }

    #[test]
    fn a_finer_grid_leaves_an_hourly_domain_striding_a_whole_hour() {
        let grid = TickGrid::new(3).unwrap();
        assert_eq!(grid.sample_stride(0), 8, "hourly domain samples every 8 ticks");
        assert_eq!(grid.sample_stride(3), 1, "a 8/hour domain samples every tick");
        assert_eq!(grid.sample_stride(1), 4, "a 2/hour domain samples every 4 ticks");
    }

    #[test]
    fn the_derived_shift_recovers_the_game_hour() {
        // §6: u = ticks_per_hour_log2 makes the term the game hour, which is what
        // makes d64's due-hour transform constant. At u = 0 it would drift by
        // (ticks_per_hour - 1) every hour.
        let grid = TickGrid::new(2).unwrap();
        assert_eq!(grid.shifted_clock(0, 2), 0);
        assert_eq!(grid.shifted_clock(3, 2), 0, "still hour 0");
        assert_eq!(grid.shifted_clock(4, 2), 1, "hour 1 begins at tick 4");
        assert_eq!(grid.shifted_clock(8, 2), 2);

        // The failure the shift exists to prevent: an hour-valued countdown with
        // u = 0 is not constant across an hour boundary.
        let remaining_at_tick_0: i64 = 10;
        let remaining_at_tick_4: i64 = 9; // decremented once per game hour
        let with_shift = |tick: u64, remaining: i64| remaining + grid.shifted_clock(tick, 2) as i64;
        let without_shift = |tick: u64, remaining: i64| remaining + tick as i64;
        assert_eq!(
            with_shift(0, remaining_at_tick_0),
            with_shift(4, remaining_at_tick_4),
            "with the shift the stored value is constant, so UNCHANGED fires"
        );
        assert_ne!(
            without_shift(0, remaining_at_tick_0),
            without_shift(4, remaining_at_tick_4),
            "without it the row deltas every sample forever"
        );
    }
}
