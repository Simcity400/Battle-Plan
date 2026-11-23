use crate::{clock::qpc, table_capture::Arena};
use engine_native_memory::Memory;
use engine_native::{ccountry, crelation::status as crelation_status};

#[derive(Default, Clone, Copy)]
struct Cell {
    value: Option<crelation_status::ReverseZeroVisibilityInputs>,
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

    pub fn prepare(
        &mut self,
        memory: &impl Memory,
        hour: Option<i32>,
        observation: Option<ccountry::ReservedVisibilityObservation>,
        prefix_ready: bool,
    ) -> Option<crelation_status::ReservedVisibilitySource> {
        self.reset();
        self.attempted = true;
        self.sample_hour = hour;
        self.prefix_ready = prefix_ready;
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
                if let Err(reason) = engine_native_memory::walk::Buffer::Growing(&mut self.cells.first)
                    .ensure(self.cells.required)
                {
                    self.source_reason = Some(reason);
                    return None;
                }
                self.cells.len = self.cells.required;
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
        *cell = Cell {
            value: captured.value.ok(),
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
                (!self.prefix_ready).then_some("reserved naming singleton prefix unavailable")
            })
    }

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
}

#[cfg(test)]
mod tests;
