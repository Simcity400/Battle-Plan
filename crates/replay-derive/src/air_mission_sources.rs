//! The air bases this hour's reference registry resolves.
//!
//! AirBases is the manager's roster, not the registry. A witness names every
//! resolved handle and carries the base's own fields where the roster lacks
//! it, so a registry miss stays distinct from a base missing from the roster.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay_data::rows::{
    AirBaseRow, AirReferenceWitnessRow, AirWingRow, ProvinceRow, TableSchema, tables,
};
use replay_frame::frame::{DecodedFrame, TableView};

/// Incomplete tables are unknown; a table holding another table's rows is corrupt.
pub(crate) fn complete<T: TableSchema>(frame: &DecodedFrame) -> io::Result<Option<&[T::Row]>> {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(_) => frame.complete::<T>().map(Some),
        TableView::Truncated { rows, .. } => {
            if rows.rows::<T>().is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "hour {} {:?} contains another table's rows",
                        frame.hour,
                        T::ID
                    ),
                ));
            }
            Ok(None)
        }
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => Ok(None),
    }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[derive(Debug, Default)]
pub struct AirRegistrySources {
    pub bases: BTreeMap<u64, AirBaseRow>,
    /// A base's host task-force roster owner, when the host resolves.
    pub host_owners: BTreeMap<u64, u32>,
    /// Bases whose host ship resolves with a null task force.
    pub hosts_without_task_force: BTreeSet<u64>,
}

impl AirRegistrySources {
    /// `None` without a complete witness table.
    pub fn read(frame: &DecodedFrame) -> io::Result<Option<Self>> {
        let Some(witnesses) = complete::<tables::AirReferenceWitnesses>(frame)? else {
            return Ok(None);
        };
        let manager = complete::<tables::AirBases>(frame)?;
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
                    carrier: replay_data::rows::Recorded::NotRecorded,
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

    /// Reject a transfer only when its base resolves.
    pub fn transfer_resolves(&self, handle: u64) -> bool {
        handle != 0 && self.bases.contains_key(&handle)
    }

    /// A stateless base uses its host's roster owner, or raw zero when the host does not resolve. A
    /// null task force leaves the controller unavailable.
    pub fn host_owner(&self, base: u64) -> Option<u32> {
        if self.hosts_without_task_force.contains(&base) {
            return None;
        }
        Some(self.host_owners.get(&base).copied().unwrap_or(0))
    }
}

/// A raw country reference as its canonical slot; zero is the null country.
pub fn canonical(raw: u32, links: &BTreeMap<u32, u32>) -> Option<u32> {
    if raw == 0 {
        Some(0)
    } else {
        links.get(&raw).copied()
    }
}

/// The wing's raw effective country: a positive override, else its owner.
pub fn effective_raw(wing: &AirWingRow) -> u32 {
    if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    }
}

/// A mission country's canonical slot, the key of every same-hour join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissionCountry {
    pub slot: u32,
}

/// Effective mission-country sources, shared by sortie counts and target scans. These sources read no
/// port or naval target table.
pub struct MissionCountrySources<'a> {
    registry: &'a AirRegistrySources,
    provinces: Option<&'a [ProvinceRow]>,
    volunteer_hosts: Option<BTreeMap<u32, BTreeSet<u32>>>,
    links: &'a BTreeMap<u32, u32>,
}

impl<'a> MissionCountrySources<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        registry: &'a AirRegistrySources,
    ) -> io::Result<Self> {
        let volunteer_hosts = complete::<tables::VolunteerHosts>(frame)?.and_then(|hosts| {
            let mut by_country = BTreeMap::<u32, BTreeSet<u32>>::new();
            for row in hosts {
                let country = canonical(row.country, links)?;
                // Match by raw reference or canonical country identity. An unlinked host never
                // matches; a zero host matches a null controller by raw equality, stopping the base
                // search there.
                if let Some(host) = canonical(row.host, links) {
                    by_country.entry(country).or_default().insert(host);
                }
            }
            Some(by_country)
        });
        Ok(Self {
            registry,
            provinces: complete::<tables::Provinces>(frame)?,
            volunteer_hosts,
            links,
        })
    }

    /// The effective raw country and its recorded canonical slot.
    pub fn effective(&self, wing: &AirWingRow) -> Option<(u32, u32)> {
        let raw = effective_raw(wing);
        Some((raw, *self.links.get(&raw)?))
    }

    /// Try the pool-base controller, then the transfer-base controller, stopping at the first
    /// volunteer host of the effective country. Use that host only when its raw reference is signed-
    /// positive; otherwise use the effective country.
    pub fn resolve(&self, wing: &AirWingRow) -> Option<MissionCountry> {
        let hosts = self.volunteer_hosts.as_ref()?;
        let fallback = MissionCountry {
            slot: canonical(effective_raw(wing), self.links)?,
        };
        let Some(hosts) = hosts.get(&fallback.slot).filter(|hosts| !hosts.is_empty()) else {
            return Some(fallback);
        };
        for handle in [wing.air_base, wing.base_override] {
            if handle == 0 {
                continue;
            }
            // A handle the complete registry does not resolve is a native miss.
            let Some(base) = self.registry.bases.get(&handle) else {
                continue;
            };
            let controller_raw = self.base_controller(base)?;
            let controller = canonical(controller_raw, self.links)?;
            if hosts.contains(&controller) {
                // No other base is tried after the first eligible one.
                return Some(if (controller_raw as i32) > 0 {
                    MissionCountry { slot: controller }
                } else {
                    fallback
                });
            }
        }
        Some(fallback)
    }

    /// Air-base controller: a state base's province controller, with refusal for a state base lacking a
    /// province; a stateless base's host task-force owner, or zero when the host is null or unresolved.
    /// A null task-force result is refused.
    fn base_controller(&self, base: &AirBaseRow) -> Option<u32> {
        let Some(state) = base.state else {
            return if base.host == 0 {
                Some(0)
            } else {
                self.registry.host_owner(base.id)
            };
        };
        let provinces = self.provinces?;
        let at = provinces
            .binary_search_by_key(&base.province?, |row| row.id)
            .ok()?;
        let province = &provinces[at];
        (province.state == state).then_some(province.controller)
    }
}

#[cfg(test)]
mod tests;
