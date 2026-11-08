//! DivisionModelInputs: Map's witness of the land model's DivisionStatus
//! columns, read by its own job over the hour's division census.
use crate::table_conversion::table;
use crate::{division_census::UnitInputs, sampling::qpc, table_capture::CaptureRows};
use collector_extended_support::outcomes::elapsed;
use engine_native::{Memory, carmy, walk::Outcome};
use replay_data::rows::{DivisionCamouflage, DivisionModelInputRow, TableId, tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionModelInputs];

/// Rows grow with the census inside the job.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

collector_primitives::capacity_max!(Capacities;);

pub struct Captured {
    rows: CaptureRows<carmy::ModelInputsRow>,
    /// Status's reads and memos, grown with the census inside the job.
    packet: carmy::StatusPacket,
    /// This hour's job: its worker ticks and its training rows.
    captured: Option<(u64, u32)>,
}

impl Captured {
    /// Allocation-free placeholder while the job owns the pack.
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

    /// A job that did not finish fails its rows and keeps no ticks.
    pub fn fail_job(&mut self, reason: &'static str) {
        self.fail_capture(reason);
        self.captured = Some((0, 0));
    }

    /// This hour's rows whose orders group trained, once the job read them.
    pub fn training(&self) -> Option<u32> {
        self.captured
            .filter(|_| !matches!(self.rows.outcome, Outcome::Unavailable { .. }))
            .map(|(_, training)| training)
    }

    /// The job's read of `inputs`, the hour's division census when it completed.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        inputs: Option<&UnitInputs>,
    ) {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let mut training = 0;
        match inputs {
            None => self.fail_capture("division census unavailable"),
            Some(inputs)
                if !crate::table_capture::grow_vec(&mut self.rows.rows, inputs.outcome().len()) =>
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

    /// The job's ticks; without the job the table fails.
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
