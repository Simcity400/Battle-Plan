use engine_executable::modifier_keys as keys;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use crate::{cair_wing, craid_instance, raid_speed_inputs};
use engine_native::{carmy, cunit};
use engine_native_parts::creference_registry;
use engine_sites::Rva;

mod context;
mod names;
pub use names::StoredString;
mod requirements;

pub const REFERENCES: u32 = 0;
pub const ARMY: u32 = 1;
pub const ARMY_STATS: u32 = 2;
pub const MANPOWER: u32 = 3;
pub const WING: u32 = 4;
pub const WING_STATS: u32 = 5;
pub const WING_MODIFIERS: u32 = 6;
pub const WING_COUNTRY_MODIFIERS: u32 = 7;
pub const ACE: u32 = 8;
pub const ACE_MODIFIERS: u32 = 9;
pub const ACE_TYPE_MODIFIERS: u32 = 10;
pub const ACE_OWNER_MODIFIERS: u32 = 11;
pub const ACE_WING: u32 = 12;
pub const TARGET_STATE: u32 = 13;
pub const TARGET_UNIT_OWNERS: u32 = 14;
pub const CONTEXT: u32 = 15;
pub const WEATHER_MODIFIERS: u32 = 16;
pub const NIGHT_MODIFIERS: u32 = 17;
pub const INTERCEPTION_ENABLED: u32 = 18;
pub const INTERCEPTION_SUPPORTED: u32 = 19;
pub const INTERCEPTION_REGION: u32 = 20;
pub const INTERCEPTION_PLANES: u32 = 21;
pub const ANTI_AIR_LEVELS: u32 = 22;
pub const TARGET_CORES: u32 = 23;
pub const NAME: u32 = 24;
pub const NAME_GROUP: u32 = 25;
pub const CUSTOM_NAME: u32 = 26;
pub const ESSENTIAL_ORDER: u32 = 27;
pub const ADDITIONAL_ORDER: u32 = 28;
pub const SOURCE_NAME: u32 = 29;
pub const SOURCE_NAME_GROUP: u32 = 30;
pub const SOURCE_CUSTOM_NAME: u32 = 31;
pub const SUBUNIT_ACTIVE: u32 = 32;
pub const UNREADABLE: u32 = 33;
pub const WING_NAME: u32 = 34;
pub const SOURCE_CACHED_NAME: u32 = 35;
pub const NUCLEAR_BOMB_TOKEN: u32 = 0x4bf0;
pub const THERMONUCLEAR_BOMB_TOKEN: u32 = 0x4bf1;
pub fn definition_key(token: u32) -> u64 {
    u64::from(token) << 32
}
pub const ARMY_RESOLVED: u32 = 0x1000;
pub const WING_RESOLVED: u32 = 0x1001;
pub const UNIT_RAID_REFERENCE: u32 = 0x1002;
pub const UNIT_RAID_RESOLVED: u32 = 0x1003;
pub const SOURCE_SHIP_RESOLVED: u32 = 0x1005;
pub const SOURCE_AIR_BASE_KIND: u32 = 0x1006;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum NameReadError {
    Header = -1,
    NullPointer = -2,
    Terminator = -3,
    Overflow = -4,
    Unreadable = -5,
}

impl NameReadError {
    pub const fn code(self) -> i64 {
        self as i64
    }
    pub fn from_code(code: i64) -> Option<Self> {
        [
            Self::Header,
            Self::NullPointer,
            Self::Terminator,
            Self::Overflow,
            Self::Unreadable,
        ]
        .into_iter()
        .find(|error| error.code() == code)
    }
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Header => "native string length exceeds capacity or capacity is invalid",
            Self::NullPointer => "native string data pointer is null",
            Self::Terminator => "native string terminator is invalid",
            Self::Overflow => "native string address, length or chunk index overflows",
            Self::Unreadable => "native string header or bytes are unreadable",
        }
    }
}

pub fn wing_name_error(code: i64) -> &'static str {
    NameReadError::from_code(code)
        .map(NameReadError::reason)
        .unwrap_or("unsupported wing-name read failure code")
}
pub const TARGET_PROVINCE: u32 = 0;
pub const TARGET_REGION: u32 = 1;
pub const WING_REGION: u32 = 2;
pub const BASE_REGION: u32 = 3;
pub const WING_MISSION_MASK: u32 = 4;
pub const CARRIER_MISSION: u32 = 5;
pub const NIGHT_FACTOR: u32 = 6;
pub const TARGET_COUNTRY: u32 = 7;
pub const LOOKUP: u32 = 1 << 31;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub raid: u64,
    pub ordinal: u32,
    pub kind: u32,
    pub key: u32,
    pub raw: i64,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    at: usize,
    offset: usize,
) -> Result<T, &'static str> {
    at.checked_add(offset)
        .filter(|_| at != 0)
        .and_then(|at| read(memory, at))
        .ok_or("raid tooltip operand unreadable")
}

struct Sink<'a> {
    rows: engine_native_memory::walk::Buffer<'a, Row>,
    required: usize,
    raid: u64,
    ordinal: u32,
}
impl Sink<'_> {
    fn push(&mut self, kind: u32, key: u32, raw: i64) -> Result<(), &'static str> {
        self.rows.ensure(
            self.required
                .checked_add(1)
                .ok_or("raid tooltip row count overflow")?,
        )?;
        if let Some(row) = self.rows.get_mut(self.required) {
            *row = Row {
                raid: self.raid,
                ordinal: self.ordinal,
                kind,
                key,
                raw,
            };
        }
        self.ordinal = self
            .ordinal
            .checked_add(1)
            .ok_or("raid tooltip operand ordinal overflow")?;
        self.required = self
            .required
            .checked_add(1)
            .ok_or("raid tooltip operand demand overflow")?;
        Ok(())
    }
}

fn unit(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    vtable: Rva,
) -> Result<Option<usize>, &'static str> {
    let Some(pointer) = creference_registry::resolve(memory, base, handle)?
        .and_then(|p| p.checked_sub(0x10))
        .filter(|&p| p != 0)
    else {
        return Ok(None);
    };
    if Some(field::<u64>(memory, pointer, 0)?) != engine_sites::at(base, vtable).map(|a| a as u64) {
        return Err("raid tooltip unit vtable mismatch");
    }
    Ok(Some(pointer))
}

fn army(memory: &impl Memory, pointer: usize, sink: &mut Sink<'_>) -> Result<(), &'static str> {
    for offset in [0x420, 0x428, 0x430] {
        sink.push(ARMY, offset as u32, field(memory, pointer, offset)?)?;
    }
    let stats = field::<u64>(memory, pointer, cunit::STAT_BLOCK)? as usize;
    for offset in [0xd0, 0x288] {
        sink.push(ARMY_STATS, offset as u32, field(memory, stats, offset)?)?;
    }
    let roster = Array::read(memory, pointer + carmy::MANPOWER_DATA, 8)
        .ok_or("raid tooltip manpower header invalid")?;
    for index in 0..roster.count {
        sink.push(
            MANPOWER,
            index,
            i64::from(field::<i32>(
                memory,
                roster
                    .element(index, 8)
                    .ok_or("raid tooltip manpower address overflow")?,
                4,
            )?),
        )?;
    }
    Ok(())
}

fn wing_stats(
    memory: &impl Memory,
    pointer: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    sink.push(
        WING,
        cair_wing::EXPERIENCE as u32,
        field(memory, pointer, cair_wing::EXPERIENCE)?,
    )?;
    let index = cair_wing::mission_stat_index(memory, pointer, 8).map_err(raid_stat_reason)?;
    let stats = cair_wing::mission_stat_record(memory, pointer, index).map_err(raid_stat_reason)?;
    for key in [0x2f, 0x31, 0x32, 0x41, 0x4c] {
        sink.push(WING_STATS, key, field(memory, stats, key as usize * 8)?)?;
    }
    Ok(())
}

fn raid_stat_reason(reason: cair_wing::MissionStatError) -> &'static str {
    match reason {
        cair_wing::MissionStatError::MultipleMissionBits => reason.reason(),
        cair_wing::MissionStatError::IndexUnreadable => "raid tooltip operand unreadable",
        cair_wing::MissionStatError::AddressOverflow => "raid tooltip stat address overflow",
        cair_wing::MissionStatError::HeaderInvalid => "raid tooltip mission stat header invalid",
        cair_wing::MissionStatError::IndexNegative => "raid tooltip mission stat index negative",
        cair_wing::MissionStatError::IndexOutsideArray => {
            "raid tooltip mission stat index outside array"
        }
    }
}

fn modifiers(
    memory: &impl Memory,
    at: usize,
    kind: u32,
    keys: &[u32],
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let array = Array::read(memory, at, 16).ok_or("raid tooltip modifier header invalid")?;
    for index in 0..array.count {
        let bytes = field::<[u8; 16]>(
            memory,
            array
                .element(index, 16)
                .ok_or("raid tooltip modifier address overflow")?,
            0,
        )?;
        let key = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        if keys.contains(&key) && matches!(key, keys::AIR_DEFENCE_FACTOR | keys::AIR_AGILITY_FACTOR)
        {
            sink.push(
                kind,
                key,
                i64::from_le_bytes(bytes[8..].try_into().unwrap()),
            )?;
        }
    }
    for &key in keys {
        let (mut lower, mut upper) = (0, array.count);
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            let at = array
                .element(middle, 16)
                .ok_or("raid tooltip modifier address overflow")?;
            let found = field::<u32>(memory, at, 0)?;
            if found < key {
                lower = middle + 1;
            } else {
                upper = middle;
            }
        }
        if let Some(at) = array.element(lower, 16)
            && field::<u32>(memory, at, 0)? == key
        {
            sink.push(kind, key | LOOKUP, field(memory, at, 8)?)?;
        }
    }
    Ok(())
}

fn object(
    memory: &impl Memory,
    base: usize,
    game: usize,
    id: u32,
    province: bool,
) -> Result<usize, &'static str> {
    if id == 0 {
        return Ok(0);
    }
    let (offset, vtable, id_offset) = if province {
        (
            engine_native::cgame_state::PROVINCES,
            engine_native::cprovince::VTABLE_RVA,
            engine_native::cprovince::ID,
        )
    } else {
        (
            engine_native::cgame_state::STATES,
            engine_native::cstate::VTABLE_RVA,
            engine_native::cstate::ID,
        )
    };
    let roster =
        Array::read(memory, game + offset, 8).ok_or("raid tooltip target roster invalid")?;
    let Some(at) = roster.element(id, 8) else {
        return Ok(0);
    };
    let pointer = field::<u64>(memory, at, 0)? as usize;
    if pointer != 0
        && (Some(field::<u64>(memory, pointer, 0)?)
            != engine_sites::at(base, vtable).map(|a| a as u64)
            || field::<u32>(memory, pointer, id_offset)? != id)
    {
        return Err("raid tooltip target identity mismatch");
    }
    Ok(pointer)
}

fn nearest(memory: &impl Memory, state: usize) -> Result<usize, &'static str> {
    let template = field::<u64>(memory, state, engine_native::cstate::TEMPLATE)? as usize;
    let point = |at, offset| -> Result<[i32; 2], &'static str> {
        let bytes = field::<[u8; 8]>(memory, at, offset)?;
        Ok([
            i32::from_le_bytes(bytes[..4].try_into().unwrap()),
            i32::from_le_bytes(bytes[4..].try_into().unwrap()),
        ])
    };
    let centre = point(template, 0x148)?;
    let roster = Array::read(memory, state + engine_native::cstate::PROVINCES, 8)
        .ok_or("raid tooltip state provinces invalid")?;
    let mut best = None;
    for index in 0..roster.count {
        let province = field::<u64>(
            memory,
            roster
                .element(index, 8)
                .ok_or("raid tooltip province address overflow")?,
            0,
        )? as usize;
        let definition = field::<u64>(memory, province, engine_native::cprovince::DEFINITION)? as usize;
        let pos = point(definition, 0xb8)?;
        let dx = pos[0].wrapping_sub(centre[0]);
        let dy = pos[1].wrapping_sub(centre[1]);
        let distance = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        if best.is_none_or(|(_, old)| distance < old) {
            best = Some((province, distance));
        }
    }
    best.map(|(province, _)| province)
        .ok_or("raid tooltip target state has no provinces")
}

fn targets(
    memory: &impl Memory,
    base: usize,
    game: usize,
    raid: usize,
) -> Result<(usize, usize), &'static str> {
    let scope = crate::craid_target::read_row(memory, base, raid + craid_instance::TARGET)?;
    let province = object(memory, base, game, scope.province, true)?;
    let secondary = object(memory, base, game, scope.leader_province, true)?;
    let explicit_state = object(memory, base, game, scope.state, false)?;
    let building_location = object(
        memory,
        base,
        game,
        scope.building_location,
        scope.building_flags & 1 != 0,
    )?;
    let building_state = if building_location != 0 && scope.building_flags & 1 != 0 {
        field::<u64>(memory, building_location, engine_native::cprovince::STATE)? as usize
    } else {
        building_location
    };
    let state = if explicit_state != 0 {
        explicit_state
    } else if province != 0 {
        field::<u64>(memory, province, engine_native::cprovince::STATE)? as usize
    } else if secondary != 0 {
        field::<u64>(memory, secondary, engine_native::cprovince::STATE)? as usize
    } else {
        building_state
    };
    let province = if province != 0 {
        province
    } else if building_location != 0 && scope.building_flags & 1 != 0 {
        building_location
    } else if secondary != 0 {
        secondary
    } else if explicit_state != 0 {
        nearest(memory, explicit_state)?
    } else if building_state != 0 {
        nearest(memory, building_state)?
    } else {
        0
    };
    Ok((state, province))
}

fn target_inputs(
    memory: &impl Memory,
    state: usize,
    province: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    if state != 0 {
        sink.push(TARGET_STATE, 0x278, field(memory, state, 0x278)?)?;
    }
    if province != 0 {
        let roster =
            Array::read(memory, province + 0x110, 8).ok_or("raid tooltip target units invalid")?;
        for index in 0..roster.count {
            let pointer = field::<u64>(
                memory,
                roster
                    .element(index, 8)
                    .ok_or("raid tooltip target unit address overflow")?,
                0,
            )? as usize;
            sink.push(
                TARGET_UNIT_OWNERS,
                index,
                i64::from(field::<u32>(memory, pointer, 0x1d8)?),
            )?;
        }
    }
    Ok(())
}

fn wing(
    memory: &impl Memory,
    base: usize,
    game: usize,
    pointer: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    wing_stats(memory, pointer, sink)?;
    let countries = engine_native::ccountry::Reader::new(memory, base, game)?;
    let raw = field::<i32>(memory, pointer, cair_wing::COUNTRY_OVERRIDE)?;
    let raw = if raw > 0 {
        raw as u32
    } else {
        let pool = field::<u64>(memory, pointer, engine_native_parts::cair_wing_layout::POOL)? as usize;
        let holder = field::<u64>(memory, pool, raid_speed_inputs::WING_POOL_OWNER)? as usize;
        field::<u32>(memory, holder, raid_speed_inputs::POOL_OWNER_COUNTRY)?
    };
    let slot = countries
        .link(raw)?
        .ok_or("raid tooltip wing country unresolved")?;
    let country = countries
        .at(slot)?
        .ok_or("raid tooltip wing country absent")?;
    modifiers(
        memory,
        pointer + 0x238,
        WING_MODIFIERS,
        &[
            keys::AIR_DEFENCE_FACTOR,
            keys::AIR_AGILITY_FACTOR,
            keys::AIR_NIGHT_PENALTY,
            keys::AIR_WEATHER_PENALTY,
        ],
        sink,
    )?;
    modifiers(
        memory,
        country.pointer + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        WING_COUNTRY_MODIFIERS,
        &[
            keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR,
            keys::AIR_DEFENCE_FACTOR,
            keys::AIR_AGILITY_FACTOR,
            keys::AIR_ACE_BONUSES_FACTOR,
            keys::AIR_HOME_DEFENCE_FACTOR,
            keys::AIR_POWER_PROJECTION_FACTOR,
            keys::AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR,
            keys::AIR_STRATEGIC_BOMBER_NIGHT_PENALTY,
            keys::AIR_NIGHT_PENALTY,
            keys::AIR_WEATHER_PENALTY,
            keys::AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR,
        ],
        sink,
    )?;
    let ace_ref = field::<u64>(memory, pointer, cair_wing::ACE_REFERENCE)?;
    sink.push(WING, cair_wing::ACE_REFERENCE as u32, ace_ref as i64)?;
    let Some(ace) = creference_registry::resolve(memory, base, ace_ref)? else {
        return Ok(());
    };
    context::authenticate(memory, base, ace, crate::cace::VTABLE_RVA)?;
    let alive = field::<u8>(memory, ace, crate::cace::ALIVE)?;
    sink.push(ACE, crate::cace::ALIVE as u32, i64::from(alive))?;
    if alive == 0 {
        return Ok(());
    }
    modifiers(
        memory,
        ace + crate::cace::MODIFIERS,
        ACE_MODIFIERS,
        &[
            keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR,
            keys::AIR_DEFENCE_FACTOR,
            keys::AIR_AGILITY_FACTOR,
            keys::AIR_NIGHT_PENALTY,
            keys::AIR_WEATHER_PENALTY,
        ],
        sink,
    )?;
    let kind = field::<u64>(memory, ace, crate::raid_launch_inputs::ACE_TYPE_DEFINITION)? as usize;
    modifiers(
        memory,
        kind + raid_speed_inputs::ACE_TYPE_ACCUMULATOR,
        ACE_TYPE_MODIFIERS,
        &[
            keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR,
            keys::AIR_DEFENCE_FACTOR,
            keys::AIR_AGILITY_FACTOR,
            keys::AIR_NIGHT_PENALTY,
            keys::AIR_WEATHER_PENALTY,
        ],
        sink,
    )?;
    let owner = field::<u64>(memory, ace, crate::cace::OWNER_COUNTRY)? as usize;
    countries.of_pointer(owner)?;
    modifiers(
        memory,
        owner + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        ACE_OWNER_MODIFIERS,
        &[keys::ACE_EFFECTIVENESS_FACTOR],
        sink,
    )?;
    let assigned = field::<u64>(memory, ace, crate::cace::ASSIGNED_WING_REFERENCE)?;
    if let Some(wing) = unit(memory, base, assigned, engine_native_parts::cair_wing_layout::VTABLE_RVA)? {
        sink.push(
            ACE_WING,
            cair_wing::PLANE_COUNT as u32,
            i64::from(field::<i32>(memory, wing, cair_wing::PLANE_COUNT)?),
        )?;
    }
    Ok(())
}

fn operands(
    memory: &impl Memory,
    base: usize,
    game: usize,
    raid: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let army_ref = field::<u64>(memory, raid, craid_instance::UNIT)?;
    let wing_ref = field::<u64>(memory, raid, craid_instance::UNIT + 8)?;
    sink.push(REFERENCES, 0, army_ref as i64)?;
    sink.push(REFERENCES, 1, wing_ref as i64)?;
    let (state, province) = targets(memory, base, game, raid)?;
    target_inputs(memory, state, province, sink)?;
    context::target(memory, base, game, raid, state, province, sink)?;
    let army_pointer = unit(memory, base, army_ref, carmy::VTABLE_RVA)?;
    let wing_pointer = unit(memory, base, wing_ref, engine_native_parts::cair_wing_layout::VTABLE_RVA)?;
    names::capture(memory, base, army_pointer, wing_pointer, sink)?;
    names::source(memory, base, raid, sink)?;
    context::source(memory, base, game, raid, sink)?;
    if let Some(pointer) = army_pointer {
        army(memory, pointer, sink)?;
    }
    if let Some(pointer) = wing_pointer {
        wing(memory, base, game, pointer, sink)?;
        context::wing(memory, base, game, pointer, province, sink)?;
    }
    Ok(())
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game: usize,
    rows: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game, engine_native_memory::walk::Buffer::Growing(rows))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    rows: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let mut sink = Sink {
        rows,
        required: 0,
        raid: 0,
        ordinal: 0,
    };
    let result = requirements::capture(memory, base, &mut sink).and_then(|()| {
        raid_speed_inputs::each_raid(memory, base, game, |_, raid| {
            if raid.phase == craid_instance::PHASE_ENDED {
                return Ok(());
            }
            sink.raid = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
            sink.ordinal = 0;
            let first = sink.required;
            if operands(memory, base, game, raid.pointer, &mut sink).is_err() {
                sink.required = first;
                sink.ordinal = 0;
                sink.push(UNREADABLE, 0, 0)?;
            }
            Ok(())
        })
    });
    match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if sink.required <= sink.rows.len() => Outcome::Rows { len: sink.required },
        Ok(()) => Outcome::Truncated {
            len: sink.rows.len(),
            unread_rows: u32::try_from(sink.required - sink.rows.len()).unwrap_or(u32::MAX),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    #[test]
    fn native_lower_bound_and_direct_occurrences_stay_separate() {
        let mut bytes = vec![0; 256];
        bytes[16..24].copy_from_slice(&64u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x0000000300000003u64.to_le_bytes());
        for (i, (key, raw)) in [
            (0x70u32, 7i64),
            (keys::AIR_DEFENCE_FACTOR, 9),
            (keys::AIR_NIGHT_PENALTY, -11),
        ]
        .into_iter()
        .enumerate()
        {
            bytes[64 + i * 16..68 + i * 16].copy_from_slice(&key.to_le_bytes());
            bytes[72 + i * 16..80 + i * 16].copy_from_slice(&raw.to_le_bytes());
        }
        let mut rows = vec![Row::default(); 4];
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
            required: 0,
            raid: 3,
            ordinal: 0,
        };
        modifiers(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            16,
            WING_MODIFIERS,
            &[
                keys::AIR_DEFENCE_FACTOR,
                keys::AIR_AGILITY_FACTOR,
                keys::AIR_NIGHT_PENALTY,
            ],
            &mut sink,
        )
        .unwrap();
        assert_eq!(sink.required, 4);
        assert_eq!(
            rows.iter().map(|r| (r.key, r.raw)).collect::<Vec<_>>(),
            [
                (keys::AIR_DEFENCE_FACTOR, 7),
                (keys::AIR_DEFENCE_FACTOR, 9),
                (LOOKUP | keys::AIR_DEFENCE_FACTOR, 7),
                (LOOKUP | keys::AIR_NIGHT_PENALTY, -11)
            ]
        );
    }
    #[test]
    fn mission_stats_select_only_the_stored_mission_record_and_refuse_bad_indexes() {
        let pointer = 16;
        let mut bytes = vec![0; 0x2000];
        bytes[pointer + 0x218..pointer + 0x220].copy_from_slice(&123i64.to_le_bytes());
        bytes[pointer + 0x983] = 255;
        bytes[pointer + 0x390 + 0x2f * 8..pointer + 0x398 + 0x2f * 8]
            .copy_from_slice(&987i64.to_le_bytes());
        let mut rows = vec![Row::default(); 6];
        let run = |bytes: &[u8], rows: &mut Vec<Row>| {
            wing_stats(
                &SliceMemory { base: 0, bytes },
                pointer,
                &mut Sink {
                    rows: engine_native_memory::walk::Buffer::Growing(rows),
                    required: 0,
                    raid: 77,
                    ordinal: 0,
                },
            )
        };
        assert_eq!(run(&bytes, &mut rows), Ok(()));
        assert_eq!(rows[0].raw, 123);
        assert_eq!(rows[1].raw, 987);
        bytes[pointer + 0x983] = 0;
        assert_eq!(
            run(&bytes, &mut rows),
            Err("raid tooltip mission stat index outside array")
        );
        bytes[pointer + 0x983] = 254;
        assert_eq!(
            run(&bytes, &mut rows),
            Err("raid tooltip mission stat index negative")
        );
    }
    #[test]
    fn game_score_shared_mission_stats_preserve_raid_refusals() {
        let pointer = 16;
        let mut bytes = vec![0; pointer + 0x984];
        bytes[pointer + 0x974..pointer + 0x978].copy_from_slice(&1u32.to_le_bytes());
        let run = |bytes: &[u8]| {
            wing_stats(
                &SliceMemory { base: 0, bytes },
                pointer,
                &mut Sink {
                    rows: engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 6]),
                    required: 0,
                    raid: 77,
                    ordinal: 0,
                },
            )
        };
        assert_eq!(run(&bytes), Err("raid tooltip mission stat header invalid"));
        assert_eq!(
            run(&bytes[..pointer + 0x983]),
            Err("raid tooltip operand unreadable")
        );
        assert_eq!(
            cair_wing::mission_stat_record(
                &SliceMemory {
                    base: 0,
                    bytes: &[]
                },
                usize::MAX,
                -1
            )
            .map_err(raid_stat_reason),
            Err("raid tooltip stat address overflow")
        );
    }
    #[test]
    fn signed_manpower_entries_and_truncation_demand_are_preserved() {
        let pointer = 16;
        let mut bytes = vec![0; 0x2000];
        bytes[pointer + cunit::STAT_BLOCK..pointer + cunit::STAT_BLOCK + 8]
            .copy_from_slice(&0x1000u64.to_le_bytes());
        bytes[pointer + 0x3e0..pointer + 0x3e8].copy_from_slice(&0x1800u64.to_le_bytes());
        bytes[pointer + 0x3e8..pointer + 0x3f0]
            .copy_from_slice(&0x0000000300000003u64.to_le_bytes());
        for (i, value) in [3i32, -4, i32::MAX].into_iter().enumerate() {
            bytes[0x1804 + i * 8..0x1808 + i * 8].copy_from_slice(&value.to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 8];
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
            required: 0,
            raid: 77,
            ordinal: 0,
        };
        assert_eq!(army(&memory, pointer, &mut sink), Ok(()));
        assert_eq!(sink.required, 8);
        assert_eq!(
            rows[5..].iter().map(|r| r.raw).collect::<Vec<_>>(),
            [3, -4, i64::from(i32::MAX)]
        );
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Count,
            required: 0,
            raid: 77,
            ordinal: 0,
        };
        assert_eq!(army(&memory, pointer, &mut sink), Ok(()));
        assert_eq!(sink.required, 8);
        assert!(
            army(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..128]
                },
                pointer,
                &mut sink
            )
            .is_err()
        );
    }
}
