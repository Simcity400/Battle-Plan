
use crate::{
    Memory, ccountry, ccountry_supply_system, cgame_state, cpdx_array::Array, read,
    supply_companion,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_02b8);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029f_02e0);
pub const SECONDARY_BASE: usize = 0x08;
pub const PROVINCE_DATA: usize = 0x10;
pub const PROVINCE_COUNT: usize = 0x1c;
pub const FOREIGN_SOURCE_DATA: usize = 0x40;
pub const FOREIGN_SOURCE_STRIDE: usize = 0x78;
pub const FOREIGN_SOURCE_RELATED_COUNTRIES: usize = 0x58;
pub const COUNTRY_SYSTEM_DATA: usize = 0x108;
pub const COUNTRY_COMPANION_DATA: usize = 0x120;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00f3_ce90);

pub fn read_root(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let pointer = game_state
        .checked_add(cgame_state::SUPPLY_SYSTEM)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("supply system pointer unreadable or null")?;
    let primary = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let secondary =
        engine_sites::at(base, SECONDARY_VTABLE_RVA).ok_or("module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("supply system vtables unreadable")?;
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != primary as u64
        || u64::from_le_bytes(bytes[8..].try_into().unwrap()) != secondary as u64
    {
        return Err("supply system vtable mismatch");
    }
    Ok(pointer)
}

pub struct CountrySystem {
    pub pointer: usize,
    pub country: ccountry::Country,
    pub companion: usize,
}

pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    countries: ccountry::Reader<'a, M>,
    country_slots: Array,
    systems: Array,
    companions: Array,
    root: usize,
    primary: usize,
    secondary: usize,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, game_state: usize) -> Result<Self, &'static str> {
        let root = read_root(memory, base, game_state)?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let country_slots = game_state
            .checked_add(cgame_state::COUNTRIES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("country array unreadable or invalid")?;
        let systems = root
            .checked_add(COUNTRY_SYSTEM_DATA)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("country supply array unreadable or invalid")?;
        let companions = root
            .checked_add(COUNTRY_COMPANION_DATA)
            .and_then(|p| Array::read(memory, p, supply_companion::STRIDE))
            .ok_or("country supply companion array unreadable or invalid")?;
        if systems.count != countries.count() || companions.count != countries.count() {
            return Err("country supply array counts differ from country slots");
        }
        Ok(Self {
            memory,
            countries,
            country_slots,
            systems,
            companions,
            root,
            primary: engine_sites::at(base, ccountry_supply_system::VTABLE_RVA)
                .ok_or("module address overflow")?,
            secondary: engine_sites::at(base, ccountry_supply_system::SECONDARY_VTABLE_RVA)
                .ok_or("module address overflow")?,
        })
    }

    pub fn count(&self) -> u32 {
        self.systems.count
    }

    pub(crate) fn root(&self) -> usize {
        self.root
    }

    pub(crate) fn resolve_country(&self, raw: u32) -> Result<ccountry::Country, &'static str> {
        self.countries.resolve(raw)
    }

    pub(crate) fn canonical_slot(&self, raw: u32) -> Result<Option<u32>, &'static str> {
        if raw == 0 {
            return Ok(Some(0));
        }
        self.countries.link(raw)
    }

    pub(crate) fn same_country(&self, left: u32, right: u32) -> Result<bool, &'static str> {
        if left == right {
            return Ok(true);
        }
        if left == 0 || right == 0 {
            return Ok(false);
        }
        Ok(self.canonical_slot(left)? == self.canonical_slot(right)?)
    }

    pub fn country(&self, slot: u32) -> Result<Option<ccountry::Country>, &'static str> {
        self.countries.at(slot)
    }

    pub fn at(&self, slot: u32) -> Result<Option<CountrySystem>, &'static str> {
        if slot == 0 {
            return Err("country supply sentinel is not a country");
        }
        let pointer = self
            .systems
            .element(slot, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("country supply slot unreadable")?;
        let raw_country = self
            .country_slots
            .element(slot, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .ok_or("supply country slot unreadable")?;
        if raw_country == 0 {
            return if pointer == 0 {
                Ok(None)
            } else {
                Err("country supply has no country")
            };
        }
        let Some(country) = self.countries.at(slot)? else {
            return Ok(None);
        };
        if pointer == 0 {
            return Err("country has no supply system");
        }
        let companion = self
            .companions
            .element(slot, supply_companion::STRIDE)
            .ok_or("country supply companion address overflow")?;
        let mut vtables = [0_u8; 16];
        let mut owners = [0_u8; 24];
        let owners_at = pointer
            .checked_add(ccountry_supply_system::SUPPLY_SYSTEM)
            .ok_or("country supply address overflow")?;
        if self
            .memory
            .copy_scattered_prefetched([pointer, owners_at], &mut [&mut vtables, &mut owners])
            != 2
        {
            return Err("country supply authentication fields unreadable");
        }
        if u64::from_le_bytes(vtables[..8].try_into().unwrap()) != self.primary as u64
            || u64::from_le_bytes(vtables[8..].try_into().unwrap()) != self.secondary as u64
        {
            return Err("country supply vtable mismatch");
        }
        if u64::from_le_bytes(owners[..8].try_into().unwrap()) != self.root as u64
            || u64::from_le_bytes(owners[8..16].try_into().unwrap()) != country.pointer as u64
            || u64::from_le_bytes(owners[16..].try_into().unwrap()) != companion as u64
        {
            return Err("country supply ownership mismatch");
        }
        Ok(Some(CountrySystem {
            pointer,
            country,
            companion,
        }))
    }
}
