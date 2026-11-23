use crate::division_census::UnitInputs;
use collector_extended_support::outcomes::elapsed;
use collector_primitives::{clock::qpc, table_capture::CaptureRows};
use collector_projection::table_conversion::table;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::carmy;
use replay_data::rows::{DivisionCamouflage, DivisionModelInputRow, TableId, tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionModelInputs];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

collector_primitives::capacity_max!(Capacities;);

pub struct Captured {
    rows: CaptureRows<carmy::ModelInputsRow>,
    packet: carmy::StatusPacket,
    captured: Option<(u64, u32)>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self {
            rows: CaptureRows::new(0),
            packet: carmy::StatusPacket::vacant(),
            captured: None,
        }
    }

    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(0),
            packet: carmy::StatusPacket::new(0).unwrap_or_else(|_| carmy::StatusPacket::vacant()),
            captured: None,
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.captured = None;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.packet.allocated_bytes()
    }

    pub fn grow(&mut self, _capacity: &mut Capacities) {}

    pub fn fail_capture(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }

    pub fn fail_job(&mut self, reason: &'static str) {
        self.fail_capture(reason);
        self.captured = Some((0, 0));
    }

    pub fn training(&self) -> Option<u32> {
        self.captured
            .filter(|_| !matches!(self.rows.outcome, Outcome::Unavailable { .. }))
            .map(|(_, training)| training)
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        inputs: Option<&UnitInputs>,
    ) {
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let mut training = 0;
        match inputs {
            None => self.fail_capture("division census unavailable"),
            Some(inputs)
                if !collector_primitives::table_capture::grow_vec(
                    &mut self.rows.rows,
                    inputs.outcome().len(),
                ) =>
            {
                self.fail_capture("division model input row allocation failed");
            }
            Some(inputs) => {
                let report = self.packet.capture_model_inputs(
                    memory,
                    base,
                    state,
                    inputs.seeds(),
                    inputs.outcome(),
                    &mut self.rows.rows,
                );
                training = report.training;
                self.rows.outcome = report.outcome;
            }
        }
        self.captured = Some((elapsed(begin), training));
    }

    pub fn finish(&mut self) -> [u64; N] {
        match self.captured {
            Some((ticks, _)) => [ticks],
            None => {
                self.fail_capture("division model input acquisition not supplied");
                [0]
            }
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::DivisionModelInputs, _>(
            self.rows.outcome,
            &self.rows.rows,
            |row| DivisionModelInputRow {
                division: row.division,
                model_number: row.model_number,
                camouflage: row.camouflage.map(|camouflage| DivisionCamouflage {
                    category: camouflage.category,
                }),
                exercising: row.exercising,
            },
        )]
    }
}
