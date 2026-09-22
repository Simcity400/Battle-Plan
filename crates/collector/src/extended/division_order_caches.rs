//! Hourly COrdersGroupMember order caches projected from the shared authenticated division census.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, carmy, cequipment_variant_pool::DivisionSource, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{DivisionOrderCacheRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionOrderCaches];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 8192 }
    }
}
pub struct Captured {
    rows: CaptureRows<carmy::OrderCacheRow>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
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
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sources: super::production::Sources<'_, DivisionSource>,
        seeds: Option<&[carmy::DivisionSeed]>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome = read(
            memory,
            base,
            sources.rows,
            sources.outcome,
            &mut self.rows.rows,
            seeds,
        );
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::DivisionOrderCaches, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| DivisionOrderCacheRow {
                division: r.division,
                root_group: r.root_group,
                root_instance: r.root_instance,
                fallback_group: r.fallback_group,
                fallback_instance: r.fallback_instance,
                cache_dirty: r.cache_dirty,
            },
        )]
    }
}
fn read(
    memory: &impl Memory,
    base: usize,
    sources: &[DivisionSource],
    source: Outcome,
    rows: &mut [carmy::OrderCacheRow],
    seeds: Option<&[carmy::DivisionSeed]>,
) -> Outcome {
    if let Outcome::Unavailable { .. } = source {
        return source;
    }
    let Some(sources) = sources.get(..source.len()) else {
        return Outcome::Unavailable {
            reason: "division order cache source count exceeds census",
        };
    };
    let len = sources.len().min(rows.len());
    for (index, (out, source)) in rows[..len].iter_mut().zip(sources).enumerate() {
        let seed = match seeds {
            Some(seeds) => match seeds.get(index) {
                Some(seed) => Some(seed),
                None => {
                    return Outcome::Unavailable {
                        reason: "division order cache source seed absent",
                    };
                }
            },
            None => None,
        };
        match carmy::read_order_caches(memory, base, *source, seed) {
            Ok(row) => *out = row,
            Err(reason) => return Outcome::Unavailable { reason },
        }
    }
    let missing = sources
        .len()
        .saturating_sub(len)
        .saturating_add(match source {
            Outcome::Truncated { unread_rows, .. } => unread_rows as usize,
            _ => 0,
        });
    if missing == 0 {
        Outcome::Rows { len }
    } else {
        super::truncated_source(len, missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn missing_or_truncated_census_never_becomes_empty_success() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut rows = [];
        assert_eq!(
            read(&memory, 0, &[], super::super::MISSING, &mut rows, None),
            super::super::MISSING
        );
        assert!(matches!(
            read(&memory, 0, &[], Outcome::Rows { len: 1 }, &mut rows, None),
            Outcome::Unavailable { .. }
        ));
        assert_eq!(
            read(
                &memory,
                0,
                &[DivisionSource::default()],
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 3
                },
                &mut rows,
                None
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 4
            }
        );
    }
    #[test]
    fn supplied_seeds_keep_source_prefix_capacity_and_first_failure() {
        let source = DivisionSource {
            pointer: 0x100,
            division: 51 | (9 << 32),
            ..Default::default()
        };
        let seed = carmy::DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            primary_vtable: Some(carmy::VTABLE_RVA as u64),
            handle: Some(source.division),
            ..Default::default()
        };
        let memory = SliceMemory {
            base: 0,
            bytes: &[0; 0x200],
        };
        let mut rows = [carmy::OrderCacheRow::default(); 2];
        assert_eq!(
            read(
                &memory,
                0,
                &[source],
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 3
                },
                &mut rows,
                Some(&[seed])
            ),
            Outcome::Truncated {
                len: 1,
                unread_rows: 3
            }
        );
        assert_eq!(rows[0].division, source.division);
        assert_eq!(
            read(
                &memory,
                0,
                &[source],
                Outcome::Rows { len: 1 },
                &mut [],
                Some(&[])
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            read(
                &memory,
                0,
                &[source],
                super::super::MISSING,
                &mut rows,
                Some(&[])
            ),
            super::super::MISSING
        );
        assert_eq!(
            read(
                &memory,
                0,
                &[source],
                Outcome::Rows { len: 1 },
                &mut rows,
                Some(&[])
            ),
            Outcome::Unavailable {
                reason: "division order cache source seed absent"
            }
        );
        let missing = SliceMemory {
            base: 0,
            bytes: &[],
        };
        assert_eq!(
            read(
                &missing,
                0,
                &[source; 2],
                Outcome::Rows { len: 2 },
                &mut rows,
                Some(&[seed])
            ),
            Outcome::Unavailable {
                reason: "army order caches unreadable"
            }
        );
    }
    #[test]
    fn cached_order_identities_and_dirty_byte_preserve_primary_rows_and_growth() {
        let mut capacity = Capacities { rows: 1 };
        let mut capture = Captured::with_capacities(&capacity);
        capture.rows.rows[0] = carmy::OrderCacheRow {
            division: (7 << 32) | 51,
            root_group: (3 << 32) | 53,
            root_instance: 4,
            fallback_group: 0,
            fallback_instance: 0,
            cache_dirty: 1,
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::DivisionOrderCaches(rows))] =
            capture.tables()
        else {
            panic!("complete primary row");
        };
        assert_eq!(rows[0].division, (7 << 32) | 51);
        assert_eq!(rows[0].root_group, (3 << 32) | 53);
        assert_eq!(rows[0].root_instance, 4);
        assert_eq!(rows[0].cache_dirty, 1);
        capture.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 8,
        };
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 9);
        assert_eq!(capture.rows.rows[0].cache_dirty, 1);
    }
}
