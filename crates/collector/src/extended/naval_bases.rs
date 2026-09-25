use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
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
    /// Country-list entries across all bases.
    countries: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 1024,
            countries: 4096,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<cnaval_base::Row>,
    countries: Arena<u32>,
}

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            countries: 0,
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            countries: Arena::new(c.countries),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.countries.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.countries.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.countries.grow(&mut c.countries);
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.reset();
        let walked = cnaval_base::walk(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.countries.first,
        );
        self.rows.outcome = walked.outcome;
        self.countries.len = walked.countries;
        self.countries.required = walked.required_countries;
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.outcomes()[0];
        let Some(countries) = self.countries.first.get(..self.countries.len) else {
            return [unavailable("naval base country arena length invalid")];
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return [unavailable("walker row count exceeds allocated buffer")];
        };
        if rows.iter().any(|r| {
            r.countries_start
                .checked_add(r.countries_len as usize)
                .is_none_or(|end| end > countries.len())
        }) {
            return [unavailable("naval base country list lies outside captured arena")];
        }
        [table::<tables::NavalBases, _>(outcome, rows, |r| NavalBaseRow {
            province: r.province,
            country: r.country,
            level: r.level,
            healthy_level: r.healthy_level,
            countries: countries[r.countries_start..r.countries_start + r.countries_len as usize]
                .to_vec(),
            countries_unreadable: r.countries_unreadable,
        })]
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
