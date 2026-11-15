//! The hour part of a railway gun's land counter: the source it stands in (province and stack
//! kind), its ordinal there and the operands its plate joins and draws by, decided from values.
//! Today's hour tables (`add_railway_guns`) and the map's gun counter layer both decide through
//! [`gun_member`]; only how they join the hour's tables differs ([`GunJoins`]).

use super::*;
use replay_data::rows::{RailwayGunCounterInputRow, RailwayGunEtaOperands};
use viewer_foundation::map::railway_gun_counter::{self as guns, GunOperands, RailStatus};

/// One `RailwayGunCounterInputs` row's values.
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
    /// The recorded first-edge operands; `None` when not recorded or refused.
    pub eta: Option<RailwayGunEtaOperands>,
}

impl<'a> GunFields<'a> {
    pub(super) fn of(row: &'a RailwayGunCounterInputRow) -> Self {
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

    /// Its class-13 route ([`counter_move::UnitRoute::railway_gun`]).
    pub fn route(&self) -> counter_move::UnitRoute<'a> {
        counter_move::UnitRoute {
            class: viewer_foundation::unit_class::CounterClass::RailwayGun,
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

/// What places a gun: the content's stack anchors, which provinces the map defines as land, and
/// the movement map.
#[derive(Clone, Copy)]
pub struct GunGround<'a> {
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    pub land: &'a dyn Fn(u32) -> bool,
    pub movement: Option<&'a MovementContent>,
}

impl<'a> GunGround<'a> {
    /// The ground of the counters' content and map; `anchor` and `land` are
    /// [`Self::anchor_of`] and [`Self::land_of`] of them.
    pub fn anchor_of(content: &CounterContent) -> impl Fn(u32, u32) -> Option<[f32; 3]> + '_ {
        |id, slot| content.anchor(id, slot)
    }

    pub fn land_of(map: &MapData) -> impl Fn(u32) -> bool + '_ {
        |province| matches!(map.definitions.get(&province).map(|it| it.kind), Some(ProvinceKind::Land))
    }
}

/// What a gun's member reads besides its own row.
pub trait GunJoins {
    /// A raw country reference's canonical slot.
    fn owner(&self, raw: u32) -> Option<u32>;
    /// `None` without a whole occupant order; `Some(None)` when `province`'s mixed array does not
    /// hold the gun.
    fn mixed(&self, gun: u64, province: u32) -> Option<Option<u32>>;
    fn vision(&self) -> &Vision;
    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side>;
    /// The authored sub-unit key its token names this hour, or why the hour cannot name it.
    fn name(&self, token: u32) -> Result<String, String>;
    /// The province's rail status this hour; `None` when the hour cannot give it.
    fn rail(&self, province: u32) -> Option<RailStatus>;
    /// Whether its last ordinary-route province's controller is at war with its logical owner.
    fn destination_enemy(&self, gun: &GunFields<'_>) -> Result<bool, &'static str>;
}

/// A railway gun as one counter member.
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
    /// Fixed5 [`guns::strength_ratio`].
    pub hp: i64,
    pub supply_ratio: i64,
    pub naval_transfer: bool,
    pub own: bool,
    pub name: String,
    /// The native first-edge duration and the route's placed kind (none when not placed).
    pub timing: viewer_foundation::map::counter_hovers::land::Timing,
    pub location_warning: Option<bool>,
    pub join_action: u8,
    pub action: counter_action::Action,
    /// A route the content cannot place: it stands on its province.
    pub unplaced: bool,
}

/// What today's pass makes of a gun.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gun {
    Member(GunMember),
    /// Not offered (unlocated, unlinked, not in its province's array, unseen, docked), by why.
    Out(&'static str),
    /// Offered but its sub-unit cannot be named this hour: counted unavailable, by why.
    Unnamed(String),
}

/// One gun's hour part, as the counters' class-13 pass decides it.
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
    let class = viewer_foundation::unit_class::CounterClass::RailwayGun;
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
    // The game skips source bucket 19 for every class: a docked gun is not on the map.
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
        timing: viewer_foundation::map::counter_hovers::land::Timing {
            next: ground.movement.and_then(|content| first_edge(gun, content)),
            kind: placed,
        },
        location_warning,
        join_action: guns::join_action_of(&gun.operands),
        action: guns::action_code_of(&gun.operands, joins.destination_enemy(gun)),
        unplaced,
    })
}

/// The native first-edge duration over the content's movement map.
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

/// The authored sub-unit key a gun's token names, from the hour's spelling of it; why not.
pub fn gun_name(spelling: Result<&str, String>, subunits: Result<&TokenKeys, &str>) -> Result<String, String> {
    let spelling = spelling?;
    subunits
        .map_err(|reason| format!("sub-units unavailable: {reason}"))?
        .resolve(spelling)?
        .map(str::to_owned)
        .ok_or_else(|| format!("sub-unit {spelling} is not authored"))
}

impl GunMember {
    /// The land unit it stands for, at `position`.
    pub(super) fn unit(self, position: Option<counter_move::Position>) -> LandUnit<'static> {
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
            status: counter_status::Member {
                supply_ratio: self.supply_ratio,
                naval_transfer: self.naval_transfer,
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

/// Today's joins over the hour's tables.
pub(super) struct TodayGunJoins<'a, 'b> {
    pub(super) land: &'a CounterTables<'b>,
    pub(super) owner: &'a dyn Fn(u32) -> Option<u32>,
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

/// Today's railway guns as values: each gun's decision, the census and the names refused.
pub struct TodayGuns {
    pub guns: Vec<(u64, Gun)>,
    /// Whether the gun census is whole; when not, no gun is decided.
    pub complete: bool,
    pub recorded: usize,
    pub refused: BTreeSet<String>,
}

impl HourTables {
    /// Every recorded railway gun's hour part as today's hour tables decide it.
    pub fn gun_members(&self, ground: GunGround<'_>) -> TodayGuns {
        let land = self.land();
        let owner = |raw: u32| self.links.get(&raw).copied();
        let joins = TodayGunJoins { land: &land, owner: &owner };
        let guns = land
            .guns
            .rows
            .iter()
            .flatten()
            .map(|row| (row.gun, gun_member(&GunFields::of(row), ground, &joins)))
            .collect();
        TodayGuns { guns, complete: land.guns.rows.is_some(), recorded: land.guns.recorded, refused: land.guns.refused.clone() }
    }
}

impl HourTables {
    /// Each moving gun's destination hostility this hour: the relations a gun counter layer is
    /// given until it reads them from their owner.
    pub fn gun_hostility(&self) -> BTreeMap<u64, Result<bool, &'static str>> {
        self.guns.destination_enemy.clone()
    }
}
