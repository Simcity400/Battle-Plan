//! Admission predicates and bounds shared by the extractor (producer) and the
//! replay interface (consumer).
//!
//! # Why this crate exists
//!
//! `extractor/native` and `replay_interface/native` are two separate cargo
//! workspaces that deliberately share no runtime dependency: the `.battleplan`
//! file is their sole runtime interface. The consequence is that every
//! admission rule -- the condition deciding whether a record is kept -- has
//! historically been written TWICE, once where the row is produced and once
//! where replay re-validates it, with nothing forcing the two copies to agree.
//!
//! That has cost real defects. Twice, a lane widened a producer-side rule, the
//! data was then collected correctly, and the replay side re-applied the
//! identical superseded rejection and discarded it. The producer-only fix
//! "worked" and produced nothing.
//!
//! A rule enforced in one place and not its sibling is not enforced. This crate
//! is the one place. It is path-depended on by both workspaces, so a change to
//! a value here lands on both sides by construction.
//!
//! # What belongs here
//!
//! Only values that are genuinely duplicated ACROSS the workspace boundary: a
//! bound or constant that the producer applies when admitting a row and that
//! replay re-applies when reading it back. Per-domain rules that only one side
//! can evaluate (they need collector-only engine state, or replay-only
//! cross-domain join context) do not belong here and cannot be shared; see
//! `documents/` for the inventory of which are which.
//!
//! # What is NOT authority
//!
//! These are wire-contract and fail-closed sanity bounds. They are not
//! reverse-engineered engine constants and must not be cited as such. Where a
//! value derives from game content rather than from the engine binary it is
//! marked, because game content is mod-variable and a fixed bound is a known
//! limitation rather than a proven fact.

// ---------------------------------------------------------------------------
// Clausewitz calendar
// ---------------------------------------------------------------------------

/// Offset between the engine's stored absolute hour counter (`raw_date`) and
/// the replay schema's canonical sort key `hour_index`.
///
/// `hour_index = raw_date - DATE_EPOCH_CONST`, i.e.
/// `((year * 365 + day_of_year) * 24 + hour)` on the engine's 365-day, no-leap
/// calendar. See `replay_interface/documents/REPLAY_SCHEMA.md` section 0.3.
pub const DATE_EPOCH_CONST: i32 = 43_799_999;

/// Inclusive lower bound of the fail-closed sanity window for a stored engine
/// hour counter: the first hour of year 1850.
pub const RAW_DATE_MIN: i32 = (1850 * 365 * 24) + DATE_EPOCH_CONST;

/// Inclusive upper bound of the fail-closed sanity window for a stored engine
/// hour counter: the first hour of year 2200.
///
/// This bound is INCLUSIVE. That is not arbitrary: the extractor side pins the
/// inclusive reading with a boundary test that exercises `RAW_DATE_MAX - 1`,
/// `RAW_DATE_MAX` and `RAW_DATE_MAX + 1`
/// (`extractor/native/wire_format/src/bin/bench_real_d67.rs`), and every
/// producer copy of this window admits `RAW_DATE_MAX`.
pub const RAW_DATE_MAX: i32 = (2200 * 365 * 24) + DATE_EPOCH_CONST;

/// Whether a stored engine hour counter is inside the sanity window.
pub const fn raw_date_in_range(stored: i32) -> bool {
    stored >= RAW_DATE_MIN && stored <= RAW_DATE_MAX
}

/// Schema `hour_index` from the engine's stored hour counter, or `None` when
/// the raw value is outside the plausible calendar window (fail closed).
pub const fn hour_index_from_raw(stored: i32) -> Option<i64> {
    if !raw_date_in_range(stored) {
        return None;
    }
    Some((stored - DATE_EPOCH_CONST) as i64)
}

// ---------------------------------------------------------------------------
// Identity bounds
// ---------------------------------------------------------------------------

/// Upper bound (inclusive) on a country `tag_db_id` carried in a compact
/// record. Zero is the engine's absence marker and is never a valid owner, so
/// the admitted range is `1..=MAX_COMPACT_OWNER_TAG_DB_ID`.
pub const MAX_COMPACT_OWNER_TAG_DB_ID: u32 = 2_048;

/// Upper bound (inclusive) on a province id in the supply/attrition family of
/// domains. Zero is the engine's absence marker, so the admitted range is
/// `1..=SUPPLY_MAX_PROVINCE_ID`.
///
/// GAME CONTENT, NOT AN ENGINE CONSTANT. This is the vanilla province count.
/// A mod that adds provinces raises it, and rows above this bound are dropped
/// by both sides today. That is a known limitation of the current wire
/// contract, recorded here so the number has one home rather than six; it is
/// not evidence that the engine cannot produce a higher id.
pub const SUPPLY_MAX_PROVINCE_ID: u32 = 13_413;

/// Upper bound (inclusive) on an owner/originator `tag_db_id` carried by the
/// A19 delayed-event lifecycle ledger (D64's `DEL2` trailer).
///
/// DELIBERATELY WIDER THAN [`MAX_COMPACT_OWNER_TAG_DB_ID`] and NOT unified with
/// it. The lifecycle rows are read straight off `CDelayedEvent+0x18`/`+0xC0`
/// with no country-table binding step, so their producer ceiling is a
/// fail-closed plausibility bound rather than the compact-record identity
/// contract that `MAX_COMPACT_OWNER_TAG_DB_ID` states. This constant is that
/// producer ceiling, moved here so replay applies the SAME one: before this
/// existed the producer capped at 100_000 and replay applied no upper bound at
/// all, which is the fail-open half of the same drift.
///
/// Narrowing this to the compact identity ceiling is a separate decision that
/// needs evidence about the two raw fields, not a tidy-up.
pub const MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID: u32 = 100_000;

// ---------------------------------------------------------------------------
// Cardinality bounds
// ---------------------------------------------------------------------------

/// Upper bound (inclusive) on the number of rows in one hour's A19
/// delayed-event lifecycle ledger.
///
/// PRODUCER-DERIVED. This is the collector's reservable lifecycle queue
/// capacity (`MAX_PENDING_EVENTS` in
/// `extractor/native/lanes/scripted_state_flags_variables_live/src/a729_delayed_event_lifecycle.rs`):
/// an enqueue that would exceed it is refused and charged to `dropped_overflow`,
/// which clears the ledger's `complete` flag. One drain therefore cannot carry
/// more rows than this, and the ledger is per-hour, not cumulative — so the
/// value is an exact statement of what the producer can emit, and admitting
/// more at replay only widens what a corrupted file can spend.
pub const MAX_DELAYED_EVENT_LIFECYCLE_EVENTS: usize = 65_536;

/// Upper bound (inclusive) on the number of rows in D45's cumulative direct
/// FRD transition history.
///
/// UNLIKE the delayed-event ledger, this history is CUMULATIVE: the fold
/// appends every drain's rows to one vector that is re-emitted whole every
/// hour. It has no natural producer ceiling, so producer and consumer have to
/// agree on an artificial one or the two disagree by construction. They did:
/// replay rejected the whole history above 1_000_000 rows while the producer
/// appended without limit, so a long enough campaign built a history that
/// replay discarded in full — and discarded silently, at replay, long after
/// the recording could be redone.
///
/// This is a fail-closed capacity bound, NOT a reverse-engineered engine
/// constant. Its one requirement is that both sides spell it the same, and
/// that the producer refuses at the ceiling instead of sailing past it: a
/// recorder that stops with a named error is recoverable, a recording that is
/// silently unreadable is not.
pub const MAX_FRD_DIRECT_TRANSITIONS: usize = 1_000_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_index_matches_the_replay_schema_worked_example() {
        // REPLAY_SCHEMA section 0.3: 1940.9.2 hour 21 -> 17_000_277.
        let doy_index = 31 + 28 + 31 + 30 + 31 + 30 + 31 + 31 + 1;
        let hour_index = (1940i64 * 365 + doy_index) * 24 + 21;
        assert_eq!(hour_index, 17_000_277);
        let raw = DATE_EPOCH_CONST + hour_index as i32;
        assert_eq!(hour_index_from_raw(raw), Some(hour_index));
    }

    #[test]
    fn the_sanity_window_is_closed_at_both_ends() {
        assert_eq!(hour_index_from_raw(RAW_DATE_MIN - 1), None);
        assert!(hour_index_from_raw(RAW_DATE_MIN).is_some());
        // The upper bound is INCLUSIVE. A copy of this window that spells the
        // upper comparison `>=` rejects exactly this one value while every
        // other copy admits it -- which is the drift this crate exists to make
        // impossible.
        assert!(hour_index_from_raw(RAW_DATE_MAX).is_some());
        assert_eq!(hour_index_from_raw(RAW_DATE_MAX + 1), None);
    }

    #[test]
    fn out_of_range_raw_dates_fail_closed() {
        assert_eq!(hour_index_from_raw(0), None);
        assert_eq!(hour_index_from_raw(i32::MIN), None);
        assert_eq!(hour_index_from_raw(i32::MAX), None);
    }

    /// The shared bounds are pinned ABSOLUTELY, not relative to each other or
    /// to the callers that read them. A test that spells the bound as the
    /// constant it is testing passes for every value the constant could take,
    /// which is exactly the failure it exists to catch: the number itself is
    /// the cross-workspace contract.
    #[test]
    fn the_shared_bounds_are_pinned_to_their_exact_values() {
        assert_eq!(MAX_COMPACT_OWNER_TAG_DB_ID, 2_048);
        assert_eq!(SUPPLY_MAX_PROVINCE_ID, 13_413);
        assert_eq!(MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID, 100_000);
        assert_eq!(MAX_DELAYED_EVENT_LIFECYCLE_EVENTS, 65_536);
        assert_eq!(MAX_FRD_DIRECT_TRANSITIONS, 1_000_000);
    }

    /// The delayed-event lifecycle ceiling is deliberately NOT the compact
    /// identity ceiling. Collapsing the two is a decision that needs evidence
    /// about `CDelayedEvent+0x18`/`+0xC0`; doing it as a tidy-up would silently
    /// narrow what the producer already emits.
    #[test]
    fn the_delayed_lifecycle_tag_ceiling_is_not_silently_unified() {
        assert!(MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID > MAX_COMPACT_OWNER_TAG_DB_ID);
    }
}
