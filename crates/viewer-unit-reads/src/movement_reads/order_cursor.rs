
use super::*;
use viewer_runtime::unit_handle::Unit;
use content_reader_unit_path::unit_path::{
    CommonClass, CommonMoveWorld, CommonUnit,
    order_cursor::{CursorFacts, CursorRequest, FirstSelected, OrderCursor},
};
use replay_frame::frame::DecodedFrame;

const QUEUED_ACTIONS_NOT_RECORDED: &str =
    "order cursor: units' queued actions are not recorded";
const QUEUED_ACTIONS_REFUSED: &str =
    "order cursor: the unit's queued actions were refused when read";
const THEATRE_AREAS_NOT_RECORDED: &str =
    "order cursor: land theatres' controller areas are not recorded";
const TRUCE_NOT_RECORDED: &str = "order cursor: truces are not recorded";

pub fn order_cursor(
    frame: &DecodedFrame,
    reads: ReadOperands<'_>,
    first: Unit,
    target: u32,
    viewer: Country,
    shift: bool,
) -> Fact<OrderCursor> {
    let world = World::read(frame, reads.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    Moves::for_common(&world, reads, &cells, &common)?.order_cursor(first, target, viewer, shift)
}

pub fn rule_relation(
    frame: &DecodedFrame,
    reads: ReadOperands<'_>,
    rule: &content_reader_map::adjacency_rules::AdjacencyRule,
    country: Country,
) -> Fact<content_reader_unit_path::unit_path::RuleReason> {
    if let Some(reason) = &rule.unsupported {
        return Err(format!("adjacency rule {}: {reason}", rule.name));
    }
    let world = World::read(frame, reads.trigger_content.facts());
    let cells = Cells::default();
    let common = CommonCells::new(frame);
    let moves = Moves::for_common(&world, reads, &cells, &common)?;
    let graph = reads.moves.as_ref().map_err(Clone::clone)?;
    Mover { graph, world: &moves }.common_rule_relation(rule, country)
}

impl Moves<'_, '_> {
    fn order_cursor(
        &self,
        first: Unit,
        target: u32,
        viewer: Country,
        shift: bool,
    ) -> Fact<OrderCursor> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let viewer_raw = self.actual_country_raw(viewer)?;
        let mover = Mover { graph, world: self };
        let facts = Facts { moves: self };
        let (class, handle, (army, logical_owner_raw, combat_count)) = match first {
            Unit::TaskForce(_) => {
                let request = CursorRequest {
                    viewer_raw,
                    first: FirstSelected::TaskForce,
                    target,
                    shift,
                };
                return mover.order_cursor(&request, &facts);
            }
            Unit::Division(handle) => {
                (CommonClass::Army, handle, self.common_army_scalars(handle)?)
            }
            Unit::RailwayGun(handle) => (
                CommonClass::RailwayGun,
                handle,
                self.common_gun_scalars(handle)?,
            ),
        };
        let unit = CommonUnit {
            class,
            handle,
            logical_owner_raw,
            combat_count,
            army: &army,
        };
        let request = CursorRequest {
            viewer_raw,
            first: FirstSelected::Unit(unit),
            target,
            shift,
        };
        mover.order_cursor(&request, &facts)
    }
}

fn queued(count: &replay_data::rows::Recorded<Option<i32>>) -> Fact<bool> {
    match count {
        replay_data::rows::Recorded::Value(Some(count)) => Ok(*count != 0),
        replay_data::rows::Recorded::Value(None) => Err(QUEUED_ACTIONS_REFUSED.into()),
        replay_data::rows::Recorded::NotRecorded => Err(QUEUED_ACTIONS_NOT_RECORDED.into()),
    }
}

struct Facts<'m, 'w, 'a> {
    moves: &'m Moves<'w, 'a>,
}

impl CursorFacts for Facts<'_, '_, '_> {
    fn queued_actions(&self, unit: &CommonUnit<'_>) -> Fact<bool> {
        queued(match unit.class {
            CommonClass::Army => &self.moves.movement(unit.handle)?.queued_actions,
            CommonClass::RailwayGun => &self.moves.common_gun(unit.handle)?.queued_actions,
        })
    }

    fn route_end(&self, unit: &CommonUnit<'_>, alternate: bool) -> Fact<Option<u32>> {
        let route = match unit.class {
            CommonClass::Army => {
                let row = self.moves.movement(unit.handle)?;
                let (total, path) = if alternate {
                    (row.alternate_total, &row.alternate_path)
                } else {
                    (row.path_total, &row.path)
                };
                if total as i32 <= 0 {
                    return Ok(None);
                }
                if path.len() != total as usize {
                    return Err(format!(
                        "order cursor: division {:#x} route copy is incomplete",
                        unit.handle
                    ));
                }
                path
            }
            CommonClass::RailwayGun => {
                let row = self.moves.common_gun(unit.handle)?;
                if alternate {
                    &row.alternate_path
                } else {
                    &row.ordinary_path
                }
            }
        };
        Ok(route.last().copied())
    }

    fn theatre_holds_area(&self, _: u32, _: u32) -> Fact<bool> {
        Err(THEATRE_AREAS_NOT_RECORDED.into())
    }

    fn truce(&self, _: Country, _: Country) -> Fact<bool> {
        Err(TRUCE_NOT_RECORDED.into())
    }

    fn roster_owner(&self, handle: u64, province: u32) -> Fact<u32> {
        let moves = self.moves;
        let division = moves
            .division(handle)?
            .map(|row| (row.province, row.roster_owner));
        let forces = moves.table::<tables::TaskForces>()?;
        let force = forces
            .binary_search_by_key(&handle, |row| row.id)
            .ok()
            .map(|at| (forces[at].province, forces[at].roster_owner));
        let guns = moves.table::<tables::RailwayGunCounterInputs>()?;
        let gun = guns
            .binary_search_by_key(&handle, |row| row.gun)
            .ok()
            .map(|at| (guns[at].province, guns[at].roster_owner));
        match (division, force, gun) {
            (Some(found), None, None) | (None, Some(found), None) | (None, None, Some(found))
                if found.0 == province =>
            {
                Ok(found.1)
            }
            _ => Err(format!(
                "order cursor: occupant {handle:#x} of province {province} has no single unit row there"
            )),
        }
    }

    fn state_provinces(&self, province: u32) -> Fact<Vec<u32>> {
        let Some(state) = self.moves.state_of(province)? else {
            return Ok(Vec::new());
        };
        self.moves
            .assets
            .trigger_content
            .state_provinces
            .get(&state)
            .cloned()
            .ok_or_else(|| format!("order cursor: state {state} has no provinces in the content"))
    }

    fn reveal_skip(&self, unit: &CommonUnit<'_>) -> Fact<bool> {
        let row = self.moves.division(unit.handle)?.ok_or_else(|| {
            format!(
                "order cursor: division {:#x} is missing from Divisions",
                unit.handle
            )
        })?;
        Ok(row.reveal_skip != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::Recorded;

    #[test]
    fn a_recorded_order_list_answers_queued_and_its_absence_is_named() {
        assert_eq!(queued(&Recorded::Value(Some(2))), Ok(true));
        assert_eq!(queued(&Recorded::Value(Some(0))), Ok(false));
        assert_eq!(queued(&Recorded::Value(None)), Err(QUEUED_ACTIONS_REFUSED.to_owned()));
        assert_eq!(queued(&Recorded::NotRecorded), Err(QUEUED_ACTIONS_NOT_RECORDED.to_owned()));
    }
}
