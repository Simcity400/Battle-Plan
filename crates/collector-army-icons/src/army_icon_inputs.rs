use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{carmy, cequipment_variant::pool::DivisionSource};
use replay_data::{rows::ArmyIconInputRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ArmyIconInputs];
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let count =
            collector_extended_support::demand::rows(carmy::walk(memory, base, state, &mut []))?;
        crate::table_capture::grow_capacity(&mut self.rows, count);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<carmy::IconInputsRow>,
    early: Option<u64>,
}
impl Captured {
    pub fn fail_capture(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }

    pub fn set_early(&mut self, ticks: u64) {
        self.early = Some(ticks);
    }

    pub fn take_early(&mut self) -> Option<u64> {
        self.early.take()
    }

    pub fn fail_late(&mut self, reason: &'static str) {
        if self.early.take().is_none() {
            self.fail_capture(reason);
        }
    }

    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            early: None,
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
        if !crate::table_capture::grow_vec(&mut self.rows.rows, sources.outcome.len()) {
            self.fail_capture("army icon row allocation failed");
            return [qpc().saturating_sub(begin).max(0) as u64];
        }
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
        [table::<tables::ArmyIconInputs, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ArmyIconInputRow {
                division: r.division,
                separate_counter: r.separate_counter,
                alternate_template: r.alternate_template,
            },
        )]
    }
}
fn read(
    memory: &impl Memory,
    base: usize,
    sources: &[DivisionSource],
    source: Outcome,
    rows: &mut [carmy::IconInputsRow],
    seeds: Option<&[carmy::DivisionSeed]>,
) -> Outcome {
    if let Outcome::Unavailable { .. } = source {
        return source;
    }
    let Some(sources) = sources.get(..source.len()) else {
        return Outcome::Unavailable {
            reason: "army icon source count exceeds census",
        };
    };
    let len = sources.len().min(rows.len());
    for (index, (out, source)) in rows[..len].iter_mut().zip(sources).enumerate() {
        let seed = match seeds {
            Some(seeds) => match seeds.get(index) {
                Some(seed) => Some(seed),
                None => {
                    return Outcome::Unavailable {
                        reason: "army icon source seed absent",
                    };
                }
            },
            None => None,
        };
        match carmy::read_icon_inputs(memory, base, *source, seed) {
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
    use engine_native_memory::memory::SliceMemory;
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
        assert_eq!(
            read(&memory, 0, &[], Outcome::Rows { len: 0 }, &mut rows, None),
            Outcome::Rows { len: 0 }
        );
    }
    #[test]
    fn supplied_seeds_keep_source_prefix_capacity_and_first_failure() {
        let source = DivisionSource {
            pointer: 0x100,
            division: 51 | (9 << 32),
            country: 25,
            template: 0,
        };
        let seed = carmy::DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
            secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA.0 as u64),
            handle: Some(source.division),
            roster_owner: Some(source.country),
            separate_counter: Some(255),
            alternate_template: Some(0),
            ..Default::default()
        };
        let mut bytes = [0; 0x800];
        bytes[source.pointer + carmy::SEPARATE_COUNTER] = 255;
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = [carmy::IconInputsRow::default(); 2];
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
        assert_eq!(rows[0].separate_counter, 255);
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
                reason: "army icon source seed absent"
            }
        );
        let missing = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let failed = carmy::DivisionSeed {
            separate_counter: None,
            ..seed
        };
        assert_eq!(
            read(
                &missing,
                0,
                &[source; 2],
                Outcome::Rows { len: 2 },
                &mut rows,
                Some(&[failed])
            ),
            Outcome::Unavailable {
                reason: "army separate-counter byte unreadable"
            }
        );
    }
    #[test]
    fn raw_flags_and_alternate_handles_preserve_primary_rows_and_growth() {
        let mut capacity = Capacities { rows: 1 };
        let mut capture = Captured::with_capacities(&capacity);
        capture.rows.rows[0] = carmy::IconInputsRow {
            division: 51 | (7 << 32),
            separate_counter: 255,
            alternate_template: 52 | (9 << 32),
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay_data::rows::TableRows::ArmyIconInputs(rows))] =
            capture.tables()
        else {
            panic!("complete primary row");
        };
        assert_eq!(rows[0].division, 51 | (7 << 32));
        assert_eq!(rows[0].separate_counter, 255);
        assert_eq!(rows[0].alternate_template, 52 | (9 << 32));
        capture.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 8,
        };
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 9);
        assert_eq!(capture.rows.rows[0].separate_counter, 255);
        assert_eq!(capture.rows.rows[0].alternate_template, 52 | (9 << 32));
    }
}
