
use crate::table_conversion::unavailable;
use collector_projection::table_conversion::RowOrder;
use engine_native_memory::walk::Outcome;
use replay_data::{
    cadence::SKIPPED_REASON,
    rows::{self, RowCodec, TableId, tables},
};
use replay_frame::frame::{TableInput, TableView};
use std::{
    mem::size_of,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) enum AcquiredRows {
    Propaganda(Vec<crate::propaganda_changes::Row>),
    Contributions(Vec<rows::IntelContributionRow>),
    Sources(Vec<rows::IntelSourceRow>),
    NetworkAuxiliaries(Vec<rows::IntelNetworkAuxiliaryRow>),
}

impl AcquiredRows {
    fn table(&self) -> TableId {
        match self {
            Self::Propaganda(_) => TableId::PropagandaEfforts,
            Self::Contributions(_) => TableId::IntelContributions,
            Self::Sources(_) => TableId::IntelSources,
            Self::NetworkAuxiliaries(_) => TableId::IntelNetworkAuxiliaries,
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Propaganda(rows) => rows.len(),
            Self::Contributions(rows) => rows.len(),
            Self::Sources(rows) => rows.len(),
            Self::NetworkAuxiliaries(rows) => rows.len(),
        }
    }
    fn truncate(&mut self, len: usize) {
        match self {
            Self::Propaganda(rows) => rows.truncate(len),
            Self::Contributions(rows) => rows.truncate(len),
            Self::Sources(rows) => rows.truncate(len),
            Self::NetworkAuxiliaries(rows) => rows.truncate(len),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Propaganda(rows) => rows.capacity() * size_of::<crate::propaganda_changes::Row>(),
            Self::Contributions(rows) => rows.capacity() * size_of::<rows::IntelContributionRow>(),
            Self::Sources(rows) => rows.capacity() * size_of::<rows::IntelSourceRow>(),
            Self::NetworkAuxiliaries(rows) => {
                rows.capacity() * size_of::<rows::IntelNetworkAuxiliaryRow>()
            }
        }
    }
    fn project(&self, order: &mut RowOrder) -> TableView {
        let outcome = Outcome::Rows { len: self.len() };
        match self {
            Self::Propaganda(rows) => order.table::<tables::PropagandaEfforts, _>(
                outcome,
                rows,
                |r| (u128::from(r.slot) << 64) | u128::from(r.sequence),
                |r| rows::PropagandaEffortRow {
                    slot: r.slot,
                    country: r.country,
                    sequence: r.sequence,
                    stability: r.stability,
                    war_support: r.war_support,
                    derived_drift: r.derived_drift,
                },
            ),
            Self::Contributions(rows) => {
                order.table::<tables::IntelContributions, _>(outcome, rows, RowCodec::key, |r| *r)
            }
            Self::Sources(rows) => {
                order.table::<tables::IntelSources, _>(outcome, rows, RowCodec::key, |r| *r)
            }
            Self::NetworkAuxiliaries(rows) => order.table::<tables::IntelNetworkAuxiliaries, _>(
                outcome,
                rows,
                RowCodec::key,
                |r| *r,
            ),
        }
    }
}

struct ProjectionOrder {
    rows: Mutex<RowOrder>,
    bytes: AtomicUsize,
}

pub(crate) struct Snapshot {
    table: TableId,
    acquired: Mutex<Option<AcquiredRows>>,
    acquired_bytes: AtomicUsize,
    projected: OnceLock<TableInput>,
    ordering: OnceLock<Arc<ProjectionOrder>>,
}
impl Snapshot {
    fn projection(&self) -> Option<&TableInput> {
        if let Some(projected) = self.projected.get() {
            return Some(projected);
        }
        let acquired = self
            .acquired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.projected.get().is_none() {
            let acquired = acquired
                .as_ref()
                .expect("acquisition recycled before projection");
            let mut fresh_order = None;
            let view = if let Some(order) = self.ordering.get() {
                let mut rows = order
                    .rows
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let view = acquired.project(&mut rows);
                order.bytes.store(rows.allocated_bytes(), Ordering::Relaxed);
                view
            } else {
                let mut rows = RowOrder::default();
                let view = acquired.project(&mut rows);
                fresh_order = Some(rows);
                view
            };
            if matches!(&view, TableView::Unavailable { reason } if reason.is_empty()) {
                return None;
            }
            if matches!(&view, TableView::Complete(_))
                && let Some(rows) = fresh_order
            {
                let bytes = AtomicUsize::new(rows.allocated_bytes());
                let _ = self.ordering.set(Arc::new(ProjectionOrder {
                    rows: Mutex::new(rows),
                    bytes,
                }));
            }
            let input = match view {
                TableView::Complete(rows) => TableInput::SharedComplete(Arc::new(rows)),
                other => TableInput::Sampled(other),
            };
            let _ = self.projected.set(input);
        }
        self.projected.get()
    }
    pub(crate) fn input(&self) -> TableInput {
        match self.projection() {
            Some(projected) => projected.clone(),
            None => TableInput::Sampled(unavailable(refusal(self.table))),
        }
    }
    pub(crate) fn recycle_acquired(&self) -> Option<AcquiredRows> {
        self.projection()?;
        let rows = self
            .acquired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.acquired_bytes.store(0, Ordering::Relaxed);
        rows
    }
    pub(crate) fn acquired_bytes(&self) -> usize {
        self.acquired_bytes.load(Ordering::Relaxed)
    }
}

pub(crate) struct RetainedSnapshot {
    rows: Option<Arc<Snapshot>>,
}
impl RetainedSnapshot {
    pub(crate) const fn new() -> Self {
        Self { rows: None }
    }
    pub(crate) fn clear(&mut self) {
        self.rows = None;
    }

    pub(crate) fn publish(
        &mut self,
        table: TableId,
        outcome: Outcome,
        acquire: impl FnOnce() -> AcquiredRows,
        shared: &mut Option<Arc<Snapshot>>,
    ) -> Outcome {
        *shared = None;
        match outcome {
            Outcome::Rows { len } => {
                let mut acquired = acquire();
                if acquired.table() != table || acquired.len() < len {
                    self.clear();
                    return Outcome::Unavailable {
                        reason: refusal(table),
                    };
                }
                acquired.truncate(len);
                let bytes = acquired.bytes();
                let rows = Arc::new(Snapshot {
                    table,
                    acquired: Mutex::new(Some(acquired)),
                    acquired_bytes: AtomicUsize::new(bytes),
                    projected: OnceLock::new(),
                    ordering: self
                        .rows
                        .as_ref()
                        .and_then(|rows| rows.ordering.get())
                        .map_or_else(OnceLock::new, |order| OnceLock::from(Arc::clone(order))),
                });
                self.rows = Some(Arc::clone(&rows));
                *shared = Some(rows);
                outcome
            }
            Outcome::Unavailable {
                reason: SKIPPED_REASON,
            } => {
                if let Some(rows) = &self.rows {
                    *shared = Some(Arc::clone(rows));
                    outcome
                } else {
                    Outcome::Unavailable {
                        reason: refusal(table),
                    }
                }
            }
            _ => {
                self.clear();
                outcome
            }
        }
    }
}
fn refusal(table: TableId) -> &'static str {
    match table {
        TableId::PropagandaEfforts => "propaganda certified acquisition unavailable",
        TableId::IntelNetworkAuxiliaries => "network auxiliary certified acquisition unavailable",
        TableId::IntelContributions => "intel contribution certified acquisition unavailable",
        TableId::IntelSources => "intel source certified acquisition unavailable",
        _ => "intel certified acquisition table mismatch",
    }
}

#[cfg(test)]
pub(crate) fn fixture(rows: rows::TableRows) -> AcquiredRows {
    match rows {
        rows::TableRows::PropagandaEfforts(mut rows) => {
            rows.reverse();
            AcquiredRows::Propaganda(
                rows.into_iter()
                    .map(|r| crate::propaganda_changes::Row {
                        slot: r.slot,
                        country: r.country,
                        sequence: r.sequence,
                        stability: r.stability,
                        war_support: r.war_support,
                        derived_drift: r.derived_drift,
                    })
                    .collect(),
            )
        }
        rows::TableRows::IntelContributions(mut rows) => {
            rows.reverse();
            AcquiredRows::Contributions(rows)
        }
        rows::TableRows::IntelSources(mut rows) => {
            rows.reverse();
            AcquiredRows::Sources(rows)
        }
        rows::TableRows::IntelNetworkAuxiliaries(mut rows) => {
            rows.reverse();
            AcquiredRows::NetworkAuxiliaries(rows)
        }
        _ => panic!("unexpected intel fixture table"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_without_baseline_or_after_loss_refuses_until_authenticated_recovery() {
        for table in [
            TableId::PropagandaEfforts,
            TableId::IntelContributions,
            TableId::IntelSources,
            TableId::IntelNetworkAuxiliaries,
        ] {
            let mut retained = RetainedSnapshot::new();
            let mut shared = None;
            let quiet = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            let refused = Outcome::Unavailable {
                reason: refusal(table),
            };
            assert_eq!(
                retained.publish(table, quiet, || panic!("quiet acquisition"), &mut shared),
                refused
            );
            assert!(shared.is_none());
            assert_eq!(
                retained.publish(
                    table,
                    Outcome::Rows { len: 0 },
                    || fixture(rows::TableRows::empty(table)),
                    &mut shared
                ),
                Outcome::Rows { len: 0 }
            );
            let empty = shared.take().unwrap();
            crate::test_support::with_allocations_denied(|| {
                assert_eq!(
                    retained.publish(table, quiet, || panic!("quiet acquisition"), &mut shared),
                    quiet
                );
                assert!(Arc::ptr_eq(&empty, shared.as_ref().unwrap()));
                assert!(empty.projected.get().is_none());
            });
            let TableInput::SharedComplete(rows) = empty.input() else {
                panic!("observed empty refused")
            };
            assert!(rows.is_empty());
            let lost = Outcome::Unavailable {
                reason: "synthetic writer coverage lost",
            };
            assert_eq!(
                retained.publish(table, lost, || panic!("lost acquisition"), &mut shared),
                lost
            );
            assert!(shared.is_none());
            assert_eq!(
                retained.publish(table, quiet, || panic!("revoked acquisition"), &mut shared),
                refused
            );
            assert_eq!(
                retained.publish(
                    table,
                    Outcome::Rows { len: 0 },
                    || fixture(rows::TableRows::empty(table)),
                    &mut shared
                ),
                Outcome::Rows { len: 0 }
            );
            assert!(shared.is_some());
            assert_eq!(
                retained.publish(
                    table,
                    Outcome::Rows { len: 1 },
                    || fixture(rows::TableRows::empty(table)),
                    &mut shared
                ),
                refused
            );
            assert!(shared.is_none());
            assert_eq!(
                retained.publish(
                    table,
                    quiet,
                    || panic!("failed acquisition reused"),
                    &mut shared
                ),
                refused
            );
            assert!(retained.rows.is_none());
        }
    }

    #[test]
    fn duplicate_keys_refuse_on_the_spool_and_stay_refused_on_quiet_and_recovery_encoders() {
        for table in [
            TableId::PropagandaEfforts,
            TableId::IntelContributions,
            TableId::IntelSources,
            TableId::IntelNetworkAuxiliaries,
        ] {
            let rows = match table {
                TableId::PropagandaEfforts => {
                    rows::TableRows::PropagandaEfforts(vec![Default::default(); 2])
                }
                TableId::IntelContributions => {
                    rows::TableRows::IntelContributions(vec![Default::default(); 2])
                }
                TableId::IntelSources => rows::TableRows::IntelSources(vec![Default::default(); 2]),
                TableId::IntelNetworkAuxiliaries => {
                    rows::TableRows::IntelNetworkAuxiliaries(vec![Default::default(); 2])
                }
                _ => unreachable!(),
            };
            let mut retained = RetainedSnapshot::new();
            let mut shared = None;
            retained.publish(
                table,
                Outcome::Rows { len: 2 },
                || fixture(rows),
                &mut shared,
            );
            let snapshot = shared.unwrap();
            assert!(snapshot.projected.get().is_none());
            for hour in 1..=2 {
                let mut encoder = replay_frame::frame::FrameEncoder::new();
                let mut tables = std::array::from_fn(|_| {
                    TableInput::Sampled(TableView::Unavailable {
                        reason: "outside duplicate regression".into(),
                    })
                });
                tables[table.index()] = snapshot.input();
                let frame = encoder
                    .encode_input(
                        replay_frame::frame::FrameInput {
                            branch: 1,
                            hour,
                            tables,
                            metrics: Default::default(),
                        },
                        true,
                    )
                    .unwrap();
                assert!(
                    matches!(&frame.tables[table.index()], replay_frame::frame::TableUpdate::Unavailable { reason }
                    if reason == "walker produced duplicate engine ids")
                );
            }
        }
    }

    #[test]
    fn failed_advisory_projection_keeps_native_rows_for_an_authenticated_spool_retry() {
        let mut retained = RetainedSnapshot::new();
        let mut shared = None;
        retained.publish(
            TableId::IntelSources,
            Outcome::Rows { len: 1 },
            || AcquiredRows::Sources(vec![rows::IntelSourceRow::default()]),
            &mut shared,
        );
        let snapshot = shared.unwrap();
        crate::test_support::with_allocations_denied(|| {
            assert!(engine_allocation::advisory(|| snapshot.recycle_acquired()).is_none());
            assert!(snapshot.projected.get().is_none());
            assert!(snapshot.acquired_bytes.load(Ordering::Relaxed) > 0);
        });
        let TableInput::SharedComplete(rows) = snapshot.input() else {
            panic!("projection did not recover")
        };
        assert_eq!(rows.len(), 1);
        assert!(snapshot.recycle_acquired().is_some());
    }

}
