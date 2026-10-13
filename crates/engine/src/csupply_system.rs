//! Game-state-rooted supply layout. Province-supply records have stride
//! 0x20 and signed count in the array header. Country-vector capacity and
//! count remain separate. Layout alone establishes no hourly consistency.

use crate::{
    Memory, ccountry, ccountry_supply_system, cgame_state, cpdx_array::Array, read,
    supply_companion,
};

/// CSupplySystem primary vtable.
pub const VTABLE_RVA: usize = 0x0295_d0a8;
/// CSupplySystem secondary vtable at +8.
pub const SECONDARY_VTABLE_RVA: usize = 0x0295_d0d0;
/// CSupplySystem secondary-base displacement.
pub const SECONDARY_BASE: usize = 0x08;
/// CSupplySystem SProvinceSupplyData array pointer.
pub const PROVINCE_DATA: usize = 0x10;
/// Province-supply signed count; checksum traverses entries excluding slot zero.
pub const PROVINCE_COUNT: usize = 0x1c;
/// Opaque auxiliary array with stride 0x28; dirty clear resets each bit-zero flag.
pub const AUXILIARY_DATA: usize = 0x28;
/// CSupplySystem foreign-source inline array pointer.
pub const FOREIGN_SOURCE_DATA: usize = 0x40;
/// CSupplySystem foreign-source array u32 capacity.
pub const FOREIGN_SOURCE_CAPACITY: usize = 0x48;
/// CSupplySystem foreign-source array u32 count.
pub const FOREIGN_SOURCE_COUNT: usize = 0x4c;
/// Dirty-province ID vector; clear resets the corresponding province-data bit 2.
pub const DIRTY_PROVINCES: usize = 0x58;
/// Dirty-province signed count, consumed then zeroed.
pub const DIRTY_PROVINCE_COUNT: usize = 0x64;
/// CSupplySystem railway-dirty i32 province id vector.
pub const RAILWAY_DIRTY_PROVINCES: usize = 0x70;
/// Railway-dirty signed count, consumed then zeroed.
pub const RAILWAY_DIRTY_PROVINCE_COUNT: usize = 0x7c;
/// Auxiliary-dirty index vector selecting bit-zero flags to clear.
pub const DIRTY_AUXILIARY_INDICES: usize = 0x88;
/// Auxiliary-dirty signed count, consumed then zeroed.
pub const DIRTY_AUXILIARY_COUNT: usize = 0x94;
/// Dirty country-link vector for foreign-source bit 1, resolved through country links.
pub const FOREIGN_SOURCE_DIRTY_BIT1: usize = 0xa0;
/// Foreign-source bit-1 dirty signed count, consumed then zeroed.
pub const FOREIGN_SOURCE_DIRTY_BIT1_COUNT: usize = 0xac;
/// Dirty country-link vector for foreign-source bit 2.
pub const FOREIGN_SOURCE_DIRTY_BIT2: usize = 0xb8;
/// Foreign-source bit-2 dirty signed count, consumed then zeroed.
pub const FOREIGN_SOURCE_DIRTY_BIT2_COUNT: usize = 0xc4;
/// Dirty country-link vector for foreign-source bit 3.
pub const FOREIGN_SOURCE_DIRTY_BIT3: usize = 0xd0;
/// Foreign-source bit-3 dirty signed count, consumed then zeroed.
pub const FOREIGN_SOURCE_DIRTY_BIT3_COUNT: usize = 0xdc;
/// Country-supply pointer array selected by canonical country-link slot.
pub const COUNTRY_SYSTEM_DATA: usize = 0x108;
/// Country-array capacity, u32, at header +8.
pub const COUNTRY_SYSTEM_CAPACITY: usize = 0x110;
/// Country-array count, u32, at header +0xc.
pub const COUNTRY_SYSTEM_COUNT: usize = 0x114;
/// CSupplySystem inline companion array pointer.
pub const COUNTRY_COMPANION_DATA: usize = 0x120;
/// CSupplySystem companion-array u32 capacity.
pub const COUNTRY_COMPANION_CAPACITY: usize = 0x128;
/// CSupplySystem companion-array u32 count.
pub const COUNTRY_COMPANION_COUNT: usize = 0x12c;
/// Construction allocating per-country supply objects.
pub const CREATE_COUNTRY_SYSTEMS_RVA: usize = 0x00eb_5d70;
/// Supply ownership update entry.
pub const COUNTRY_SYSTEM_OWNERSHIP_RVA: usize = 0x0120_3360;
/// Canonical-country supply lookup.
pub const COUNTRY_SYSTEM_LOOKUP_RVA: usize = 0x0120_ec90;
/// Supply-system constructor installing primary identity.
pub const CONSTRUCTOR_RVA: usize = 0x00ea_b480;
/// Supply update entry with boolean operand.
pub const UPDATE_SUPPLY_RVA: usize = 0x00eb_8270;
/// Province-supply checksum entry.
pub const CHECKSUM_RVA: usize = 0x00eb_08f0;
/// Province-dirty setter assigning bit 2.
pub const MARK_PROVINCE_DIRTY_RVA: usize = 0x00eb_5b60;
/// Railway-dirty setter assigning bit 3 and appending on zero-to-one transition.
pub const MARK_PROVINCE_DIRTY_RAILWAY_RVA: usize = 0x00eb_5ca0;
/// Dirty clear resetting flags and list counts.
pub const CLEAR_DIRTY_RVA: usize = 0x00eb_31c0;
/// Province-supply recomputation writing node/naval levels and flags.
pub const RECOMPUTE_PROVINCE_DATA_RVA: usize = 0x00eb_71b0;
/// Infrastructure update writing province-data healthy and total levels.
pub const UPDATE_INFRASTRUCTURE_RVA: usize = 0x00eb_7690;
/// Railway-notification virtual-dispatch byte offset.
pub const RAILWAY_NOTIFICATION_VTABLE_SLOT: usize = 0x18;
/// Railway notification reaching both dirty setters.
pub const RAILWAY_NOTIFICATION_RVA: usize = 0x00eb_4fa0;
/// General province-dirty wrapper.
pub const MARK_PROVINCE_DIRTY_WRAPPER_RVA: usize = 0x00ea_f770;
/// Railway-dirty wrapper.
pub const MARK_RAILWAY_DIRTY_WRAPPER_RVA: usize = 0x00ea_f900;
/// Virtual target at byte offset zero; semantic purpose unresolved.
pub const VTABLE_SLOT_00_TARGET_RVA: usize = 0x0012_9db0;
/// Virtual target at byte offset eight; semantic purpose unresolved.
pub const VTABLE_SLOT_08_TARGET_RVA: usize = 0x00ea_f580;
/// Virtual target at byte offset sixteen; semantic purpose unresolved.
pub const VTABLE_SLOT_10_TARGET_RVA: usize = 0x00eb_4e80;

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
pub(crate) struct CountrySystem {
    pub pointer: usize,
    pub country: ccountry::Country,
    pub companion: usize,
}

/// Shares the authenticated supply/country roots across a complete table walk.
pub(crate) struct Reader<'a, M: Memory> {
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
    pub(crate) fn new(memory: &'a M, base: usize, game_state: usize) -> Result<Self, &'static str> {
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

    pub(crate) fn count(&self) -> u32 {
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

    pub(crate) fn at(&self, slot: u32) -> Result<Option<CountrySystem>, &'static str> {
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
