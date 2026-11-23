
use content_reader_core::map::{MapData, ProvinceKind};
use crate::land::{CounterTables, GunUnit, LandUnit};
use dashboard_text::table_access::TokenKeys;
use std::collections::BTreeSet;
use viewer_counter_kit::{CounterMap, type_tint, unit_flag_owner};
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::perspective::{SharingUnknown, Sight, Vision};
use viewer_foundation::unit_rules::SHIP_IN_PORT;
use viewer_unit_reads::{counter_action, counter_move, counter_status};
use replay_data::rows::{RailwayGunCounterInputRow, RailwayGunEtaOperands};
use viewer_unit_reads::railway_gun_counter::{self as guns, GunOperands, RailStatus};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GunFields<'a> {
    pub id: u64,
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
    pub logical_owner: u32,
    pub province: u32,
    pub assignee_group: u64,
    pub subunit_token: u32,
    pub current_strength: i64,
    pub maximum_strength: i64,
    pub received_supply_ratio: i64,
    pub combat_role: u8,
    pub combat_province: u32,
    pub operands: GunOperands,
    pub ordinary_path: &'a [u32],
    pub alternate_path: &'a [u32],
    pub eta: Option<RailwayGunEtaOperands>,
}

impl<'a> GunFields<'a> {
    pub fn of(row: &'a RailwayGunCounterInputRow) -> Self {
        Self {
            id: row.gun,
            roster_owner: row.roster_owner,
            expeditionary_owner: row.expeditionary_owner,
            logical_owner: row.logical_owner,
            province: row.province,
            assignee_group: row.assignee_group,
            subunit_token: row.subunit_token,
            current_strength: row.current_strength,
            maximum_strength: row.maximum_strength,
            received_supply_ratio: row.received_supply_ratio,
            combat_role: row.combat_role,
            combat_province: row.combat_province,
            operands: GunOperands::of(row),
            ordinary_path: &row.ordinary_path,
            alternate_path: &row.alternate_path,
            eta: row.eta.require("RailwayGunCounterInputs.eta").ok().copied().flatten(),
        }
    }

    pub fn route(&self) -> counter_move::UnitRoute<'a> {
        counter_move::UnitRoute {
            class: viewer_runtime::unit_class::CounterClass::RailwayGun,
            combat_role: self.combat_role,
            combat_count: self.operands.combat_count,
            combat_province: self.combat_province,
            target: self.operands.target,
            naval_transfer: self.operands.naval_transfer != 0,
            path_total: self.ordinary_path.len() as u32,
            path: self.ordinary_path,
            alternate_total: self.alternate_path.len() as u32,
            alternate_path: self.alternate_path,
        }
    }
}

#[derive(Clone, Copy)]
pub struct GunGround<'a> {
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    pub land: &'a dyn Fn(u32) -> bool,
    pub movement: Option<&'a MovementContent>,
}

impl<'a> GunGround<'a> {
    pub fn anchor_of(content: &CounterMap) -> impl Fn(u32, u32) -> Option<[f32; 3]> + '_ {
        |id, slot| content.anchor(id, slot)
    }

    pub fn land_of(map: &MapData) -> impl Fn(u32) -> bool + '_ {
        |province| matches!(map.definitions.get(&province).map(|it| it.kind), Some(ProvinceKind::Land))
    }
}

pub trait GunJoins {
    fn owner(&self, raw: u32) -> Option<u32>;
    fn mixed(&self, gun: u64, province: u32) -> Option<Option<u32>>;
    fn vision(&self) -> &Vision;
    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side>;
    fn name(&self, token: u32) -> Result<String, String>;
    fn rail(&self, province: u32) -> Option<RailStatus>;
    fn destination_enemy(&self, gun: &GunFields<'_>) -> Result<bool, &'static str>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GunMember {
    pub id: u64,
    pub province: u32,
    pub kind: u32,
    pub ordinal: u32,
    pub owner: u32,
    pub logical: Option<u32>,
    pub flag_owner: u32,
    pub sight: Sight,
    pub shared_vision: Result<bool, SharingUnknown>,
    pub type_tint: Option<type_tint::Side>,
    pub orders_group: u64,
    pub hp: i64,
    pub supply_ratio: i64,
    pub naval_transfer: bool,
    pub own: bool,
    pub name: String,
    pub timing: viewer_native_gui::counter_hovers::land::Timing,
    pub location_warning: Option<bool>,
    pub join_action: u8,
    pub action: counter_action::Action,
    pub unplaced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gun {
    Member(GunMember),
    Out(&'static str),
    Unnamed(String),
}

pub fn gun_member(gun: &GunFields<'_>, ground: GunGround<'_>, joins: &impl GunJoins) -> Gun {
    if gun.province == 0 {
        return Gun::Out("unlocated");
    }
    let Some(slot) = joins.owner(gun.roster_owner) else {
        return Gun::Out("roster owner not linked");
    };
    let ordinal = match joins.mixed(gun.id, gun.province) {
        Some(Some(ordinal)) => ordinal,
        Some(None) => return Gun::Out("not in its province's mixed array"),
        None => 0,
    };
    let vision = joins.vision();
    let sight = vision.sees_land(gun.id, slot, gun.province);
    if sight == Sight::None {
        return Gun::Out("unseen");
    }
    let name = match joins.name(gun.subunit_token) {
        Ok(name) => name,
        Err(reason) => return Gun::Unnamed(reason),
    };
    let route = gun.route();
    let class = viewer_runtime::unit_class::CounterClass::RailwayGun;
    let placed = ground.movement.and_then(|movement| {
        route.stack_kind(gun.province, &movement.map, &|id| (ground.anchor)(id, class.standstill(true)))
    });
    let unplaced = placed.is_none() && !route.display_path().is_empty();
    let kind = placed.unwrap_or_else(|| {
        counter_move::stack_kind(counter_move::StackKindInputs {
            class,
            combat_role: gun.combat_role,
            ..Default::default()
        })
    });
    if kind == SHIP_IN_PORT {
        return Gun::Out("docked");
    }
    let on_land = ground
        .movement
        .and_then(|movement| movement.map.terrain(gun.province))
        .map_or_else(|| (ground.land)(gun.province), |terrain| terrain.flags & 1 != 0);
    let location_warning = guns::location_warning_of(&gun.operands, on_land, joins.rail(gun.province));
    let owner = |raw| joins.owner(raw);
    Gun::Member(GunMember {
        id: gun.id,
        province: gun.province,
        kind,
        ordinal,
        owner: slot,
        logical: joins.owner(gun.logical_owner),
        flag_owner: unit_flag_owner(slot, gun.expeditionary_owner, false, &owner),
        sight,
        shared_vision: vision.shares_vision_proven(slot),
        type_tint: joins.type_tint(gun.roster_owner, gun.logical_owner),
        orders_group: gun.assignee_group,
        hp: guns::strength_ratio(gun.current_strength, gun.maximum_strength),
        supply_ratio: gun.received_supply_ratio,
        naval_transfer: gun.operands.naval_transfer != 0,
        own: slot == vision.ours(),
        name,
        timing: viewer_native_gui::counter_hovers::land::Timing {
            next: ground.movement.and_then(|content| first_edge(gun, content)),
            kind: placed,
        },
        location_warning,
        join_action: guns::join_action_of(&gun.operands),
        action: guns::action_code_of(&gun.operands, joins.destination_enemy(gun)),
        unplaced,
    })
}

fn first_edge(gun: &GunFields<'_>, content: &MovementContent) -> Option<i64> {
    replay_derive::eta_derive::railway_gun_first_edge_of(
        gun.id,
        gun.province,
        gun.ordinary_path.first().copied(),
        || gun.eta,
        |id| {
            content.map.terrain(id).map(|terrain| replay_derive::eta_derive::ProvinceOperands {
                terrain_flags: terrain.flags,
                ..Default::default()
            })
        },
        |from, to| content.map.distance(from, to),
        &content.defines,
    )
}

pub fn gun_name(spelling: Result<&str, String>, subunits: Result<&TokenKeys, &str>) -> Result<String, String> {
    let spelling = spelling?;
    subunits
        .map_err(|reason| format!("sub-units unavailable: {reason}"))?
        .resolve(spelling)?
        .map(str::to_owned)
        .ok_or_else(|| format!("sub-unit {spelling} is not authored"))
}

impl GunMember {
    pub fn unit(self, position: Option<counter_move::Position>) -> LandUnit<'static> {
        LandUnit {
            ordinal: self.ordinal,
            alternate_template: 0,
            shared_vision: self.shared_vision,
            separate: false,
            id: self.id,
            type_tint: self.type_tint,
            owner: self.owner,
            logical: self.logical,
            flag_owner: self.flag_owner,
            template: 0,
            orders_group: self.orders_group,
            hq: false,
            sight: self.sight,
            hp: self.hp,
            org: 0,
            bars: true,
            status_recorded: true,
            status: counter_status::Member {
                supply_ratio: counter_status::SupplyRatio::exact(self.supply_ratio),
                naval_transfer: Some(self.naval_transfer),
                own: self.own,
                ..Default::default()
            },
            movement: None,
            action: Ok(1),
            group_parent: None,
            parent_branch: None,
            withdrawing: None,
            defending_area: None,
            commander: Some(None),
            gun: Some(GunUnit {
                name: self.name,
                timing: self.timing,
                location_warning: self.location_warning,
                join_action: self.join_action,
                action: self.action,
                position,
            }),
        }
    }
}

pub struct TodayGunJoins<'a, 'b> {
    pub land: &'a CounterTables<'b>,
    pub owner: &'a dyn Fn(u32) -> Option<u32>,
}

impl GunJoins for TodayGunJoins<'_, '_> {
    fn owner(&self, raw: u32) -> Option<u32> {
        (self.owner)(raw)
    }

    fn mixed(&self, gun: u64, province: u32) -> Option<Option<u32>> {
        self.land.order.map(|order| order.mixed(gun, province))
    }

    fn vision(&self) -> &Vision {
        self.land.vision
    }

    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side> {
        self.land.type_tints.of(roster, logical)
    }

    fn name(&self, token: u32) -> Result<String, String> {
        self.land.guns.names.get(&token).cloned().unwrap_or_else(|| Err(format!("token {token} was not asked")))
    }

    fn rail(&self, province: u32) -> Option<RailStatus> {
        self.land.guns.rail.as_ref().and_then(|rail| rail.get(&province).copied())
    }

    fn destination_enemy(&self, gun: &GunFields<'_>) -> Result<bool, &'static str> {
        self.land.guns.destination_enemy.get(&gun.id).copied().unwrap_or(Err("last ordinary-route province hostility"))
    }
}

pub struct TodayGuns {
    pub guns: Vec<(u64, Gun)>,
    pub complete: bool,
    pub recorded: usize,
    pub refused: BTreeSet<String>,
}
