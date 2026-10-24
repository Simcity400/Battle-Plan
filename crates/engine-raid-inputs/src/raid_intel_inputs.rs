//! Target-intel operands. The game computes national counterintelligence on
//! demand, using the country modifier and its strategic-operative mission
//! roster. The collector copies those stored operands, not the rating or the
//! agency's differently transformed cache. Mission changes and modifier
//! recalculation can occur between daily ticks. Each country's modifier
//! accumulator is copied once, when its first key is needed, and both keys are
//! searched in the copy ([`Accumulator`]). A caller may carry both keys' raw
//! answers between hours in place of that copy while it certifies the country's
//! accumulator unchanged ([`Carry`]); the header read before it and the
//! effective values (from the current modifier definitions) are read as without
//! a carry.

use crate::{
    Memory, ccountry, ccryptology, cgame_state, cintelligence_agency as agency,
    coperative_leader as operative, cpdx_array::Array, cstrategic_operative as strategic,
    cstrategic_operative_manager as manager, modifier_accumulator, modifier_keys,
    operative_action as action, read, walk::Outcome,
};

/// Counterintelligence reference roster, distinct from the agency active roster.
const MISSIONS: usize = 0x28;
/// Counterintelligence modifier object, with its embedded accumulator.
const OPERATIVE_MODIFIER: usize = 0x408;
/// Bound static modifier selected when the subject occurs in sorted operative nationalities.
const STATIC_DATABASE_RVA: usize = 0x0333_0098;
const NATIONALITY_MODIFIER: usize = 0x130;
/// Counterintelligence mission country-tag virtual dispatch.
const MISSION_VTABLE_RVA: usize = 0x02a1_fca0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub strategic_present: bool,
    pub country_defense: i64,
    pub crypto_strength: Option<i64>,
    pub operatives_start: usize,
    pub operatives_len: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operative {
    pub handle: u64,
    pub defense: i64,
    pub context: i64,
}

pub struct Walk {
    pub outcome: Outcome,
    pub operatives: usize,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("raid intel address overflow")
}
fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("raid intel field unreadable")
}
fn pointer(memory: &impl Memory, address: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, address, offset)?)
        .map_err(|_| "raid intel pointer overflow")
}
fn array(
    memory: &impl Memory,
    address: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(address, offset)?, stride).ok_or("raid intel array invalid")
}
fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: usize,
) -> Result<(), &'static str> {
    if object == 0 || field::<u64>(memory, object, 0)? != at(base, vtable)? as u64 {
        return Err("raid intel object vtable mismatch");
    }
    Ok(())
}
fn modifier(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
) -> Result<i64, &'static str> {
    modifier_accumulator::lookup_effective_at(memory, base, accumulator, key).map(|v| v.effective)
}

/// The country keys a walk looks up, by [`Carry`] answer index.
pub const CRYPTO: usize = 0;
pub const DEFENSE: usize = 1;
pub const KEYS: [u32; 2] = [
    modifier_keys::CRYPTO_STRENGTH,
    modifier_keys::INTELLIGENCE_AGENCY_DEFENSE,
];

/// A key's raw answer in a country's accumulator: `None` when the key is
/// absent (effective zero), else its stored operand.
pub type RawAnswer = Option<i64>;

/// Raw accumulator answers a caller carries between walks, per country.
/// Asked once per country, at its first key lookup, right after the
/// accumulator header read the lookup begins with and in place of the
/// entries copy that follows it: `held` returns both keys' answers when the
/// caller certifies the country's accumulator unchanged since it saw this
/// header. Otherwise the entries are copied and searched as without a carry,
/// and `read` receives both keys' answers in that copy (when both searches
/// succeed). Every effective value is derived from the current modifier
/// definition either way.
pub trait Carry {
    fn held(&mut self, country: &ccountry::Country, header: &[u8; 16]) -> Option<[RawAnswer; 2]>;
    fn read(&mut self, country: &ccountry::Country, header: &[u8; 16], answers: [RawAnswer; 2]);
}

/// Carries nothing: every accumulator is copied.
pub struct NoCarry;
impl Carry for NoCarry {
    fn held(&mut self, _: &ccountry::Country, _: &[u8; 16]) -> Option<[RawAnswer; 2]> {
        None
    }
    fn read(&mut self, _: &ccountry::Country, _: &[u8; 16], _: [RawAnswer; 2]) {}
}

/// Country accumulator entries one guarded copy takes; a longer accumulator
/// is searched by [`modifier`].
const ACCUMULATOR_ENTRIES: usize = 1024;
const STRIDE: usize = crate::modifier_entry::STRIDE;

/// What a country's first lookup found; every later lookup reuses it.
#[derive(Clone, Copy)]
enum Loaded {
    /// The header or entries refused; every lookup returns the refusal.
    Refused(&'static str),
    /// Longer than the buffer: each key is searched in place.
    Oversize,
    /// The [`Carry`] answers.
    Held([RawAnswer; 2]),
    /// The entries' byte length, copied into the buffer, and both keys'
    /// answers in them (`None` when a search refused; each lookup then
    /// searches again and returns its refusal).
    Copied(usize, Option<[RawAnswer; 2]>),
}

/// One country's modifier accumulator (`CCountry+MODIFIER_ACCUMULATOR`), copied
/// whole on its first lookup and searched locally after, with the value,
/// failure and failure order of `modifier_accumulator::lookup_effective_at`.
struct Accumulator<'b> {
    country: ccountry::Country,
    entries: &'b mut [u8; ACCUMULATOR_ENTRIES * STRIDE],
    loaded: Option<Loaded>,
}

impl<'b> Accumulator<'b> {
    fn new(
        country: ccountry::Country,
        entries: &'b mut [u8; ACCUMULATOR_ENTRIES * STRIDE],
    ) -> Self {
        Self {
            country,
            entries,
            loaded: None,
        }
    }

    fn address(&self) -> Result<usize, &'static str> {
        at(self.country.pointer, ccountry::MODIFIER_ACCUMULATOR)
    }

    /// The header read, then the carry's answers or the entries copy.
    fn load(
        &mut self,
        memory: &impl Memory,
        carry: &mut impl Carry,
    ) -> Result<Loaded, &'static str> {
        // One refusal for an unreadable or invalid header, as `copy` had.
        const HEADER: &str = "modifier accumulator header unreadable or invalid";
        let header = read::<[u8; 16]>(memory, self.address()?).ok_or(HEADER)?;
        let array = Array::from_header(header, STRIDE).ok_or(HEADER)?;
        let len = array.count as usize * STRIDE;
        if len > self.entries.len() {
            return Ok(Loaded::Oversize);
        }
        if let Some(answers) = carry.held(&self.country, &header) {
            return Ok(Loaded::Held(answers));
        }
        let entries = &mut self.entries[..len];
        if !entries.is_empty() && !memory.copy(array.data, entries) {
            return Err("modifier accumulator entry unreadable");
        }
        modifier_accumulator::validate_snapshot(entries)?;
        let entries = &*entries;
        let answer = |key| {
            modifier_accumulator::lookup_snapshot(entries, key)
                .map(|value| value.present.then_some(value.raw))
        };
        let answers = match (answer(KEYS[CRYPTO]), answer(KEYS[DEFENSE])) {
            (Ok(crypto), Ok(defense)) => Some([crypto, defense]),
            _ => None,
        };
        if let Some(answers) = answers {
            carry.read(&self.country, &header, answers);
        }
        Ok(Loaded::Copied(len, answers))
    }

    /// The effective value of `KEYS[key]`.
    fn effective(
        &mut self,
        memory: &impl Memory,
        base: usize,
        key: usize,
        carry: &mut impl Carry,
    ) -> Result<i64, &'static str> {
        let loaded = match self.loaded {
            Some(loaded) => loaded,
            None => {
                let loaded = self.load(memory, carry).unwrap_or_else(Loaded::Refused);
                self.loaded = Some(loaded);
                loaded
            }
        };
        let raw = match loaded {
            Loaded::Refused(reason) => return Err(reason),
            Loaded::Oversize => return modifier(memory, base, self.address()?, KEYS[key]),
            Loaded::Held(answers) | Loaded::Copied(_, Some(answers)) => answers[key],
            Loaded::Copied(len, None) => {
                let value = modifier_accumulator::lookup_snapshot(&self.entries[..len], KEYS[key])?;
                value.present.then_some(value.raw)
            }
        };
        match raw {
            None => Ok(0),
            Some(raw) => crate::modifier_definition::effective(memory, base, KEYS[key], raw),
        }
    }
}

/// Exact operative-roster lower-bound, including aliases and the native zero
/// tag.
fn has_nationality(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    object: usize,
    raw: u32,
) -> Result<bool, &'static str> {
    let values = array(memory, object, operative::NATIONALITIES, 4)?;
    let canonical = |id| {
        if id == 0 {
            Ok(0)
        } else {
            countries.resolve(id).map(|c| c.canonical_slot)
        }
    };
    let target = canonical(raw)?;
    let mut first = 0;
    let mut count = values.count;
    while count != 0 {
        let half = count / 2;
        let id = field::<u32>(
            memory,
            values
                .element(first + half, 4)
                .ok_or("raid nationality index overflow")?,
            0,
        )?;
        if id != raw && canonical(id)? < target {
            first += half + count % 2;
        }
        count = half;
    }
    if first == values.count {
        return Ok(false);
    }
    let id = field::<u32>(
        memory,
        values
            .element(first, 4)
            .ok_or("raid nationality index overflow")?,
        0,
    )?;
    Ok(id == raw || canonical(id)? <= target)
}

fn contribution(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: u32,
    handle: u64,
) -> Result<Option<Operative>, &'static str> {
    let Some(object) = crate::creference_registry::resolve(memory, base, handle)? else {
        return Ok(None);
    };
    authenticate(memory, base, object, operative::VTABLE_RVA)?;
    if field::<u64>(memory, object, operative::IDENTITY)? != handle {
        return Err("raid counteroperative identity mismatch");
    }
    let mission = at(object, operative::ACTION)?;
    if field::<u32>(memory, mission, action::VARIANT)? != action::COUNTER_INTELLIGENCE {
        return Ok(None);
    }
    let implementation = pointer(memory, mission, action::IMPLEMENTATION)?;
    authenticate(memory, base, implementation, MISSION_VTABLE_RVA)?;
    let target = field::<u32>(
        memory,
        implementation,
        crate::operative_action_implementation::TARGET_COUNTRY,
    )?;
    if target != country
        && (target == 0
            || countries.resolve(target)?.canonical_slot
                != countries.resolve(country)?.canonical_slot)
    {
        return Ok(None);
    }
    let mut context = 0;
    if has_nationality(memory, countries, object, country)? {
        let database = pointer(memory, base, STATIC_DATABASE_RVA)?;
        if database != 0 {
            let bound = pointer(memory, database, NATIONALITY_MODIFIER)?;
            if bound != 0 {
                authenticate(memory, base, bound, crate::cstatic_modifier::VTABLE_RVA)?;
                context = modifier(
                    memory,
                    base,
                    at(bound, crate::cmodifier::ACCUMULATOR)?,
                    modifier_keys::INTELLIGENCE_AGENCY_DEFENSE,
                )?;
            }
        }
    }
    Ok(Some(Operative {
        handle,
        context,
        defense: modifier(
            memory,
            base,
            at(object, OPERATIVE_MODIFIER + crate::cmodifier::ACCUMULATOR)?,
            modifier_keys::INTELLIGENCE_AGENCY_DEFENSE,
        )?,
    }))
}

/// Off-thread sizing hint: all mission-roster occurrences, before the native
/// counterintelligence filter. A later growth is reported by `walk`.
pub fn required_operatives(
    memory: &impl Memory,
    base: usize,
    game: usize,
) -> Result<usize, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game)?;
    let manager = pointer(memory, game, cgame_state::STRATEGIC_OPERATIVE_MANAGER)?;
    if manager == 0 {
        return Ok(0);
    }
    let roster = array(memory, manager, manager::OPERATIVES_DATA, 8)?;
    let mut count = 0usize;
    for slot in 1..countries.count() {
        if countries.at(slot)?.is_none() {
            continue;
        }
        let strategic = pointer(
            memory,
            roster
                .element(slot, 8)
                .ok_or("raid strategic country slot absent")?,
            0,
        )?;
        if strategic == 0 {
            continue;
        }
        authenticate(memory, base, strategic, strategic::VTABLE_RVA)?;
        count = count
            .checked_add(array(memory, strategic, MISSIONS, 8)?.count as usize)
            .ok_or("raid mission count overflow")?;
    }
    Ok(count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: &mut [Row],
    operatives: &mut [Operative],
) -> Walk {
    walk_buffer(
        memory,
        base,
        game,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(operatives),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: &mut Vec<Row>,
    operatives: &mut Vec<Operative>,
    carry: &mut impl Carry,
) -> Walk {
    walk_carrying(
        memory,
        base,
        game,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(operatives),
        carry,
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: crate::walk::Buffer<'_, Row>,
    operatives: crate::walk::Buffer<'_, Operative>,
) -> Walk {
    walk_carrying(memory, base, game, output, operatives, &mut NoCarry)
}

fn walk_carrying(
    memory: &impl Memory,
    base: usize,
    game: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut operatives: crate::walk::Buffer<'_, Operative>,
    carry: &mut impl Carry,
) -> Walk {
    let mut operative_len = 0usize;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game)?;
        let manager = pointer(memory, game, cgame_state::STRATEGIC_OPERATIVE_MANAGER)?;
        let strategic_roster = if manager == 0 {
            None
        } else {
            Some(array(memory, manager, manager::OPERATIVES_DATA, 8)?)
        };
        let mut len = 0usize;
        let mut entries = [0_u8; ACCUMULATOR_ENTRIES * STRIDE];
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let mut modifiers = Accumulator::new(country, &mut entries);
            let mut row = Row {
                country: country.id,
                operatives_start: operative_len,
                ..Default::default()
            };
            if let Some(agency) = agency::root(memory, base, country)? {
                let crypto = pointer(memory, agency, agency::CRYPTOLOGY)?;
                if crypto != 0 {
                    authenticate(memory, base, crypto, ccryptology::VTABLE_RVA)?;
                    if pointer(memory, crypto, ccryptology::OWNER_COUNTRY)? != country.pointer {
                        return Err("raid cryptology owner mismatch");
                    }
                    row.crypto_strength = Some(modifiers.effective(memory, base, CRYPTO, carry)?);
                }
            }
            if let Some(roster) = strategic_roster {
                let strategic = pointer(
                    memory,
                    roster
                        .element(slot, 8)
                        .ok_or("raid strategic country slot absent")?,
                    0,
                )?;
                if strategic != 0 {
                    authenticate(memory, base, strategic, strategic::VTABLE_RVA)?;
                    let owner = field::<u32>(memory, strategic, strategic::OWNER)?;
                    if countries.resolve(owner)?.pointer != country.pointer {
                        return Err("raid strategic owner mismatch");
                    }
                    row.strategic_present = true;
                    row.country_defense = modifiers.effective(memory, base, DEFENSE, carry)?;
                    let missions = array(memory, strategic, MISSIONS, 8)?;
                    operatives.ensure(
                        operative_len
                            .checked_add(missions.count as usize)
                            .ok_or("raid operative row count overflow")?,
                    )?;
                    for ordinal in 0..missions.count {
                        let handle = field::<u64>(
                            memory,
                            missions
                                .element(ordinal, 8)
                                .ok_or("raid mission address overflow")?,
                            0,
                        )?;
                        if let Some(value) =
                            contribution(memory, base, &countries, country.id, handle)?
                        {
                            if let Some(out) = operatives.get_mut(operative_len) {
                                *out = value;
                            }
                            operative_len = operative_len
                                .checked_add(1)
                                .ok_or("raid operative count overflow")?;
                        }
                    }
                    row.operatives_len = operative_len - row.operatives_start;
                }
            }
            output.ensure(
                len.checked_add(1)
                    .ok_or("raid intel country count overflow")?,
            )?;
            if let Some(out) = output.get_mut(len) {
                *out = row;
            }
            len = len
                .checked_add(1)
                .ok_or("raid intel country count overflow")?;
        }
        if operative_len > operatives.len() {
            return Err("raid counteroperative arena too small");
        }
        if len > output.len() {
            Ok(Outcome::Truncated {
                len: output.len(),
                unread_rows: u32::try_from(len - output.len())
                    .map_err(|_| "raid intel country count overflow")?,
            })
        } else {
            Ok(Outcome::Rows { len })
        }
    })();
    Walk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        operatives: operative_len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        creference_registry as registry, modifier_definition as definition,
        reference_bucket as bucket,
    };
    use std::collections::BTreeMap;

    const GAME: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const DEFENSE: u32 = modifier_keys::INTELLIGENCE_AGENCY_DEFENSE;
    const CRYPTO: u32 = modifier_keys::CRYPTO_STRENGTH;
    const HANDLE: u64 = (91u64 << 32) | 7;
    struct Fixture {
        regions: BTreeMap<usize, Vec<u8>>,
        country: usize,
        strategic: usize,
        operative: usize,
        mission: usize,
    }
    impl Memory for Fixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let Some((&base, bytes)) = self.regions.range(..=at).next_back() else {
                return false;
            };
            let Some(source) = at
                .checked_sub(base)
                .and_then(|offset| bytes.get(offset..offset.checked_add(out.len())?))
            else {
                return false;
            };
            out.copy_from_slice(source);
            true
        }
    }
    impl Fixture {
        fn block(&mut self, at: usize, len: usize) {
            assert!(self.regions.insert(at, vec![0; len]).is_none());
        }
        fn put(&mut self, at: usize, bytes: &[u8]) {
            let (&base, target) = self.regions.range_mut(..=at).next_back().unwrap();
            target[at - base..at - base + bytes.len()].copy_from_slice(bytes);
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: usize) {
            self.u64(at, data as u64);
            self.u32(at + 8, count as u32);
            self.u32(at + 12, count as u32);
        }
        fn accumulator(&mut self, at: usize, entries: &[(u32, i64)]) {
            let data = at + 0x20;
            self.array(at, data, entries.len());
            for (i, &(key, value)) in entries.iter().enumerate() {
                self.u32(data + i * 16, key);
                self.u64(data + i * 16 + 8, value as u64);
            }
        }
        fn new(count: usize) -> Self {
            let slots = count + 1;
            let roster = 0x3000;
            let links = roster + slots * 8;
            let strategic_roster = links + (slots + 1) * 4;
            let objects = strategic_roster + slots * 8;
            let end = objects + count * 0x1400;
            let mut f = Self {
                regions: BTreeMap::new(),
                country: objects,
                strategic: objects + 0x1000,
                operative: end + 0x2000,
                mission: end + 0x4000,
            };
            f.block(GAME, end + 0x5000 - GAME);
            f.array(GAME + cgame_state::COUNTRIES, roster, slots);
            f.array(GAME + cgame_state::COUNTRY_LINKS, links, slots + 1);
            f.u64(
                GAME + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
                MANAGER as u64,
            );
            f.array(MANAGER + manager::OPERATIVES_DATA, strategic_roster, slots);
            for slot in 1..slots {
                let country = objects + (slot - 1) * 0x1400;
                let strategic = country + 0x1000;
                f.u64(roster + slot * 8, country as u64);
                f.u32(links + slot * 4, slot as u32);
                f.u64(country, ccountry::VTABLE_RVA as u64);
                f.u32(country + ccountry::ID, slot as u32);
                f.u64(strategic_roster + slot * 8, strategic as u64);
                f.u64(strategic, strategic::VTABLE_RVA as u64);
                f.u32(strategic + strategic::OWNER, slot as u32);
                f.array(strategic + MISSIONS, 0, 0);
            }
            // Raw alias for country 1; native nationalities and mission target may use it.
            f.u32(links + slots * 4, 1);
            let agency = f.country + 0x1100;
            let crypto = f.country + 0x1280;
            f.u64(f.country + ccountry::INTELLIGENCE_AGENCY, agency as u64);
            f.u64(agency, agency::VTABLE_RVA as u64);
            f.u64(agency + agency::OWNER_COUNTRY, f.country as u64);
            f.u64(agency + agency::CRYPTOLOGY, crypto as u64);
            f.u64(crypto, ccryptology::VTABLE_RVA as u64);
            f.u64(crypto + ccryptology::OWNER_COUNTRY, f.country as u64);
            f.accumulator(
                f.country + ccountry::MODIFIER_ACCUMULATOR,
                &[(DEFENSE, -125000), (CRYPTO, 249999)],
            );
            f.block(definition::TABLE_RVA, 8);
            f.u64(definition::TABLE_RVA, 0x0800_0000);
            f.block(0x0800_0000, (CRYPTO as usize + 1) * definition::STRIDE);
            // Unclamped, ungated synthetic modifier definitions.
            f.block(registry::LOW_ROOTS_RVA + 7 * 8, 8);
            f.u64(registry::LOW_ROOTS_RVA + 7 * 8, (end + 0x100) as u64);
            f.u64(end + 0x100 + registry::ENTRIES, (end + 0x200) as u64);
            f.u32(end + 0x100 + registry::COUNT, 1);
            f.u32(end + 0x100 + registry::MASK, 7);
            f.put(end + 0x100 + registry::OVERFLOW_DISTANCE, &[5]);
            f.put(end + 0x200 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
            let hash = HANDLE
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
            let cell = end + 0x200 + index * bucket::STRIDE;
            f.put(cell + bucket::DISTANCE, &[1]);
            f.u64(cell + 8, HANDLE);
            f.u64(cell + 16, f.operative as u64);
            f.u64(f.operative, operative::VTABLE_RVA as u64);
            f.u64(f.operative + operative::IDENTITY, HANDLE);
            f.u32(
                f.operative + operative::ACTION + action::VARIANT,
                action::COUNTER_INTELLIGENCE,
            );
            f.u64(
                f.operative + operative::ACTION + action::IMPLEMENTATION,
                f.mission as u64,
            );
            f.u64(f.mission, MISSION_VTABLE_RVA as u64);
            f.u32(
                f.mission + crate::operative_action_implementation::TARGET_COUNTRY,
                slots as u32,
            );
            f.accumulator(
                f.operative + OPERATIVE_MODIFIER + crate::cmodifier::ACCUMULATOR,
                &[(DEFENSE, 45678)],
            );
            f.array(f.operative + operative::NATIONALITIES, end + 0x4800, 1);
            f.u32(end + 0x4800, slots as u32);
            f.block(STATIC_DATABASE_RVA, 8);
            f.u64(STATIC_DATABASE_RVA, (end + 0x600) as u64);
            f.u64(end + 0x600 + NATIONALITY_MODIFIER, (end + 0x800) as u64);
            f.u64(end + 0x800, crate::cstatic_modifier::VTABLE_RVA as u64);
            f.accumulator(
                end + 0x800 + crate::cmodifier::ACCUMULATOR,
                &[(DEFENSE, -33333)],
            );
            f.array(f.strategic + MISSIONS, end + 0x1000, 3);
            f.u64(end + 0x1000, HANDLE);
            f.u64(end + 0x1008, 0);
            f.u64(end + 0x1010, HANDLE);
            f
        }
        fn run(&self, rows: &mut [Row], operatives: &mut [Operative]) -> Walk {
            walk(self, 0, GAME, rows, operatives)
        }
    }

    #[test]
    fn exact_operands_aliases_and_duplicates() {
        let f = Fixture::new(3);
        let mut rows = vec![Row::default(); 3];
        let mut operatives = [Operative::default(); 2];
        assert_eq!(required_operatives(&f, 0, GAME), Ok(3));
        assert_eq!(
            f.run(&mut rows, &mut operatives).outcome,
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            rows[0],
            Row {
                country: 1,
                strategic_present: true,
                country_defense: -125000,
                crypto_strength: Some(249999),
                operatives_start: 0,
                operatives_len: 2
            }
        );
        assert_eq!(
            operatives,
            [Operative {
                handle: HANDLE,
                defense: 45678,
                context: -33333
            }; 2]
        );
        assert_eq!(rows[2].country, 3);
        assert_eq!(
            f.run(&mut rows[..1], &mut operatives).outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        let short = f.run(&mut rows, &mut operatives[..1]);
        assert_eq!(short.operatives, 2);
        assert_eq!(
            short.outcome,
            Outcome::Unavailable {
                reason: "raid counteroperative arena too small"
            }
        );
    }

    /// Both country keys come from one copy of the country's accumulator,
    /// with the getter's values; entries out of key order still fail as the
    /// getter's own read does.
    #[test]
    fn both_country_keys_share_one_accumulator_copy() {
        struct Counted<'a>(&'a Fixture, std::cell::Cell<usize>, usize);
        impl Memory for Counted<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at == self.2 {
                    self.1.set(self.1.get() + 1);
                }
                self.0.copy(at, out)
            }
        }
        let mut f = Fixture::new(1);
        let entries = f.country + ccountry::MODIFIER_ACCUMULATOR + 0x20;
        let counted = Counted(&f, std::cell::Cell::new(0), entries);
        let mut rows = [Row::default()];
        let walk = walk(&counted, 0, GAME, &mut rows, &mut [Operative::default(); 2]);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (rows[0].country_defense, rows[0].crypto_strength),
            (-125000, Some(249999))
        );
        assert_eq!(counted.1.get(), 1);
        f.accumulator(
            f.country + ccountry::MODIFIER_ACCUMULATOR,
            &[(CRYPTO, 249999), (DEFENSE, -125000)],
        );
        assert_eq!(
            f.run(&mut rows, &mut [Operative::default(); 2]).outcome,
            Outcome::Unavailable {
                reason: "modifier accumulator keys not positive and increasing"
            }
        );
    }

    /// A carry holding a country's answers replaces only its entries copy:
    /// the header is still read first, and the effective values still come
    /// from the modifier definitions. A copy hands the carry both keys'
    /// answers with the header it read; a header the carry does not hold is
    /// copied again.
    #[test]
    fn a_held_country_skips_only_its_entries_copy() {
        struct Counted<'a> {
            fixture: &'a Fixture,
            watched: [usize; 2],
            reads: std::cell::RefCell<Vec<usize>>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if self.watched.contains(&at) {
                    self.reads.borrow_mut().push(at);
                }
                self.fixture.copy(at, out)
            }
        }
        #[derive(Default)]
        struct Holding {
            held: Option<([u8; 16], [RawAnswer; 2])>,
            asked: Vec<u32>,
            read: Vec<([u8; 16], [RawAnswer; 2])>,
        }
        impl Carry for Holding {
            fn held(
                &mut self,
                country: &ccountry::Country,
                header: &[u8; 16],
            ) -> Option<[RawAnswer; 2]> {
                self.asked.push(country.id);
                self.held
                    .filter(|(held, _)| held == header)
                    .map(|(_, answers)| answers)
            }
            fn read(&mut self, _: &ccountry::Country, header: &[u8; 16], answers: [RawAnswer; 2]) {
                self.read.push((*header, answers));
            }
        }
        let f = Fixture::new(1);
        let header = f.country + ccountry::MODIFIER_ACCUMULATOR;
        let entries = header + 0x20;
        let counted = Counted {
            fixture: &f,
            watched: [header, entries],
            reads: Default::default(),
        };
        let mut carry = Holding::default();
        let (mut rows, mut operatives) = (Vec::new(), Vec::new());
        let mut hour = |carry: &mut Holding| {
            counted.reads.borrow_mut().clear();
            let walk = walk_growing(&counted, 0, GAME, &mut rows, &mut operatives, carry);
            assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
            (
                rows[0].crypto_strength,
                rows[0].country_defense,
                counted.reads.borrow().clone(),
            )
        };
        assert_eq!(
            hour(&mut carry),
            (Some(249999), -125000, vec![header, entries])
        );
        assert_eq!(carry.asked, [1], "asked once per country");
        assert_eq!(carry.read.len(), 1);
        let (seen, answers) = carry.read[0];
        assert_eq!(answers, [Some(249999), Some(-125000)]);
        // Held answers stand in for the copy, through the definitions.
        carry.held = Some((seen, [Some(7), None]));
        assert_eq!(hour(&mut carry), (Some(7), 0, vec![header]));
        assert_eq!(carry.read.len(), 1, "nothing read, nothing handed back");
        // A header it does not hold: copied and handed back again.
        carry.held = Some(([0; 16], [Some(7), None]));
        assert_eq!(
            hour(&mut carry),
            (Some(249999), -125000, vec![header, entries])
        );
        assert_eq!(carry.read.len(), 2);
    }

    #[test]
    fn mission_filter_does_not_read_an_inapplicable_implementation() {
        let mut f = Fixture::new(1);
        let mut rows = [Row::default()];
        f.u32(
            f.operative + operative::ACTION + action::VARIANT,
            action::BUILD_NETWORK,
        );
        f.u64(
            f.operative + operative::ACTION + action::IMPLEMENTATION,
            u64::MAX,
        );
        assert_eq!(f.run(&mut rows, &mut []).outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].operatives_len, 0);
        f.u64(GAME + cgame_state::STRATEGIC_OPERATIVE_MANAGER, 0);
        assert_eq!(f.run(&mut rows, &mut []).outcome, Outcome::Rows { len: 1 });
        assert!(!rows[0].strategic_present);
        assert_eq!(rows[0].country_defense, 0);
        assert_eq!(rows[0].crypto_strength, Some(249999));
    }

    #[test]
    fn identity_owner_and_short_read_failures_never_become_zero() {
        let mut f = Fixture::new(1);
        let mut rows = [Row::default()];
        let mut ops = [Operative::default(); 2];
        f.u64(f.operative + operative::IDENTITY, HANDLE + 1);
        assert_eq!(
            f.run(&mut rows, &mut ops).outcome,
            Outcome::Unavailable {
                reason: "raid counteroperative identity mismatch"
            }
        );
        f.u64(f.operative + operative::IDENTITY, HANDLE);
        f.u64(f.mission, 0);
        assert_eq!(
            f.run(&mut rows, &mut ops).outcome,
            Outcome::Unavailable {
                reason: "raid intel object vtable mismatch"
            }
        );
        f.u64(f.mission, MISSION_VTABLE_RVA as u64);
        f.u64(f.country + 0x1280 + ccryptology::OWNER_COUNTRY, 0);
        assert_eq!(
            f.run(&mut rows, &mut ops).outcome,
            Outcome::Unavailable {
                reason: "raid cryptology owner mismatch"
            }
        );
        f.u64(
            f.country + 0x1280 + ccryptology::OWNER_COUNTRY,
            f.country as u64,
        );
        f.regions.remove(&STATIC_DATABASE_RVA);
        assert_eq!(
            f.run(&mut rows, &mut ops).outcome,
            Outcome::Unavailable {
                reason: "raid intel field unreadable"
            }
        );
    }
    #[test]
    #[ignore = "synthetic acquisition cost; release, affinity 0xF"]
    fn acquisition_cost() {
        let countries = std::env::var("BATTLE_PLAN_RAID_SCORE_COUNTRIES").or_else(|_| std::env::var("HOI4_RAID_SCORE_COUNTRIES"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1500);
        let memory = Fixture::new(countries);
        let mut rows = vec![Row::default(); countries];
        let mut ops = [Operative::default(); 2];
        let mut samples = Vec::new();
        for _ in 0..40 {
            let start = std::time::Instant::now();
            let result = memory.run(&mut rows, &mut ops).outcome;
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(matches!(result, Outcome::Rows { .. }));
            std::hint::black_box(&rows);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "raid_intel_inputs countries={countries} median_ms={} max_ms={} output_bytes={}",
            samples[20],
            samples[39],
            std::mem::size_of_val(rows.as_slice())
        );
    }
}
