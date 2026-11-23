
use engine_native_memory::{
    Memory, cpdx_array::Array,
    memory::{Raw, RecordField, RecordLayout, RecordSources, packed_fields},
    read, walk::Outcome,
};
use engine_native_memory::msvc_string::Literal;
use engine_native::{ccountry, cstate, crelation::subject::{Name, NameWalk}};
use engine_native_parts::{
    cideology_group,
    modifier_accumulator,
    modifier_definition::NamedKey,
};
use crate::{
    cintelligence_agency,
    coperative_leader::{
        ACTION,
        ASSIGNMENT,
        COOLDOWN_ASSIGNMENT,
        COOLDOWN_END_HOUR,
        COOLDOWN_KIND,
        COOLDOWN_START_HOUR,
        IDENTITY,
        MODIFIER_ACCUMULATOR,
        NAME_MAX_LENGTH,
        NATIONALITIES,
        OWNER_COUNTRY,
        PENDING_MISSION,
        PENDING_MISSION_PRESENT,
        PORTRAIT_GFX,
    },
    operative_action,
    operative_action_implementation,
};
use engine_native_parts::coperative_leader_layout::VTABLE_RVA;
use engine_sites::Rva;

pub const ROOT_OUT_FACTOR: &[u8] = b"root_out_resistance_effectiveness_factor";
pub const RESISTANCE_COUNTRY: usize = 0x50;
pub const RESISTANCE_ACTIVE: usize = 0x208;
const MAP_SELECTED: usize = 0xf64;
const OPERATIVE_CONTROLLER: usize = 0xa48;
const WARNING_HANDLES: usize = 0x1b8;
const STATIC_MODIFIER_DATABASE: Rva = Rva(0x033d_e590);
const NATIONALITY_MODIFIER: usize = 0x130;
const NETWORK_GAIN_CHEAT: Rva = Rva(0x033d_da94);
const CLIENT_COUNTRY: usize = 0x520;
const CLIENT_COUNTRY_FALLBACK: usize = 0x524;

fn warning_handles(memory: &impl Memory, base: usize) -> Result<Vec<u64>, &'static str> {
    use engine_native_air::command_inputs::{IDLER_RVA, IN_GAME_IDLER_VTABLE_RVA};
    let idler = field::<u64>(memory, site(base, IDLER_RVA)?, 0)? as usize;
    if idler == 0
        || read::<u64>(memory, idler)
            != engine_sites::at(base, IN_GAME_IDLER_VTABLE_RVA).map(|value| value as u64)
    {
        return Err("operative map idler unavailable");
    }
    let controller = field::<u64>(memory, idler, OPERATIVE_CONTROLLER)? as usize;
    if controller == 0 {
        return Ok(Vec::new());
    }
    let address = at(controller, WARNING_HANDLES)?;
    let array = Array::read(memory, address, 8).ok_or("operative warning roster invalid")?;
    let mut handles = vec![0; array.count as usize];
    for (index, handle) in handles.iter_mut().enumerate() {
        *handle = array
            .element(index as u32, 8)
            .and_then(|address| read(memory, address))
            .ok_or("operative warning handle unreadable")?;
    }
    if Array::read(memory, address, 8) != Some(array)
        || field::<u64>(memory, idler, OPERATIVE_CONTROLLER)? != controller as u64
    {
        return Err("operative warning roster changed during capture");
    }
    handles.sort_unstable();
    Ok(handles)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub native_pointer: usize,
    pub map_selected: Option<u8>,
    pub map_warning: Option<u8>,
    pub roster_country: u32,
    pub id: u64,
    pub ordinal: u32,
    pub owner_country: u32,
    pub assignment: u32,
    pub cooldown_kind: Option<u32>,
    pub cooldown_start_hour: Option<i32>,
    pub cooldown_end_hour: Option<i32>,
    pub variant: Option<u32>,
    pub target_country: Option<u32>,
    pub target_state: Option<i32>,
    pub ideology: Name,
    pub pending_variant: Option<u32>,
    pub pending_country: Option<u32>,
    pub pending_state: Option<i32>,
    pub pending_ideology: Name,
    pub portrait: Name,
    pub root_out_country_factor: Option<i64>,
    pub root_out_operative_factor: Option<i64>,
    pub root_out_nationality: Option<u8>,
}

pub struct Walk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub failed_operative: Option<(u64, u32)>,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    bytes: &mut Vec<u8>,
    root_out: &mut NamedKey,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(bytes),
        root_out,
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    bytes: engine_native_memory::walk::Buffer<'_, u8>,
    root_out: &mut NamedKey,
) -> Walk {
    walk_keyed(memory, base, game_state, output, bytes, &mut || {
        resolve_root_out(root_out, memory, base)
    })
}

pub const fn root_out_key() -> NamedKey {
    NamedKey::new(ROOT_OUT_FACTOR)
}

fn resolve_root_out(
    key: &mut NamedKey,
    memory: &impl Memory,
    base: usize,
) -> Result<u32, &'static str> {
    key.get(memory, base)?
        .ok_or("root out resistance factor modifier is not registered")
}

fn walk_keyed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
    root_out: &mut impl FnMut() -> Result<u32, &'static str>,
) -> Walk {
    let mut failed_operative = None;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut mission_vtables = operative_action_implementation::Vtables::new(base);
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("operative vtable withheld")?;
        let expected_agency =
            engine_sites::at(base, cintelligence_agency::VTABLE_RVA).map(|v| v as u64);
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let warnings = warning_handles(memory, base).ok();
        let (mut bytes_len, mut required_bytes) = (0_usize, 0_usize);
        const BATCH: usize = 64;
        const COUNTRY_AGENCY_FIELDS: [RecordField; 1] =
            packed_fields([(ccountry::INTELLIGENCE_AGENCY, 8)]);
        const COUNTRY_AGENCY_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&COUNTRY_AGENCY_FIELDS, 8).unwrap();
        const MISSION_AGENCY_FIELDS: [RecordField; 3] = packed_fields([
            (0, 8),
            (cintelligence_agency::OWNER_COUNTRY, 8),
            (cintelligence_agency::OPERATIVES, 16),
        ]);
        const MISSION_AGENCY_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&MISSION_AGENCY_FIELDS, 32).unwrap();
        let mut country_slots = [engine_native_memory::country_identity::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        }; BATCH];
        let mut country_ptrs = [0_usize; BATCH];
        let mut agency_ptr_bytes = [0_u8; BATCH * 8];
        let mut agency_ptrs = [0_usize; BATCH];
        let mut agency_map = [u8::MAX; BATCH];
        let mut agency_records = [0_u8; BATCH * 32];
        let mut next = 1_u32;
        while next < countries.count() {
            let end = countries.count().min(next + BATCH as u32);
            let mut present = 0_usize;
            for slot in next..end {
                if let Some(country) = countries.at(slot)? {
                    country_slots[present] = country;
                    country_ptrs[present] = country.pointer;
                    present += 1;
                }
            }
            next = end;
            if present == 0 {
                continue;
            }
            let ptrs_copied = memory.copy_records(
                RecordSources::Pointers(&country_ptrs[..present]),
                &COUNTRY_AGENCY_LAYOUT,
                &mut agency_ptr_bytes[..present * 8],
            );
            let mut live = 0_usize;
            agency_map[..present].fill(u8::MAX);
            for index in 0..ptrs_copied.min(present) {
                let ptr = u64::from_le_bytes(
                    agency_ptr_bytes[index * 8..(index + 1) * 8]
                        .try_into()
                        .unwrap(),
                );
                if let Ok(ptr) = usize::try_from(ptr)
                    && ptr != 0
                {
                    agency_map[index] = live as u8;
                    agency_ptrs[live] = ptr;
                    live += 1;
                }
            }
            let agencies_copied = if live == 0 {
                0
            } else {
                memory.copy_records(
                    RecordSources::Pointers(&agency_ptrs[..live]),
                    &MISSION_AGENCY_LAYOUT,
                    &mut agency_records[..live * 32],
                )
            };
            for index in 0..present {
                let country = country_slots[index];
                let array = if index < ptrs_copied {
                    let agency = usize::try_from(u64::from_le_bytes(
                        agency_ptr_bytes[index * 8..(index + 1) * 8]
                            .try_into()
                            .unwrap(),
                    ))
                    .map_err(|_| "agency pointer overflow")?;
                    if agency == 0 {
                        continue;
                    }
                    let slot = agency_map[index] as usize;
                    if agency_map[index] != u8::MAX && (slot + 1) * 3 <= agencies_copied {
                        let rec = &agency_records[slot * 32..(slot + 1) * 32];
                        let expected_a = expected_agency.ok_or("agency vtable address overflow")?;
                        if u64::from_le_bytes(rec[0..8].try_into().unwrap()) != expected_a {
                            return Err("agency vtable mismatch");
                        }
                        if u64::from_le_bytes(rec[8..16].try_into().unwrap())
                            != country.pointer as u64
                        {
                            return Err("agency owner pointer mismatch");
                        }
                        Array::from_header(rec[16..32].try_into().unwrap(), 8)
                            .ok_or("operative roster unreadable or invalid")?
                    } else {
                        let Some(agency) = cintelligence_agency::root(memory, base, country)?
                        else {
                            continue;
                        };
                        Array::read(memory, at(agency, cintelligence_agency::OPERATIVES)?, 8)
                            .ok_or("operative roster unreadable or invalid")?
                    }
                } else {
                    let Some(agency) = cintelligence_agency::root(memory, base, country)? else {
                        continue;
                    };
                    Array::read(memory, at(agency, cintelligence_agency::OPERATIVES)?, 8)
                        .ok_or("operative roster unreadable or invalid")?
                };
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("operative mission row demand overflow")?,
            )?;
            let roster_start = len;
            for index in 0..array.count {
                let pointer = array
                    .element(index, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("operative roster pointer unreadable or null")?;
                if read::<u64>(memory, pointer) != Some(expected as u64) {
                    return Err("operative vtable mismatch");
                }
                let id = field::<u64>(memory, pointer, IDENTITY)?;
                if id == 0 {
                    return Err("operative has a null handle");
                }
                failed_operative = Some((id, country.id));
                let portrait = literal(memory, at(pointer, PORTRAIT_GFX)?, PORTRAIT_REFUSALS)?;
                let (mut row, ideologies) = read_row(
                    memory,
                    base,
                    &countries,
                    pointer,
                    (id, country.id),
                    &mut mission_vtables,
                    &mut *root_out,
                )?;
                let [active, pending] = ideologies;
                row.map_selected = field::<u8>(memory, pointer, MAP_SELECTED)
                    .ok()
                    .map(|value| u8::from(value != 0));
                row.map_warning = if row.map_selected == Some(1) {
                    Some(0)
                } else {
                    warnings
                        .as_ref()
                        .map(|handles| u8::from(handles.binary_search(&id).is_ok()))
                };
                let strings = [active, pending, Some(portrait)];
                let row_bytes = strings
                    .iter()
                    .flatten()
                    .try_fold(0_usize, |sum, string| sum.checked_add(string.len))
                    .ok_or("operative mission string size overflow")?;
                required_bytes = required_bytes
                    .checked_add(row_bytes)
                    .ok_or("operative mission string size overflow")?;
                bytes.ensure(required_bytes)?;
                if len == output.len() || row_bytes > bytes.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("operative mission row count overflow")?;
                    failed_operative = None;
                    continue;
                }
                row.ordinal = index;
                if output[roster_start..len].iter().any(|old| old.id == row.id) {
                    return Err("duplicate operative handle within active roster");
                }
                let mut spans = [Name::default(); 3];
                for (span, string) in spans.iter_mut().zip(&strings) {
                    let Some(string) = string else {
                        continue;
                    };
                    Literal::copy_all(
                        memory,
                        [string],
                        &mut bytes[bytes_len..bytes_len + string.len],
                    )
                    .map_err(|_| "operative mission string bytes unreadable")?;
                    *span = Name {
                        start: bytes_len,
                        len: string.len,
                    };
                    bytes_len += string.len;
                }
                [row.ideology, row.pending_ideology, row.portrait] = spans;
                output[len] = row;
                len += 1;
                failed_operative = None;
            }
            }
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    let read = result.unwrap_or_else(engine_native::crelation::subject::unavailable);
    Walk {
        outcome: read.outcome,
        bytes_len: read.bytes_len,
        required_bytes: read.required_bytes,
        failed_operative,
    }
}

type Refusals = [&'static str; 4];
const PORTRAIT_REFUSALS: Refusals = [
    "operative portrait header unreadable",
    "operative portrait exceeds native string maximum",
    "operative portrait unreadable or invalid",
    "operative portrait terminator unreadable or invalid",
];
const IDEOLOGY_REFUSALS: Refusals = [
    "operative mission ideology header unreadable",
    "operative mission ideology exceeds native string maximum",
    "operative mission ideology unreadable or invalid",
    "operative mission ideology terminator unreadable or invalid",
];

fn literal(
    memory: &impl Memory,
    address: usize,
    refusals: Refusals,
) -> Result<Literal, &'static str> {
    let header = read::<[u8; 32]>(memory, address).ok_or(refusals[0])?;
    if u64::from_le_bytes(header[16..24].try_into().unwrap()) > NAME_MAX_LENGTH
        || u64::from_le_bytes(header[24..32].try_into().unwrap()) > NAME_MAX_LENGTH
    {
        return Err(refusals[1]);
    }
    let string = Literal::from_header(address, header).map_err(|_| refusals[2])?;
    Literal::validate(memory, [&string]).map_err(|_| refusals[3])?;
    Ok(string)
}

fn ideology(
    memory: &impl Memory,
    base: usize,
    group: u64,
) -> Result<Option<Literal>, &'static str> {
    let group = usize::try_from(group).map_err(|_| "operative mission ideology overflow")?;
    if group == 0 {
        return Ok(None);
    }
    let expected = engine_sites::at(base, cideology_group::VTABLE_RVA)
        .ok_or("operative mission ideology vtable withheld")?;
    if read::<u64>(memory, group) != Some(expected as u64) {
        return Err("operative mission ideology vtable mismatch");
    }
    literal(
        memory,
        at(group, cideology_group::TOKEN)?,
        IDEOLOGY_REFUSALS,
    )
    .map(Some)
}

fn read_row<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    pointer: usize,
    (id, roster_country): (u64, u32),
    mission_vtables: &mut operative_action_implementation::Vtables,
    root_out: &mut impl FnMut() -> Result<u32, &'static str>,
) -> Result<(Row, [Option<Literal>; 2]), &'static str> {
    let owner_country = field::<u32>(memory, pointer, OWNER_COUNTRY)?;
    let assignment = field::<u32>(memory, pointer, ASSIGNMENT)?;
    let (cooldown_kind, cooldown_start_hour, cooldown_end_hour) =
        if assignment == COOLDOWN_ASSIGNMENT {
            (
                Some(field(memory, pointer, COOLDOWN_KIND)?),
                Some(field(memory, pointer, COOLDOWN_START_HOUR)?),
                Some(field(memory, pointer, COOLDOWN_END_HOUR)?),
            )
        } else {
            (None, None, None)
        };
    let mission = at(pointer, ACTION)?;
    let variant = field::<u32>(memory, mission, operative_action::VARIANT)?;
    let variant = (variant != 0).then_some(variant);
    let mut root_out_operands = None;
    let (target_country, target_state, active_ideology) = match variant {
        Some(variant) => {
            let targets = mission_vtables.targets(memory, mission, variant)?;
            let target_state = targets
                .state
                .map(|state| state_id(memory, base, state))
                .transpose()?;
            if variant == operative_action::ROOT_OUT_RESISTANCE {
                root_out_operands = Some(root_out_operands_of(
                    memory,
                    base,
                    countries,
                    pointer,
                    owner_country,
                    targets,
                    root_out()?,
                )?);
            }
            let group = if variant == operative_action::BOOST_IDEOLOGY {
                let implementation = usize::try_from(field::<u64>(
                    memory,
                    mission,
                    operative_action::IMPLEMENTATION,
                )?)
                .map_err(|_| "operative mission implementation overflow")?;
                ideology(
                    memory,
                    base,
                    field(
                        memory,
                        implementation,
                        operative_action_implementation::BOOST_IDEOLOGY,
                    )?,
                )?
            } else {
                None
            };
            (Some(targets.country), target_state, group)
        }
        None => (None, None, None),
    };
    let (pending_variant, pending_country, pending_state, pending_ideology) =
        if field::<u8>(memory, pointer, PENDING_MISSION_PRESENT)? != 0 {
            let pending = at(pointer, PENDING_MISSION)?;
            let state = usize::try_from(field::<u64>(
                memory,
                pending,
                operative_action::PENDING_TARGET_STATE,
            )?)
            .map_err(|_| "operative pending mission state overflow")?;
            (
                Some(field(memory, pending, operative_action::PENDING_VARIANT)?),
                Some(field(
                    memory,
                    pending,
                    operative_action::PENDING_TARGET_COUNTRY,
                )?),
                (state != 0)
                    .then(|| state_id(memory, base, state))
                    .transpose()?,
                ideology(
                    memory,
                    base,
                    field(memory, pending, operative_action::PENDING_IDEOLOGY)?,
                )?,
            )
        } else {
            (None, None, None, None)
        };
    let row = Row {
        native_pointer: pointer,
        map_selected: None,
        map_warning: None,
        roster_country,
        id,
        ordinal: 0,
        owner_country,
        assignment,
        cooldown_kind,
        cooldown_start_hour,
        cooldown_end_hour,
        variant,
        target_country,
        target_state,
        ideology: Name::default(),
        pending_variant,
        pending_country,
        pending_state,
        pending_ideology: Name::default(),
        portrait: Name::default(),
        root_out_country_factor: root_out_operands.map(|operands| operands.0),
        root_out_operative_factor: root_out_operands.map(|operands| operands.1),
        root_out_nationality: root_out_operands.map(|operands| operands.2),
    };
    Ok((row, [active_ideology, pending_ideology]))
}

pub struct EffectRoots {
    pub keys: [u32; 2],
    pub key_count: usize,
    pub roots: [usize; 3],
    pub owned_states: i32,
    pub network_gain_scale: i32,
}

pub fn effect_roots<M: Memory>(
    memory: &M,
    base: usize,
    game: usize,
    countries: &ccountry::Reader<'_, M>,
    row: &Row,
    pending: bool,
    root_out: &mut NamedKey,
) -> Result<Option<EffectRoots>, &'static str> {
    let (variant, target, state) = if pending {
        (row.pending_variant, row.pending_country, row.pending_state)
    } else {
        (row.variant, row.target_country, row.target_state)
    };
    let Some(variant) = variant else {
        return Ok(None);
    };
    if variant == operative_action::QUIET_NETWORK {
        return Ok(None);
    }
    let (keys, key_count) = match variant {
        operative_action::BUILD_NETWORK => ([0x212, 0x213], 2),
        operative_action::COUNTER_INTELLIGENCE => ([0x1ff, 0], 1),
        operative_action::ROOT_OUT_RESISTANCE => {
            ([resolve_root_out(root_out, memory, base)?, 0], 1)
        }
        operative_action::BOOST_IDEOLOGY => ([0x20e, 0], 1),
        operative_action::CONTROL_TRADE => ([0x20b, 0], 1),
        operative_action::DIPLOMATIC_PRESSURE => ([0x20c, 0], 1),
        operative_action::PROPAGANDA => ([0x20d, 0], 1),
        _ => return Err("operative effect variant unsupported"),
    };
    if field::<u64>(memory, row.native_pointer, IDENTITY)? != row.id
        || field::<u32>(memory, row.native_pointer, OWNER_COUNTRY)? != row.owner_country
    {
        return Err("operative effect identity changed");
    }
    let target = target.ok_or("operative effect target absent")?;
    let mut context = target;
    if variant == operative_action::ROOT_OUT_RESISTANCE
        && let Some(id) = state.filter(|&id| id > 0)
    {
        let states = Array::read(memory, at(game, engine_native::cgame_state::STATES)?, 8)
            .ok_or("operative effect states invalid")?;
        let state = states
            .element(id as u32, 8)
            .and_then(|address| read::<u64>(memory, address))
            .filter(|&pointer| pointer != 0)
            .ok_or("operative effect state unreadable")? as usize;
        if state_id(memory, base, state)? != id {
            return Err("operative effect state identity mismatch");
        }
        let resistance = at(state, cstate::RESISTANCE)?;
        let country = field::<u32>(memory, resistance, RESISTANCE_COUNTRY)?;
        if field::<u8>(memory, resistance, RESISTANCE_ACTIVE)? != 0 && country as i32 > 0 {
            context = country;
        }
    }
    let mut roots = [0; 3];
    if variant != operative_action::COUNTER_INTELLIGENCE {
        roots[0] = at(
            countries.resolve(row.owner_country)?.pointer,
            engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        )?;
    }
    roots[1] = at(row.native_pointer, MODIFIER_ACCUMULATOR)?;
    if has_nationality(memory, countries, row.native_pointer, context)? {
        let database = field::<u64>(memory, site(base, STATIC_MODIFIER_DATABASE)?, 0)? as usize;
        if database != 0 {
            let modifier = field::<u64>(memory, database, NATIONALITY_MODIFIER)? as usize;
            if modifier != 0 {
                if field::<u64>(memory, modifier, 0)?
                    != site(base, engine_native_parts::cstatic_modifier::VTABLE_RVA)? as u64
                {
                    return Err("operative nationality modifier identity mismatch");
                }
                roots[2] = at(modifier, engine_native_parts::cstatic_modifier::ACCUMULATOR)?;
            }
        }
    }
    let owned_states = if target as i32 > 0 {
        field(
            memory,
            countries.resolve(target)?.pointer,
            ccountry::OWNED_STATE_COUNT,
        )?
    } else {
        0
    };
    let mut network_gain_scale = 1;
    if variant == operative_action::BUILD_NETWORK
        && field::<u8>(memory, site(base, NETWORK_GAIN_CHEAT)?, 0)? != 0
    {
        let mut client = field::<u32>(memory, game, CLIENT_COUNTRY)?;
        if client as i32 <= 0 {
            client = field(memory, game, CLIENT_COUNTRY_FALLBACK)?;
        }
        if client == row.owner_country
            || (client != 0
                && countries.resolve(client)?.canonical_slot
                    == countries.resolve(row.owner_country)?.canonical_slot)
        {
            network_gain_scale = 100;
        }
    }
    Ok(Some(EffectRoots {
        keys,
        key_count,
        roots,
        owned_states,
        network_gain_scale,
    }))
}

fn root_out_operands_of<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    operative: usize,
    owner: u32,
    targets: operative_action_implementation::Targets,
    key: u32,
) -> Result<(i64, i64, u8), &'static str> {
    let owner = countries.resolve(owner)?;
    let factor = |accumulator| {
        modifier_accumulator::lookup_effective_at(memory, base, accumulator, key)
            .map(|value| value.effective)
    };
    let country_factor = factor(at(owner.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?)?;
    let operative_factor = factor(at(operative, MODIFIER_ACCUMULATOR)?)?;
    let mut context = targets.country;
    if let Some(state) = targets.state {
        let resistance = at(state, cstate::RESISTANCE)?;
        let active = field::<u8>(memory, resistance, RESISTANCE_ACTIVE)?;
        let country = field::<u32>(memory, resistance, RESISTANCE_COUNTRY)?;
        if active != 0 && (country as i32) > 0 {
            context = country;
        }
    }
    let nationality = has_nationality(memory, countries, operative, context)?;
    Ok((country_factor, operative_factor, u8::from(nationality)))
}

fn has_nationality<M: Memory>(
    memory: &M,
    countries: &ccountry::Reader<'_, M>,
    operative: usize,
    raw: u32,
) -> Result<bool, &'static str> {
    let values = Array::read(memory, at(operative, NATIONALITIES)?, 4)
        .ok_or("operative nationalities unreadable or invalid")?;
    if values.count == 0 {
        return Ok(false);
    }
    let canonical = |id: u32| {
        if id == 0 {
            Ok(0)
        } else {
            countries.resolve(id).map(|country| country.canonical_slot)
        }
    };
    let entry = |index: u32| {
        values
            .element(index, 4)
            .and_then(|address| read::<u32>(memory, address))
            .ok_or("operative nationality unreadable")
    };
    let target = canonical(raw)?;
    let mut first = 0;
    let mut count = values.count;
    while count != 0 {
        let half = count / 2;
        let id = entry(first + half)?;
        if id != raw && canonical(id)? < target {
            first += half + count % 2;
        }
        count = half;
    }
    if first == values.count {
        return Ok(false);
    }
    let id = entry(first)?;
    Ok(id == raw || canonical(id)? <= target)
}

fn state_id(memory: &impl Memory, base: usize, state: usize) -> Result<i32, &'static str> {
    let expected = engine_sites::at(base, cstate::VTABLE_RVA)
        .ok_or("operative mission state vtable withheld")?;
    if read::<u64>(memory, state) != Some(expected as u64) {
        return Err("operative mission state vtable mismatch");
    }
    field(memory, state, cstate::ID)
}

fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("operative site withheld")
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("operative address overflow")
}

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("operative field unreadable")
}

#[cfg(test)]
mod tests;
