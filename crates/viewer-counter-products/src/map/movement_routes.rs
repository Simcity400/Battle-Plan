
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};
#[cfg(test)]
use viewer_foundation::test_support::test_frame;
#[cfg(test)]
use viewer_foundation::test_support::test_put;
use viewer_foundation::movement_timing::MovementContent;

use replay_data::{rows::Recorded, rows::TableSchema, rows::tables};
use replay_derive::{eta_derive, eta_derive::ProvinceOperands};
use replay_frame::frame::DecodedFrame;

use crate::map::counter_content::Content;
use viewer_unit_reads::movement_routes::Route;

use viewer_unit_reads::counter_move::{self, RouteWorld};
use replay_derive::directed_wars::Owner;
use viewer_gpu::province_map::HeightField;
use viewer_runtime::{unit_class::CounterClass, unit_handle::Unit};

pub fn selected_provinces(frame: &DecodedFrame, selected: &BTreeSet<Unit>) -> BTreeSet<u32> {
    selected
        .iter()
        .filter_map(|&unit| match unit {
            Unit::Division(id) => find::<tables::Divisions>(frame, id, |row| row.id)
                .map(|row| row.province)
                .ok(),
            Unit::RailwayGun(id) => {
                find::<tables::RailwayGunCounterInputs>(frame, id, |row| row.gun)
                    .map(|row| row.province)
                    .ok()
            }
            Unit::TaskForce(id) => find::<tables::TaskForces>(frame, id, |row| row.id)
                .map(|row| row.province)
                .ok(),
        })
        .collect()
}

pub fn build(
    content: &Content,
    heights: HeightField<'_>,
    width: f32,
    frame: &DecodedFrame,
    ours: u32,
    selected: &BTreeSet<Unit>,
) -> (Vec<Route>, Vec<String>) {
    if selected.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let context = Context::new(content, heights, width, frame, ours);
    let mut routes = Vec::new();
    let mut refusals = Vec::new();
    for &unit in selected {
        let result = context
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|context| context.unit(unit));
        match result {
            Ok(Some((route, skipped))) => {
                routes.push(route);
                if skipped != 0 {
                    refusals.push(format!(
                        "Movement arrow {unit:?}: {skipped} route hops skipped by native content geometry"
                    ));
                }
            }
            Ok(None) => {}
            Err(reason) => refusals.push(format!("Movement arrow {unit:?}: {reason}")),
        }
    }
    (routes, refusals)
}

struct Context<'a> {
    content: &'a Content,
    heights: HeightField<'a>,
    width: f32,
    frame: &'a DecodedFrame,
    viewer_raw: u32,
    links: BTreeMap<u32, u32>,
    wars: RefCell<BTreeMap<u32, Result<Owner<'a>, String>>>,
}

impl<'a> Context<'a> {
    fn new(
        content: &'a Content,
        heights: HeightField<'a>,
        width: f32,
        frame: &'a DecodedFrame,
        ours: u32,
    ) -> Result<Self, String> {
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let viewer_raw = if ours == 0 {
            0
        } else {
            countries
                .iter()
                .find(|country| country.canonical_slot == ours)
                .ok_or_else(|| format!("viewing country slot {ours} has no Countries row"))?
                .id
        };
        Ok(Self {
            content,
            heights,
            width,
            frame,
            viewer_raw,
            links: links
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect(),
            wars: RefCell::default(),
        })
    }

    fn own(&self, owner: u32) -> Result<bool, String> {
        same_country(owner, self.viewer_raw, &self.links)
    }

    fn unit(&self, unit: Unit) -> Result<Option<(Route, usize)>, String> {
        let input = match unit {
            Unit::Division(id) => {
                let division = find::<tables::Divisions>(self.frame, id, |row| row.id)?;
                if !self.own(division.roster_owner)? {
                    return Ok(None);
                }
                let movement =
                    find::<tables::DivisionMovement>(self.frame, id, |row| row.division)?;
                Inputs {
                    province: division.province,
                    logical_owner: division.logical_owner,
                    ordinary: &movement.path,
                    ordinary_total: movement.path_total as usize,
                    alternate: &movement.alternate_path,
                    alternate_total: movement.alternate_total as usize,
                    numerator: Ok(movement.numerator),
                    style: StyleInputs {
                        class: CounterClass::Division,
                        retreat: Ok(movement.retreat),
                        withdrawal: recorded(&movement.withdrawal, "DivisionMovement.withdrawal"),
                        mission: 0,
                        target: movement.target,
                        transfer: Ok(movement.naval_transfer),
                        priority: recorded(
                            &movement.movement_priority,
                            "DivisionMovement.movement_priority",
                        ),
                        orders_group: division.orders_group,
                    },
                }
            }
            Unit::RailwayGun(id) => {
                let gun = find::<tables::RailwayGunCounterInputs>(self.frame, id, |row| row.gun)?;
                if !self.own(gun.roster_owner)? {
                    return Ok(None);
                }
                Inputs {
                    province: gun.province,
                    logical_owner: gun.logical_owner,
                    ordinary: &gun.ordinary_path,
                    ordinary_total: gun.ordinary_path.len(),
                    alternate: &gun.alternate_path,
                    alternate_total: gun.alternate_path.len(),
                    numerator: gun
                        .eta
                        .require("RailwayGunCounterInputs.eta")
                        .and_then(|eta| {
                            eta.as_ref().map(|eta| eta.numerator).ok_or_else(|| {
                                "RailwayGunCounterInputs.eta operands unavailable".into()
                            })
                        }),
                    style: StyleInputs {
                        class: CounterClass::RailwayGun,
                        retreat: Ok(gun.retreat),
                        withdrawal: recorded(&gun.withdrawal, "RailwayGunCounterInputs.withdrawal"),
                        mission: 0,
                        target: gun.target,
                        transfer: Ok(gun.naval_transfer),
                        priority: Ok(0),
                        orders_group: 0,
                    },
                }
            }
            Unit::TaskForce(id) => {
                let task_force = find::<tables::TaskForces>(self.frame, id, |row| row.id)?;
                if !self.own(task_force.roster_owner)? {
                    return Ok(None);
                }
                let motion =
                    find::<tables::TaskForceMapMotion>(self.frame, id, |row| row.task_force)?;
                Inputs {
                    province: task_force.province,
                    logical_owner: task_force.logical_owner,
                    ordinary: &motion.path,
                    ordinary_total: motion.path.len(),
                    alternate: &motion.alternate_path,
                    alternate_total: motion.alternate_path.len(),
                    numerator: recorded(
                        &motion.movement_numerator,
                        "TaskForceMapMotion.movement_numerator",
                    ),
                    style: StyleInputs {
                        class: CounterClass::TaskForce,
                        retreat: recorded(&motion.retreat, "TaskForceMapMotion.retreat"),
                        withdrawal: recorded(&motion.withdrawal, "TaskForceMapMotion.withdrawal"),
                        mission: task_force.mission as i32,
                        target: motion.target,
                        transfer: recorded(
                            &motion.naval_transfer,
                            "TaskForceMapMotion.naval_transfer",
                        ),
                        priority: Ok(0),
                        orders_group: 0,
                    },
                }
            }
        };
        self.place(unit, input)
    }

    fn place(&self, unit: Unit, input: Inputs<'_>) -> Result<Option<(Route, usize)>, String> {
        let (path, total) = if input.alternate_total != 0 {
            (input.alternate, input.alternate_total)
        } else {
            (input.ordinary, input.ordinary_total)
        };
        if path.len() != total {
            return Err(format!(
                "display route has {} of {total} provinces",
                path.len()
            ));
        }
        if path.is_empty() {
            return Ok(None);
        }
        let style = choose_style(&input.style, || self.hostile(&input))?;
        let movement =
            self.content.movement.as_deref().ok_or_else(|| {
                "recorded content's movement geometry/defines unavailable".to_owned()
            })?;
        let heights = self.heights;
        let world = RouteWorld {
            map: &movement.map,
            heights,
            width: self.width,
        };
        let anchor = |province| {
            let land = input.style.class == CounterClass::TaskForce
                && movement.map.terrain(province)?.flags & 1 != 0;
            viewer_unit_reads::counter_anchors::stack_anchor(
                &self.content.anchors,
                &self.content.default_anchors,
                province,
                input.style.class.standstill(land),
            )
        };
        let walked = counter_move::route_controls(input.province, path, &anchor, &world);
        if walked.controls.len() < 4 {
            return Ok(None);
        }
        let total_arc = walked.total_arc;
        let progress = self.progress(&input, movement)?;
        let fill_target = (progress as f32 / 100_000.0) * walked.next_arc;
        if !total_arc.is_finite() || !fill_target.is_finite() {
            return Err("native route arc/fill target is not finite".into());
        }
        Ok(Some((
            Route {
                unit,
                province: input.province,
                style,
                controls: walked.controls,
                total_arc,
                fill_target,
                free_arc: counter_move::SETTLED_ARC,
            },
            walked.skipped,
        )))
    }

    fn progress(&self, input: &Inputs<'_>, movement: &MovementContent) -> Result<i64, String> {
        if input.ordinary_total == 0 {
            return if input.ordinary.is_empty() {
                Ok(0)
            } else {
                Err("zero ordinary route count has recorded provinces".into())
            };
        }
        let first = *input
            .ordinary
            .first()
            .ok_or_else(|| "ordinary route's first province unavailable".to_owned())?;
        let provinces = self
            .frame
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?;
        let province = |id| {
            let row = provinces
                .binary_search_by_key(&id, |row| row.id)
                .ok()
                .map(|index| &provinces[index])?;
            let terrain = movement.map.terrain(id)?;
            Some(ProvinceOperands {
                terrain_flags: terrain.flags,
                terrain_index: terrain.index,
                fort_positive: row.fort_positive != 0,
                state: row.state,
            })
        };
        let class = match input.style.class {
            CounterClass::Division => 0,
            CounterClass::RailwayGun => 13,
            CounterClass::TaskForce => 1,
        };
        eta_derive::unit_progress(
            class,
            input.numerator.clone()?,
            input.province,
            first,
            province,
            |from, to| movement.map.distance(from, to),
            &movement.defines,
        )
        .ok_or_else(|| "native first-edge progress lacks recorded province/content operands".into())
    }

    fn hostile(&self, input: &Inputs<'_>) -> Result<bool, String> {
        route_hostile(
            self.frame,
            input.ordinary,
            input.ordinary_total,
            input.logical_owner,
            &self.wars,
        )
    }
}

fn route_hostile<'a>(
    frame: &'a DecodedFrame,
    ordinary: &[u32],
    total: usize,
    logical_owner: u32,
    owners: &RefCell<BTreeMap<u32, Result<Owner<'a>, String>>>,
) -> Result<bool, String> {
    if (total as i32) <= 0 || logical_owner as i32 <= 0 {
        return Ok(false);
    }
    if ordinary.len() != total {
        return Err("ordinary route's last province unavailable".into());
    }
    let destination = *ordinary.last().expect("positive complete route");
    if destination as i32 <= 0 {
        return Ok(false);
    }
    let province = find::<tables::Provinces>(frame, destination as u64, |row| row.id as u64)?;
    if province.controller as i32 <= 0 {
        return Ok(false);
    }
    let mut owners = owners.borrow_mut();
    owners
        .entry(logical_owner)
        .or_insert_with(|| Owner::new(frame, logical_owner))
        .as_ref()
        .map_err(Clone::clone)?
        .at_war(province.controller)
}

struct Inputs<'a> {
    province: u32,
    logical_owner: u32,
    ordinary: &'a [u32],
    ordinary_total: usize,
    alternate: &'a [u32],
    alternate_total: usize,
    numerator: Result<i64, String>,
    style: StyleInputs,
}

struct StyleInputs {
    class: CounterClass,
    retreat: Result<u8, String>,
    withdrawal: Result<u8, String>,
    mission: i32,
    target: u32,
    transfer: Result<u8, String>,
    priority: Result<i32, String>,
    orders_group: u64,
}

fn choose_style(
    input: &StyleInputs,
    hostile: impl FnOnce() -> Result<bool, String>,
) -> Result<&'static [u8], String> {
    Ok(if input.retreat.clone()? != 0 {
        b"retreatArrow"
    } else if input.withdrawal.clone()? != 0 {
        b"withdrawArrow"
    } else if input.class == CounterClass::TaskForce && 0 < input.mission && input.mission < 10 {
        b"navalMissionArrow"
    } else if input.target != 0 {
        b"supportArrow"
    } else if hostile()? {
        b"attackArrow"
    } else if input.transfer.clone()? != 0 {
        b"transportArrow"
    } else if input.class == CounterClass::Division
        && input.priority.clone()? < 1
        && input.orders_group != 0
    {
        b"followOrdersArrow"
    } else {
        b"moveArrow"
    })
}

fn recorded<T: Copy>(operand: &Recorded<Option<T>>, field: &str) -> Result<T, String> {
    operand
        .require(field)?
        .as_ref()
        .copied()
        .ok_or_else(|| format!("{field} operands unavailable"))
}

fn canonical(raw: u32, links: &BTreeMap<u32, u32>) -> Result<u32, String> {
    if raw == 0 {
        return Ok(0);
    }
    links
        .get(&raw)
        .copied()
        .ok_or_else(|| format!("country reference {raw} has no CountryLinks row"))
}

fn same_country(owner: u32, viewer: u32, links: &BTreeMap<u32, u32>) -> Result<bool, String> {
    if owner == viewer {
        return Ok(true);
    }
    if owner == 0 || viewer == 0 {
        return Ok(false);
    }
    Ok(canonical(owner, links)? == canonical(viewer, links)?)
}

fn find<T: TableSchema>(
    frame: &DecodedFrame,
    id: u64,
    key: impl Fn(&T::Row) -> u64,
) -> Result<&T::Row, String> {
    let rows = frame.complete::<T>().map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, key)
        .map(|index| &rows[index])
        .map_err(|_| format!("{:?} has no row for reference {id}", T::ID))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_arrow_uses_logical_to_controller_direction_and_unknown_is_not_move() {
        use replay_data::{
            rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow,
            rows::DirectedWars, rows::ProvinceRow, rows::TableRows, rows::WarCell,
        };
        let mut frame = test_frame(12);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 100,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 101,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 200,
                    canonical_slot: 2,
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 100,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 200,
                    canonical_slot: 2,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 20,
                    controller: 200,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 21,
                    controller: 101,
                    ..Default::default()
                },
            ]),
        );
        let packets = |active: Option<bool>| {
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 100,
                    directed_wars: Recorded::Value(Some(DirectedWars {
                        sample_hour: replay_data::rows::PacketHour::Stamped(12),
                        receiver_raw_id: 100,
                        receiver_slot: 1,
                        matrix_count: 3,
                        cells: active
                            .map(|active| WarCell {
                                target_slot: 2,
                                active,
                            })
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                CountryCounterInputRow {
                    country: 200,
                    directed_wars: Recorded::Value(Some(DirectedWars {
                        sample_hour: replay_data::rows::PacketHour::Stamped(12),
                        receiver_raw_id: 200,
                        receiver_slot: 2,
                        matrix_count: 3,
                        cells: vec![WarCell {
                            target_slot: 1,
                            active: true,
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            ])
        };
        test_put(&mut frame, packets(Some(false)));
        assert_eq!(
            route_hostile(&frame, &[20], 1, 100, &RefCell::default()),
            Ok(false)
        );
        test_put(&mut frame, packets(Some(true)));
        assert_eq!(
            route_hostile(&frame, &[20], 1, 100, &RefCell::default()),
            Ok(true)
        );
        test_put(&mut frame, packets(None));
        assert!(route_hostile(&frame, &[20], 1, 100, &RefCell::default()).is_err());
        assert_eq!(
            route_hostile(&frame, &[21], 1, 100, &RefCell::default()),
            Ok(false)
        );
        assert_eq!(
            route_hostile(&frame, &[], u32::MAX as usize, 100, &RefCell::default()),
            Ok(false)
        );
        assert_eq!(
            route_hostile(&frame, &[20], 1, 0, &RefCell::default()),
            Ok(false)
        );
    }

    fn moving() -> StyleInputs {
        StyleInputs {
            class: CounterClass::Division,
            retreat: Ok(0),
            withdrawal: Ok(0),
            mission: 0,
            target: 0,
            transfer: Ok(0),
            priority: Ok(1),
            orders_group: 0,
        }
    }

    #[test]
    fn native_precedence_does_not_require_later_operands() {
        let mut input = moving();
        input.retreat = Ok(1);
        input.withdrawal = Err("old format".into());
        assert_eq!(
            choose_style(&input, || panic!("retreat never asks hostility")).unwrap(),
            b"retreatArrow"
        );
        input.retreat = Ok(0);
        input.withdrawal = Ok(2);
        assert_eq!(
            choose_style(&input, || panic!("withdrawal never asks hostility")).unwrap(),
            b"withdrawArrow"
        );
        input.withdrawal = Ok(0);
        input.class = CounterClass::TaskForce;
        input.mission = 9;
        input.target = 1;
        assert_eq!(
            choose_style(&input, || panic!("mission never asks hostility")).unwrap(),
            b"navalMissionArrow"
        );
        input.mission = 10;
        input.transfer = Err("unavailable".into());
        assert_eq!(
            choose_style(&input, || panic!("support never asks hostility")).unwrap(),
            b"supportArrow"
        );
        input.target = 0;
        assert_eq!(choose_style(&input, || Ok(true)).unwrap(), b"attackArrow");
        assert!(choose_style(&input, || Ok(false)).is_err());
    }

    #[test]
    fn follow_orders_is_class_zero_and_reads_signed_priority() {
        let mut input = moving();
        input.orders_group = 5;
        input.priority = Ok(-1);
        assert_eq!(
            choose_style(&input, || Ok(false)).unwrap(),
            b"followOrdersArrow"
        );
        input.priority = recorded(&Recorded::NotRecorded, "DivisionMovement.movement_priority");
        assert_eq!(
            choose_style(&input, || Ok(false)).unwrap_err(),
            "DivisionMovement.movement_priority not recorded"
        );
        input.class = CounterClass::RailwayGun;
        assert_eq!(choose_style(&input, || Ok(false)).unwrap(), b"moveArrow");
        input.class = CounterClass::TaskForce;
        for mission in [-1, 0, 10] {
            input.mission = mission;
            assert_eq!(choose_style(&input, || Ok(false)).unwrap(), b"moveArrow");
        }
    }

    #[test]
    fn roster_admission_compares_raw_then_nonzero_aliases() {
        let links = BTreeMap::from([(1, 7), (2, 7), (3, 8)]);
        assert!(same_country(1, 1, &BTreeMap::new()).unwrap());
        assert!(same_country(0, 0, &BTreeMap::new()).unwrap());
        assert!(!same_country(0, 1, &links).unwrap());
        assert!(same_country(1, 2, &links).unwrap());
        assert!(!same_country(1, 3, &links).unwrap());
        assert!(
            same_country(1, 4, &links)
                .unwrap_err()
                .contains("CountryLinks")
        );
    }
}
