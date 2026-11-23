
use crate::{Memory, carmy, cpdx_array::Array, memory::FieldBatch, read};
use engine_sites::Rva;

mod combat_references;
pub use combat_references::CombatReferencesCapture;
pub use combat_references::{COMBAT_BORDER_WAR, COMBAT_LAND, COMBAT_NAVAL, MapCombatReference};
pub(crate) use combat_references::{MapCombatIdentityMemo, read_unit_combat_references};
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
                != engine_sites::at(base, crate::carmy::VTABLE_RVA)
                    .ok_or("counter unit module overflow")? as u64)
        || (class == 13
            && vtable
                != engine_sites::at(base, crate::crailway_gun::VTABLE_RVA)
                    .ok_or("counter railway module overflow")? as u64)
        || (class == 1
            && vtable
                != engine_sites::at(base, crate::ctask_force::VTABLE_RVA)
                    .ok_or("counter unit module overflow")? as u64)
    {
        return Err("counter unit identity invalid");
    }
    if resolve(handle)? != pointer.checked_add(0x10) {
        return Err("counter unit registry identity mismatch");
    }
    Ok((handle, class))
}

pub fn viewer_land_units(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, u64>,
) -> crate::walk::Outcome {
    match viewer_land_units_into(memory, base, state, &mut output) {
        Ok(len) => crate::walk::Outcome::Rows { len },
        Err(reason) => crate::walk::Outcome::Unavailable { reason },
    }
}

fn viewer_land_units_into(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut crate::walk::Buffer<'_, u64>,
) -> Result<usize, &'static str> {
    use crate::memory::{RecordField, RecordLayout, RecordSources};
    const STRIDE: usize = 24;
    const FIELDS: [RecordField; 3] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: CLASS,
            destination_offset: 8,
            len: 4,
        },
        RecordField {
            source_offset: HANDLE,
            destination_offset: 16,
            len: 8,
        },
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, STRIDE).expect("valid record layout");
    let units = state
        .checked_add(crate::cgame_state::VIEWER_UNITS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("viewer unit vector unreadable or invalid")?;
    if !output.ensure(units.count as usize)? {
        return Err("viewer unit storage short");
    }
    let vtable = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("viewer unit module address overflow")
    };
    let army = vtable(carmy::VTABLE_RVA)?;
    let gun = vtable(crate::crailway_gun::VTABLE_RVA)?;
    let task_force = vtable(crate::ctask_force::VTABLE_RVA)?;
    let mut records = [0_u8; crate::cpdx_array::POINTER_PAGE * STRIDE];
    let mut len = 0;
    crate::cpdx_array::pointer_pages(
        memory,
        units,
        "viewer unit pointer unreadable",
        |_, page| {
            if page.contains(&0) {
                return Err("viewer unit pointer null");
            }
            let records = &mut records[..page.len() * STRIDE];
            if memory.copy_records(RecordSources::Pointers(page), &LAYOUT, records)
                != page.len() * FIELDS.len()
            {
                return Err("viewer unit header unreadable");
            }
            for record in records.chunks_exact(STRIDE) {
                let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
                let class = u32::from_le_bytes(record[8..12].try_into().unwrap());
                let (expected, kept) = match class {
                    0 => (army, true),
                    13 => (gun, true),
                    1 => (task_force, false),
                    _ => return Err("viewer unit class unknown"),
                };
                if word(0) != expected {
                    return Err("viewer unit vtable mismatch");
                }
                let handle = word(16);
                if handle >> 32 == 0 {
                    return Err("viewer unit handle has no id");
                }
                if kept {
                    output[len] = handle;
                    len += 1;
                }
            }
            Ok(())
        },
    )?;
    Ok(len)
}

pub const CLASS: usize = 0x08;
pub const SELECTED: usize = 0x0c;
pub const HANDLE: usize = 0x18;
pub const TYPE: usize = 0x18;
pub const ID: usize = 0x1c;
pub const RECEIVED_SUPPLY_RATIO: usize = 0x40;
pub const SCHEDULED_MEMBER: usize = 0xb8;
pub const STAT_BLOCK: usize = 0x138;
pub const COMBAT_REFERENCE_DATA: usize = 0x1a8;
pub const COMBAT_REFERENCE_COUNT: usize = 0x1b4;
pub const ROSTER_OWNER: usize = 0x1d8;
pub const EXPEDITIONARY_OWNER: usize = 0x1dc;
pub const LOGICAL_OWNER: usize = 0x1e0;
pub const MOVEMENT_TARGET: usize = 0x1e8;
pub const LOCATION: usize = 0x1f0;
pub const ORDINARY_ROUTE_DATA: usize = 0x200;
#[cfg(any(test, feature = "layout-fixtures"))]
pub const ORDINARY_ROUTE_CAPACITY: usize = 0x208;
pub const ORDINARY_ROUTE_COUNT: usize = 0x20c;
pub const MOVEMENT_PRIORITY: usize = 0x248;
pub const EMBARK_MOVEMENT_DISTANCE_RVA: Rva = Rva(0x033e_1048);
pub const ALTERNATE_ROUTE_DATA: usize = 0x220;
pub const RETREAT: usize = 0x24c;
pub const WITHDRAWAL: usize = 0x24d;
pub const INVALID_ORDERS_REASON: usize = 0x2a8;
pub const EXILED: usize = 0x2ae;
pub const STRATEGIC_REDEPLOYMENT: usize = 0x2af;
pub const NAVAL_TRANSFER_MEMBERSHIP: usize = 0x2f8;
pub const VIEWER_DATA: usize = 0x278;
#[cfg(any(test, feature = "layout-fixtures"))]
pub const VIEWER_COUNT: usize = 0x284;
pub const MAP_ENTITY: usize = 0x298;
pub const THEATRE: usize = 0x2a0;
pub const QUEUED_ACTION_COUNT: usize = 0x2cc;

pub const COMBAT_MODIFIER_DATA: usize = 0x170;

pub const ATTACK_ACCUMULATOR: usize = 0x188;

pub const DESTRUCTOR_RVA: Rva = Rva(0x00c4_e950);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapMotion {
    pub target: u32,
    pub combat_role: u8,
    pub combat_province: u32,
    pub combat_count: u32,
    pub attacking: bool,
    pub ordinary: Array,
    pub alternate: Array,
    combat_header: [u8; 16],
    first_combat: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MovementArrowScalars {
    pub numerator: Option<i64>,
    pub retreat: Option<u8>,
    pub withdrawal: Option<u8>,
    pub naval_transfer: Option<u8>,
}

pub(crate) fn read_movement_arrow_scalars(
    memory: &impl Memory,
    unit: usize,
) -> MovementArrowScalars {
    if unit == 0 {
        return MovementArrowScalars::default();
    }
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
    let primary = usize::try_from(primary)
        .ok()
        .filter(|&pointer| pointer >= base)
        .ok_or("map combat primary identity invalid")?;
    let classes = [
        crate::cland_combat::VTABLE_RVA,
        crate::cland_border_war_combat::VTABLE_RVA,
        crate::cnaval_combat::VTABLE_RVA,
    ];
    let (secondary_rva, kind) = match engine_sites::which(base, primary, &classes) {
        Some(crate::cland_combat::VTABLE_RVA) => {
            (crate::cland_combat::BODY_VTABLE_RVA, COMBAT_LAND)
        }
        Some(crate::cland_border_war_combat::VTABLE_RVA) => (
            crate::cland_border_war_combat::BODY_VTABLE_RVA,
            COMBAT_BORDER_WAR,
        ),
        Some(crate::cnaval_combat::VTABLE_RVA) => (Rva(0x02a5_6e70), COMBAT_NAVAL),
        _ => return Err("map combat class unsupported"),
    };
    if engine_sites::at(base, secondary_rva).map(|pointer| pointer as u64) != Some(secondary)
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

pub fn registered_province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    pointer: usize,
) -> Result<u32, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let expected = engine_sites::at(base, crate::cprovince::VTABLE_RVA)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FirstCombatKey {
    pub(crate) base: usize,
    pub(crate) game_state: usize,
    pub(crate) pointer: usize,
    pub(crate) owner: u32,
}

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
    .any(|&rva| engine_sites::at(base, rva).map(|p| p as u64) == Some(vtable))
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
    .any(|&rva| engine_sites::at(base, rva).map(|p| p as u64) == Some(vtable))
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

    pub fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        let additional = rows.saturating_sub(self.entries.len());
        crate::allocation::reserve_map(&mut self.entries, additional)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * (size_of::<(FirstCombatKey, (u8, u32))>() + 1)
    }

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

#[cfg(test)]
mod viewer_unit_tests {
    use super::*;
    use crate::walk::{Buffer, Outcome};

    const BASE: usize = 0x1000;
    const STATE: usize = 0x100;
    const DATA: usize = 0x800;
    const UNITS: usize = 0x1000;

    fn handle(id: u32) -> u64 {
        (u64::from(id) << 32) | 9
    }

    fn world(units: &[(Rva, u32, u64)]) -> Vec<u8> {
        let mut bytes = vec![0; UNITS + units.len() * 0x100];
        let header = STATE + crate::cgame_state::VIEWER_UNITS;
        bytes[header..header + 8].copy_from_slice(&(DATA as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&(units.len() as u32).to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&(units.len() as u32).to_le_bytes());
        for (index, &(vtable, class, handle)) in units.iter().enumerate() {
            let unit = UNITS + index * 0x100;
            bytes[DATA + index * 8..][..8].copy_from_slice(&(unit as u64).to_le_bytes());
            let vtable = engine_sites::at(BASE, vtable).unwrap() as u64;
            bytes[unit..unit + 8].copy_from_slice(&vtable.to_le_bytes());
            bytes[unit + CLASS..][..4].copy_from_slice(&class.to_le_bytes());
            bytes[unit + HANDLE..][..8].copy_from_slice(&handle.to_le_bytes());
        }
        bytes
    }

    fn read(bytes: &[u8]) -> Result<Vec<u64>, &'static str> {
        let memory = crate::memory::CountingMemory::new(bytes);
        let mut output = Vec::new();
        match viewer_land_units(&memory, BASE, STATE, Buffer::Growing(&mut output)) {
            Outcome::Rows { len } => Ok(output[..len].to_vec()),
            Outcome::Unavailable { reason } => Err(reason),
            Outcome::Truncated { .. } => unreachable!(),
        }
    }

    #[test]
    fn divisions_and_railway_guns_keep_vector_order_without_task_forces() {
        let army = carmy::VTABLE_RVA;
        let gun = crate::crailway_gun::VTABLE_RVA;
        let fleet = crate::ctask_force::VTABLE_RVA;
        let bytes = world(&[
            (army, 0, handle(5)),
            (fleet, 1, handle(70)),
            (gun, 13, handle(3)),
            (army, 0, handle(1)),
        ]);
        assert_eq!(read(&bytes), Ok(vec![handle(5), handle(3), handle(1)]));
        assert_eq!(read(&world(&[])), Ok(vec![]));
    }

    #[test]
    fn an_entry_that_is_not_a_known_unit_refuses_the_list() {
        let army = carmy::VTABLE_RVA;
        let fleet = crate::ctask_force::VTABLE_RVA;
        assert_eq!(
            read(&world(&[(army, 0, handle(5)), (fleet, 0, handle(6))])),
            Err("viewer unit vtable mismatch")
        );
        assert_eq!(
            read(&world(&[(army, 2, handle(5))])),
            Err("viewer unit class unknown")
        );
        assert_eq!(
            read(&world(&[(army, 0, 9)])),
            Err("viewer unit handle has no id")
        );
        let mut null = world(&[(army, 0, handle(5))]);
        null[DATA..DATA + 8].fill(0);
        assert_eq!(read(&null), Err("viewer unit pointer null"));
    }
}

pub use game_model::movement::unit::{EtaInputs, EtaLeg, EtaSpeed, eta_hours, eta_ratio, eta_remaining};
