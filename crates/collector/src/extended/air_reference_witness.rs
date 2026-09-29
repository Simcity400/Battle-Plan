//! Registry witnesses for the air bases the flying-aircraft map resolves.
//! Acquisition keeps each probed handle's result; conversion drops misses and
//! keeps a base's fields only where the AirBases roster lacks or disagrees.

use engine::{cair_base, map_air_base_reference::BaseReference, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{AirReferenceWitnessRow, tables},
};

use crate::spooling::table;

#[derive(Clone, Copy, Default)]
pub(super) struct BaseWitness {
    pub handle: u64,
    pub base: Option<BaseReference>,
    /// A stateless base's host task-force owner; None when the host misses
    /// or its task force is null, which the flag marks.
    pub host_owner: Option<u32>,
    pub host_without_task_force: bool,
}

/// `witnesses` are this hour's probes and `outcome` says whether they are
/// complete. `manager` is the complete AirBases census, if there is one.
pub(super) fn table_view(
    outcome: Outcome,
    witnesses: &[BaseWitness],
    manager: Option<&[cair_base::Row]>,
) -> TableView {
    // Conversion runs off the game thread; the census is in manager order.
    let mut listed: Vec<&cair_base::Row> = manager.into_iter().flatten().collect();
    listed.sort_unstable_by_key(|row| row.id);
    let rows: Vec<_> = witnesses
        .iter()
        .filter_map(|witness| {
            let base = witness.base?;
            let listed = listed
                .binary_search_by_key(&witness.handle, |row| row.id)
                .ok()
                .map(|index| listed[index])
                .is_some_and(|row| {
                    row.kind == base.kind
                        && row.state == base.state
                        && match base.state {
                            Some(_) => row.province == base.province,
                            None => row.host == base.host,
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
                    host: base.state.is_none().then_some(base.host),
                    host_owner: witness.host_owner,
                    host_without_task_force: witness.host_without_task_force,
                }
            })
        })
        .collect();
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
    use replay::rows::TableRows;

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
}
