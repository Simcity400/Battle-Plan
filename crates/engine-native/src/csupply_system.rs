//! Game-state-rooted supply layout. Province-supply records have stride
//! 0x20 and signed count in the array header. Country-vector capacity and
//! count remain separate. Layout alone establishes no hourly consistency.

use crate::{
    Memory, ccountry, ccountry_supply_system, cgame_state, cpdx_array::Array, read,
    supply_companion,
};

/// CSupplySystem primary vtable.
pub const VTABLE_RVA: usize = 0x0297_3cc8;
/// CSupplySystem secondary vtable (`SECONDARY_BASE`).
pub const SECONDARY_VTABLE_RVA: usize = 0x0297_3cf0;
/// CSupplySystem secondary-base displacement.
pub const SECONDARY_BASE: usize = 0x08;
/// CSupplySystem SProvinceSupplyData array pointer.
pub const PROVINCE_DATA: usize = 0x10;
/// Province-supply signed count; checksum traverses entries excluding slot zero.
pub const PROVINCE_COUNT: usize = 0x1c;
/// Country-supply pointer array selected by canonical country-link slot.
pub const COUNTRY_SYSTEM_DATA: usize = 0x108;
/// CSupplySystem inline companion array pointer.
pub const COUNTRY_COMPANION_DATA: usize = 0x120;
/// Supply-system constructor installing primary identity.
pub const CONSTRUCTOR_RVA: usize = 0x00ec_0610;

/// Authenticates the root without allocating or calling engine functions.
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
    let primary = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let secondary = base
        .checked_add(SECONDARY_VTABLE_RVA)
        .ok_or("module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("supply system vtables unreadable")?;
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != primary as u64
        || u64::from_le_bytes(bytes[8..].try_into().unwrap()) != secondary as u64
    {
        return Err("supply system vtable mismatch");
    }
    Ok(pointer)
}

/// Country supply objects use canonical slot rather than raw country tag.
pub struct CountrySystem {
    pub pointer: usize,
    pub country: ccountry::Country,
    pub companion: usize,
}

/// Shares the authenticated supply/country roots across a complete table walk.
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
            primary: base
                .checked_add(ccountry_supply_system::VTABLE_RVA)
                .ok_or("module address overflow")?,
            secondary: base
                .checked_add(ccountry_supply_system::SECONDARY_VTABLE_RVA)
                .ok_or("module address overflow")?,
        })
    }

    pub fn count(&self) -> u32 {
        self.systems.count
    }

    pub(crate) fn resolve_country(&self, raw: u32) -> Result<ccountry::Country, &'static str> {
        self.countries.resolve(raw)
    }

    /// The canonical country at `slot`, read from the country census alone,
    /// so a walk of selected countries skips a slot without reading or
    /// authenticating its supply objects ([`Self::at`] reads both).
    pub(crate) fn country(&self, slot: u32) -> Result<Option<ccountry::Country>, &'static str> {
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
