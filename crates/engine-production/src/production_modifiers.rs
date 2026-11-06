//! Passive production-hover operands. Cached MIO bonuses are copied without
//! refreshing them. Shared passive readers acquire source trees; this pack owns
//! country selection and daily cadence. Requests use the public accumulator/key
//! reader, independently of army-specific demand selection.

use crate::{
    Memory, attrition_modifier_breakdown::Request, ccountry, cequipment_production_line as line,
    cequipment_variant, cpdx_array::Array, cproduction_line, cproduction_status,
    modifier_keys as keys, read,
};
use engine_sites::Rva;

/// Production-tooltip modifier keys in increasing order.
pub const KEYS: [u32; 11] = [
    keys::LINE_CHANGE_PRODUCTION_EFFICIENCY_FACTOR,
    keys::OUT_OF_POWER_IMPACT_FACTOR,
    keys::PRODUCTION_FACTORY_MAX_EFFICIENCY_FACTOR,
    keys::PRODUCTION_FACTORY_EFFICIENCY_GAIN_FACTOR,
    keys::PRODUCTION_FACTORY_START_EFFICIENCY_FACTOR,
    keys::INDUSTRIAL_CAPACITY_FACTORY,
    keys::INDUSTRIAL_CAPACITY_FACTORY_POWERED,
    keys::INDUSTRIAL_CAPACITY_DOCKYARD,
    keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED,
    keys::EQUIPMENT_CONVERSION_SPEED,
    keys::REFIT_SPEED,
];
/// Seven production categories mapped through the runtime token table, distinct from token IDs.
pub const BONUS_COUNT: usize = 7;
const BONUS_KEYS_RVA: Rva = Rva(0x0343_8320);

/// Signed i32 start/cap percentages, signed fixed5 gain/balance and four fixed5 factory rates.
const DEFINE_RVAS: [Rva; 8] = [
    Rva(0x033e_0dec),
    Rva(0x033e_0e68),
    Rva(0x033e_0ef0),
    Rva(0x033e_0f98),
    Rva(0x033e_0bc0),
    Rva(0x033e_0d68),
    Rva(0x033e_0b28),
    Rva(0x033e_0cd8),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub start: i32,
    pub cap: i32,
    pub gain: i64,
    pub balance: i64,
    /// Unpowered/powered military, then unpowered/powered naval IC rates.
    pub factory_speed: [i64; 4],
    /// Switch percentages for same variant, parent, family and archetype.
    pub retention: [i64; 4],
    /// Stored fixed5 base conversion-speed bonus.
    pub conversion: i64,
    pub module_mask: u64,
    pub bonus_keys: [u32; BONUS_COUNT],
}

impl Context {
    pub fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut result = Self {
            start: global(memory, base, DEFINE_RVAS[0])?,
            cap: global(memory, base, DEFINE_RVAS[1])?,
            gain: global(memory, base, DEFINE_RVAS[2])?,
            balance: global(memory, base, DEFINE_RVAS[3])?,
            ..Self::default()
        };
        for (value, rva) in result.factory_speed.iter_mut().zip(&DEFINE_RVAS[4..]) {
            *value = global(memory, base, *rva)?;
        }
        let bonus_keys = module_at(base, BONUS_KEYS_RVA)?;
        for (category, key) in result.bonus_keys.iter_mut().enumerate() {
            *key = field(memory, bonus_keys, category * 0x28)?;
        }
        for (value, rva) in result.retention.iter_mut().zip([
            Rva(0x033e_1020),
            Rva(0x033e_109c),
            Rva(0x033e_1130),
            Rva(0x033e_11b4),
        ]) {
            *value = i64::from(global::<i32>(memory, base, rva)?).wrapping_mul(1000);
        }
        result.conversion = global(memory, base, Rva(0x033e_23e8))?;
        // Equipment-category feature gates.
        let mask = global::<u32>(memory, base, crate::dlc_feature_state::MASK_RVA)?;
        result.module_mask = (if mask & (1 << 4) != 0 {
            0x80004001c0
        } else {
            0
        }) | (if mask & (1 << 7) != 0 { 4 } else { 0 })
            | (if mask & (1 << 8) != 0 {
                0x1d0026fc00
            } else {
                0
            });
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub country: u32,
    /// Pass-local tree address, never serialized as an identity.
    pub accumulator: usize,
    /// Stored powered-factory fraction.
    pub power_raw: i64,
    /// The country's census slot, pass-local: it places the country in a
    /// slot range of a capture split into parts ([`capture_growing_slots`]).
    pub slot: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub handle: u64,
    pub country: u32,
    pub variant: u64,
    pub organisation: u64,
    pub kind: u8,
    pub active_factories: i32,
    /// Growth/output operand.
    pub resource_penalty: i64,
    /// the stored bonus; a getter would refresh the bonus cache when nonzero.
    pub organisation_dirty: bool,
    /// The line's conversion control (`is_converting` in saves).
    pub conversion: bool,
    /// Positive variant licensing selects the licence-speed contribution.
    pub licensed: bool,
    pub conversion_output: bool,
    pub refit_output: bool,
    /// Type archetype selection independent of cached variant flags.
    pub naval_efficiency: bool,
    /// Stored variant ship-family mask used for output.
    pub naval_output: bool,
    pub bonuses: [i64; BONUS_COUNT],
    /// All saved factory-efficiency occurrences, in native order.
    pub efficiencies: [usize; 2],
}

pub struct Buffers<'a> {
    pub countries: &'a mut [Country],
    pub country_len: usize,
    pub country_required: usize,
    pub lines: &'a mut [Line],
    pub line_len: usize,
    pub line_required: usize,
    pub efficiencies: &'a mut [i64],
    pub efficiency_len: usize,
    pub efficiency_required: usize,
    pub requests: &'a mut [Request],
    pub request_len: usize,
    pub request_required: usize,
    pub archetypes: &'a mut [usize],
    pub archetype_required: usize,
}

pub struct GrowingBuffers<'a> {
    pub countries: &'a mut Vec<Country>,
    pub country_len: usize,
    pub country_required: usize,
    pub lines: &'a mut Vec<Line>,
    pub line_len: usize,
    pub line_required: usize,
    pub efficiencies: &'a mut Vec<i64>,
    pub efficiency_len: usize,
    pub efficiency_required: usize,
    pub requests: &'a mut Vec<Request>,
    pub request_len: usize,
    pub request_required: usize,
    pub archetypes: &'a mut Vec<usize>,
    pub archetype_required: usize,
}

pub struct BuffersBuffer<'a> {
    pub countries: crate::walk::Buffer<'a, Country>,
    pub country_len: usize,
    pub country_required: usize,
    pub lines: crate::walk::Buffer<'a, Line>,
    pub line_len: usize,
    pub line_required: usize,
    pub efficiencies: crate::walk::Buffer<'a, i64>,
    pub efficiency_len: usize,
    pub efficiency_required: usize,
    pub requests: crate::walk::Buffer<'a, Request>,
    pub request_len: usize,
    pub request_required: usize,
    pub archetypes: crate::walk::Buffer<'a, usize>,
    pub archetype_required: usize,
}

impl<'a> From<Buffers<'a>> for BuffersBuffer<'a> {
    fn from(value: Buffers<'a>) -> Self {
        Self {
            countries: crate::walk::Buffer::Fixed(value.countries),
            country_len: value.country_len,
            country_required: value.country_required,
            lines: crate::walk::Buffer::Fixed(value.lines),
            line_len: value.line_len,
            line_required: value.line_required,
            efficiencies: crate::walk::Buffer::Fixed(value.efficiencies),
            efficiency_len: value.efficiency_len,
            efficiency_required: value.efficiency_required,
            requests: crate::walk::Buffer::Fixed(value.requests),
            request_len: value.request_len,
            request_required: value.request_required,
            archetypes: crate::walk::Buffer::Fixed(value.archetypes),
            archetype_required: value.archetype_required,
        }
    }
}

impl<'a> From<GrowingBuffers<'a>> for BuffersBuffer<'a> {
    fn from(value: GrowingBuffers<'a>) -> Self {
        Self {
            countries: crate::walk::Buffer::Growing(value.countries),
            country_len: value.country_len,
            country_required: value.country_required,
            lines: crate::walk::Buffer::Growing(value.lines),
            line_len: value.line_len,
            line_required: value.line_required,
            efficiencies: crate::walk::Buffer::Growing(value.efficiencies),
            efficiency_len: value.efficiency_len,
            efficiency_required: value.efficiency_required,
            requests: crate::walk::Buffer::Growing(value.requests),
            request_len: value.request_len,
            request_required: value.request_required,
            archetypes: crate::walk::Buffer::Growing(value.archetypes),
            archetype_required: value.archetype_required,
        }
    }
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("production modifier address overflow")
}

fn module_at(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("production modifier address overflow")
}

/// Whether military production converts into `variant` while its line's
/// conversion control is set: the native target gate, before the module
/// restriction the conversion-output bonus adds. A design with a parent, or of
/// a conversion-family type, is a target unless licensed; any design of a type
/// with a convert-from list is one. No game function is called.
pub fn conversion_target(memory: &impl Memory, variant: usize) -> Result<bool, &'static str> {
    fn operand<T: crate::memory::Raw>(
        memory: &impl Memory,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read(memory, address))
            .ok_or("conversion target operand unreadable")
    }
    if variant == 0 {
        return Ok(false);
    }
    let definition = usize::try_from(operand::<u64>(
        memory,
        variant,
        cequipment_variant::DEFINITION,
    )?)
    .ok()
    .filter(|&pointer| pointer != 0)
    .ok_or("conversion target equipment type null")?;
    use crate::cequipment_type::{CONVERSION_FAMILY, CONVERT_FROM_COUNT};
    Ok(
        (operand::<u64>(memory, variant, cequipment_variant::PARENT)? != 0
            || operand::<u8>(memory, definition, CONVERSION_FAMILY)? != 0)
            && operand::<i32>(memory, variant, cequipment_variant::ORIGIN_LINK)? <= 0
            || operand::<i32>(memory, definition, CONVERT_FROM_COUNT)? != 0,
    )
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("production modifier operand unreadable")
}
fn global<T: crate::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    read(memory, module_at(base, key)?).ok_or("production modifier operand unreadable")
}
fn add(value: &mut usize, amount: usize) -> Result<(), &'static str> {
    *value = value
        .checked_add(amount)
        .ok_or("production modifier demand overflow")?;
    Ok(())
}
fn normalized(mut flags: u64) -> u64 {
    use crate::cequipment_type as ty;
    // Category normalization preserves branch order.
    if flags & ty::SHIP_MASK != 0 {
        if flags & ty::NORMALIZE_FIRST_SELECTOR != 0 {
            flags &= !ty::NORMALIZE_FIRST_CLEAR;
        } else if flags & ty::NORMALIZE_SECOND_SELECTOR != 0 {
            flags &= !ty::NORMALIZE_SECOND_CLEAR;
        } else if flags & ty::NORMALIZE_THIRD_SELECTOR != 0 {
            flags &= !ty::SPECIAL_POOL_MASK;
        }
    }
    flags
}

/// Effective archetype precedes the current type; caller scratch retains that order without a depth cap.
fn naval(
    memory: &impl Memory,
    base: usize,
    mut pointer: usize,
    out: &mut BuffersBuffer<'_>,
) -> Result<bool, &'static str> {
    use crate::cequipment_type as ty;
    let mut depth = 0;
    loop {
        if pointer == 0 || out.archetypes[..depth].contains(&pointer) {
            return Err("production modifier equipment archetype null or cyclic");
        }
        if field::<u64>(memory, pointer, 0)? != module_at(base, ty::VTABLE_RVA)? as u64 {
            return Err("production modifier equipment type vtable mismatch");
        }
        out.archetype_required = out.archetype_required.max(depth + 1);
        out.archetypes.ensure(
            depth
                .checked_add(1)
                .ok_or("production modifier archetype depth overflow")?,
        )?;
        let Some(slot) = out.archetypes.get_mut(depth) else {
            return Err("production modifier archetype scratch capacity exceeded");
        };
        *slot = pointer;
        depth += 1;
        let parent = field::<u64>(memory, pointer, ty::ARCHETYPE)?;
        let parent = usize::try_from(parent)
            .map_err(|_| "production modifier archetype pointer overflow")?;
        if parent == pointer {
            break;
        }
        pointer = parent;
    }
    while depth > 0 {
        depth -= 1;
        let current = out.archetypes[depth];
        let mut primary = field::<u64>(memory, current, ty::PRIMARY_CATEGORY_FLAGS)?;
        if primary == 0 {
            let parent = field::<u64>(memory, current, ty::ARCHETYPE)? as usize;
            primary = field(memory, parent, ty::PRIMARY_CATEGORY_FLAGS)?;
        }
        let flags =
            normalized(primary | field::<u64>(memory, current, ty::SECONDARY_CATEGORY_FLAGS)?);
        if flags & ty::SHIP_MASK != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub countries: usize,
    pub lines: usize,
    pub efficiencies: usize,
    pub requests: usize,
}

/// Off-thread headers only. Production refresh sizes retained efficiency
/// entries from the factory limit; active factories and country count do not
/// bound them. Values are acquired independently.
pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Result<Demand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut demand = Demand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let status = cproduction_status::manager(memory, base, country)?;
        let roster_at = at(status, cproduction_status::LINES)?;
        let roster = Array::read(memory, roster_at, 8)
            .ok_or("production modifier line roster unreadable or invalid")?;
        if roster.count == 0 {
            continue;
        }
        add(&mut demand.countries, 1)?;
        add(&mut demand.requests, KEYS.len())?;
        add(&mut demand.lines, roster.count as usize)?;
        for ordinal in 0..roster.count {
            let pointer = roster
                .element(ordinal, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p != 0)
                .ok_or("production modifier line pointer unreadable or null")?;
            if field::<u64>(memory, pointer, cproduction_line::PRODUCTION_STATUS)? != status as u64
            {
                return Err("production modifier line identity or owner mismatch");
            }
            let factory_at = at(pointer, line::FACTORY_EFFICIENCIES)?;
            let header = read::<[u8; 16]>(memory, factory_at)
                .ok_or("production modifier factory efficiencies unreadable or invalid")?;
            let factory = Array::from_header(header, 8)
                .ok_or("production modifier factory efficiencies unreadable or invalid")?;
            add(&mut demand.efficiencies, factory.count as usize)?;
            if read::<[u8; 16]>(memory, factory_at) != Some(header) {
                return Err("production modifier sizing factory header changed");
            }
        }
        if Array::read(memory, roster_at, 8) != Some(roster) {
            return Err("production modifier sizing line roster changed");
        }
    }
    Ok(demand)
}

/// One authenticated production roster. No allocation, game calls, or retries.
/// Short caller-owned arenas retain demand for off-hour growth and refuse the
/// supplement; the existing ProductionLines census remains independent.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    context: &mut Option<Context>,
    out: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    let mut buffer = BuffersBuffer {
        countries: crate::walk::Buffer::Fixed(&mut *out.countries),
        country_len: out.country_len,
        country_required: out.country_required,
        lines: crate::walk::Buffer::Fixed(&mut *out.lines),
        line_len: out.line_len,
        line_required: out.line_required,
        efficiencies: crate::walk::Buffer::Fixed(&mut *out.efficiencies),
        efficiency_len: out.efficiency_len,
        efficiency_required: out.efficiency_required,
        requests: crate::walk::Buffer::Fixed(&mut *out.requests),
        request_len: out.request_len,
        request_required: out.request_required,
        archetypes: crate::walk::Buffer::Fixed(&mut *out.archetypes),
        archetype_required: out.archetype_required,
    };
    let result = capture_buffer(memory, base, state, 1..u32::MAX, None, context, &mut buffer);
    out.country_len = buffer.country_len;
    out.country_required = buffer.country_required;
    out.line_len = buffer.line_len;
    out.line_required = buffer.line_required;
    out.efficiency_len = buffer.efficiency_len;
    out.efficiency_required = buffer.efficiency_required;
    out.request_len = buffer.request_len;
    out.request_required = buffer.request_required;
    out.archetype_required = buffer.archetype_required;
    result
}

pub fn capture_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    context: &mut Option<Context>,
    out: &mut GrowingBuffers<'_>,
) -> Result<(), &'static str> {
    capture_growing_slots(memory, base, state, 1..u32::MAX, None, context, out)
}

/// [`capture_growing`] over the census slots `slots` only, one part of a
/// capture split by consecutive slot ranges: each part's rows are the whole
/// walk's for its slots, in order, and its first failure is the one the whole
/// walk meets in them. `provided`, when given, is the define context read once
/// for every part, used at a part's first country with lines in place of
/// reading it there (the whole walk reads it once, at the first such country).
pub fn capture_growing_slots(
    memory: &impl Memory,
    base: usize,
    state: usize,
    slots: std::ops::Range<u32>,
    provided: Option<Result<Context, &'static str>>,
    context: &mut Option<Context>,
    out: &mut GrowingBuffers<'_>,
) -> Result<(), &'static str> {
    let mut buffer = BuffersBuffer {
        countries: crate::walk::Buffer::Growing(&mut *out.countries),
        country_len: out.country_len,
        country_required: out.country_required,
        lines: crate::walk::Buffer::Growing(&mut *out.lines),
        line_len: out.line_len,
        line_required: out.line_required,
        efficiencies: crate::walk::Buffer::Growing(&mut *out.efficiencies),
        efficiency_len: out.efficiency_len,
        efficiency_required: out.efficiency_required,
        requests: crate::walk::Buffer::Growing(&mut *out.requests),
        request_len: out.request_len,
        request_required: out.request_required,
        archetypes: crate::walk::Buffer::Growing(&mut *out.archetypes),
        archetype_required: out.archetype_required,
    };
    let result = capture_buffer(memory, base, state, slots, provided, context, &mut buffer);
    out.country_len = buffer.country_len;
    out.country_required = buffer.country_required;
    out.line_len = buffer.line_len;
    out.line_required = buffer.line_required;
    out.efficiency_len = buffer.efficiency_len;
    out.efficiency_required = buffer.efficiency_required;
    out.request_len = buffer.request_len;
    out.request_required = buffer.request_required;
    out.archetype_required = buffer.archetype_required;
    result
}

fn capture_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    slots: std::ops::Range<u32>,
    provided: Option<Result<Context, &'static str>>,
    context: &mut Option<Context>,
    out: &mut BuffersBuffer<'_>,
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    for slot in slots.start.max(1)..slots.end.min(countries.count()) {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let status = cproduction_status::manager(memory, base, country)?;
        let roster = Array::read(memory, at(status, cproduction_status::LINES)?, 8)
            .ok_or("production modifier line roster unreadable or invalid")?;
        // Idle countries need no hover trees or factory operands.
        if roster.count == 0 {
            continue;
        }
        if context.is_none() {
            *context = Some(match provided {
                Some(read) => read?,
                None => Context::read(memory, base)?,
            });
        }
        let context = context.as_ref().expect("nonempty production context");
        let accumulator = at(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?;
        add(&mut out.country_required, 1)?;
        out.countries.ensure(out.country_required)?;
        if let Some(row) = out.countries.get_mut(out.country_len) {
            *row = Country {
                country: country.id,
                accumulator,
                power_raw: field(memory, status, 0x3c0)?,
                slot,
            };
            out.country_len += 1;
        }
        for key in KEYS {
            add(&mut out.request_required, 1)?;
            out.requests.ensure(out.request_required)?;
            if let Some(request) = out.requests.get_mut(out.request_len) {
                *request = Request { accumulator, key };
                out.request_len += 1;
            }
        }
        add(&mut out.line_required, roster.count as usize)?;
        out.lines.ensure(out.line_required)?;
        for ordinal in 0..roster.count {
            let pointer = roster
                .element(ordinal, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p != 0)
                .ok_or("production modifier line pointer unreadable or null")?;
            let identity = field::<[u8; 16]>(memory, pointer, 0)?;
            let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
            let address =
                usize::try_from(vtable).map_err(|_| "production modifier line vtable mismatch")?;
            use crate::{
                cmilitary_production_line as military, cnaval_production_line as naval_line,
                crailway_gun_production_line as railway_gun, cship_refit_production_line as refit,
            };
            let candidates = [
                military::VTABLE_RVA,
                naval_line::VTABLE_RVA,
                refit::VTABLE_RVA,
                railway_gun::VTABLE_RVA,
            ];
            let kind = match engine_sites::which(base, address, &candidates) {
                Some(military::VTABLE_RVA) => line::KIND_MILITARY,
                Some(naval_line::VTABLE_RVA) => line::KIND_NAVAL,
                Some(refit::VTABLE_RVA) => line::KIND_SHIP_REFIT,
                Some(railway_gun::VTABLE_RVA) => line::KIND_RAILWAY_GUN,
                _ => return Err("production modifier line vtable mismatch"),
            };
            let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
            if handle == 0
                || field::<u64>(memory, pointer, cproduction_line::PRODUCTION_STATUS)?
                    != status as u64
            {
                return Err("production modifier line identity or owner mismatch");
            }
            let factory = Array::read(memory, at(pointer, line::FACTORY_EFFICIENCIES)?, 8)
                .ok_or("production modifier factory efficiencies unreadable or invalid")?;
            let start = out.efficiency_len;
            add(&mut out.efficiency_required, factory.count as usize)?;
            out.efficiencies.ensure(out.efficiency_required)?;
            if factory.count as usize > out.efficiencies.len().saturating_sub(start) {
                continue;
            }
            // A fixed stack batch bounds read size, never the live count.
            for index in (0..factory.count as usize).step_by(256) {
                let count = (factory.count as usize - index).min(256);
                let address = factory
                    .element(index as u32, 8)
                    .ok_or("production modifier factory efficiency address overflow")?;
                let mut bytes = [0_u8; 256 * 8];
                if !crate::read_slice(memory, address, &mut bytes[..count * 8]) {
                    return Err("production modifier factory efficiency unreadable");
                }
                for (offset, bytes) in bytes[..count * 8].chunks_exact(8).enumerate() {
                    out.efficiencies[start + index + offset] =
                        i64::from_le_bytes(bytes.try_into().unwrap());
                }
            }
            out.efficiency_len += factory.count as usize;
            if out.line_len >= out.lines.len() {
                continue;
            }
            let bonus = Array::read(memory, at(pointer, 0xa0)?, 16)
                .ok_or("production modifier organisation cache unreadable or invalid")?;
            let mut bonuses = [0_i64; BONUS_COUNT];
            // Production bonus selection uses the first equal key.
            let mut found = [false; BONUS_COUNT];
            for index in 0..bonus.count {
                let entry = bonus
                    .element(index, 16)
                    .and_then(|p| read::<[u8; 16]>(memory, p))
                    .ok_or("production modifier organisation bonus unreadable")?;
                let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
                for category in 0..BONUS_COUNT {
                    if !found[category] && context.bonus_keys[category] == key {
                        bonuses[category] = i64::from_le_bytes(entry[8..].try_into().unwrap());
                        found[category] = true;
                    }
                }
                if found.iter().all(|found| *found) {
                    break;
                }
            }
            let variant_pointer = field::<u64>(memory, pointer, line::EQUIPMENT_VARIANT)?;
            let variant_pointer = usize::try_from(variant_pointer)
                .map_err(|_| "production modifier variant pointer overflow")?;
            let variant = cequipment_variant::identity(memory, base, variant_pointer)?;
            let definition = field::<u64>(memory, variant_pointer, cequipment_variant::DEFINITION)?;
            let definition = usize::try_from(definition)
                .map_err(|_| "production modifier equipment type pointer overflow")?;
            let naval_efficiency = naval(memory, base, definition, out)?;
            let naval_output = field::<u64>(
                memory,
                variant_pointer,
                cequipment_variant::CLASSIFIER_FLAGS,
            )? & crate::cequipment_type::SHIP_MASK
                != 0;
            let conversion = field::<u8>(memory, pointer, line::CONVERSION)? != 0;
            let licensed =
                field::<i32>(memory, variant_pointer, cequipment_variant::ORIGIN_LINK)? > 0;
            // Production-speed operands include the conversion branch.
            let mut conversion_output = conversion && conversion_target(memory, variant_pointer)?;
            // Enabled module slots gate the conversion bonus.
            if conversion_output
                && field::<i32>(memory, definition, crate::cequipment_type::SLOTS_COUNT)? != 0
            {
                let mut primary = field::<u64>(
                    memory,
                    definition,
                    crate::cequipment_type::PRIMARY_CATEGORY_FLAGS,
                )?;
                if primary == 0 {
                    let parent =
                        field::<u64>(memory, definition, crate::cequipment_type::ARCHETYPE)?
                            as usize;
                    primary = field(
                        memory,
                        parent,
                        crate::cequipment_type::PRIMARY_CATEGORY_FLAGS,
                    )?;
                }
                conversion_output = normalized(
                    primary
                        | field::<u64>(
                            memory,
                            definition,
                            crate::cequipment_type::SECONDARY_CATEGORY_FLAGS,
                        )?,
                ) & context.module_mask
                    == 0;
            }
            // The four supported production forms have constant eligibility predicates.
            // Refit selects true; the others select zero. Refuse a different implementation.
            let implementation = usize::try_from(field::<u64>(memory, vtable as usize, 0xd8)?)
                .map_err(|_| "unreached production refit virtual implementation")?;
            let refit_output = match engine_sites::which(
                base,
                implementation,
                &[Rva(0x0018_8d90), Rva(0x0011_d530)],
            ) {
                Some(Rva(0x0018_8d90)) => true,
                Some(Rva(0x0011_d530)) => false,
                _ => return Err("unreached production refit virtual implementation"),
            };
            out.lines[out.line_len] = Line {
                handle,
                country: country.id,
                kind,
                variant,
                organisation: field(memory, pointer, line::INDUSTRIAL_ORGANISATION_CID)?,
                active_factories: field(memory, pointer, cproduction_line::ACTIVE_FACTORIES)?,
                resource_penalty: field(memory, pointer, 0xd8)?,
                organisation_dirty: field::<u8>(memory, pointer, 0xb8)? != 0,
                conversion,
                licensed,
                conversion_output,
                refit_output,
                naval_efficiency,
                naval_output,
                bonuses,
                efficiencies: [start, factory.count as usize],
            };
            out.line_len += 1;
        }
    }
    if out.country_len != out.country_required
        || out.line_len != out.line_required
        || out.request_len != out.request_required
        || out.efficiency_len != out.efficiency_required
    {
        Err("production modifier caller arena capacity exceeded")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
