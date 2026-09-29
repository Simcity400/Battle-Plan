//! The air bases this hour's reference registry resolves.
//!
//! AirBases is the manager's roster, not the registry. A witness names every
//! resolved handle and carries the base's own fields where the roster lacks
//! it, so a registry miss stays distinct from a base missing from the roster.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay::{
    frame::DecodedFrame,
    rows::{AirBaseRow, AirReferenceWitnessRow, tables},
};

use super::tables::{Available, rows};

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[derive(Debug, Default)]
pub(super) struct AirRegistrySources {
    pub(super) bases: BTreeMap<u64, AirBaseRow>,
    /// A stateless base's host task-force roster owner, when the host resolves.
    pub(super) host_owners: BTreeMap<u64, u32>,
    /// Stateless bases whose host ship resolves with a null task force.
    pub(super) hosts_without_task_force: BTreeSet<u64>,
}

impl AirRegistrySources {
    /// `None` without a complete witness table.
    pub(super) fn read(frame: &DecodedFrame) -> io::Result<Option<Self>> {
        let Available::Complete(witnesses) = rows::<tables::AirReferenceWitnesses>(frame)? else {
            return Ok(None);
        };
        let manager = match rows::<tables::AirBases>(frame)? {
            Available::Complete(rows) => Some(rows),
            _ => None,
        };
        Self::from_rows(witnesses, manager).map(Some)
    }

    fn from_rows(
        witnesses: &[AirReferenceWitnessRow],
        manager: Option<&[AirBaseRow]>,
    ) -> io::Result<Self> {
        let mut sources = Self::default();
        for row in witnesses {
            let base = match row.base_kind {
                Some(kind) => AirBaseRow {
                    id: row.handle,
                    kind,
                    state: row.state,
                    province: row.province,
                    host: row.host.unwrap_or(0),
                    // Only the roster records capacity; the map does not use it.
                    capacity: 0,
                },
                None => manager
                    .and_then(|rows| {
                        let index = rows.binary_search_by_key(&row.handle, |base| base.id);
                        index.ok().map(|index| &rows[index])
                    })
                    .ok_or_else(|| invalid("handle-only air base witness has no AirBases row"))?
                    .clone(),
            };
            if let Some(owner) = row.host_owner {
                sources.host_owners.insert(row.handle, owner);
            }
            if row.host_without_task_force {
                sources.hosts_without_task_force.insert(row.handle);
            }
            sources.bases.insert(row.handle, base);
        }
        Ok(sources)
    }

    /// `IsAirWingScenarioEligible@140F66100` rejects a transfer only when its
    /// base resolves.
    pub(super) fn transfer_resolves(&self, handle: u64) -> bool {
        handle != 0 && self.bases.contains_key(&handle)
    }

    /// A stateless base's controller: its host's roster owner, or zero (the
    /// native null controller) when the host does not resolve. `None` for a
    /// null task force, which `GetTaskForceOwnerTag@140C215C0` dereferences.
    pub(super) fn host_owner(&self, base: u64) -> Option<u32> {
        if self.hosts_without_task_force.contains(&base) {
            return None;
        }
        Some(self.host_owners.get(&base).copied().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_only_witnesses_take_the_roster_and_others_their_own_fields() {
        let manager = [AirBaseRow {
            id: 10,
            state: Some(7),
            province: Some(9),
            capacity: 5,
            ..Default::default()
        }];
        let witnesses = [
            AirReferenceWitnessRow {
                handle: 10,
                ..Default::default()
            },
            AirReferenceWitnessRow {
                handle: 11,
                base_kind: Some(0),
                host: Some(77),
                host_owner: Some(3),
                ..Default::default()
            },
            AirReferenceWitnessRow {
                handle: 12,
                base_kind: Some(0),
                host: Some(78),
                host_without_task_force: true,
                ..Default::default()
            },
        ];
        let sources = AirRegistrySources::from_rows(&witnesses, Some(&manager)).unwrap();
        assert_eq!(sources.bases[&10].capacity, 5);
        assert_eq!(sources.bases[&11].host, 77);
        assert!(sources.transfer_resolves(11));
        assert!(!sources.transfer_resolves(13));
        assert_eq!(
            [10, 11, 12].map(|base| sources.host_owner(base)),
            [Some(0), Some(3), None]
        );
        assert!(AirRegistrySources::from_rows(&witnesses, None).is_err());
    }
}
