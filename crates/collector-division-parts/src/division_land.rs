//! Hourly companion columns of Divisions, acquired on its Presentation worker.
use crate::{
    clock::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
    table_conversion::table,
};
use engine_allocation as allocation;
use engine_land_inputs::division_land_inputs as native;
use engine_native::{Memory, carmy::DivisionSeed, walk::Outcome};
use replay_data::{
    rows::DivisionLandInputsRow, rows::DivisionManpowerContributor, rows::RetiredI64, rows::tables,
};
use replay_frame::frame::TableView;

pub struct Captured {
    rows: CaptureRows<native::Row>,
    contributors: Vec<native::Contributor>,
    pointers: Vec<usize>,
    ordinals: Vec<usize>,
    first: Vec<[u8; 2 * engine_deployment::manpower_value_entry::STRIDE]>,
    used: usize,
    required: usize,
}
impl Captured {
    pub fn new(capacity: usize) -> Self {
        Self {
            rows: CaptureRows::new(capacity),
            contributors: allocation::owned_vec(allocation::default_vec(
                capacity.saturating_mul(2),
            )),
            pointers: allocation::owned_vec(allocation::default_vec(capacity)),
            ordinals: allocation::owned_vec(allocation::default_vec(capacity)),
            first: allocation::owned_vec(allocation::default_vec(capacity)),
            used: 0,
            required: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.used = 0;
        self.required = 0;
    }
    pub fn reserve(&mut self, capacity: usize) {
        let mut rows = self.rows.rows.len().max(capacity);
        self.rows.grow(&mut rows);
        grow_vec(&mut self.pointers, rows);
        grow_vec(&mut self.ordinals, rows);
        grow_vec(&mut self.first, rows);
        let mut entries = self.contributors.len().max(capacity.saturating_mul(2));
        grow_capacity(&mut entries, self.required);
        grow_vec(&mut self.contributors, entries);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
        self.used = 0;
    }
    pub fn outcome(&self) -> Outcome {
        self.rows.outcome
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.contributors.capacity() * size_of::<native::Contributor>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.ordinals.capacity() * size_of::<usize>()
            + self.first.capacity()
                * size_of::<[u8; 2 * engine_deployment::manpower_value_entry::STRIDE]>()
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sources: &[DivisionSeed],
        source: Outcome,
    ) -> u64 {
        let _reads = crate::read_accounting::table(replay_data::rows::TableId::DivisionLandInputs);
        let start = qpc();
        // Presentation is worker-only; scratch follows this original admitted
        // census before the reader visits the retained contributor headers.
        let prepared = (|| {
            for buffer in [&mut self.pointers, &mut self.ordinals] {
                engine_native::walk::Buffer::Growing(buffer).ensure(source.len())?;
            }
            engine_native::walk::Buffer::Growing(&mut self.first).ensure(source.len())?;
            Ok::<_, &'static str>(())
        })();
        if let Err(reason) = prepared {
            self.fail(reason);
            return qpc().saturating_sub(start).max(0) as u64;
        }
        let result = native::capture_growing(
            memory,
            base,
            sources,
            source,
            &mut self.rows.rows,
            &mut self.contributors,
            native::Scratch {
                pointers: &mut self.pointers,
                ordinals: &mut self.ordinals,
                first: &mut self.first,
            },
        );
        self.rows.outcome = result.outcome;
        self.used = result.contributors;
        self.required = result.required_contributors;
        qpc().saturating_sub(start).max(0) as u64
    }
    pub fn table(&self) -> TableView {
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return crate::table_conversion::unavailable(
                "walker row count exceeds allocated buffer",
            );
        };
        if self.used > self.contributors.len()
            || rows.iter().any(|row| {
                row.start
                    .checked_add(row.len)
                    .is_none_or(|end| end > self.used)
            })
        {
            return crate::table_conversion::unavailable("land manpower span exceeds copied arena");
        }
        table::<tables::DivisionLandInputs, _>(self.rows.outcome, &self.rows.rows, |r| {
            DivisionLandInputsRow {
                division: r.division,
                // DivisionStatus records the division's supply ratio and grace.
                received_supply: RetiredI64::default(),
                maximum_grace: RetiredI64::default(),
                remaining_grace: RetiredI64::default(),
                naval_transfer: r.naval_transfer,
                manpower: allocation::owned_vec(allocation::map_slice(
                    &self.contributors[r.start..r.start + r.len],
                    |r| DivisionManpowerContributor {
                        country: r.country,
                        amount: r.amount,
                    },
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn land_failed_publication_retains_row_and_contributor_growth_demand() {
        let mut captured = Captured::new(8);
        captured.rows.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 19,
        };
        captured.required = 42;
        captured.used = 4;
        captured.fail("accepted land acquisition panicked");
        assert_eq!(
            captured.outcome(),
            Outcome::Unavailable {
                reason: "accepted land acquisition panicked",
            }
        );
        captured.reserve(8);
        assert!(captured.rows.rows.len() >= 21);
        assert!(captured.contributors.len() >= 42);
        assert_eq!(captured.used, 0);
    }
}
