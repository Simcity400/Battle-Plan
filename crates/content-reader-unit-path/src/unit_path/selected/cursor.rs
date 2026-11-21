
use super::{CommonClass, CommonMoveWorld, CommonUnit, Country, Fact, MoveOptions, Mover, Silent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderCursor {
    NoMove,
    FriendlyMove,
    AttackMove,
    MoveToEnemyProvince,
    AddUnits,
    ChainMove,
    ChainMoveNaval,
}

pub enum FirstSelected<'a> {
    TaskForce,
    Unit(CommonUnit<'a>),
}

pub struct CursorRequest<'a> {
    pub viewer_raw: u32,
    pub first: FirstSelected<'a>,
    pub target: u32,
    pub shift: bool,
}

pub trait CursorFacts {
    fn queued_actions(&self, unit: &CommonUnit<'_>) -> Fact<bool>;
    fn route_end(&self, unit: &CommonUnit<'_>, alternate: bool) -> Fact<Option<u32>>;
    fn theatre_holds_area(&self, owner_raw: u32, area: u32) -> Fact<bool>;
    fn truce(&self, receiver: Country, target: Country) -> Fact<bool>;
    fn roster_owner(&self, handle: u64, province: u32) -> Fact<u32>;
    fn state_provinces(&self, province: u32) -> Fact<Vec<u32>>;
    fn reveal_skip(&self, unit: &CommonUnit<'_>) -> Fact<bool>;
}

pub const TASK_FORCE_ENTRY_UNRECORDED: &str = "order cursor: a task force's province-entry inputs (fleet base, mission, navy range and theatres) are not recorded";

impl<W: CommonMoveWorld + ?Sized> Mover<'_, W> {
    pub fn order_cursor(
        &self,
        request: &CursorRequest<'_>,
        facts: &impl CursorFacts,
    ) -> Fact<OrderCursor> {
        match &request.first {
            FirstSelected::TaskForce => self.naval_cursor(request, facts),
            FirstSelected::Unit(unit) => self.land_cursor(request, unit, facts),
        }
    }

    fn naval_cursor(
        &self,
        request: &CursorRequest<'_>,
        facts: &impl CursorFacts,
    ) -> Fact<OrderCursor> {
        let world = self.world;
        let target = request.target;
        let kind = self.graph.kind(target)?;
        if kind & 3 == 0 {
            for &handle in world.province_units(target)? {
                let owner = world.canonical_country(facts.roster_owner(handle, target)?)?;
                if self.raw_at_war(owner, request.viewer_raw)? {
                    return Ok(OrderCursor::AttackMove);
                }
            }
            return Ok(OrderCursor::FriendlyMove);
        }
        if kind & 2 != 0 || world.impassable(target)? {
            return Ok(OrderCursor::NoMove);
        }
        let mut base = target;
        if kind & 1 != 0 && !world.has_naval_base(target)? {
            for province in facts.state_provinces(target)? {
                if world.has_naval_base(province)? {
                    base = province;
                    break;
                }
            }
        }
        if !world.has_naval_base(base)? {
            return Ok(OrderCursor::NoMove);
        }
        if self.raw_same(request.viewer_raw, world.state_owner_raw(base)?)? {
            return Ok(OrderCursor::FriendlyMove);
        }
        Err(TASK_FORCE_ENTRY_UNRECORDED.into())
    }

    fn land_cursor(
        &self,
        request: &CursorRequest<'_>,
        unit: &CommonUnit<'_>,
        facts: &impl CursorFacts,
    ) -> Fact<OrderCursor> {
        let world = self.world;
        let target = request.target;
        let can_move = |transport_fallback, avoid_enemy| {
            let options = MoveOptions {
                transport_fallback,
                retreat_bypass: request.shift,
                avoid_enemy,
            };
            self.selected_move(unit, target, options, &mut String::new(), &Silent)
        };
        if self.graph.kind(unit.army.location)? & 3 == 0 {
            let avoid = unit.class == CommonClass::Army && facts.reveal_skip(unit)?;
            return Ok(if can_move(false, avoid)? {
                OrderCursor::AddUnits
            } else {
                OrderCursor::NoMove
            });
        }
        let kind = self.graph.kind(target)?;
        if kind & 2 != 0 || world.impassable(target)? || !can_move(true, false)? {
            return Ok(OrderCursor::NoMove);
        }
        let controller = world.controller_raw(target)?;
        if !self.raw_same(request.viewer_raw, controller)? {
            let viewer = world.canonical_country(request.viewer_raw)?;
            if unit.class == CommonClass::Army && self.raw_at_war(viewer, controller)? {
                return Ok(OrderCursor::AttackMove);
            }
            if kind & 3 == 0 {
                return Ok(OrderCursor::NoMove);
            }
            let owner_raw = world.state_owner_raw(target)?;
            if owner_raw as i32 > 0 {
                return Ok(OrderCursor::FriendlyMove);
            }
            let owner = world.canonical_country(owner_raw)?;
            return Ok(
                if world.military_access(viewer, owner)? || facts.truce(viewer, owner)? {
                    OrderCursor::FriendlyMove
                } else {
                    OrderCursor::MoveToEnemyProvince
                },
            );
        }
        let location = unit.army.location;
        if !request.shift && world.controller_area(location)? == world.controller_area(target)? {
            return Ok(OrderCursor::FriendlyMove);
        }
        if world.has_naval_base(location)? && world.has_naval_base(target)? {
            if !request.shift {
                return Ok(OrderCursor::FriendlyMove);
            }
            if !facts.queued_actions(unit)? {
                return Ok(OrderCursor::ChainMoveNaval);
            }
            return self.chain_cursor(unit, target, true, facts);
        }
        if request.shift {
            let queued = facts.queued_actions(unit)?;
            return self.chain_cursor(unit, target, queued, facts);
        }
        self.theatre_cursor(unit, target, facts)
    }

    fn chain_cursor(
        &self,
        unit: &CommonUnit<'_>,
        target: u32,
        queued: bool,
        facts: &impl CursorFacts,
    ) -> Fact<OrderCursor> {
        let world = self.world;
        let last = if queued && let Some(last) = facts.route_end(unit, true)? {
            Some(last)
        } else {
            let ordinary = facts.route_end(unit, false)?;
            if !queued && ordinary.is_none() {
                if world.controller_area(unit.army.location)? == world.controller_area(target)? {
                    return Ok(OrderCursor::ChainMove);
                }
                return self.theatre_cursor(unit, target, facts);
            }
            ordinary
        };
        let Some(last) = last else {
            return Ok(OrderCursor::NoMove);
        };
        if last == 0 || self.graph.dense(last).is_none() {
            return Ok(OrderCursor::NoMove);
        }
        if world.controller_area(last)? == world.controller_area(target)? {
            return Ok(OrderCursor::ChainMove);
        }
        Ok(
            if world.has_naval_base(last)? && world.has_naval_base(target)? {
                OrderCursor::ChainMoveNaval
            } else {
                OrderCursor::NoMove
            },
        )
    }

    fn theatre_cursor(
        &self,
        unit: &CommonUnit<'_>,
        target: u32,
        facts: &impl CursorFacts,
    ) -> Fact<OrderCursor> {
        let world = self.world;
        Ok(
            if world.unit_exiled_raw(unit)? != 0
                || facts
                    .theatre_holds_area(unit.logical_owner_raw, world.controller_area(target)?)?
            {
                OrderCursor::FriendlyMove
            } else {
                OrderCursor::NoMove
            },
        )
    }
}
