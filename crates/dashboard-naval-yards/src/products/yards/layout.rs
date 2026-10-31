//! The yard views' publication on the series engine, which their emit writes and their readers
//! read: each view its own family, keyed alike.
//!
//! An actor is an owner, named by its `TAG@slot`; the family's own keys are [`FAMILY`]'s. Every
//! key is exact. An owner's streams ([`HOLDERS`], [`SHIPS`], [`TOTALS`], [`REFUSALS`]) are
//! events: a [`kind::KEYFRAME`] of every row the stream holds, or a [`kind::DELTA`] of the rows
//! changed since the event before, whose first cell is its keyframe's hour. A chain is a
//! keyframe and at most `CHAIN - 1` deltas, and a stream whose coverage starts writes a keyframe.
//! Hours nothing changed at write nothing. An event's read hour is its hour where the view's
//! dating table was read there.
//!
//! A row's first cell is its key, Unsigned, and its second whether it is held, Boolean; a row
//! a delta removes holds Null in every other cell. Every value is Integer, as the observer reads
//! it, or Null where it reads none: never zero for missing. A label is the Unsigned number of a
//! token ([`TOKEN`]), or Null. Rows hold only within [`OBSERVED`]'s known hours.

/// The families' keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of a family's own keys.
pub const FAMILY: u64 = u64::MAX;

/// The family's keys. OBSERVED: evidence, known where the hour's countries and their links are
/// complete ([`reason::COUNTRIES`] where not). SOURCE, split by each source table's id:
/// evidence, known where the table holds its own complete rows, its supplying read dated where
/// the table's reads are, else unknown for the number of the token spelling why. TOKEN, split
/// by token number: one [`kind::TOKEN`] event, at the hour the branch first numbered it. READS:
/// the dating table's read schedule, a run of its read period (1 hourly, 24 daily) from a read
/// off schedule, ended where a read due is missed; the last read at an hour within it is the
/// latest hour of its period's step from the run's first, and none outside it.
pub const OBSERVED: u64 = 0;
pub const SOURCE: u64 = 1;
pub const TOKEN: u64 = 2;
pub const READS: u64 = 3;

/// An owner's streams. HOLDERS: its repair ports, by province, or its refit lines, by line
/// handle. SHIPS: the ships its ports or lines hold, by full ship handle, each naming its port
/// or line. TOTALS: one row, key 0. REFUSALS: why a value is unavailable, by the reason's token,
/// with its count.
pub const HOLDERS: u64 = 0;
pub const SHIPS: u64 = 1;
pub const TOTALS: u64 = 2;
pub const REFUSALS: u64 = 3;

/// Event kinds.
pub mod kind {
    /// `[rows…]`: every row of the stream.
    pub const KEYFRAME: u64 = 0;
    /// `[Integer keyframe hour, rows…]`: the rows changed since the event before.
    pub const DELTA: u64 = 1;
    /// `[Text operand]`: the token's naming operand, as JSON.
    pub const TOKEN: u64 = 2;
}

/// Why OBSERVED is unknown.
pub mod reason {
    /// Countries or CountryLinks does not hold every row, so no owner resolves.
    pub const COUNTRIES: u64 = 1;
}

/// Events in a chain, its keyframe's included.
pub const CHAIN: u8 = 16;

/// A repair port: its key is its province.
pub mod port {
    pub const STATE: usize = 2;
    pub const NAME: usize = 3;
    pub const PRIORITY: usize = 4;
    pub const DOCKYARDS: usize = 5;
    pub const POLICY: usize = 6;
    pub const QUEUE_COUNT: usize = 7;
    /// List, flat: each entry's Integer ordinal then Integer ship, in ordinal order, repeats kept.
    pub const QUEUE: usize = 8;
    pub const CELLS: usize = 9;
}

/// A refit line: its key is its handle.
pub mod line {
    pub const LIVE: usize = 2;
    pub const PROGRESS: usize = 3;
    pub const COST: usize = 4;
    pub const DOCKYARDS: usize = 5;
    pub const TARGET: usize = 6;
    pub const TARGET_NAME: usize = 7;
    pub const REMAINING: usize = 8;
    pub const CELLS: usize = 9;
}

/// A ship of a port's queue or a line: its key is its full handle.
pub mod ship {
    /// Unsigned: the province of the port whose queue holds it, the lowest where several do
    /// (each port's queue lists it), or the handle of its line.
    pub const HOLDER: usize = 2;
    pub const HULL: usize = 3;
    pub const MAXIMUM: usize = 4;
    pub const DESIGN: usize = 5;
    pub const DESIGN_NAME: usize = 6;
    pub const CLASS: usize = 7;
    pub const TASK: usize = 8;
    pub const TASK_NAME: usize = 9;
    pub const REPAIR_MODE: usize = 10;
    pub const POLICY: usize = 11;
    pub const REPAIR_SPLIT: usize = 12;
    pub const REPAIR_TARGET: usize = 13;
    pub const REJOIN: usize = 14;
    pub const REJOIN_RESOLVES: usize = 15;
    pub const REFIT_INTENT: usize = 16;
    pub const REFIT_INTENT_NAME: usize = 17;
    pub const AFTER_REPAIR_INTENT: usize = 18;
    pub const AFTER_REPAIR_INTENT_NAME: usize = 19;
    /// The token of the name the game holds for it.
    pub const NAME: usize = 20;
    /// The token of a name rebuilt from its naming operands, where the game holds none.
    pub const NAME_REBUILT: usize = 21;
    /// List, flat: each child's Integer ordinal, Integer child and Integer resolves or Null, in
    /// ordinal order.
    pub const CHILDREN: usize = 22;
    pub const CELLS: usize = 23;
}

/// An owner's repair totals.
pub mod repair_totals {
    pub const USED_DOCKYARDS: usize = 2;
    pub const REPAIR_LIMIT: usize = 3;
    pub const BUILDING_REPAIR_LIMIT: usize = 4;
    pub const DOCKYARDS_RAW: usize = 5;
    pub const DOCKYARDS: usize = 6;
    pub const QUEUE_CENSUS_COMPLETE: usize = 7;
    pub const QUEUE_ENTRIES: usize = 8;
    pub const DISTINCT_QUEUED_REFERENCES: usize = 9;
    pub const DISTINCT_QUEUED_SHIPS: usize = 10;
    pub const CELLS: usize = 11;
}

/// An owner's refit totals.
pub mod refit_totals {
    pub const CENSUS_COMPLETE: usize = 2;
    pub const SHIPS: usize = 3;
    pub const LINES: usize = 4;
    pub const DOCKYARDS: usize = 5;
    pub const REMAINING: usize = 6;
    pub const CELLS: usize = 7;
}

/// A refusal: its key is its reason's token.
pub mod refusal {
    pub const COUNT: usize = 2;
    pub const CELLS: usize = 3;
}
