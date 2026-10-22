//! Hourly equipment of divisions minted since the daily `DivisionEquipment`
//! walk. A division handle is minted either from the unit id counter or with
//! a dynamic type from the reference registry's counter. Both counters only
//! grow, so "not in the last daily walk" is "id past the counter of its handle
//! type at that walk": a few divisions an hour. A unit mint takes the counter's
//! value before incrementing it, so the counter's own value is the next unit
//! id and is new; the registry counter is incremented first, so its value is
//! already taken.
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use engine_native::{Memory, cequipment_variant_pool as pool, cship, walk::Outcome};
use replay_data::{rows::DivisionEquipmentRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NewDivisionEquipment];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// The two id counters at the last daily walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counters {
    /// Unit id counter, for unit handles.
    pub unit: u32,
    /// Reference registry counter, for every dynamic type.
    pub registry: u32,
}

impl Counters {
    /// Whether a division handle (type low, id high) was minted after these.
    pub fn minted_after(self, handle: u64) -> bool {
        let id = (handle >> 32) as u32;
        if handle as u32 == cship::HANDLE_TYPE {
            id >= self.unit
        } else {
            id > self.registry
        }
    }

    fn pack(self) -> u64 {
        (u64::from(self.unit) << 32) | u64::from(self.registry)
    }

    fn unpack(word: u64) -> Option<Self> {
        (word != UNKNOWN).then_some(Self {
            unit: (word >> 32) as u32,
            registry: word as u32,
        })
    }
}

/// The stored word before any daily walk, or after one whose counters were
/// unreadable.
pub const UNKNOWN: u64 = u64::MAX;

/// Which divisions this hour's table covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    /// No daily walk with readable counters has run on this attachment.
    #[default]
    Unknown,
    /// The daily walk runs this hour and holds every division.
    Daily,
    /// Divisions minted after the counters of the last daily walk.
    Above(Counters),
}

/// The hook's per-hour step: a daily hour stores this hour's counters, any
/// other hour reads the stored ones. A signed unit counter below zero is
/// unreadable, never a mark.
pub fn next_mark(
    stored: &AtomicU64,
    daily: bool,
    unit: Option<i32>,
    registry: Option<u32>,
) -> Mark {
    if daily {
        let counters = unit
            .and_then(|unit| u32::try_from(unit).ok())
            .zip(registry)
            .map(|(unit, registry)| Counters { unit, registry });
        stored.store(counters.map_or(UNKNOWN, Counters::pack), Ordering::Relaxed);
        return Mark::Daily;
    }
    Counters::unpack(stored.load(Ordering::Relaxed)).map_or(Mark::Unknown, Mark::Above)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Capacities {
    pub fn raise_for_equipment(&mut self, rows: usize) {
        self.rows = self.rows.max(rows);
    }
}

pub struct Captured {
    rows: CaptureRows<pool::DivisionRow>,
    mark: Mark,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            mark: Mark::Unknown,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
    }
    /// Set by the hook before the hour's walks; see [`Mark`].
    pub fn set_mark(&mut self, mark: Mark) {
        self.mark = mark;
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sources: &[pool::DivisionSource],
        source: Outcome,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        self.rows.outcome = match (self.mark, source) {
            (Mark::Unknown, _) => Outcome::Unavailable {
                reason: "no daily division equipment walk to compare against yet",
            },
            (Mark::Daily, _) => Outcome::Rows { len: 0 },
            (Mark::Above(_), Outcome::Unavailable { reason }) => Outcome::Unavailable { reason },
            (Mark::Above(_), Outcome::Truncated { .. }) => Outcome::Unavailable {
                reason: "division census incomplete",
            },
            (Mark::Above(counters), Outcome::Rows { len }) => match sources.get(..len) {
                Some(sources) => pool::walk_divisions_where_growing(
                    memory,
                    base,
                    sources,
                    |division| counters.minted_after(division.division),
                    &mut self.rows.rows,
                ),
                None => Outcome::Unavailable {
                    reason: "division census length invalid",
                },
            },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::NewDivisionEquipment, _>(
            self.rows.outcome,
            &self.rows.rows,
            |row| DivisionEquipmentRow {
                division: row.division,
                template: row.template,
                country: row.country,
                ordinal: row.ordinal,
                variant_handle: row.variant_handle,
                amount_raw: row.amount_raw,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_handle_type_is_compared_with_its_own_counter() {
        let counters = Counters {
            unit: 100,
            registry: 5_000,
        };
        let unit = |id: u64| (id << 32) | u64::from(cship::HANDLE_TYPE);
        let dynamic = |id: u64| (id << 32) | 0x1269;
        // The unit counter holds the next id to mint.
        assert!(!counters.minted_after(unit(99)));
        assert!(counters.minted_after(unit(100)));
        // A deployed division's dynamic id is far above the unit counter
        // without being new; only the registry counter decides it.
        assert!(!counters.minted_after(dynamic(4_000)));
        assert!(counters.minted_after(dynamic(5_001)));
    }

    #[test]
    fn only_a_known_mark_admits_rows_and_the_daily_hour_is_empty() {
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture(&memory, 0, &[], Outcome::Rows { len: 0 });
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.set_mark(Mark::Daily);
        captured.capture(&memory, 0, &[], Outcome::Unavailable { reason: "x" });
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 0 });
        captured.set_mark(Mark::Above(Counters {
            unit: 7,
            registry: 7,
        }));
        // Every division was minted before its counter: nothing is read.
        let old = pool::DivisionSource {
            pointer: 1,
            division: (6 << 32) | u64::from(cship::HANDLE_TYPE),
            ..Default::default()
        };
        captured.capture(&memory, 0, &[old], Outcome::Rows { len: 1 });
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 0 });
        captured.capture(
            &memory,
            0,
            &[old],
            Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
        );
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }
}
