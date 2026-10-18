//! Actual Country0's directed disclosure matrix, acquired within Ordinary.
//! One live-M arena owns values and exact attempted failures. Native controls
//! exist only on the callback stack; projection and growth run off-thread.
use crate::{clock::qpc, table_capture::Arena};
use engine_native::{Memory, ccountry, crelation_status};

#[derive(Default, Clone, Copy)]
struct Cell {
    value: Option<crelation_status::ReverseZeroVisibilityInputs>,
    reason: Option<&'static str>,
    failures: [Option<&'static str>; 4],
}

pub struct Request<'a> {
    pub hour: Option<i32>,
    pub observation: Option<ccountry::ReservedVisibilityObservation>,
    pub prefix_ready: bool,
    pub capture: &'a mut Captured,
}

pub struct Captured {
    cells: Arena<Cell>,
    sample_hour: Option<i32>,
    attempted: bool,
    source_reason: Option<&'static str>,
    scalar_reason: Option<&'static str>,
    prefix_ready: bool,
    ticks: u64,
}

impl Captured {
    pub fn new(slots: usize) -> Self {
        Self {
            cells: Arena::new(slots),
            sample_hour: None,
            attempted: false,
            source_reason: None,
            scalar_reason: None,
            prefix_ready: false,
            ticks: 0,
        }
    }

    pub fn reset(&mut self) {
        self.cells.first[..self.cells.len].fill(Cell::default());
        self.cells.reset();
        self.sample_hour = None;
        self.attempted = false;
        self.source_reason = None;
        self.scalar_reason = None;
        self.prefix_ready = false;
        self.ticks = 0;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare(
        &mut self,
        memory: &impl Memory,
        hour: Option<i32>,
        observation: Option<ccountry::ReservedVisibilityObservation>,
        prefix_ready: bool,
    ) -> Option<crelation_status::ReservedVisibilitySource> {
        self.prepare_storage(memory, hour, observation, prefix_ready, false)
    }

    /// Worker-only preparation continues from the original matrix controls.
    pub fn prepare_growing(
        &mut self,
        memory: &impl Memory,
        hour: Option<i32>,
        observation: Option<ccountry::ReservedVisibilityObservation>,
        prefix_ready: bool,
    ) -> Option<crelation_status::ReservedVisibilitySource> {
        self.prepare_storage(memory, hour, observation, prefix_ready, true)
    }

    fn prepare_storage(
        &mut self,
        memory: &impl Memory,
        hour: Option<i32>,
        observation: Option<ccountry::ReservedVisibilityObservation>,
        prefix_ready: bool,
        growing: bool,
    ) -> Option<crelation_status::ReservedVisibilitySource> {
        self.reset();
        self.attempted = true;
        self.sample_hour = hour;
        self.prefix_ready = prefix_ready;
        // The early scalar attempt remains observable even when required
        // naming later refused. This accessor never reads live memory.
        self.scalar_reason = observation.as_ref().and_then(|value| value.faction().err());
        let _reads =
            crate::read_accounting::table(replay_data::rows::TableId::ReservedCountryNaming);
        let begin = qpc();
        let source = observation
            .as_ref()
            .ok_or("reserved visibility controls unobserved")
            .and_then(|observation| {
                crelation_status::ReservedVisibilitySource::new(memory, observation)
            });
        self.ticks = qpc().saturating_sub(begin).max(0) as u64;
        match source {
            Ok(source) => {
                self.cells.required = source.required_slots();
                // Full country visibility uses the original matrix extent.
                if growing
                    && let Err(reason) = engine_native::walk::Buffer::Growing(&mut self.cells.first)
                        .ensure(self.cells.required)
                {
                    self.source_reason = Some(reason);
                    return None;
                }
                self.cells.len = self.cells.required.min(self.cells.first.len());
                self.cells.first[..self.cells.len].fill(Cell::default());
                Some(source)
            }
            Err(reason) => {
                self.source_reason = Some(reason);
                None
            }
        }
    }

    pub fn capture_cell<M: Memory>(
        &mut self,
        source: &crelation_status::ReservedVisibilitySource,
        memory: &M,
        base: usize,
        countries: Option<&ccountry::Reader<'_, M>>,
        slot: u32,
        forward: Option<&crelation_status::ForwardZeroObservation>,
    ) {
        // A required-prefix or whole-M storage refusal cannot commit any
        // later vector body. The exact header demand still drives growth.
        if self.refusal().is_some() {
            return;
        }
        let Some(cell) = self
            .cells
            .first
            .get_mut(slot as usize)
            .filter(|_| (slot as usize) < self.cells.len)
        else {
            return;
        };
        let _reads =
            crate::read_accounting::table(replay_data::rows::TableId::ReservedCountryNaming);
        let begin = qpc();
        let captured = source.cell(memory, base, countries, slot, forward);
        self.ticks = self
            .ticks
            .saturating_add(qpc().saturating_sub(begin).max(0) as u64);
        let (value, reason) = match captured.value {
            Ok(value) => (Some(value), None),
            Err(reason) => (None, Some(reason)),
        };
        *cell = Cell {
            value,
            reason,
            failures: captured.failures,
        };
    }

    fn refusal(&self) -> Option<&'static str> {
        self.source_reason
            .or_else(|| {
                self.sample_hour
                    .is_none()
                    .then_some("reserved reverse visibility hour unavailable")
            })
            .or_else(|| {
                (self.cells.required > self.cells.first.len())
                    .then_some("reserved reverse visibility arena capacity exceeded")
            })
            .or_else(|| {
                (!self.prefix_ready).then_some("reserved naming singleton prefix unavailable")
            })
    }

    /// The hour this matrix was attempted for; its packet stores none.
    pub fn sample_hour(&self) -> Option<i32> {
        self.sample_hour
    }

    pub fn projection(
        &self,
    ) -> std::io::Result<Option<replay_data::rows::ReservedReverseVisibilityInputs>> {
        if !self.attempted || self.refusal().is_some() {
            return Ok(None);
        }
        let cells = self
            .cells
            .first
            .get(..self.cells.len)
            .filter(|cells| cells.len() == self.cells.required)
            .ok_or_else(|| std::io::Error::other("reserved reverse visibility arena invalid"))?;
        Ok(Some(replay_data::rows::ReservedReverseVisibilityInputs {
            sample_hour: replay_data::rows::PacketHour::Host,
            cells: engine_allocation::collect_vec(cells.iter().map(|cell| {
                cell.value
                    .map(|value| replay_data::rows::ReverseZeroVisibilityInputs {
                        target: value.target,
                        war: value.war,
                        subject: value.subject.map(|subject| {
                            replay_data::rows::ZeroSubjectInputs {
                                relation_type: subject.relation_type,
                                parties: subject.parties,
                            }
                        }),
                        military_access: value.military_access,
                    })
            }))?,
        }))
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }
    pub fn allocated_bytes(&self) -> usize {
        self.cells.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut usize) {
        self.cells.grow(capacity);
    }
    pub fn observe(&self, work: &mut Work) -> std::io::Result<()> {
        work.observe(self)
    }
}

#[derive(Default)]
pub struct Work {
    attempted: u64,
    available: u64,
    refused: u64,
    cells_attempted: u64,
    cells_available: u64,
    cells_refused: u64,
    field_faults: [u64; 4],
    scalar_faults: u64,
    refusals: Vec<(&'static str, u64)>,
    required: usize,
    allocated: usize,
    bytes: usize,
}

impl Work {
    fn refuse(&mut self, reason: &'static str) -> std::io::Result<()> {
        let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                engine_allocation::reserve(&mut self.refusals, 1)?;
                self.refusals.insert(index, (reason, 0));
                index
            }
        };
        self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
        Ok(())
    }

    fn observe(&mut self, capture: &Captured) -> std::io::Result<()> {
        if !capture.attempted {
            return Ok(());
        }
        self.attempted = self.attempted.saturating_add(1);
        if let Some(reason) = capture.refusal() {
            self.refused = self.refused.saturating_add(1);
            self.refuse(reason)?;
        } else {
            self.available = self.available.saturating_add(1);
        }
        if let Some(reason) = capture.scalar_reason {
            self.scalar_faults = self.scalar_faults.saturating_add(1);
            // A root/diplomacy failure can be the same whole-source reason;
            // count that exact attempted refusal once in the reason map.
            if capture.refusal() != Some(reason) {
                self.refuse(reason)?;
            }
        }
        for cell in &capture.cells.first[..capture.cells.len] {
            if let Some(reason) = cell.reason {
                self.cells_attempted = self.cells_attempted.saturating_add(1);
                self.cells_refused = self.cells_refused.saturating_add(1);
                self.refuse(reason)?;
            } else if cell.value.is_some() {
                self.cells_attempted = self.cells_attempted.saturating_add(1);
                self.cells_available = self.cells_available.saturating_add(1);
            }
            for (index, reason) in cell.failures.iter().enumerate() {
                if let Some(reason) = reason {
                    self.field_faults[index] = self.field_faults[index].saturating_add(1);
                    self.refuse(reason)?;
                }
            }
        }
        self.required = capture.cells.required;
        self.allocated = capture.cells.first.len();
        self.bytes = capture.allocated_bytes();
        Ok(())
    }

    pub fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "attempted": self.attempted,
            "available": self.available,
            "refused": self.refused,
            "cells_attempted": self.cells_attempted,
            "cells_available": self.cells_available,
            "cells_refused": self.cells_refused,
            "source_scalar_faults": self.scalar_faults,
            "target_faults": self.field_faults[0],
            "war_faults": self.field_faults[1],
            "subject_faults": self.field_faults[2],
            "military_access_faults": self.field_faults[3],
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "required": self.required,
            "allocated": self.allocated,
            "bytes": self.bytes,
            "scope": "cumulative actual same-hour attempts; latest live matrix demand and owned arena before growth; full changed Opaque field projection/encoding is O(M), bytes also included in pool_bytes",
        })
    }
}

#[cfg(test)]
mod tests;
