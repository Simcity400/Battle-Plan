use engine_native_memory::{
    Memory,
    cgame_date,
    cpdx_array::Array,
    msvc_string::Descriptor,
    read,
};
use engine_allocation as allocation;
use engine_native::{carmy, cprovince};
use engine_sites::Rva;

pub const DIVISION_HISTORY: usize = 0x638;
pub const HISTORY_VTABLE_RVA: Rva = Rva(0x2a3bcf8);
pub const STORE_VTABLE_RVA: Rva = Rva(0x2a3bca8);
pub const ENTRY_VTABLE_RVA: Rva = Rva(0x2a3c018);
pub const MEDAL_VTABLE_RVA: Rva = Rva(0x2a3bc58);
pub const NULL_MEDAL_VTABLE_RVA: Rva = Rva(0x29c27b0);
pub const UNIT_BACKLINK: usize = 8;
pub const MEDAL_STORE: usize = 0x20;
pub const HISTORY_ENTRIES: usize = 0x28;
pub const AWARD_ENTRIES: usize = 8;
pub const AWARD_AMOUNT: usize = 0x260;
const ENTRY_SIZE: usize = 0x148;
const NAME: usize = 8;
const CUSTOM_NAME: usize = 0x28;
const KIND: usize = 0x48;
const TARGET_COUNTRY: usize = 0x4c;
const DATE: usize = 0x50;
const HOUR: usize = 0x58;
const DATE_PERSISTENT: usize = 0x60;
const MEDAL: usize = 0x68;
const MEDAL_COUNT: usize = 0x70;
const INHERIT: usize = 0x71;
const HAS_SUNK_SHIP: usize = 0x72;
const SUNK_SHIP: usize = 0x78;
const PROVINCES: usize = 0x120;
const MULTIPLIER: usize = 0x138;
const ORDERS: usize = 0x140;
const MEDAL_ENABLED: usize = 0x10;
const MEDAL_ORDINAL: usize = 0x428;

#[derive(Clone, Copy, Debug)]
pub struct Roots {
    pub history: usize,
    pub store: usize,
    pub amount: i32,
    entries: [Array; 2],
}

#[derive(Debug, Default)]
pub struct Entry {
    pub name: Vec<u8>,
    pub custom_name: Vec<u8>,
    pub kind: i32,
    pub target_country: u32,
    pub hour: i32,
    pub medal: Option<(i32, i32)>,
    pub medal_enabled: u8,
    pub medal_count: u8,
    pub inherit: u8,
    pub multiplier: i64,
    pub orders: i32,
    pub provinces: Vec<u32>,
    pub sunk_ship: Option<usize>,
}

fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("unit history site withheld")
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("unit history address overflow")
}
fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| value != 0)
        .ok_or("unit history pointer unreadable or null")
}
fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected: Rva,
) -> Result<(), &'static str> {
    if read::<u64>(memory, pointer) != Some(site(base, expected)? as u64) {
        return Err("unit history object identity mismatch");
    }
    Ok(())
}

pub fn roots(
    memory: &impl Memory,
    base: usize,
    seed: &carmy::DivisionSeed,
) -> Result<Roots, &'static str> {
    validate_seed(base, seed)?;
    let history = at(seed.pointer, DIVISION_HISTORY)?;
    identity(memory, base, history, HISTORY_VTABLE_RVA)?;
    if pointer(memory, at(history, UNIT_BACKLINK)?)? != seed.pointer {
        return Err("unit history division backlink mismatch");
    }
    let store = pointer(memory, at(history, MEDAL_STORE)?)?;
    identity(memory, base, store, STORE_VTABLE_RVA)?;
    let entries = [
        Array::read(memory, at(history, HISTORY_ENTRIES)?, 8)
            .ok_or("unit career entry vector invalid")?,
        Array::read(memory, at(store, AWARD_ENTRIES)?, 8)
            .ok_or("unit medal entry vector invalid")?,
    ];
    let amount = read(memory, at(store, AWARD_AMOUNT)?).ok_or("unit medal amount unreadable")?;
    Ok(Roots {
        history,
        store,
        entries,
        amount,
    })
}

#[derive(Clone, Copy)]
pub struct SeedVtables {
    primary: Option<u64>,
    secondary: Option<u64>,
}

impl SeedVtables {
    pub fn resolve(base: usize) -> Self {
        Self {
            primary: engine_sites::at(base, carmy::VTABLE_RVA).map(|at| at as u64),
            secondary: engine_sites::at(base, carmy::SECONDARY_VTABLE_RVA).map(|at| at as u64),
        }
    }
}

pub fn validate_seed(base: usize, seed: &carmy::DivisionSeed) -> Result<(), &'static str> {
    validate_seed_with(SeedVtables::resolve(base), seed)
}

#[inline]
pub fn validate_seed_with(
    vtables: SeedVtables,
    seed: &carmy::DivisionSeed,
) -> Result<(), &'static str> {
    let primary = vtables.primary.ok_or("unit history site withheld")?;
    let secondary = vtables.secondary.ok_or("unit history site withheld")?;
    if seed.division == 0
        || seed.handle != Some(seed.division)
        || seed.class != Some(0)
        || seed.primary_vtable != Some(primary)
        || seed.secondary_vtable != Some(secondary)
    {
        return Err("unit history division identity unavailable");
    }
    Ok(())
}

impl Roots {
    pub fn count(&self, awards: bool) -> usize {
        self.entries[usize::from(awards)].count as usize
    }

    pub fn visit(
        &self,
        memory: &impl Memory,
        base: usize,
        awards: bool,
        mut visit: impl FnMut(u32, Entry) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.visit_range(memory, base, awards, 0, |ordinal, _, entry| {
            visit(ordinal, entry)
        })
    }
    pub fn visit_range(
        &self,
        memory: &impl Memory,
        base: usize,
        awards: bool,
        first: u32,
        mut visit: impl FnMut(u32, usize, Entry) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let array = self.entries[usize::from(awards)];
        if first > array.count {
            return Err("unit history append prefix exceeds list");
        }
        for ordinal in first..array.count {
            let slot = array
                .element(ordinal, 8)
                .ok_or("unit history entry slot overflow")?;
            let address = pointer(memory, slot)?;
            visit(ordinal, address, entry(memory, base, address)?)?;
        }
        Ok(())
    }
}

pub fn province_count(memory: &impl Memory, address: usize) -> Result<u32, &'static str> {
    Array::read(memory, at(address, PROVINCES)?, 8)
        .map(|array| array.count)
        .ok_or("unit history province vector invalid")
}

const PROVINCE_HEADER_FIELDS: [engine_native_memory::memory::RecordField; 1] = [engine_native_memory::memory::RecordField {
    source_offset: PROVINCES,
    destination_offset: 0,
    len: 16,
}];
const PROVINCE_HEADER_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&PROVINCE_HEADER_FIELDS, 16).expect("valid record layout");

pub fn province_counts(
    memory: &impl Memory,
    addresses: &[usize],
    out: &mut [Result<u32, &'static str>],
) {
    debug_assert_eq!(addresses.len(), out.len());
    const BATCH: usize = 64;
    let mut bytes = [0u8; BATCH * 16];
    for (addrs, results) in addresses.chunks(BATCH).zip(out.chunks_mut(BATCH)) {
        let mut start = 0;
        while start < addrs.len() {
            let copied = memory.copy_records(
                engine_native_memory::memory::RecordSources::Pointers(&addrs[start..]),
                &PROVINCE_HEADER_LAYOUT,
                &mut bytes[start * 16..addrs.len() * 16],
            );
            let complete = start + copied;
            for slot in start..complete {
                let header: [u8; 16] = bytes[slot * 16..slot * 16 + 16].try_into().unwrap();
                results[slot] = Array::from_header(header, 8)
                    .map(|array| array.count)
                    .ok_or("unit history province vector invalid");
            }
            if complete == addrs.len() {
                break;
            }
            results[complete] = province_count(memory, addrs[complete]);
            start = complete + 1;
        }
    }
}

pub fn entry(memory: &impl Memory, base: usize, address: usize) -> Result<Entry, &'static str> {
    let raw = read::<[u8; ENTRY_SIZE]>(memory, address).ok_or("unit history entry unreadable")?;
    let u64_at = |offset: usize| u64::from_le_bytes(raw[offset..offset + 8].try_into().unwrap());
    let i32_at = |offset: usize| i32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
    if u64_at(0) != site(base, ENTRY_VTABLE_RVA)? as u64 {
        return Err("unit history entry identity mismatch");
    }
    if u64_at(DATE) != site(base, cgame_date::VTABLE_RVA)? as u64
        || u64_at(DATE_PERSISTENT) != site(base, cgame_date::PERSISTENT_VTABLE_RVA)? as u64
    {
        return Err("unit history date identity mismatch");
    }
    let text = |offset: usize| {
        let descriptor = Descriptor::from_header(
            memory,
            at(address, offset)?,
            raw[offset..offset + 32].try_into().unwrap(),
        )?;
        let mut bytes = allocation::default_vec(descriptor.len)
            .map_err(|_| "unit history text allocation failed")?;
        descriptor.copy(memory, &mut bytes)?;
        Ok::<_, &'static str>(bytes)
    };
    let mut medal_enabled = 0;
    let medal = if u64_at(MEDAL) == 0 {
        None
    } else {
        let definition =
            usize::try_from(u64_at(MEDAL)).map_err(|_| "unit history medal pointer overflow")?;
        let words =
            read::<[u8; 12]>(memory, definition).ok_or("unit history medal identity unreadable")?;
        let vtable = u64::from_le_bytes(words[..8].try_into().unwrap());
        if engine_sites::which(
            base,
            vtable as usize,
            &[MEDAL_VTABLE_RVA, NULL_MEDAL_VTABLE_RVA],
        )
        .is_none()
        {
            return Err("unit history medal identity mismatch");
        }
        medal_enabled = read(memory, at(definition, MEDAL_ENABLED)?)
            .ok_or("unit history medal enable flag unreadable")?;
        Some((
            i32::from_le_bytes(words[8..].try_into().unwrap()),
            read(memory, at(definition, MEDAL_ORDINAL)?)
                .ok_or("unit history medal ordinal unreadable")?,
        ))
    };
    let array = Array::from_header(raw[PROVINCES..PROVINCES + 16].try_into().unwrap(), 8)
        .ok_or("unit history province vector invalid")?;
    let mut provinces = allocation::vec(array.count as usize)
        .map_err(|_| "unit history province allocation failed")?;
    for ordinal in 0..array.count {
        let slot = array
            .element(ordinal, 8)
            .ok_or("unit history province slot overflow")?;
        let province = pointer(memory, slot)?;
        provinces.push(
            read(memory, at(province, cprovince::ID)?)
                .ok_or("unit history province id unreadable")?,
        );
    }
    Ok(Entry {
        name: text(NAME)?,
        custom_name: text(CUSTOM_NAME)?,
        kind: i32_at(KIND),
        target_country: i32_at(TARGET_COUNTRY) as u32,
        hour: i32_at(HOUR),
        medal,
        medal_enabled,
        medal_count: raw[MEDAL_COUNT],
        inherit: raw[INHERIT],
        multiplier: u64_at(MULTIPLIER) as i64,
        orders: i32_at(ORDERS),
        provinces,
        sunk_ship: (raw[HAS_SUNK_SHIP] != 0)
            .then(|| at(address, SUNK_SHIP))
            .transpose()?,
    })
}

pub const OFFICER: usize = 0x4f8;
const OFFICER_VTABLE_RVA: Rva = Rva(0x29d4838);
const OFFICER_EXPERIENCE: usize = 0x88;
pub const CAREER_COUNTER: usize = 0x678;
pub fn officer_experience(
    memory: &impl Memory,
    base: usize,
    army: usize,
) -> Result<i64, &'static str> {
    let officer = at(army, OFFICER)?;
    identity(memory, base, officer, OFFICER_VTABLE_RVA)?;
    read(memory, at(officer, OFFICER_EXPERIENCE)?).ok_or("division officer experience unreadable")
}
pub fn career_counter(memory: &impl Memory, army: usize) -> Result<i32, &'static str> {
    read(memory, at(army, CAREER_COUNTER)?).ok_or("division career counter unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    fn put(bytes: &mut [u8], at: usize, word: u64) {
        bytes[at..at + 8].copy_from_slice(&word.to_le_bytes());
    }
    #[test]
    fn null_medal_identity_preserves_a_complete_unawarded_entry() {
        let mut bytes = vec![0; 0x3000];
        let address = 0x200;
        let medal = 0x2000;
        put(&mut bytes, address, ENTRY_VTABLE_RVA.0 as u64);
        put(&mut bytes, address + NAME + 24, 15);
        put(&mut bytes, address + CUSTOM_NAME + 24, 15);
        put(&mut bytes, address + DATE, cgame_date::VTABLE_RVA.0 as u64);
        put(
            &mut bytes,
            address + DATE_PERSISTENT,
            cgame_date::PERSISTENT_VTABLE_RVA.0 as u64,
        );
        put(&mut bytes, address + MEDAL, medal as u64);
        put(&mut bytes, medal, NULL_MEDAL_VTABLE_RVA.0 as u64);
        let row = entry(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            address,
        )
        .unwrap();
        assert_eq!(row.medal, Some((0, 0)));
        assert_eq!(row.medal_enabled, 0);
    }
    #[test]
    fn career_scalars_keep_signed_words_and_authenticate_the_officer_holder() {
        let mut bytes = vec![0; 0x2000];
        let army = 0x200;
        put(&mut bytes, army + OFFICER, OFFICER_VTABLE_RVA.0 as u64);
        put(
            &mut bytes,
            army + OFFICER + OFFICER_EXPERIENCE,
            i64::MIN as u64,
        );
        bytes[army + CAREER_COUNTER..army + CAREER_COUNTER + 4]
            .copy_from_slice(&i32::MIN.to_le_bytes());
        let view = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(officer_experience(&view, 0, army), Ok(i64::MIN));
        assert_eq!(career_counter(&view, army), Ok(i32::MIN));
        put(&mut bytes, army + OFFICER, 0);
        assert!(
            officer_experience(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                army
            )
            .is_err()
        );
    }
}
