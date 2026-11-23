
use engine_allocation as allocation;
use engine_map_air::{map_air_base_reference::BaseReference, map_air_base_reference::Host};
use engine_native_air::cair_base;
use engine_native_memory::walk::Outcome;
use replay_data::{rows::AirReferenceWitnessRow, rows::tables};
use replay_frame::frame::TableView;

use crate::table_conversion::table;

#[derive(Clone, Copy, Default)]
pub struct BaseWitness {
    pub handle: u64,
    pub base: Option<BaseReference>,
    pub host_owner: Option<u32>,
    pub host_without_task_force: bool,
}

pub fn capture_hosts(
    witnesses: &mut [BaseWitness],
    mut read: impl FnMut(u64) -> Result<Host, &'static str>,
) -> Result<(), &'static str> {
    witnesses.sort_unstable_by_key(|witness| witness.base.map_or(0, |base| base.host));
    let result = (|| {
        let mut previous = 0;
        let mut host = Host::Unresolved;
        for witness in witnesses.iter_mut() {
            let handle = witness.base.map_or(0, |base| base.host);
            if handle != previous {
                host = read(handle)?;
                previous = handle;
            }
            witness.host_owner = match host {
                Host::Owner(owner) => Some(owner),
                _ => None,
            };
            witness.host_without_task_force = host == Host::NoTaskForce;
        }
        Ok(())
    })();
    witnesses.sort_unstable_by_key(|witness| witness.handle);
    result
}

pub fn table_view(
    outcome: Outcome,
    witnesses: &[BaseWitness],
    manager: Option<&[cair_base::Row]>,
) -> TableView {
    let mut listed: Vec<&cair_base::Row> = Vec::new();
    if allocation::reserve(&mut listed, manager.map_or(0, <[cair_base::Row]>::len)).is_err() {
        return crate::table_conversion::unavailable("");
    }
    listed.extend(manager.into_iter().flatten());
    listed.sort_unstable_by_key(|row| row.id);
    let mut rows = Vec::new();
    let count = witnesses
        .iter()
        .filter(|witness| witness.base.is_some())
        .count();
    if allocation::reserve_push_batch(&mut rows, count).is_err() {
        return crate::table_conversion::unavailable("");
    }
    rows.extend(witnesses.iter().filter_map(|witness| {
        let base = witness.base?;
        let listed = listed
            .binary_search_by_key(&witness.handle, |row| row.id)
            .ok()
            .map(|index| listed[index])
            .is_some_and(|row| {
                row.kind == base.kind
                    && row.state == base.state
                    && row.host == base.host
                    && match base.state {
                        Some(_) => row.province == base.province,
                        None => true,
                    }
            });
        Some(if listed {
            AirReferenceWitnessRow {
                handle: witness.handle,
                host_owner: witness.host_owner,
                host_without_task_force: witness.host_without_task_force,
                ..Default::default()
            }
        } else {
            AirReferenceWitnessRow {
                handle: witness.handle,
                base_kind: Some(base.kind),
                state: base.state,
                province: base.province,
                host: Some(base.host),
                host_owner: witness.host_owner,
                host_without_task_force: witness.host_without_task_force,
            }
        })
    }));
    let outcome = match outcome {
        Outcome::Rows { .. } => Outcome::Rows { len: rows.len() },
        Outcome::Truncated { unread_rows, .. } => Outcome::Truncated {
            len: rows.len(),
            unread_rows,
        },
        unavailable => unavailable,
    };
    table::<tables::AirReferenceWitnesses, _>(outcome, &rows, Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    fn rows(view: TableView) -> Vec<AirReferenceWitnessRow> {
        let TableView::Complete(TableRows::AirReferenceWitnesses(rows)) = view else {
            panic!("complete witness table expected");
        };
        rows
    }

    #[test]
    fn listed_bases_carry_only_the_handle_and_misses_are_dropped() {
        let manager = [cair_base::Row {
            id: 8,
            state: Some(4),
            province: Some(5),
            ..Default::default()
        }];
        let witnesses = [
            BaseWitness {
                handle: 8,
                base: Some(BaseReference {
                    state: Some(4),
                    province: Some(5),
                    ..Default::default()
                }),
                host_owner: None,
                host_without_task_force: false,
            },
            BaseWitness {
                handle: 9,
                base: Some(BaseReference {
                    host: 77,
                    ..Default::default()
                }),
                host_owner: Some(3),
                host_without_task_force: false,
            },
            BaseWitness {
                handle: 10,
                base: None,
                host_owner: None,
                host_without_task_force: false,
            },
            BaseWitness {
                handle: 11,
                base: Some(BaseReference {
                    host: 78,
                    ..Default::default()
                }),
                host_owner: None,
                host_without_task_force: true,
            },
        ];
        let rows = rows(table_view(
            Outcome::Rows { len: 0 },
            &witnesses,
            Some(&manager),
        ));
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].handle, rows[0].base_kind), (8, None));
        assert_eq!((rows[1].handle, rows[1].host), (9, Some(77)));
        assert_eq!(rows[1].host_owner, Some(3));
        assert_eq!(
            (rows[2].host_owner, rows[2].host_without_task_force),
            (None, true)
        );
    }

    #[test]
    fn without_the_manager_every_base_keeps_its_fields() {
        let witnesses = [BaseWitness {
            handle: 8,
            base: Some(BaseReference {
                state: Some(4),
                kind: 2,
                ..Default::default()
            }),
            host_owner: None,
            host_without_task_force: false,
        }];
        let rows = rows(table_view(Outcome::Rows { len: 0 }, &witnesses, None));
        assert_eq!((rows[0].base_kind, rows[0].state), (Some(2), Some(4)));
        assert!(matches!(
            table_view(Outcome::Unavailable { reason: "x" }, &witnesses, None),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn stateful_host_drift_keeps_the_actual_full_reference_and_owner() {
        let manager = [cair_base::Row {
            id: 8,
            state: Some(4),
            province: Some(5),
            host: 7,
            ..Default::default()
        }];
        let witnesses = [BaseWitness {
            handle: 8,
            base: Some(BaseReference {
                state: Some(4),
                province: Some(5),
                host: (9_u64 << 32) | 50,
                ..Default::default()
            }),
            host_owner: Some(3),
            host_without_task_force: false,
        }];
        let captured = rows(table_view(
            Outcome::Rows { len: 1 },
            &witnesses,
            Some(&manager),
        ));
        assert_eq!(captured[0].base_kind, Some(0));
        assert_eq!(captured[0].state, Some(4));
        assert_eq!(captured[0].host, Some((9_u64 << 32) | 50));
        assert_eq!(captured[0].host_owner, Some(3));
    }

    #[test]
    fn host_memo_preserves_full_identity_misses_and_handle_order_for_stateful_bases() {
        let first = (5_u64 << 32) | 50;
        let second = (6_u64 << 32) | 50;
        let witness = |handle, host| BaseWitness {
            handle,
            base: Some(BaseReference {
                host,
                state: Some(4),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut witnesses = [
            witness(4, first),
            witness(2, second),
            witness(3, first),
            witness(1, 0),
        ];
        let mut reads = Vec::new();
        capture_hosts(&mut witnesses, |host| {
            reads.push(host);
            Ok(if host == first {
                Host::Unresolved
            } else {
                Host::Owner(7)
            })
        })
        .unwrap();
        assert_eq!(reads, [first, second]);
        assert_eq!(witnesses.map(|row| row.handle), [1, 2, 3, 4]);
        assert_eq!(
            witnesses.map(|row| row.host_owner),
            [None, Some(7), None, None]
        );
        reads.clear();
        assert_eq!(
            capture_hosts(&mut witnesses, |host| {
                reads.push(host);
                Err("host fault")
            }),
            Err("host fault")
        );
        assert_eq!(reads, [first], "a failed shared host cannot be retried");
        assert_eq!(witnesses.map(|row| row.handle), [1, 2, 3, 4]);
    }
}
