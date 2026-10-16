//! Shared primary unit base for divisions and task forces.

use crate::{Memory, carmy, cpdx_array::Array, memory::FieldBatch, read};

mod combat_references;
pub use combat_references::CombatReferencesCapture;
pub use combat_references::{COMBAT_BORDER_WAR, COMBAT_LAND, COMBAT_NAVAL, MapCombatReference};
pub(crate) use combat_references::{MapCombatIdentityMemo, read_unit_combat_references};
/// Native counter references resolve to the secondary reference object at
/// primary+10, and the counter consumes classes 0, 1 and 13. The reference type
/// is not a class witness: id assignment replaces a null or colliding id with a
/// dynamic id, type 0x1268 plus the dynamic type counter (4713 in saved
/// divisions). Exact vtable, class and registry join authenticate.
pub fn counter_identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    registry: &mut crate::creference_registry::HeaderResolver,
) -> Result<(u64, u32), &'static str> {
    counter_identity_with(memory, base, pointer, |handle| {
        registry.resolve(memory, base, handle)
    })
}

/// The same native identity with a capture-owned resolver. Transport retains
/// failed registry roots for its entire quiescent pass as well as failed units.
pub fn counter_identity_with(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    resolve: impl FnOnce(u64) -> Result<Option<usize>, &'static str>,
) -> Result<(u64, u32), &'static str> {
    let header = crate::read::<[u8; 32]>(memory, pointer)
        .filter(|_| pointer != 0)
        .ok_or("counter unit header unreadable")?;
    let class = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let handle = u64::from_le_bytes(header[24..32].try_into().unwrap());
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
    if ![0, 1, 13].contains(&class)
        || handle >> 32 == 0
        || (class == 0
            && vtable
                != base
                    .checked_add(crate::carmy::VTABLE_RVA)
                    .ok_or("counter unit module overflow")? as u64)
        || (class == 13
            && vtable
                != base
                    .checked_add(crate::crailway_gun::VTABLE_RVA)
                    .ok_or("counter railway module overflow")? as u64)
        || (class == 1
            && vtable
                != base
                    .checked_add(crate::ctask_force::VTABLE_RVA)
                    .ok_or("counter unit module overflow")? as u64)
    {
        return Err("counter unit identity invalid");
    }
    if resolve(handle)? != pointer.checked_add(0x10) {
        return Err("counter unit registry identity mismatch");
    }
    Ok((handle, class))
}

/// CUnit class discriminator u32.
pub const CLASS: usize = 0x08;
/// CSelectable selected byte at CUnit primary +0x0c.
pub const SELECTED: usize = 0x0c;
/// CUnit packed type-low/id-high u64 reference handle.
pub const HANDLE: usize = 0x18;
/// CUnit handle low u32 type word.
pub const TYPE: usize = 0x18;
/// CUnit handle high u32 id word.
pub const ID: usize = 0x1c;
/// CUnit inherited supply cache.
pub const SUPPLY_CACHE: usize = 0x28;
/// CSupplyConsumer received-supply ratio, signed fixed5. The railway gun's
/// low-supply strip reads it (CArmy documents the same base field as
/// `carmy::COUNTER_SUPPLY_STATUS`).
pub const RECEIVED_SUPPLY_RATIO: usize = 0x40;
/// CUnit embedded COrdersGroupMember base.
pub const SCHEDULED_MEMBER: usize = 0xb8;
/// CUnit cached CSubUnitDefinition pointer.
pub const STAT_BLOCK: usize = 0x138;
/// CUnit combat-reference vector data.
pub const COMBAT_REFERENCE_DATA: usize = 0x1a8;
/// CUnit combat-reference vector u32 capacity.
pub const COMBAT_REFERENCE_CAPACITY: usize = 0x1b0;
/// CUnit combat-reference vector u32 count.
pub const COMBAT_REFERENCE_COUNT: usize = 0x1b4;
/// CUnit roster-owner CCountryTag raw32.
pub const ROSTER_OWNER: usize = 0x1d8;
/// CUnit expeditionary-owner CCountryTag raw32.
pub const EXPEDITIONARY_OWNER: usize = 0x1dc;
/// CUnit logical-owner CCountryTag raw32.
pub const LOGICAL_OWNER: usize = 0x1e0;
/// Movement-target province pointer; setting a target creates a one-entry ordinary route.
pub const MOVEMENT_TARGET: usize = 0x1e8;
/// CUnit CProvince location pointer.
pub const LOCATION: usize = 0x1f0;
/// CUnit ordinary-route u32 province-id vector.
pub const ORDINARY_ROUTE_DATA: usize = 0x200;
/// CUnit ordinary-route u32 capacity.
pub const ORDINARY_ROUTE_CAPACITY: usize = 0x208;
/// CUnit ordinary-route u32 count.
pub const ORDINARY_ROUTE_COUNT: usize = 0x20c;
/// Movement-priority enum; independent of arrival time.
pub const MOVEMENT_PRIORITY: usize = 0x248;
/// Embark movement denominator, used in place of neighbor distance.
pub const EMBARK_MOVEMENT_DISTANCE_RVA: usize = 0x0331_d600;
/// CUnit alternate-route u32 province-id vector.
pub const ALTERNATE_ROUTE_DATA: usize = 0x220;
/// CUnit alternate-route u32 capacity.
pub const ALTERNATE_ROUTE_CAPACITY: usize = 0x228;
/// CUnit alternate-route u32 count.
pub const ALTERNATE_ROUTE_COUNT: usize = 0x22c;
/// Retreat byte used by action selection.
pub const RETREAT: usize = 0x24c;
/// Stored withdrawal byte; serialization retains it, and movement-arrow style
/// tests the full byte before mission, target and hostility styles.
pub const WITHDRAWAL: usize = 0x24d;
/// Invalid-order reason, signed i32; used from the first member with its
/// invalid-order byte set. Unknown values produce no warning key, so capture
/// preserves every bit. This reason is not serialized.
pub const INVALID_ORDERS_REASON: usize = 0x2a8;
/// Exile byte; railway-gun location status tests it before railway availability.
pub const EXILED: usize = 0x2ae;
/// Strategic-redeployment byte used by action selection.
pub const STRATEGIC_REDEPLOYMENT: usize = 0x2af;
/// Naval-transfer membership pointer; presence selects coastal stack kinds 19 and 20.
pub const NAVAL_TRANSFER_MEMBERSHIP: usize = 0x2f8;
/// CUnit per-viewer level vector data.
pub const VIEWER_DATA: usize = 0x278;
/// CUnit per-viewer level u32 count.
pub const VIEWER_COUNT: usize = 0x284;
/// CUnit owned map-entity pointer.
pub const MAP_ENTITY: usize = 0x298;
/// CUnit pathfinding UI-cache vector data, distinct from viewer levels.
pub const PATHFINDING_CACHE_DATA: usize = 0x308;
/// CUnit pathfinding UI-cache u32 count.
pub const PATHFINDING_CACHE_COUNT: usize = 0x314;
/// CUnit viewer-level reader.
pub const READ_VIEWER_LEVEL_RVA: usize = 0x00be_ea00;
/// CUnit viewer-level upsert.
pub const UPSERT_VIEWER_LEVEL_RVA: usize = 0x00bf_2640;
/// CUnit viewer-level decay/removal.
pub const DECAY_VIEWER_LEVELS_RVA: usize = 0x00bf_0300;
/// CUnit pathfinding cache helper.
pub const PATHFINDING_CACHE_HELPER_RVA: usize = 0x00be_a0b0;
/// CUnit object-deleted callback.
pub const OBJECT_DELETED_RVA: usize = 0x00f3_6250;
/// CUnit coarse strength helper.
pub const COARSE_STRENGTH_RVA: usize = 0x00be_cad0;
/// CUnit stack-kind selector.
pub const MAP_STACK_KIND_RVA: usize = 0x0127_9110;
/// CUnit combat-presence getter.
pub const HAS_COMBAT_RVA: usize = 0x00bf_0ad0;
/// CUnit first-combat attacker-country predicate.
pub const IS_FIRST_COMBAT_ATTACKER_RVA: usize = 0x00bf_0fc0;
/// CUnit first-combat lookup.
pub const FIRST_COMBAT_RVA: usize = 0x00be_cac0;
/// CUnit fallback target-province getter.
pub const TARGET_PROVINCE_RVA: usize = 0x00be_dec0;

/// CUnit combat-modifier vector data.
pub const COMBAT_MODIFIER_DATA: usize = 0x170;

/// CUnit inherited combat-modifier u32 capacity.
pub const COMBAT_MODIFIER_CAPACITY: usize = 0x178;

/// CUnit combat-modifier count, u32.
pub const COMBAT_MODIFIER_COUNT: usize = 0x17c;

/// CUnit attack accumulator, i64/100000.
pub const ATTACK_ACCUMULATOR: usize = 0x188;

/// CUnit defence accumulator, i64/100000.
pub const DEFENCE_ACCUMULATOR: usize = 0x190;

/// CUnit attack base, i64/100000.
pub const ATTACK_BASE: usize = 0x198;

/// CUnit defence base, i64/100000.
pub const DEFENCE_BASE: usize = 0x1a0;

/// CUnit destructor.
pub const DESTRUCTOR_RVA: usize = 0x00be_99a0;

/// CUnit combat-modifier clear.
pub const CLEAR_COMBAT_MODIFIERS_RVA: usize = 0x00be_c5d0;

/// CUnit modifier append/fold.
pub const APPEND_COMBAT_MODIFIER_RVA: usize = 0x00be_a290;

/// CUnit end-of-tick scale clamp.
pub const CLAMP_COMBAT_SCALE_RVA: usize = 0x00be_c520;

/// CUnit attack-scale getter.
pub const ATTACK_SCALE_GETTER_RVA: usize = 0x00be_ca30;

/// CUnit defence-scale getter.
pub const DEFENCE_SCALE_GETTER_RVA: usize = 0x00be_cb50;

/// CUnit modifier-writer tick callers.
pub const COMBAT_SCALE_WRITER_CALLER_RVAS: [usize; 4] =
    [0x0129_5a30, 0x0129_9f00, 0x0129_a700, 0x0129_d9f0];

/// CUnit observed additional scale writers.
pub const OTHER_SCALE_WRITER_RVAS: [usize; 9] = [
    0x00be_9370,
    0x00be_9620,
    0x00be_add0,
    0x00be_3b90,
    0x00be_6450,
    0x00be_6e60,
    0x00be_72b0,
    0x00be_8100,
    0x00be_6dc0,
];

/// Simulation operands for map routes and stack kinds, independent of the map entity.
/// The caller first authenticates the unit's class and roster. Routes are copied
/// only into a complete arena slice, so an incomplete route never looks stationary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapMotion {
    /// Authenticated movement-target province id; zero for null.
    pub target: u32,
    /// [`first_combat_side`] of the first combat reference: 0 without a combat,
    /// 1 when the logical owner is on the attacking side, else 2.
    pub combat_role: u8,
    /// Province of the first combat; zero without one.
    pub combat_province: u32,
    /// Stored count of the combat-reference vector (`CUnit+0x1B4`).
    pub combat_count: u32,
    /// Whether the logical owner attacks in any of this unit's combats.
    /// Combats are visited in vector order and stop at the first attacking one.
    pub attacking: bool,
    pub ordinary: Array,
    pub alternate: Array,
    // Current visitor-only roots, reused by the optional combat-link suffix.
    // They are never projected into a capture row or kept after this unit.
    combat_header: [u8; 16],
    first_combat: usize,
}

/// Fresh stored movement-arrow operands of an already authenticated unit.
/// None is an independent guarded-read refusal, never a native zero. No
/// selection, animator, map entity or other presentation state is read.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MovementArrowScalars {
    pub numerator: Option<i64>,
    pub retreat: Option<u8>,
    pub withdrawal: Option<u8>,
    pub naval_transfer: Option<u8>,
}

/// Stored movement numerator and independent style bytes and transfer presence.
/// Each read stands alone so a failed suffix cannot invalidate an established
/// map-motion row or later units.
pub(crate) fn read_movement_arrow_scalars(
    memory: &impl Memory,
    unit: usize,
) -> MovementArrowScalars {
    if unit == 0 {
        return MovementArrowScalars::default();
    }
    // The four fields in one guarded call; each stays independent
    // (`FieldBatch`: a failed field fails alone, later ones read singly).
    let offsets = [
        carmy::MOVEMENT_NUMERATOR,
        RETREAT,
        WITHDRAWAL,
        NAVAL_TRANSFER_MEMBERSHIP,
    ];
    let Some(addresses) = gathered_addresses(unit, offsets) else {
        return MovementArrowScalars {
            numerator: unit
                .checked_add(carmy::MOVEMENT_NUMERATOR)
                .and_then(|at| read(memory, at)),
            retreat: unit.checked_add(RETREAT).and_then(|at| read(memory, at)),
            withdrawal: read_withdrawal(memory, unit),
            naval_transfer: unit
                .checked_add(NAVAL_TRANSFER_MEMBERSHIP)
                .and_then(|at| read::<u64>(memory, at))
                .map(|pointer| u8::from(pointer != 0)),
        };
    };
    let fields = FieldBatch::gather(
        memory,
        [
            (addresses[0], 8),
            (addresses[1], 1),
            (addresses[2], 1),
            (addresses[3], 8),
        ],
    );
    MovementArrowScalars {
        numerator: fields.get(memory, 0),
        retreat: fields.get(memory, 1),
        withdrawal: fields.get(memory, 2),
        naval_transfer: fields
            .get::<u64>(memory, 3)
            .map(|pointer| u8::from(pointer != 0)),
    }
}

/// `unit`'s field addresses, or `None` when one overflows (the caller then
/// reads singly, keeping its own refusals).
fn gathered_addresses<const N: usize>(unit: usize, offsets: [usize; N]) -> Option<[usize; N]> {
    let addresses = offsets.map(|offset| unit.checked_add(offset));
    addresses
        .iter()
        .all(Option::is_some)
        .then(|| addresses.map(Option::unwrap))
}

pub fn read_withdrawal(memory: &impl Memory, unit: usize) -> Option<u8> {
    if unit == 0 {
        return None;
    }
    unit.checked_add(WITHDRAWAL).and_then(|at| read(memory, at))
}

impl MapMotion {
    pub fn required_paths(self) -> usize {
        self.ordinary.count as usize + self.alternate.count as usize
    }

    /// Copies both routes, ordinary first, into exactly `required_paths` cells.
    pub fn copy_paths(self, memory: &impl Memory, output: &mut [u32]) -> Result<(), &'static str> {
        if output.len() != self.required_paths() {
            return Err("unit route arena slice is not the route length");
        }
        let (ordinary, alternate) = output.split_at_mut(self.ordinary.count as usize);
        crate::cpdx_array::copy_u32s(
            memory,
            self.ordinary,
            ordinary,
            "unit ordinary route unreadable",
        )?;
        crate::cpdx_array::copy_u32s(
            memory,
            self.alternate,
            alternate,
            "unit alternate route unreadable",
        )
    }
}

/// Reads [`MapMotion`] for one authenticated unit whose raw logical owner is
/// `owner`; its first combat's side goes through the capture's shared `combats`
/// memo.
///
/// A pathfinder route visits each province at most once, so a route longer
/// than the live province roster (`provinces.count`) is refused as corrupt.
/// That bound comes from the loaded map, not from a vanilla size.
#[allow(clippy::too_many_arguments)]
pub fn read_map_motion(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: Array,
    unit: usize,
    owner: u32,
    combats: &mut FirstCombatMemo,
) -> Result<MapMotion, &'static str> {
    read_map_motion_impl(
        memory, base, game_state, provinces, unit, owner, combats, None, None,
    )
}

/// Retains the signed count from the original combat-header attempt, including
/// a failed copy and a count refused by array validation. None means the motion
/// reader stopped before that attempt.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_map_motion_with_combat_count(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: Array,
    unit: usize,
    owner: u32,
    combats: &mut FirstCombatMemo,
    count: &mut Option<Result<i32, &'static str>>,
) -> Result<MapMotion, &'static str> {
    read_map_motion_impl(
        memory,
        base,
        game_state,
        provinces,
        unit,
        owner,
        combats,
        None,
        Some(count),
    )
}

/// Optional full references from the same legacy pointer observations. Old
/// role/province demand stops as before; only identity capture visits the tail.
#[allow(clippy::too_many_arguments)]
pub fn read_map_motion_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: Array,
    unit: usize,
    owner: u32,
    combats: &mut FirstCombatMemo,
    registry: &mut crate::creference_registry::HeaderResolver,
    references: &mut CombatReferencesCapture<'_>,
) -> Result<MapMotion, &'static str> {
    read_map_motion_impl(
        memory,
        base,
        game_state,
        provinces,
        unit,
        owner,
        combats,
        Some((registry, references)),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_map_motion_impl(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: Array,
    unit: usize,
    owner: u32,
    combats: &mut FirstCombatMemo,
    mut references: Option<(
        &mut crate::creference_registry::HeaderResolver,
        &mut CombatReferencesCapture<'_>,
    )>,
    count: Option<&mut Option<Result<i32, &'static str>>>,
) -> Result<MapMotion, &'static str> {
    let at = |offset| {
        unit.checked_add(offset)
            .ok_or("unit motion address overflow")
    };
    // The target and the three headers in one guarded call, each consulted
    // where it is read (`FieldBatch`); a unit whose fields would overflow
    // reads them singly.
    let fields = gathered_addresses(
        unit,
        [
            MOVEMENT_TARGET,
            ORDINARY_ROUTE_DATA,
            ALTERNATE_ROUTE_DATA,
            COMBAT_REFERENCE_DATA,
        ],
    )
    .map(|[target, ordinary, alternate, combats]| {
        FieldBatch::gather(
            memory,
            [(target, 8), (ordinary, 16), (alternate, 16), (combats, 16)],
        )
    });
    let header = |index: usize, offset| match &fields {
        Some(fields) => Ok(fields.get::<[u8; 16]>(memory, index)),
        None => at(offset).map(|address| read::<[u8; 16]>(memory, address)),
    };
    let target_pointer = match &fields {
        Some(fields) => fields.get::<u64>(memory, 0),
        None => read::<u64>(memory, at(MOVEMENT_TARGET)?),
    }
    .and_then(|p| usize::try_from(p).ok())
    .ok_or("unit movement target pointer unreadable")?;
    let target = registered_province(memory, base, provinces, target_pointer)?;
    let route = |index, offset| {
        header(index, offset)?
            .and_then(|header| Array::from_header(header, 4))
            .filter(|route| route.count <= provinces.count)
            .ok_or("unit route invalid or longer than the province roster")
    };
    let ordinary = route(1, ORDINARY_ROUTE_DATA)?;
    let alternate = route(2, ALTERNATE_ROUTE_DATA)?;
    // The vector is walked only until the first combat the owner attacks in, as
    // `Unit_GetActionCode` walks it; its header bounds the walk.
    let combat_header = header(3, COMBAT_REFERENCE_DATA)
        .and_then(|header| header.ok_or("unit combat vector invalid"));
    if let Some(count) = count {
        *count = Some(combat_header.map(|header| {
            i32::from_le_bytes(header[12..16].try_into().expect("signed combat count"))
        }));
    }
    let combat_header = combat_header?;
    let vector = Array::from_header(combat_header, 8).ok_or("unit combat vector invalid")?;
    if let Some((_, capture)) = references.as_mut() {
        capture.begin(vector.count);
    }
    let combat = |index: u32| {
        vector
            .element(index, 8)
            .and_then(|slot| read::<u64>(memory, slot))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|p| *p != 0)
            .ok_or("unit combat pointer unreadable or null")
    };
    let first_combat = if vector.count == 0 { 0 } else { combat(0)? };
    let (combat_role, combat_province) = if vector.count == 0 {
        (0, 0)
    } else {
        combats.capture(
            memory,
            base,
            game_state,
            first_combat,
            Some(owner),
            |pointer| registered_province(memory, base, provinces, pointer),
        )?
    };
    if let Some((registry, capture)) = references.as_mut()
        && vector.count != 0
    {
        capture.observe(memory, base, first_combat, registry);
    }
    let mut visited = u32::from(vector.count != 0);
    let mut attacking = combat_role == 1;
    // Preserve role/province demand exactly. Each pointer used by that demand
    // also feeds optional identity capture; never walk that prefix twice.
    for index in 1..vector.count {
        if attacking {
            break;
        }
        let pointer = combat(index)?;
        attacking = combats.role(memory, base, game_state, pointer, owner)? == 1;
        if let Some((registry, capture)) = references.as_mut() {
            capture.observe(memory, base, pointer, registry);
        }
        visited = index + 1;
    }
    if let Some((registry, capture)) = references.as_mut() {
        // No role, country membership or province read is demanded here.
        for index in visited..vector.count {
            if capture.failure.is_some() {
                break;
            }
            match combat(index) {
                Ok(pointer) => capture.observe(memory, base, pointer, registry),
                Err(reason) => {
                    capture.refuse(reason);
                    break;
                }
            }
        }
        capture.finish(memory, unit, combat_header);
    }
    Ok(MapMotion {
        target,
        combat_role,
        combat_province,
        combat_count: vector.count,
        attacking,
        ordinary,
        alternate,
        combat_header,
        first_combat,
    })
}

/// Naval-wrapper roots from the unit's ordered simulation combat vector.
/// The cached path repeats the first entry; the fresh path selects the first
/// naval class. Supported primary vtables establish class without a game call.
/// Zero proves empty/no naval combat. The motion row already authenticates the
/// first combat's side; extra identity, registry and header failures affect only this suffix.
pub(crate) fn read_map_motion_combat_links(
    memory: &impl Memory,
    base: usize,
    unit: usize,
    motion: MapMotion,
    registry: &mut crate::creference_registry::HeaderResolver,
) -> Result<[u64; 2], &'static str> {
    let vector =
        Array::from_header(motion.combat_header, 8).ok_or("unit combat-link vector invalid")?;
    let mut links = [0; 2];
    for index in 0..vector.count {
        let combat = if index == 0 {
            motion.first_combat
        } else {
            vector
                .element(index, 8)
                .and_then(|slot| read::<u64>(memory, slot))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|pointer| *pointer != 0)
                .ok_or("unit combat-link pointer unreadable or null")?
        };
        let (reference, kind) = map_combat_identity(memory, base, combat, registry)?;
        if index == 0 {
            links[0] = reference;
        }
        if kind == COMBAT_NAVAL {
            links[1] = reference;
            break;
        }
    }
    let header_at = unit
        .checked_add(COMBAT_REFERENCE_DATA)
        .ok_or("unit combat-link header overflow")?;
    if read::<[u8; 16]>(memory, header_at) != Some(motion.combat_header) {
        return Err("unit combat-link vector changed or unreadable");
    }
    Ok(links)
}

fn map_combat_identity(
    memory: &impl Memory,
    base: usize,
    combat: usize,
    registry: &mut crate::creference_registry::HeaderResolver,
) -> Result<(u64, u8), &'static str> {
    let header = read::<[u8; 32]>(memory, combat)
        .filter(|_| combat != 0)
        .ok_or("map combat identity unreadable")?;
    let primary = u64::from_le_bytes(header[..8].try_into().unwrap());
    let secondary = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let reference = u64::from_le_bytes(header[24..32].try_into().unwrap());
    let primary_rva = usize::try_from(primary)
        .ok()
        .and_then(|pointer| pointer.checked_sub(base))
        .ok_or("map combat primary identity invalid")?;
    // Land and naval classes have distinct class results. Reference types
    // are dynamically minted and cannot serve as the class admission gate.
    let (secondary_rva, kind) = match primary_rva {
        crate::cland_combat::VTABLE_RVA => (crate::cland_combat::BODY_VTABLE_RVA, COMBAT_LAND),
        crate::cland_border_war_combat::VTABLE_RVA => (
            crate::cland_border_war_combat::BODY_VTABLE_RVA,
            COMBAT_BORDER_WAR,
        ),
        // Authenticate the naval combat's secondary reference-object vtable.
        crate::cnaval_combat::VTABLE_RVA => (0x029c_6ff0, COMBAT_NAVAL),
        _ => return Err("map combat class unsupported"),
    };
    if base
        .checked_add(secondary_rva)
        .map(|pointer| pointer as u64)
        != Some(secondary)
        || reference == 0
    {
        return Err("map combat secondary identity invalid");
    }
    let secondary_pointer = combat
        .checked_add(crate::cland_combat::BODY)
        .ok_or("map combat secondary address overflow")?;
    if registry.resolve(memory, base, reference)? != Some(secondary_pointer) {
        return Err("map combat registry identity mismatch");
    }
    Ok((reference, kind))
}

/// A CProvince pointer's id, checked against its own registry slot; zero for null.
pub fn registered_province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    pointer: usize,
) -> Result<u32, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let expected = base
        .checked_add(crate::cprovince::VTABLE_RVA)
        .ok_or("unit province module overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected) {
        return Err("unit province vtable mismatch");
    }
    let id = pointer
        .checked_add(crate::cprovince::ID)
        .and_then(|address| read::<u32>(memory, address))
        .filter(|id| *id != 0)
        .ok_or("unit province id unreadable or zero")?;
    if provinces
        .element(id, 8)
        .and_then(|slot| read::<u64>(memory, slot))
        != Some(pointer as u64)
    {
        return Err("unit province differs from its registered slot");
    }
    Ok(id)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EtaLeg {
    pub distance: i64,
    pub speed: EtaSpeed,
}

/// Edge-local speed inputs; the larger common normal inputs live once per
/// division, including when its current edge is independently reacquired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtaSpeed {
    Fixed(i64),
    Scaled { speed: i64, scale: i64 },
    Normal(carmy::NormalEdgeOperands),
}
impl Default for EtaSpeed {
    fn default() -> Self {
        Self::Fixed(0)
    }
}
impl From<i64> for EtaSpeed {
    fn from(value: i64) -> Self {
        Self::Fixed(value)
    }
}
impl EtaSpeed {
    fn evaluate(self, normal: Option<carmy::PreparedNormalSpeed>) -> Result<i64, &'static str> {
        Ok(match self {
            Self::Fixed(value) => value,
            Self::Scaled { speed, scale } => {
                carmy::MovementSpeedInputs::Scaled { speed, scale }.evaluate()
            }
            Self::Normal(edge) => normal
                .ok_or("ETA normal speed inputs missing")?
                .evaluate(edge),
        })
    }
}

/// Owned callback operands only: no game pointers reach the duration evaluator.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EtaInputs {
    pub division: u64,
    pub next: u32,
    pub destination: u32,
    pub path_total: u32,
    pub leg_start: usize,
    pub leg_len: usize,
    pub next_leg_count: usize,
    pub numerator: i64,
    pub current_speed: EtaSpeed,
    pub normal: Option<carmy::NormalSpeedOperands>,
}

/// Distance or progress fraction; evaluate each leg before reaching the next.
/// Its exceptional result is the positive low-word all-ones value.
pub fn eta_ratio(value: i64, divisor: i64) -> Result<i64, &'static str> {
    if divisor == 0 || (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Ok(0xffff_ffff);
    }
    value
        .wrapping_mul(100_000)
        .checked_div(divisor)
        .ok_or("ETA native signed division overflows")
}

fn eta_route_duration(
    legs: &[EtaLeg],
    normal: Option<carmy::PreparedNormalSpeed>,
    progress: impl FnOnce() -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    let mut duration = 0_i64;
    for leg in legs {
        let speed = leg.speed.evaluate(normal)?;
        if speed == 0 {
            return Ok(0);
        }
        duration = duration.wrapping_add(eta_ratio(leg.distance, speed)?);
    }
    Ok(duration.wrapping_sub(progress()?))
}

/// Exact signed fixed5 durations to the next province and the route's end.
/// Each edge division truncates before the wrapping sum; no per-edge ceiling.
pub fn eta_remaining(input: &EtaInputs, legs: &[EtaLeg]) -> Result<(i64, i64), &'static str> {
    if input.next_leg_count > 1
        || input.next_leg_count > input.leg_len
        || input.leg_len > input.path_total as usize
    {
        return Err("ETA owned leg counts invalid");
    }
    let end = input
        .leg_start
        .checked_add(input.leg_len)
        .ok_or("ETA owned leg range overflow")?;
    let legs = legs
        .get(input.leg_start..end)
        .ok_or("ETA owned leg range invalid")?;
    if input.path_total == 0 {
        return Ok((0, 0));
    }
    if input.next == 0 || input.destination == 0 {
        return Err("ETA owned route endpoints missing");
    }
    let normal = input.normal.map(carmy::NormalSpeedOperands::prepare);
    let progress = std::cell::OnceCell::new();
    let evaluate = |legs: &[EtaLeg]| {
        eta_route_duration(legs, normal, || {
            *progress.get_or_init(|| {
                input
                    .current_speed
                    .evaluate(normal)
                    .and_then(|speed| eta_ratio(input.numerator, speed))
            })
        })
    };
    Ok((evaluate(&legs[..input.next_leg_count])?, evaluate(legs)?))
}

/// Signed fixed5 ceiling, including its wrapping round trip.
pub fn eta_hours(duration: i64) -> i64 {
    let quotient = duration / 100_000;
    if duration == quotient * 100_000 {
        return quotient;
    }
    quotient
        .wrapping_add(i64::from(duration > 0))
        .wrapping_mul(100_000)
        / 100_000
}

#[cfg(test)]
mod eta_tests {
    use super::*;

    fn input(count: usize) -> EtaInputs {
        EtaInputs {
            division: 51,
            next: 2,
            destination: 3,
            path_total: count as u32,
            leg_len: count,
            next_leg_count: usize::from(count != 0),
            current_speed: (100_000_i64).into(),
            ..EtaInputs::default()
        }
    }

    #[test]
    fn eta_divides_each_leg_before_adding_and_subtracts_current_progress() {
        let legs = [
            EtaLeg {
                distance: 1,
                speed: (3_i64).into(),
            },
            EtaLeg {
                distance: 2,
                speed: (3_i64).into(),
            },
        ];
        let mut row = input(2);
        row.numerator = 10;
        assert_eq!(eta_remaining(&row, &legs), Ok((33_323, 99_989)));
        row.numerator = 200_000;
        assert_eq!(eta_remaining(&row, &legs), Ok((-166_667, -100_001)));
    }

    #[test]
    fn eta_zero_edge_speed_returns_before_progress_but_only_for_that_route() {
        let legs = [
            EtaLeg {
                distance: 100_000,
                speed: (100_000_i64).into(),
            },
            EtaLeg {
                distance: 100_000,
                speed: (0_i64).into(),
            },
        ];
        let mut row = input(2);
        row.current_speed = (0_i64).into();
        assert_eq!(eta_remaining(&row, &legs), Ok((100_000 - 0xffff_ffff, 0)));
        let legs = [
            EtaLeg {
                distance: i64::MAX,
                speed: (0_i64).into(),
            },
            legs[0],
        ];
        assert_eq!(eta_remaining(&row, &legs), Ok((0, 0)));
    }

    #[test]
    fn eta_sentinels_wrapping_and_same_province_are_not_clamped() {
        let mut row = input(1);
        let legs = [EtaLeg {
            distance: i64::MAX,
            speed: (1_i64).into(),
        }];
        assert_eq!(eta_remaining(&row, &legs), Ok((0xffff_ffff, 0xffff_ffff)));
        row.numerator = i64::MIN;
        assert_eq!(eta_remaining(&row, &legs), Ok((0, 0)));
        row.numerator = 7;
        row.next_leg_count = 0;
        assert_eq!(eta_remaining(&row, &legs), Ok((-7, 0xffff_ffff - 7)));
        let value = i64::MAX / 2;
        assert_eq!(eta_ratio(value, 11), Ok(value.wrapping_mul(100_000) / 11));
        assert_eq!(eta_ratio(-1, 1), Ok(-100_000));
        assert_eq!(eta_ratio(i64::MIN + 1, 1), Ok(0xffff_ffff));
        assert!(eta_ratio(i64::MIN / 32, -1).is_err());
        let empty = EtaInputs {
            current_speed: (0_i64).into(),
            numerator: i64::MAX,
            ..EtaInputs::default()
        };
        assert_eq!(eta_remaining(&empty, &[]), Ok((0, 0)));
        row.leg_start = usize::MAX;
        assert!(eta_remaining(&row, &legs).is_err());
    }

    #[test]
    fn eta_ceiling_preserves_negative_values_and_native_overflow() {
        for (value, expected) in [
            (0, 0),
            (1, 1),
            (99_999, 1),
            (100_000, 1),
            (100_001, 2),
            (-1, 0),
            (-99_999, 0),
            (-100_000, -1),
            (-100_001, -1),
        ] {
            assert_eq!(eta_hours(value), expected);
        }
        let expected = (i64::MAX / 100_000 + 1).wrapping_mul(100_000) / 100_000;
        assert_eq!(eta_hours(i64::MAX), expected);
        assert!(eta_hours(i64::MAX) < 0);
    }
}

/// A per-capture memo key: the combat, its owner as recorded raw, and the
/// module and game state the memo was filled under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FirstCombatKey {
    pub(crate) base: usize,
    pub(crate) game_state: usize,
    pub(crate) pointer: usize,
    pub(crate) owner: u32,
}

/// Uncached first-combat role: one when the raw logical owner belongs to the
/// attacking side, exactly or through an alias, otherwise two. Also reads the
/// combat's authenticated province. Land, border-war and naval combat layouts
/// share these side and province fields. The attacking side must point back
/// to its combat.
///
/// Read order is native and fixed: combat identity, attacking side, side
/// identity and backlink, then the owner (a missing owner fails only after
/// both identities are authenticated), membership (lazy prefix), and last the
/// province. A membership failure therefore wins over a province failure.
pub fn first_combat_side(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: usize,
    owner: Option<u32>,
    mut province_id: impl FnMut(usize) -> Result<u32, &'static str>,
) -> Result<(u8, u32), &'static str> {
    let role = combat_role(memory, base, game_state, combat, owner)?;
    let province = combat
        .checked_add(crate::cland_combat::PROVINCE)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("division presentation field unreadable")? as usize;
    Ok((role, province_id(province)?))
}

/// [`first_combat_side`]'s role alone, in the same read order, without the province.
fn combat_role(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: usize,
    owner: Option<u32>,
) -> Result<u8, &'static str> {
    use crate::{cland_combat as combat_layout, cland_combatant as side_layout};
    let field = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("division presentation field unreadable")
    };
    let vtable = read::<u64>(memory, combat).ok_or("first combat identity unreadable")?;
    if ![
        combat_layout::VTABLE_RVA,
        crate::cland_border_war_combat::VTABLE_RVA,
        crate::cnaval_combat::VTABLE_RVA,
    ]
    .iter()
    .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
    {
        return Err("division first combat identity mismatch");
    }
    let side = field(combat, combat_layout::ATTACKER)? as usize;
    let vtable = read::<u64>(memory, side).ok_or("first combat side unreadable")?;
    if ![
        side_layout::VTABLE_RVA,
        crate::cland_border_war_combatant::VTABLE_RVA,
        crate::cnaval_combatant::VTABLE_RVA,
    ]
    .iter()
    .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
        || field(side, side_layout::COMBAT)? != combat as u64
    {
        return Err("first combat side identity mismatch");
    }
    let owner = owner.ok_or("division presentation field unreadable")?;
    let role = if side_layout::capture_country_membership(memory, base, game_state, side, owner)?
        .contains_country()
    {
        1
    } else {
        2
    };
    Ok(role)
}

/// First-combat answers shared by every map unit in one capture: divisions and
/// railway guns in the same combat read it once per raw owner. Only complete
/// answers are kept; a failed query is never memoised. Construct and
/// [`Self::reserve`] outside acquisition: a full memo answers from memory
/// without inserting, so it never allocates inside a callback. `clear` at the
/// start of each capture.
#[derive(Debug, Default)]
pub struct FirstCombatMemo {
    pub entries: std::collections::HashMap<FirstCombatKey, (u8, u32)>,
}

impl FirstCombatMemo {
    pub fn try_new(rows: usize) -> std::io::Result<Self> {
        let mut memo = Self::default();
        memo.reserve(rows)?;
        Ok(memo)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new(rows: usize) -> Self {
        Self::try_new(rows).unwrap()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Room for `rows` answers; call outside acquisition.
    pub fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        let additional = rows.saturating_sub(self.entries.len());
        crate::allocation::reserve_map(&mut self.entries, additional)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * (size_of::<(FirstCombatKey, (u8, u32))>() + 1)
    }

    /// [`first_combat_side`] through the memo. `combat` is the unit's first
    /// combat pointer, already read by the caller from its own combat vector.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        combat: usize,
        owner: Option<u32>,
        province_id: impl FnMut(usize) -> Result<u32, &'static str>,
    ) -> Result<(u8, u32), &'static str> {
        let key = owner.map(|owner| FirstCombatKey {
            base,
            game_state,
            pointer: combat,
            owner,
        });
        if let Some(value) = key.and_then(|key| self.entries.get(&key).copied()) {
            return Ok(value);
        }
        let value = first_combat_side(memory, base, game_state, combat, owner, province_id)?;
        if let Some(key) = key
            && self.entries.len() < self.entries.capacity()
        {
            self.entries.insert(key, value);
        }
        Ok(value)
    }

    /// `owner`'s role in a combat that is not the unit's first: the memo's answer
    /// when another unit's first combat already holds it, else read without the
    /// province and not memoised, since a memo entry carries the province.
    pub fn role(
        &self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        combat: usize,
        owner: u32,
    ) -> Result<u8, &'static str> {
        let key = FirstCombatKey {
            base,
            game_state,
            pointer: combat,
            owner,
        };
        match self.entries.get(&key) {
            Some(&(role, _)) => Ok(role),
            None => combat_role(memory, base, game_state, combat, Some(owner)),
        }
    }
}

#[cfg(test)]
mod arrow_tests {
    use super::*;

    /// The four movement-arrow fields are read in one guarded call, and each
    /// keeps its own value.
    #[test]
    fn movement_arrow_scalars_are_gathered_in_one_call() {
        const UNIT: usize = 0x100;
        let mut bytes = vec![0; 0x1000];
        bytes[UNIT + carmy::MOVEMENT_NUMERATOR..][..8].copy_from_slice(&(-7_i64).to_le_bytes());
        bytes[UNIT + RETREAT] = 1;
        bytes[UNIT + WITHDRAWAL] = 2;
        bytes[UNIT + NAVAL_TRANSFER_MEMBERSHIP..][..8].copy_from_slice(&9_u64.to_le_bytes());
        let memory = crate::memory::CountingMemory::new(&bytes);
        assert_eq!(
            read_movement_arrow_scalars(&memory, UNIT),
            MovementArrowScalars {
                numerator: Some(-7),
                retreat: Some(1),
                withdrawal: Some(2),
                naval_transfer: Some(1),
            }
        );
        assert_eq!(memory.calls.get(), 1);
    }
}
