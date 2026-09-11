//! World-tension acquisition: the CWorldThreat manager word, its ordered
//! contribution vector and its per-country aggregate cache.
//!
//! The hourly game speed is not a table here: `CGameState+0x4bc` is already read
//! by the authenticated clock read in the same guarded batch as the hour, so it
//! rides on the existing Clock row instead of paying a second read and a second
//! availability outcome. Pause is not recorded; see PORTING.md's D57 row.
//!
//! Only growth and replay conversion allocate.

use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cthreat_source, cworld_threat, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

/// Root authentication and both array headers are read once per pass and
/// charged to the manager row; the two vectors keep their own read times.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("world threat manager, sources and aggregates", 0, &[0, 1, 2])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub reason_bytes: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            // One manager row; a 1942 save carries 127 sources and about 90
            // aggregate slots, so both stay below the 75% growth threshold.
            rows: [2, 1024, 512],
            reason_bytes: 128 * 1024,
        }
    }
}

/// One acquisition pass: the three table outcomes plus its reason-arena use.
struct Pass {
    tables: [Outcome; N],
    bytes_len: usize,
    required_bytes: usize,
}

impl Pass {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            tables: [Outcome::Unavailable { reason }; N],
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

pub struct Captured {
    manager: CaptureRows<cworld_threat::ManagerRow>,
    sources: CaptureRows<cthreat_source::Row>,
    aggregates: CaptureRows<cworld_threat::AggregateRow>,
    reason_bytes: Arena<u8>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            manager: CaptureRows::new(capacity.rows[0], successful[0]),
            sources: CaptureRows::new(capacity.rows[1], successful[1]),
            aggregates: CaptureRows::new(capacity.rows[2], successful[2]),
            reason_bytes: Arena::new(capacity.reason_bytes, successful[1]),
        }
    }

    pub fn reset(&mut self) {
        self.manager.reset();
        self.sources.reset();
        self.aggregates.reset();
        self.reason_bytes.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.manager.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.aggregates.allocated_bytes()
            + self.reason_bytes.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.manager.grow(&mut capacity.rows[0], successful[0]);
        self.sources.grow(&mut capacity.rows[1], successful[1]);
        self.aggregates.grow(&mut capacity.rows[2], successful[2]);
        self.reason_bytes
            .grow(&mut capacity.reason_bytes, successful[1]);
    }

    /// The manager is resolved afresh for each pass, and its header is rechecked
    /// after the walk: a total or array header that moved mid-walk makes every
    /// row of this pass unavailable rather than publishing a mixed census.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];

        let begin = qpc();
        let first = self.pass(memory, base, game_state, false);
        self.manager.outcome = first.tables[0];
        self.sources.outcome = first.tables[1];
        self.aggregates.outcome = first.tables[2];
        self.reason_bytes.len = first.bytes_len;
        self.reason_bytes.required = first.required_bytes;
        ticks[0] = elapsed(begin);

        if needed.iter().any(|needed| *needed) {
            let begin = qpc();
            let second = self.pass(memory, base, game_state, true);
            // A changed reason arena is a source mismatch even when the fixed
            // fields of every row happen to be equal.
            let reasons_matched = self
                .reason_bytes
                .compare(second.bytes_len, second.required_bytes);
            self.manager.checked = needed[0];
            self.sources.checked = needed[1];
            self.aggregates.checked = needed[2];
            matched[0] = needed[0] && self.manager.compare(second.tables[0]);
            let sources_matched = self.sources.compare(second.tables[1]);
            if !reasons_matched {
                self.sources.mismatches += 1;
            }
            matched[1] = needed[1] && sources_matched && reasons_matched;
            matched[2] = needed[2] && self.aggregates.compare(second.tables[2]);
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
        }
        (matched, ticks)
    }

    /// One authenticated pass over all three tables into the chosen buffers.
    fn pass(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        second: bool,
    ) -> Pass {
        let reader = match cworld_threat::Reader::new(memory, base, game_state) {
            Ok(reader) => reader,
            Err(reason) => return Pass::unavailable(reason),
        };
        let (manager, sources, aggregates, reasons) = if second {
            (
                &mut self.manager.second,
                &mut self.sources.second,
                &mut self.aggregates.second,
                &mut self.reason_bytes.second,
            )
        } else {
            (
                &mut self.manager.rows,
                &mut self.sources.rows,
                &mut self.aggregates.rows,
                &mut self.reason_bytes.first,
            )
        };
        let manager = bounded(reader.manager(manager), manager.len());
        let result = reader.sources(memory, base, sources, reasons);
        let sources = bounded(result.outcome, sources.len());
        let aggregates = bounded(reader.aggregates(memory, aggregates), aggregates.len());
        if !reader.unchanged(memory) {
            return Pass::unavailable("world threat header changed during the walk");
        }
        Pass {
            tables: [manager, sources, aggregates],
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.manager),
            effective(&self.sources),
            effective(&self.aggregates),
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            checked(&self.manager),
            checked(&self.sources),
            checked(&self.aggregates),
        ]
    }

    /// Spool-thread conversion; raw operands and incomplete status survive.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::WorldTension, _>(outcomes[0], &self.manager.rows, |r| {
                rows::WorldTensionRow {
                    total_raw: r.total_raw,
                    source_count: r.source_count,
                    country_aggregate_count: r.country_aggregate_count,
                }
            }),
            sources(outcomes[1], &self.sources, &self.reason_bytes),
            table::<tables::ThreatCountryAggregates, _>(
                outcomes[2],
                &self.aggregates.rows,
                |r| rows::ThreatCountryAggregateRow {
                    country_slot: r.country_slot,
                    total_raw: r.total_raw,
                },
            ),
        ]
    }
}

fn sources(
    outcome: Outcome,
    rows_source: &CaptureRows<cthreat_source::Row>,
    arena: &Arena<u8>,
) -> TableView {
    if let Outcome::Unavailable { reason } = outcome {
        return unavailable(reason);
    }
    let Some(source) = rows_source.rows.get(..outcome.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    let Some(bytes) = arena.first.get(..arena.len) else {
        return unavailable("threat source reasons lie outside captured arena");
    };
    if source
        .iter()
        .any(|row| text(bytes, row.reason_start, row.reason_len).is_none())
    {
        return unavailable("threat source reasons lie outside captured arena");
    }
    table::<tables::ThreatSources, _>(outcome, source, |r| rows::ThreatSourceRow {
        ordinal: r.ordinal,
        country: r.country,
        second_country: r.second_country,
        current_raw: r.current_raw,
        final_raw: r.final_raw,
        daily_raw: r.daily_raw,
        date: r.date,
        reason: text(bytes, r.reason_start, r.reason_len).unwrap().to_vec(),
    })
}

fn text(arena: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    arena.get(start..start.checked_add(len)?)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "world threat table changed during double read",
        }
    } else {
        captured.outcome
    }
}

fn checked<R>(captured: &CaptureRows<R>) -> usize {
    if captured.checked {
        captured.mismatches
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, memory::SliceMemory, msvc_string};
    use replay::rows::TableSchema;
    use std::cell::Cell;

    const BASE: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const SOURCE_ARRAY: usize = 0x2100;
    const AGGREGATES: usize = 0x2200;
    const FIRST: usize = 0x2400;
    const SECOND: usize = 0x2500;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn source(bytes: &mut [u8], at: usize, current: i64, reason: &[u8]) {
        put(bytes, at, &(cthreat_source::VTABLE_RVA as u64).to_le_bytes());
        put(bytes, at + cthreat_source::COUNTRY, &u32::MAX.to_le_bytes());
        put(
            bytes,
            at + cthreat_source::SECOND_COUNTRY,
            &7_u32.to_le_bytes(),
        );
        put(bytes, at + cthreat_source::CURRENT, &current.to_le_bytes());
        put(bytes, at + cthreat_source::FINAL, &i64::MAX.to_le_bytes());
        put(bytes, at + cthreat_source::DAILY, &(-1_i64).to_le_bytes());
        put(bytes, at + cthreat_source::DATE, &i32::MIN.to_le_bytes());
        put(bytes, at + cthreat_source::REASON, reason);
        put(
            bytes,
            at + cthreat_source::REASON + msvc_string::LENGTH,
            &(reason.len() as u64 - 1).to_le_bytes(),
        );
        put(
            bytes,
            at + cthreat_source::REASON + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x2600];
        put(
            &mut bytes,
            BASE + cgame_state::WORLD_THREAT,
            &(MANAGER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER,
            &(cworld_threat::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER + cworld_threat::TOTAL,
            &(-125_682_i64).to_le_bytes(),
        );
        array(&mut bytes, MANAGER + cworld_threat::SOURCES, SOURCE_ARRAY, 2);
        array(
            &mut bytes,
            MANAGER + cworld_threat::COUNTRY_AGGREGATES,
            AGGREGATES,
            2,
        );
        put(&mut bytes, SOURCE_ARRAY, &(FIRST as u64).to_le_bytes());
        put(&mut bytes, SOURCE_ARRAY + 8, &(SECOND as u64).to_le_bytes());
        put(&mut bytes, AGGREGATES, &i64::MIN.to_le_bytes());
        put(&mut bytes, AGGREGATES + 8, &0_i64.to_le_bytes());
        source(&mut bytes, FIRST, i64::MIN, b"Peace\0");
        source(&mut bytes, SECOND, 0, b"\0");
        bytes
    }

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [1, 2, 2],
            reason_bytes: 8,
        };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete rows, got {view:?}")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn manager_sources_and_aggregates_keep_raw_operands_and_literal_reasons() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true; N]);
        assert_eq!(captured.checked_mismatches(), [0; N]);
        let tables = captured.tables();
        assert_eq!(
            complete::<tables::WorldTension>(&tables[0]),
            &[rows::WorldTensionRow {
                total_raw: -125_682,
                source_count: 2,
                country_aggregate_count: 2,
            }]
        );
        let sources = complete::<tables::ThreatSources>(&tables[1]);
        assert_eq!(sources.len(), 2);
        assert_eq!(
            (
                sources[0].ordinal,
                sources[0].country,
                sources[0].second_country,
                sources[0].current_raw,
                sources[0].final_raw,
                sources[0].daily_raw,
                sources[0].date,
            ),
            (0, u32::MAX, 7, i64::MIN, i64::MAX, -1, i32::MIN)
        );
        assert_eq!(sources[0].reason, b"Peace");
        assert_eq!(sources[1].reason, b"");
        assert_eq!(
            complete::<tables::ThreatCountryAggregates>(&tables[2]),
            &[
                rows::ThreatCountryAggregateRow {
                    country_slot: 0,
                    total_raw: i64::MIN,
                },
                rows::ThreatCountryAggregateRow {
                    country_slot: 1,
                    total_raw: 0,
                },
            ]
        );
    }

    #[test]
    fn capacity_truncation_and_a_broken_root_stay_distinct_and_grow_off_thread() {
        let (mut capacities, mut captured) = small();
        capacities.reason_bytes = 2;
        let mut captured_small = Captured::with_capacities(&capacities, &[0; N]);
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (matched, _) = captured_small.capture(&memory, 0, BASE, &[true; N]);
        // A truncated table never earns a consistency check, even when both
        // passes truncate identically.
        assert_eq!(matched, [true, false, true]);
        // The five-byte reason does not fit; its row is omitted and counted.
        assert_eq!(
            captured_small.outcomes()[1],
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert!(matches!(
            captured_small.tables()[1],
            TableView::Truncated { missing: 1, .. }
        ));
        captured_small.grow(&mut capacities, &[0; N]);
        assert!(capacities.reason_bytes >= 5);

        let mut broken = fixture();
        put(&mut broken, MANAGER, &0_u64.to_le_bytes());
        let (matched, _) = captured.capture(
            &SliceMemory {
                base: BASE,
                bytes: &broken,
            },
            0,
            BASE,
            &[true; N],
        );
        assert_eq!(matched, [false; N]);
        for outcome in captured.outcomes() {
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: "world threat vtable mismatch"
                }
            );
        }
    }

    struct ChangingReason<'a> {
        bytes: &'a [u8],
        copies: Cell<usize>,
    }
    impl Memory for ChangingReason<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !(SliceMemory {
                base: BASE,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if address == FIRST + cthreat_source::REASON && output.len() == 5 {
                let copies = self.copies.get();
                self.copies.set(copies + 1);
                if copies != 0 {
                    output[0] ^= 1;
                }
            }
            true
        }
    }

    #[test]
    fn a_changed_reason_arena_fails_the_source_window_and_finished_windows_release() {
        let (mut capacities, mut captured) = small();
        let bytes = fixture();
        let memory = ChangingReason {
            bytes: &bytes,
            copies: Cell::new(0),
        };
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true, false, true]);
        assert_eq!(captured.checked_mismatches(), [0, 1, 0]);
        assert!(matches!(
            captured.tables()[1],
            TableView::Unavailable { .. }
        ));
        assert!(!captured.sources.second.is_empty());
        captured.grow(&mut capacities, &[CONSISTENCY_TARGET; N]);
        // A finished window releases every comparison buffer off the game
        // thread; the first buffers keep their own growth headroom.
        assert!(captured.manager.second.is_empty());
        assert!(captured.sources.second.is_empty());
        assert!(captured.aggregates.second.is_empty());
        assert!(captured.reason_bytes.second.is_empty());
    }
}
