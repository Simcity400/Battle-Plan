//! Ordered current naval repair selection over recorded frame operands.
use crate::visibility::{AccessLinks, Relations};
use replay_data::rows::{
    NavalBaseRow, NavalRepairBaseRow, NavalRepairQueueRow, ProvinceRow, ShipModelRow, ShipRow,
    TaskForceRow, tables,
};
use replay_frame::frame::DecodedFrame;
use std::{borrow::Cow, collections::BTreeMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub used: i32,
    pub level: i32,
    /// Native text colour control, absent for the ordinary authored colour.
    pub colour: Option<char>,
}

/// One selected queue occurrence; repeated full ship handles remain separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedRepair {
    pub ordinal: u32,
    pub ship: u64,
}

/// Country-list membership accepts raw equality (including zero) and nonzero
/// aliases of the same canonical country. Missing alias evidence stays unknown.
pub fn country_list_contains(
    list: &[u32],
    raw: u32,
    links: &BTreeMap<u32, u32>,
) -> Result<bool, &'static str> {
    if list.contains(&raw) {
        return Ok(true);
    }
    if raw == 0 || list.iter().all(|&tag| tag == 0) {
        return Ok(false);
    }
    let target = links.get(&raw).ok_or("country-list target is unresolved")?;
    let mut unknown = false;
    for tag in list.iter().filter(|&&tag| tag != 0) {
        match links.get(tag) {
            Some(slot) if slot == target => return Ok(true),
            Some(_) => {}
            None => unknown = true,
        }
    }
    if unknown {
        Err("country-list alias is unresolved")
    } else {
        Ok(false)
    }
}

/// Borrow complete sorted tables once; demands remain local to each base's own queue.
/// Missing tables matter only when their native branch reaches them.
pub struct Sources<'a> {
    allocations: Option<&'a [NavalRepairBaseRow]>,
    queues: Option<&'a [NavalRepairQueueRow]>,
    ships: Option<&'a [ShipRow]>,
    models: Option<&'a [ShipModelRow]>,
    forces: Option<&'a [TaskForceRow]>,
    provinces: Option<&'a [ProvinceRow]>,
    links: &'a BTreeMap<u32, u32>,
    access: Result<Cow<'a, AccessLinks>, &'static str>,
    relations: Result<&'a Relations, &'static str>,
}
impl<'a> Sources<'a> {
    pub fn new(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        relations: Result<&'a Relations, &'static str>,
    ) -> Self {
        Self::reading(
            frame,
            links,
            relations,
            AccessLinks::read(frame, links).map(Cow::Owned),
        )
    }

    /// [`Self::new`] with access links a caller read and keeps.
    pub fn with_access(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        relations: Result<&'a Relations, &'static str>,
        access: Result<&'a AccessLinks, &'static str>,
    ) -> Self {
        Self::reading(frame, links, relations, access.map(Cow::Borrowed))
    }

    fn reading(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        relations: Result<&'a Relations, &'static str>,
        access: Result<Cow<'a, AccessLinks>, &'static str>,
    ) -> Self {
        Self {
            allocations: frame.complete::<tables::NavalRepairBases>().ok(),
            queues: frame.complete::<tables::NavalRepairQueue>().ok(),
            ships: frame.complete::<tables::Ships>().ok(),
            models: frame.complete::<tables::ShipModels>().ok(),
            forces: frame.complete::<tables::TaskForces>().ok(),
            provinces: frame.complete::<tables::Provinces>().ok(),
            links,
            access,
            relations,
        }
    }
    pub fn capacity(&self, base: &NavalBaseRow) -> Result<Capacity, &'static str> {
        let allocated = self.allocated(base)?;
        let (eligible, queued) = self.traverse(base, base.level, |_| {})?;
        Ok(Capacity {
            used: allocated.min(eligible),
            level: base.level,
            colour: if allocated < eligible {
                Some('R')
            } else if allocated < queued as i32 {
                Some('Y')
            } else {
                None
            },
        })
    }

    /// Assigned mode preserves occurrence order and does not demand an unused suffix.
    pub fn selected(&self, base: &NavalBaseRow) -> Result<Vec<SelectedRepair>, &'static str> {
        let assigned = self.allocated(base)?;
        if assigned <= 0 {
            return Ok(Vec::new());
        }
        let mut selected = Vec::new();
        self.traverse(base, assigned, |entry| {
            selected.push(SelectedRepair {
                ordinal: entry.ordinal,
                ship: entry.ship,
            })
        })?;
        Ok(selected)
    }

    fn allocated(&self, base: &NavalBaseRow) -> Result<i32, &'static str> {
        let allocations = self
            .allocations
            .ok_or("txt_capacity: NavalRepairBases unavailable")?;
        allocations
            .binary_search_by_key(&base.province, |row| row.province)
            .ok()
            .map(|at| allocations[at].allocated_dockyards)
            .ok_or("txt_capacity: base repair allocation absent")
    }

    fn traverse(
        &self,
        base: &NavalBaseRow,
        limit: i32,
        mut accept: impl FnMut(&NavalRepairQueueRow),
    ) -> Result<(i32, usize), &'static str> {
        let queues = self
            .queues
            .ok_or("txt_capacity: NavalRepairQueue unavailable")?;
        let start = queues.partition_point(|row| row.province < base.province);
        let end = queues.partition_point(|row| row.province <= base.province);
        let queue = &queues[start..end];
        let mut eligible = 0_i32;
        for entry in queue {
            if eligible >= limit {
                break;
            }
            if entry.ship == 0 {
                continue;
            }
            let ships = self.ships.ok_or("txt_capacity: Ships unavailable")?;
            let Ok(at) = ships.binary_search_by_key(&entry.ship, |row| row.id) else {
                continue;
            };
            let ship = &ships[at];
            if ship.task_force == 0 {
                continue;
            }
            let forces = self.forces.ok_or("txt_capacity: TaskForces unavailable")?;
            let Ok(at) = forces.binary_search_by_key(&ship.task_force, |row| row.id) else {
                continue;
            };
            let force = &forces[at];
            if base.countries_unreadable {
                return Err("txt_capacity: base prohibition countries unavailable");
            }
            if country_list_contains(&base.countries, force.roster_owner, self.links)? {
                continue;
            }
            let models = self.models.ok_or("txt_capacity: ShipModels unavailable")?;
            let model = models
                .binary_search_by_key(&ship.id, |row| row.ship)
                .ok()
                .map(|at| &models[at])
                .ok_or("txt_capacity: queued ship model absent")?;
            let counter = model
                .counter
                .as_ref()
                .ok_or("txt_capacity: queued ship HP maximum unavailable")?;
            // The game's maximum: the model's stored maximum, at least 100 (0.001 HP).
            if ship.strength >= counter.maximum_strength.max(100) {
                match model
                    .has_criticals
                    .require("ShipModels.has_criticals")
                    .ok()
                    .copied()
                    .flatten()
                {
                    Some(0) => continue,
                    Some(1) => {}
                    _ => {
                        return Err(
                            "txt_capacity: queued ship critical-damage predicate unavailable",
                        );
                    }
                }
            }
            let provinces = self
                .provinces
                .ok_or("txt_capacity: Provinces unavailable")?;
            let controller = provinces
                .binary_search_by_key(&base.province, |row| row.id)
                .ok()
                .map(|at| provinces[at].controller)
                .ok_or("txt_capacity: base province absent")?;
            let owner = *self
                .links
                .get(&force.roster_owner)
                .ok_or("txt_capacity: task-force country unresolved")?;
            let controller = *self
                .links
                .get(&controller)
                .ok_or("txt_capacity: base controller unresolved")?;
            let allowed = owner == controller
                || self.relations.as_ref().map_err(|why| *why)?.known_port(
                    self.access.as_deref().map_err(|why| *why)?,
                    owner,
                    controller,
                );
            if allowed && force.province == base.province {
                eligible = eligible.wrapping_add(1);
                accept(entry);
            }
        }
        Ok((eligible, queue.len()))
    }
}

#[cfg(test)]
mod tests;
