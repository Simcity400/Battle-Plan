//! Hourly counter inputs projected from the shared authenticated division census.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, carmy, cequipment_variant_pool::DivisionSource, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ArmyIconInputRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ArmyIconInputs];
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
    rows: CaptureRows<carmy::IconInputsRow>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.rows.grow(&mut capacity.rows, successful[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        needed: bool,
        sources: super::production::Sources<'_, DivisionSource>,
    ) -> ([bool; N], [u64; N]) {
        let begin = qpc();
        self.rows.outcome = read(
            memory,
            base,
            sources.first,
            sources.first_outcome,
            &mut self.rows.rows,
        );
        self.rows.checked = needed;
        let matched = if needed {
            let second = read(
                memory,
                base,
                sources.second,
                sources.second_outcome,
                &mut self.rows.second,
            );
            self.rows.compare(second)
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "army icon inputs changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
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
    for (out, source) in rows[..len].iter_mut().zip(sources) {
        match carmy::read_icon_inputs(memory, base, *source) {
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
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::memory::SliceMemory;
    #[test]
    fn missing_or_truncated_census_never_becomes_empty_success() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut rows = [];
        assert_eq!(
            read(&memory, 0, &[], super::super::MISSING, &mut rows),
            super::super::MISSING
        );
        assert!(matches!(
            read(&memory, 0, &[], Outcome::Rows { len: 1 }, &mut rows),
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
                &mut rows
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 4
            }
        );
        assert_eq!(
            read(&memory, 0, &[], Outcome::Rows { len: 0 }, &mut rows),
            Outcome::Rows { len: 0 }
        );
    }
    #[test]
    fn raw_flags_and_alternate_handles_participate_in_own_window() {
        let mut capacity = Capacities { rows: 1 };
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        capture.rows.rows[0] = carmy::IconInputsRow {
            division: 51 | (7 << 32),
            separate_counter: 255,
            alternate_template: 52 | (9 << 32),
        };
        capture.rows.second[0] = capture.rows.rows[0];
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.checked = true;
        assert!(capture.rows.compare(Outcome::Rows { len: 1 }));
        capture.rows.second[0].separate_counter = 1;
        assert!(!capture.rows.compare(Outcome::Rows { len: 1 }));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.second[0] = capture.rows.rows[0];
        capture.rows.second[0].alternate_template = 0;
        assert!(!capture.rows.compare(Outcome::Rows { len: 1 }));
        capture.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 8,
        };
        capture.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(capacity.rows >= 9);
        assert!(capture.rows.second.is_empty());
    }
}
