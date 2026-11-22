
use replay_data::{rows::ArmyOrderPlanRow, rows::ArmyOrderRow, rows::OrdersGroupRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, sync::OnceLock};
use dashboard_text::triggers::World;
use viewer_unit_reads::movement_reads::{Cells, Moves, ReadOperands};

pub struct Routes<'a> {
    frame: &'a DecodedFrame,
    assets: Option<ReadOperands<'a>>,
    world: OnceLock<Result<World<'a>, String>>,
    cells: Cells,
    routes: BTreeMap<super::OrderKey, Result<Vec<u32>, String>>,
}

impl<'a> Routes<'a> {
    pub fn new(frame: &'a DecodedFrame, assets: Option<ReadOperands<'a>>) -> Self {
        Self {
            frame,
            assets,
            world: OnceLock::new(),
            cells: Cells::default(),
            routes: BTreeMap::new(),
        }
    }

    pub fn route(
        &mut self,
        order: &ArmyOrderRow,
        plan: &ArmyOrderPlanRow,
        group: &OrdersGroupRow,
    ) -> Result<Vec<u32>, String> {
        let key = (order.owner_group, order.instance);
        if let Some(route) = self.routes.get(&key) {
            return route.clone();
        }
        let route = self.build(order, plan, group);
        self.routes.insert(key, route.clone());
        route
    }

    fn build(
        &self,
        order: &ArmyOrderRow,
        plan: &ArmyOrderPlanRow,
        group: &OrdersGroupRow,
    ) -> Result<Vec<u32>, String> {
        let Some(&target) = order.path.last() else {
            return Ok(Vec::new());
        };
        if (order.paradrop_target as i32) < 1 {
            return Ok(Vec::new());
        }
        let assets = self
            .assets
            .ok_or("invasion movement content is unavailable")?;
        let world = self
            .world
            .get_or_init(|| {
                World::new(self.frame, assets.trigger_content.facts())
                    .map_err(|why| why.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let theatre = self
            .frame
            .complete::<tables::LandTheatres>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|t| t.id == group.theatre)
            .ok_or_else(|| format!("invasion theatre {:#x} is not recorded", group.theatre))?;
        let raw = if theatre.volunteers_theatre > 0 {
            theatre.volunteers_theatre as u32
        } else {
            theatre.owner
        };
        let country = world
            .slot(raw)
            .ok_or_else(|| format!("invasion request country {raw} has no link"))?;
        let moves = Moves::new(world, assets, &self.cells);
        let Some((from, to)) = moves.invasion_seas(order.paradrop_target, target)? else {
            return Ok(Vec::new());
        };
        let mut unit = plan
            .assigned_divisions
            .first()
            .copied()
            .filter(|&id| id != 0);
        if unit.is_none() {
            unit = self
                .frame
                .complete::<tables::OrdersGroupMembers>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|m| m.group == group.handle && m.ordinal == 0)
                .map(|m| m.unit)
                .filter(|&id| id != 0);
        }
        if unit.is_none() {
            let own = world
                .raw_country(country)
                .ok_or_else(|| format!("invasion request country {raw} has no country row"))?;
            unit = self
                .frame
                .complete::<tables::CountryCounterInputs>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|row| row.country == own)
                .ok_or_else(|| format!("invasion request country {raw} has no recorded roster"))?
                .first_division
                .into();
        }
        let Some(unit) = unit.filter(|&id| id != 0) else {
            return Ok(Vec::new());
        };
        let recorded_owner = self
            .frame
            .complete::<tables::InvasionPreparations>()
            .ok()
            .and_then(|rows| {
                rows.iter().find(|row| {
                    (row.owner_group, row.instance) == (order.owner_group, order.instance)
                })
            })
            .map(|row| row.unit_owner);
        moves.invasion_route(unit, recorded_owner, raw, from, to)
    }
}
