use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cnaval_base, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{NavalBaseRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NavalBases];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 1024 }
    }
}

pub struct Captured {
    rows: CaptureRows<cnaval_base::Row>,
}

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome = cnaval_base::walk(memory, base, state, &mut self.rows.rows);
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::NavalBases, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| NavalBaseRow {
                province: r.province,
                country: r.country,
                level: r.level,
                healthy_level: r.healthy_level,
                countries: r.countries().to_vec(),
                dropped_countries: r.dropped_countries,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;

    #[test]
    fn unreadable_registrations_are_unavailable_and_reset_clears_them() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities::default());
        capture.capture(&memory, 0, 0x1000);
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
