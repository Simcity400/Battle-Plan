//! Naval-base repair text from ordered work queues and current ship eligibility.
use super::*;
use replay_data::{
    rows::NavalBaseRow, rows::NavalRepairBaseRow, rows::NavalRepairQueueRow, rows::ProvinceRow,
    rows::ShipModelRow, rows::ShipRow, rows::TaskForceRow,
};
use viewer_foundation::map::visibility::{AccessLinks, Relations};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Capacity {
    pub used: i32,
    pub level: i32,
    /// Native text colour control, absent for the ordinary authored colour.
    pub colour: Option<char>,
}
impl Capacity {
    pub fn text(self) -> String {
        let text = format!("{}/{}", self.used, self.level);
        match self.colour {
            Some(colour) => {
                let marker = viewer_foundation::map::native_text::COLOR;
                format!("{marker}{colour}{text}{marker}!")
            }
            None => text,
        }
    }
}

/// Borrow complete sorted tables once; demands remain local to each base's own queue.
/// Missing tables matter only when their native branch reaches them.
pub(super) struct Sources<'a> {
    allocations: Option<&'a [NavalRepairBaseRow]>,
    queues: Option<&'a [NavalRepairQueueRow]>,
    ships: Option<&'a [ShipRow]>,
    models: Option<&'a [ShipModelRow]>,
    forces: Option<&'a [TaskForceRow]>,
    provinces: Option<&'a [ProvinceRow]>,
    links: &'a BTreeMap<u32, u32>,
    access: Result<AccessLinks, &'static str>,
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
            access: AccessLinks::read(frame, links),
            relations,
        }
    }
    pub fn capacity(&self, base: &NavalBaseRow) -> Result<Capacity, &'static str> {
        let allocations = self
            .allocations
            .ok_or("txt_capacity: NavalRepairBases unavailable")?;
        let allocated = allocations
            .binary_search_by_key(&base.province, |row| row.province)
            .ok()
            .map(|at| allocations[at].allocated_dockyards)
            .ok_or("txt_capacity: base repair allocation absent")?;
        let queues = self
            .queues
            .ok_or("txt_capacity: NavalRepairQueue unavailable")?;
        let start = queues.partition_point(|row| row.province < base.province);
        let end = queues.partition_point(|row| row.province <= base.province);
        let queue = &queues[start..end];
        let mut eligible = 0_i32;
        for entry in queue {
            if eligible >= base.level {
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
            if bases::country_list_contains(&base.countries, force.roster_owner, self.links)? {
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
            if ship.strength >= counter.maximum_strength {
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
                    self.access.as_ref().map_err(|why| *why)?,
                    owner,
                    controller,
                );
            if allowed && force.province == base.province {
                eligible = eligible.wrapping_add(1);
            }
        }
        Ok(Capacity {
            used: allocated.min(eligible),
            level: base.level,
            colour: if allocated < eligible {
                Some('R')
            } else if allocated < queue.len() as i32 {
                Some('Y')
            } else {
                None
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use viewer_foundation::map::test_support::{test_frame, test_put as put};
    #[test]
    fn repair_capacity_preserves_queue_membership_repeats_criticals_and_total_level_cap() {
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
                province: 7,
                allocated_dockyards: 2,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::NavalRepairQueue(
                vec![0, 90, 1, 2, 3, 2]
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, ship)| NavalRepairQueueRow {
                        province: 7,
                        ordinal: ordinal as u32,
                        ship,
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::Ships(
                (1..=3)
                    .map(|id| ShipRow {
                        id,
                        task_force: 8,
                        strength: 100,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::ShipModels(
                (1..=3)
                    .map(|ship| ShipModelRow {
                        ship,
                        has_criticals: Some(u32::from(ship == 2)).into(),
                        counter: Some(ShipCounterInputs {
                            maximum_strength: 100,
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 8,
                province: 7,
                roster_owner: 11,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 7,
                controller: 11,
                ..Default::default()
            }]),
        );
        let links = BTreeMap::from([(11, 1)]);
        let mut base = NavalBaseRow {
            province: 7,
            level: 10,
            ..Default::default()
        };
        let read = |base: &NavalBaseRow| {
            Sources::new(&frame, &links, Err("unused relations")).capacity(base)
        };
        assert_eq!(
            read(&base).unwrap(),
            Capacity {
                used: 2,
                level: 10,
                colour: Some('Y')
            }
        );
        base.level = 1;
        assert_eq!(
            read(&base).unwrap(),
            Capacity {
                used: 1,
                level: 1,
                colour: Some('Y')
            }
        );
        base.countries = vec![11];
        assert_eq!(read(&base).unwrap().used, 0);
        base.countries.clear();
        base.level = 10;
        put(
            &mut frame,
            TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
                province: 7,
                allocated_dockyards: 1,
                ..Default::default()
            }]),
        );
        assert_eq!(
            Sources::new(&frame, &links, Err("unused"))
                .capacity(&base)
                .unwrap(),
            Capacity {
                used: 1,
                level: 10,
                colour: Some('R')
            }
        );
    }
    #[test]
    fn damaged_hull_does_not_need_a_critical_predicate_and_empty_queue_needs_no_ships() {
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
                province: 7,
                allocated_dockyards: 0,
                ..Default::default()
            }]),
        );
        put(&mut frame, TableRows::NavalRepairQueue(vec![]));
        let links = BTreeMap::new();
        assert_eq!(
            Sources::new(&frame, &links, Err("unused"))
                .capacity(&NavalBaseRow {
                    province: 7,
                    level: 10,
                    ..Default::default()
                })
                .unwrap(),
            Capacity {
                used: 0,
                level: 10,
                colour: None
            }
        );
        put(
            &mut frame,
            TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
                province: 7,
                ordinal: 0,
                ship: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 1,
                task_force: 8,
                strength: 99,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::ShipModels(vec![ShipModelRow {
                ship: 1,
                counter: Some(ShipCounterInputs {
                    maximum_strength: 100,
                    ..Default::default()
                }),
                has_criticals: None.into(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 8,
                province: 7,
                roster_owner: 11,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 7,
                controller: 11,
                ..Default::default()
            }]),
        );
        let links = BTreeMap::from([(11, 1)]);
        let base = NavalBaseRow {
            province: 7,
            level: 10,
            ..Default::default()
        };
        assert_eq!(
            Sources::new(&frame, &links, Err("unused"))
                .capacity(&base)
                .unwrap()
                .colour,
            Some('R')
        );
        put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 1,
                task_force: 8,
                strength: 100,
                ..Default::default()
            }]),
        );
        assert!(
            Sources::new(&frame, &links, Err("unused"))
                .capacity(&base)
                .unwrap_err()
                .contains("critical-damage")
        );
        assert_eq!(
            Capacity {
                used: 1,
                level: 10,
                colour: Some('R')
            }
            .text(),
            format!(
                "{}R1/10{}!",
                viewer_foundation::map::native_text::COLOR,
                viewer_foundation::map::native_text::COLOR
            )
        );
    }
}
