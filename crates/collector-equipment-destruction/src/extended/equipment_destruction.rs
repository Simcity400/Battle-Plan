use crate::equipment_destruction as producer;
use collector_primitives::{
    clock::qpc,
    table_capture::{CaptureRows, grow_vec},
};
use collector_projection::table_conversion::unavailable;
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use replay_data::{
    rows::DivisionDeletionDisposition, rows::DivisionEquipmentDeletionRow,
    rows::EquipmentDestructionCoverage, rows::EquipmentDestructionRow, rows::Recorded,
    rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::EquipmentDestruction,
    TableId::DivisionEquipmentDeletions,
];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<producer::Entry>,
    partial_interval: Option<&'static str>,
    deletion_unavailable: Option<&'static str>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*c),
            partial_interval: None,
            deletion_unavailable: None,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.partial_interval = None;
        self.deletion_unavailable = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        *c = (*c).max(producer::demand());
        grow_vec(&mut self.rows.rows, *c);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
        self.partial_interval = None;
        self.deletion_unavailable = Some(reason);
    }
    pub fn capture(&mut self, _memory: &impl Memory, _base: usize, _state: usize) -> [u64; N] {
        self.capture_with(producer::drain_interval)
    }
    fn capture_with(
        &mut self,
        drain: impl FnOnce(&mut Vec<producer::Entry>) -> Result<producer::Drain, &'static str>,
    ) -> [u64; N] {
        self.reset();
        let start = qpc();
        match drain(&mut self.rows.rows) {
            Ok(producer::Drain::Rows(len)) => {
                self.rows.outcome = Outcome::Rows { len };
            }
            Ok(producer::Drain::Partial(reason)) => {
                self.rows.outcome = Outcome::Rows { len: 0 };
                self.partial_interval = Some(reason);
            }
            Err(reason) => {
                self.rows.outcome = Outcome::Unavailable { reason };
            }
        }
        self.deletion_unavailable = producer::deletion_unavailable();
        [qpc().saturating_sub(start).max(0) as u64, 0]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        if let Some(reason) = self.partial_interval {
            return [Outcome::Rows { len: 1 }, Outcome::Unavailable { reason }];
        }
        [
            self.rows.outcome,
            match self.deletion_unavailable {
                Some(reason) if matches!(self.rows.outcome, Outcome::Rows { .. }) => {
                    Outcome::Unavailable { reason }
                }
                _ => self.rows.outcome,
            },
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        if let Some(reason) = self.partial_interval {
            let Ok(marker_reason) = allocation::string(reason) else {
                return std::array::from_fn(|_| unavailable("collector allocation failed"));
            };
            let Ok(mut rows) = allocation::vec(1) else {
                return std::array::from_fn(|_| unavailable("collector allocation failed"));
            };
            rows.push(EquipmentDestructionRow {
                coverage: Recorded::Value(EquipmentDestructionCoverage::PartialInterval),
                unavailable_reason: Recorded::Value(marker_reason),
                ..Default::default()
            });
            return [
                TableView::Complete(TableRows::EquipmentDestruction(rows)),
                unavailable(reason),
            ];
        }
        let Outcome::Rows { len } = self.rows.outcome else {
            let reason = match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "equipment destruction incomplete drain",
            };
            return std::array::from_fn(|_| unavailable(reason));
        };
        match aggregate(&self.rows.rows[..len])
            .and_then(|rows| deletion_rows(&self.rows.rows[..len]).map(|deleted| (rows, deleted)))
        {
            Ok((mut rows, deleted)) => {
                let deletion_table = if let Some(reason) = self.deletion_unavailable {
                    let marker_reason = allocation::string(reason);
                    if allocation::reserve_exact(&mut rows, 1).is_err() || marker_reason.is_err() {
                        return std::array::from_fn(|_| unavailable("collector allocation failed"));
                    }
                    rows.insert(
                        0,
                        EquipmentDestructionRow {
                            coverage: Recorded::Value(
                                EquipmentDestructionCoverage::SupplementalUnavailable,
                            ),
                            unavailable_reason: Recorded::Value(marker_reason.unwrap()),
                            ..Default::default()
                        },
                    );
                    unavailable(reason)
                } else {
                    TableView::Complete(TableRows::DivisionEquipmentDeletions(deleted))
                };
                [
                    TableView::Complete(TableRows::EquipmentDestruction(rows)),
                    deletion_table,
                ]
            }
            Err(reason) => {
                producer::lost_drain();
                std::array::from_fn(|_| unavailable(reason))
            }
        }
    }
}

fn aggregate(events: &[producer::Entry]) -> Result<Vec<EquipmentDestructionRow>, &'static str> {
    let mut rows =
        allocation::collect_vec(events.iter().filter(|r| r.division_handle == 0).map(|r| {
            EquipmentDestructionRow {
                country: r.country,
                cause: r.cause.into(),
                variant: r.variant,
                amount_raw: r.amount_raw,
                coverage: Recorded::Value(EquipmentDestructionCoverage::Complete),
                unavailable_reason: Recorded::Value(String::new()),
            }
        }))
        .map_err(|_| "collector allocation failed")?;
    rows.sort_unstable_by_key(EquipmentDestructionRow::key);
    let mut write = 0usize;
    for read in 0..rows.len() {
        if rows[read].amount_raw == 0 {
            return Err("equipment destruction zero event");
        }
        if write > 0 && rows[write - 1].key() == rows[read].key() {
            rows[write - 1].amount_raw = rows[write - 1]
                .amount_raw
                .checked_add(rows[read].amount_raw)
                .ok_or("equipment destruction hourly total overflow")?;
        } else {
            rows[write] = rows[read].clone();
            write += 1;
        }
    }
    rows.truncate(write);
    rows.retain(|row| row.amount_raw != 0);
    Ok(rows)
}

fn deletion_rows(
    events: &[producer::Entry],
) -> Result<Vec<DivisionEquipmentDeletionRow>, &'static str> {
    let mut rows = allocation::vec(events.len()).map_err(|_| "collector allocation failed")?;
    for (index, event) in events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.division_handle != 0)
    {
        if event.variant != 0 || event.amount_raw != 0 {
            return Err("equipment deletion witness has equipment amount");
        }
        rows.push((
            index,
            DivisionEquipmentDeletionRow {
                division_handle: event.division_handle,
                country: event.country,
                expeditionary_owner: event.expeditionary_owner,
                cause: (event.disposition
                    == engine_events::equipment_destruction::Disposition::Loss)
                    .then(|| event.cause.into()),
                disposition: match event.disposition {
                    engine_events::equipment_destruction::Disposition::Loss => {
                        DivisionDeletionDisposition::Loss
                    }
                    engine_events::equipment_destruction::Disposition::Returned => {
                        DivisionDeletionDisposition::Returned
                    }
                    engine_events::equipment_destruction::Disposition::Merged => {
                        DivisionDeletionDisposition::Merged
                    }
                    engine_events::equipment_destruction::Disposition::HqTransferred => {
                        DivisionDeletionDisposition::HqTransferred
                    }
                    engine_events::equipment_destruction::Disposition::VolunteerTransferred => {
                        DivisionDeletionDisposition::VolunteerTransferred
                    }
                    engine_events::equipment_destruction::Disposition::ExileTransferred => {
                        DivisionDeletionDisposition::ExileTransferred
                    }
                },
            },
        ));
    }
    rows.sort_unstable_by_key(|(index, row)| (row.division_handle, *index));
    let mut write = 0;
    for read in 0..rows.len() {
        if write > 0 && rows[write - 1].1.division_handle == rows[read].1.division_handle {
            if !matches!(
                rows[write - 1].1.disposition,
                DivisionDeletionDisposition::VolunteerTransferred
                    | DivisionDeletionDisposition::ExileTransferred
            ) {
                return Err("equipment deletion division recorded twice in one hour");
            }
            rows[write - 1] = rows[read].clone();
        } else {
            rows[write] = rows[read].clone();
            write += 1;
        }
    }
    rows.truncate(write);
    allocation::collect_vec(rows.into_iter().map(|(_, row)| row))
        .map_err(|_| "collector allocation failed")
}

#[cfg(test)]
#[path = "equipment_destruction/tests.rs"]
mod tests;
