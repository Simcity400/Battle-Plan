//! Ordered current naval repair selection: one rule over the operands it looks up
//! ([`Operands`]), read from a recorded frame ([`Sources`]) or from a columnar walk.
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
    slot_of: impl Fn(u32) -> Option<u32>,
) -> Result<bool, &'static str> {
    if list.contains(&raw) {
        return Ok(true);
    }
    if raw == 0 || list.iter().all(|&tag| tag == 0) {
        return Ok(false);
    }
    let target = slot_of(raw).ok_or("country-list target is unresolved")?;
    let mut unknown = false;
    for &tag in list.iter().filter(|&&tag| tag != 0) {
        match slot_of(tag) {
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

/// A looked-up table that was not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unavailable;

/// A queued ship's selection operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuedShip {
    pub task_force: u64,
    pub strength: i64,
}

/// A task force's selection operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuedForce {
    pub roster_owner: u32,
    pub province: u32,
}

/// A ship model's selection operands: its design, its stored HP maximum when its counter was
/// recorded, and its critical-damage predicate when recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuedModel {
    pub variant: u64,
    pub maximum_strength: Option<i64>,
    pub has_criticals: Option<u32>,
}

/// What the selection looks up, each lookup refusing only where a branch reaches it.
pub trait Operands {
    type Queue<'q>: Iterator<Item = (u32, u64)>
    where
        Self: 'q;

    /// The repair allocation of the base at `province`; None where it has none.
    fn allocation(&self, province: u32) -> Result<Option<i32>, Unavailable>;
    /// The queue at `province` in queue order, as (ordinal, ship), and its length.
    fn queue(&self, province: u32) -> Result<(Self::Queue<'_>, usize), Unavailable>;
    fn ship(&self, id: u64) -> Result<Option<QueuedShip>, Unavailable>;
    fn force(&self, id: u64) -> Result<Option<QueuedForce>, Unavailable>;
    fn model(&self, ship: u64) -> Result<Option<QueuedModel>, Unavailable>;
    fn controller(&self, province: u32) -> Result<Option<u32>, Unavailable>;
    /// The canonical slot raw country `raw` links to.
    fn slot_of(&self, raw: u32) -> Option<u32>;
    /// Whether `owner` may repair at a base `controller` controls.
    fn port(&self, owner: u32, controller: u32) -> Result<bool, &'static str>;
}

pub fn capacity<O: Operands + ?Sized>(
    operands: &O,
    base: &NavalBaseRow,
) -> Result<Capacity, &'static str> {
    let allocated = allocated(operands, base)?;
    let (eligible, queued) = traverse(operands, base, base.level, |_, _| {})?;
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
pub fn selected<O: Operands + ?Sized>(
    operands: &O,
    base: &NavalBaseRow,
) -> Result<Vec<SelectedRepair>, &'static str> {
    let assigned = allocated(operands, base)?;
    if assigned <= 0 {
        return Ok(Vec::new());
    }
    let mut selected = Vec::new();
    traverse(operands, base, assigned, |ordinal, ship| {
        selected.push(SelectedRepair { ordinal, ship })
    })?;
    Ok(selected)
}

fn allocated<O: Operands + ?Sized>(operands: &O, base: &NavalBaseRow) -> Result<i32, &'static str> {
    operands
        .allocation(base.province)
        .map_err(|_| "txt_capacity: NavalRepairBases unavailable")?
        .ok_or("txt_capacity: base repair allocation absent")
}

fn traverse<O: Operands + ?Sized>(
    operands: &O,
    base: &NavalBaseRow,
    limit: i32,
    mut accept: impl FnMut(u32, u64),
) -> Result<(i32, usize), &'static str> {
    let (queue, length) = operands
        .queue(base.province)
        .map_err(|_| "txt_capacity: NavalRepairQueue unavailable")?;
    let mut eligible = 0_i32;
    for (ordinal, ship) in queue {
        if eligible >= limit {
            break;
        }
        if ship == 0 {
            continue;
        }
        let Some(ship_operands) = operands
            .ship(ship)
            .map_err(|_| "txt_capacity: Ships unavailable")?
        else {
            continue;
        };
        if ship_operands.task_force == 0 {
            continue;
        }
        let Some(force) = operands
            .force(ship_operands.task_force)
            .map_err(|_| "txt_capacity: TaskForces unavailable")?
        else {
            continue;
        };
        if base.countries_unreadable {
            return Err("txt_capacity: base prohibition countries unavailable");
        }
        if country_list_contains(&base.countries, force.roster_owner, |raw| {
            operands.slot_of(raw)
        })? {
            continue;
        }
        let model = operands
            .model(ship)
            .map_err(|_| "txt_capacity: ShipModels unavailable")?
            .ok_or("txt_capacity: queued ship model absent")?;
        let maximum = model
            .maximum_strength
            .ok_or("txt_capacity: queued ship HP maximum unavailable")?;
        // The game's maximum: the model's stored maximum, at least 100 (0.001 HP).
        if ship_operands.strength >= maximum.max(100) {
            match model.has_criticals {
                Some(0) => continue,
                Some(1) => {}
                _ => {
                    return Err("txt_capacity: queued ship critical-damage predicate unavailable");
                }
            }
        }
        let controller = operands
            .controller(base.province)
            .map_err(|_| "txt_capacity: Provinces unavailable")?
            .ok_or("txt_capacity: base province absent")?;
        let owner = operands
            .slot_of(force.roster_owner)
            .ok_or("txt_capacity: task-force country unresolved")?;
        let controller = operands
            .slot_of(controller)
            .ok_or("txt_capacity: base controller unresolved")?;
        let allowed = owner == controller || operands.port(owner, controller)?;
        if allowed && force.province == base.province {
            eligible = eligible.wrapping_add(1);
            accept(ordinal, ship);
        }
    }
    Ok((eligible, length))
}

/// The operands as a recorded frame's complete sorted tables hold them, borrowed once.
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
        Self {
            allocations: frame.complete::<tables::NavalRepairBases>().ok(),
            queues: frame.complete::<tables::NavalRepairQueue>().ok(),
            ships: frame.complete::<tables::Ships>().ok(),
            models: frame.complete::<tables::ShipModels>().ok(),
            forces: frame.complete::<tables::TaskForces>().ok(),
            provinces: frame.complete::<tables::Provinces>().ok(),
            links,
            access: AccessLinks::read(frame, links).map(Cow::Owned),
            relations,
        }
    }

    pub fn capacity(&self, base: &NavalBaseRow) -> Result<Capacity, &'static str> {
        capacity(self, base)
    }

    pub fn selected(&self, base: &NavalBaseRow) -> Result<Vec<SelectedRepair>, &'static str> {
        selected(self, base)
    }
}

fn find<'r, R, K: Ord>(
    rows: Option<&'r [R]>,
    key: K,
    of: impl Fn(&R) -> K,
) -> Result<Option<&'r R>, Unavailable> {
    let rows = rows.ok_or(Unavailable)?;
    Ok(rows
        .binary_search_by(|row| of(row).cmp(&key))
        .ok()
        .map(|at| &rows[at]))
}

impl Operands for Sources<'_> {
    type Queue<'q>
        = std::iter::Map<
        std::slice::Iter<'q, NavalRepairQueueRow>,
        fn(&NavalRepairQueueRow) -> (u32, u64),
    >
    where
        Self: 'q;

    fn allocation(&self, province: u32) -> Result<Option<i32>, Unavailable> {
        Ok(
            find(self.allocations, province, |row| row.province)?
                .map(|row| row.allocated_dockyards),
        )
    }

    fn queue(&self, province: u32) -> Result<(Self::Queue<'_>, usize), Unavailable> {
        let queues = self.queues.ok_or(Unavailable)?;
        let start = queues.partition_point(|row| row.province < province);
        let end = queues.partition_point(|row| row.province <= province);
        let entry: fn(&NavalRepairQueueRow) -> (u32, u64) = |row| (row.ordinal, row.ship);
        Ok((queues[start..end].iter().map(entry), end - start))
    }

    fn ship(&self, id: u64) -> Result<Option<QueuedShip>, Unavailable> {
        Ok(find(self.ships, id, |row| row.id)?.map(|row| QueuedShip {
            task_force: row.task_force,
            strength: row.strength,
        }))
    }

    fn force(&self, id: u64) -> Result<Option<QueuedForce>, Unavailable> {
        Ok(find(self.forces, id, |row| row.id)?.map(|row| QueuedForce {
            roster_owner: row.roster_owner,
            province: row.province,
        }))
    }

    fn model(&self, ship: u64) -> Result<Option<QueuedModel>, Unavailable> {
        Ok(find(self.models, ship, |row| row.ship)?.map(model_operands))
    }

    fn controller(&self, province: u32) -> Result<Option<u32>, Unavailable> {
        Ok(find(self.provinces, province, |row| row.id)?.map(|row| row.controller))
    }

    fn slot_of(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn port(&self, owner: u32, controller: u32) -> Result<bool, &'static str> {
        Ok(self.relations.as_ref().map_err(|why| *why)?.known_port(
            self.access.as_deref().map_err(|why| *why)?,
            owner,
            controller,
        ))
    }
}

/// A model row's selection operands.
pub fn model_operands(row: &ShipModelRow) -> QueuedModel {
    QueuedModel {
        variant: row.variant,
        maximum_strength: row.counter.as_ref().map(|counter| counter.maximum_strength),
        has_criticals: row
            .has_criticals
            .require("ShipModels.has_criticals")
            .ok()
            .copied()
            .flatten(),
    }
}

#[cfg(test)]
mod tests;
