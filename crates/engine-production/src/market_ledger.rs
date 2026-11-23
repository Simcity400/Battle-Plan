
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    msvc_tree_node::Cursor,
    read,
    walk::Buffer,
};
use engine_allocation as allocation;
use engine_native::{ccountry, cequipment_variant::pool as cequipment_variant_pool};
use engine_native_parts::{cequipment_type, equipment_entry};
use engine_sites::Rva;

use crate::{cequipment_market_status as status, cpurchase_contract as contract};

pub mod selected;

pub const COUNTRY_BACKLINK: usize = 0x68;
pub const STOCKPILE_EQUIPMENT: usize = 0x38;
pub const COUNTRY_PRICE_LEVELS: usize = 0x10;
pub const COUNTRY_SUBSIDY_TOTAL: usize = 0x20;
pub const COUNTRY_SUBSIDIES: usize = 0x28;
pub const AUTOMATION: usize = 0x60;
pub const DRAFT: usize = 0x18;
pub const DRAFT_EQUIPMENT: usize = 0x48;
pub const DRAFT_SUBSIDIES: usize = 0x90;
pub const DRAFT_PRICE_LEVELS: usize = 0xb0;
pub const SUBSIDY_STRIDE: usize = 0x30;
pub const SUBSIDY_CIC: usize = 0;
pub const SUBSIDY_ARCHETYPE: usize = 8;
pub const SUBSIDY_CONDITION: usize = 0x10;
pub const SUBSIDY_KIND: usize = 0x28;
pub const SUBSIDY_TRIGGER_NAME: usize = 0x60;
pub const SUBSIDY_TRIGGER_VTABLE_RVA: Rva = Rva(0x29c00e8);
pub const PRICE_VARIANT: usize = 0x1c;
pub const PRICE_LEVEL: usize = 0x24;
pub const CONTRACT_VARIABLES: usize = 0x2f0;
pub const CONTRACT_ROUTE_POOL: usize = 0x170;
pub const CONTRACT_ROUTE_LENGTH: usize = 0x1b0;
pub const CONTRACT_ROUTE_TYPE: usize = 0x1b4;
pub const CONTRACT_ROUTE_REQUEST: usize = 0x168;
pub const CONTRACT_ROUTE_LOSSES: usize = 0x1c8;
pub const CONTRACT_PERIOD_DAYS: usize = 0x268;
pub const CONTRACT_META_CONVOYS: usize = 0x218;
pub const CONTRACT_META_CIC: usize = 0x210;
pub const CONTRACT_META_EFFICIENCY: usize = 0x220;
pub const CONTRACT_HISTORY_FACTORY: usize = 0x230;
pub const CONTRACT_HISTORY_EQUIPMENT: usize = 0x238;
pub const CONTRACT_HISTORY_COLLECTED: usize = 0x240;
pub const DEFINITION_TOTAL_COST: usize = 0xd0;
pub const DEFINITION_COST_VALID: usize = 0xc0;
pub const VARIANT_MARKET_PRICE: usize = 0x3d0;
pub const LOW_PRICE_FACTOR_RVA: Rva = Rva(0x33e3350);
pub const HIGH_PRICE_FACTOR_RVA: Rva = Rva(0x33e3460);
pub const MARKET_PRICE_FACTOR_RVA: Rva = Rva(0x33e31c8);
pub const LOW_PRICE_TOKEN: u32 = 0x4ba5;
pub const NORMAL_PRICE_TOKEN: u32 = 0x77;
pub const HIGH_PRICE_TOKEN: u32 = 0x4ba7;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub amount_raw: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pool {
    pub allow_zero: bool,
    pub equipment: Vec<Equipment>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PriceLevel {
    pub variant: u64,
    pub level: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Subsidy {
    pub cic_raw: i64,
    pub archetype: u32,
    pub kind: u8,
    pub targets: Vec<u32>,
    pub trigger: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Country {
    pub country: u32,
    pub stockpile: Pool,
    pub price_levels: Vec<PriceLevel>,
    pub subsidies: Vec<Subsidy>,
    pub subsidy_total_raw: i64,
    pub auto_send_market_access: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub stockpile: bool,
    pub price_levels: bool,
    pub subsidies: bool,
    pub automation: bool,
}

impl Demand {
    pub const ALL: Self = Self {
        stockpile: true,
        price_levels: true,
        subsidies: true,
        automation: true,
    };

    pub fn any(self) -> bool {
        self.stockpile || self.price_levels || self.subsidies || self.automation
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Draft {
    pub equipment: Pool,
    pub price_levels: Vec<PriceLevel>,
    pub subsidies: Vec<Subsidy>,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("market ledger address overflow")
}

fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| value != 0)
        .ok_or("market ledger pointer unreadable or null")
}

fn resize<T: Default>(rows: &mut Vec<T>, count: usize) -> Result<(), &'static str> {
    allocation::resize_default(rows, count).map_err(|_| "market ledger allocation failed")
}

pub fn country(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    manager: usize,
    demand: Demand,
    output: &mut Country,
) -> Result<(), &'static str> {
    if !demand.any() {
        return Ok(());
    }
    let address = pointer(
        memory,
        at(country.pointer, ccountry::EQUIPMENT_MARKET_STATUS)?,
    )?;
    if read::<u64>(memory, at(address, status::MANAGER)?) != Some(manager as u64)
        || read::<u64>(memory, at(address, COUNTRY_BACKLINK)?) != Some(country.pointer as u64)
    {
        return Err("market ledger country or manager backlink mismatch");
    }
    if demand.stockpile {
        let stockpile = pointer(memory, at(address, status::STOCKPILE)?)?;
        pool(
            memory,
            base,
            at(stockpile, STOCKPILE_EQUIPMENT)?,
            &mut output.stockpile,
        )?;
    }
    if demand.price_levels {
        prices(
            memory,
            at(address, COUNTRY_PRICE_LEVELS)?,
            &mut output.price_levels,
        )?;
    }
    if demand.subsidies {
        output.subsidy_total_raw = read(memory, at(address, COUNTRY_SUBSIDY_TOTAL)?)
            .ok_or("market subsidy total unreadable")?;
        subsidies(
            memory,
            base,
            at(address, COUNTRY_SUBSIDIES)?,
            &mut output.subsidies,
        )?;
    }
    if demand.automation {
        output.auto_send_market_access = read::<u8>(memory, at(address, AUTOMATION)?)
            .ok_or("market automation option unreadable")?
            != 0;
    }
    output.country = country.id;
    Ok(())
}

pub fn draft(
    memory: &impl Memory,
    base: usize,
    address: usize,
    output: &mut Draft,
) -> Result<(), &'static str> {
    pool(
        memory,
        base,
        at(address, DRAFT_EQUIPMENT)?,
        &mut output.equipment,
    )?;
    prices(
        memory,
        at(address, DRAFT_PRICE_LEVELS)?,
        &mut output.price_levels,
    )?;
    subsidies(
        memory,
        base,
        at(address, DRAFT_SUBSIDIES)?,
        &mut output.subsidies,
    )
}

pub fn contract_terms(
    memory: &impl Memory,
    base: usize,
    address: usize,
    id: u64,
    terms: &mut Draft,
    agreed_prices: &mut Pool,
) -> Result<(), &'static str> {
    if engine_sites::at(base, contract::VTABLE_RVA)
        .is_none_or(|value| read(memory, address) != Some(value as u64))
        || read::<u64>(memory, at(address, contract::ID_TYPE)?) != Some(id)
        || id as u32 != contract::TYPE
        || id >> 32 == 0
    {
        return Err("market terms contract identity mismatch");
    }
    let definition = at(address, DRAFT)?;
    pool(memory, base, definition, agreed_prices)?;
    draft(memory, base, definition, terms)
}

pub fn pool(
    memory: &impl Memory,
    base: usize,
    address: usize,
    output: &mut Pool,
) -> Result<(), &'static str> {
    let entries = cequipment_variant_pool::entries(memory, base, address)?;
    let allow_zero = read::<u8>(memory, at(address, cequipment_variant_pool::ALLOW_ZERO)?)
        .ok_or("market pool zero-entry flag unreadable")?
        != 0;
    resize(&mut output.equipment, entries.count as usize)?;
    for (ordinal, row) in output.equipment.iter_mut().enumerate() {
        let entry = equipment_entry::read(
            memory,
            base,
            entries
                .element(ordinal as u32, equipment_entry::STRIDE)
                .ok_or("market equipment entry address overflow")?,
            ordinal as u32,
        )?;
        *row = Equipment {
            variant: entry.variant_handle,
            amount_raw: entry.amount_raw,
        };
    }
    output.allow_zero = allow_zero;
    Ok(())
}

pub fn prices(
    memory: &impl Memory,
    address: usize,
    output: &mut Vec<PriceLevel>,
) -> Result<(), &'static str> {
    let words = read::<[u8; 16]>(memory, address).ok_or("market price map unreadable")?;
    let (head, _) = Cursor::header(words).map_err(|_| "market price map header invalid")?;
    let sentinel = read::<[u8; 26]>(memory, head).ok_or("market price map sentinel unreadable")?;
    let mut cursor =
        Cursor::from_parts(words, sentinel).map_err(|_| "market price map sentinel invalid")?;
    resize(output, cursor.count() as usize)?;
    let mut index = 0;
    while let Some(node) = cursor
        .next(memory)
        .map_err(|_| "market price map traversal invalid")?
    {
        let variant =
            read(memory, at(node, PRICE_VARIANT)?).ok_or("market price variant unreadable")?;
        let level =
            read::<i32>(memory, at(node, PRICE_LEVEL)?).ok_or("market price level unreadable")?;
        if !(0..=2).contains(&level) {
            return Err("market price level unsupported");
        }
        output[index] = PriceLevel { variant, level };
        index += 1;
    }
    Ok(())
}

pub fn subsidies(
    memory: &impl Memory,
    base: usize,
    address: usize,
    output: &mut Vec<Subsidy>,
) -> Result<(), &'static str> {
    let entries = Array::read(memory, address, SUBSIDY_STRIDE)
        .ok_or("market subsidy array unreadable or invalid")?;
    resize(output, entries.count as usize)?;
    for (ordinal, row) in output.iter_mut().enumerate() {
        let entry = entries
            .element(ordinal as u32, SUBSIDY_STRIDE)
            .ok_or("market subsidy entry address overflow")?;
        row.cic_raw =
            read(memory, at(entry, SUBSIDY_CIC)?).ok_or("market subsidy CIC unreadable")?;
        let archetype = pointer(memory, at(entry, SUBSIDY_ARCHETYPE)?)?;
        let vtable = read::<u64>(memory, archetype).ok_or("market subsidy archetype unreadable")?;
        let candidates = [
            cequipment_type::VTABLE_RVA,
            cequipment_type::NULL_VTABLE_RVA,
        ];
        if engine_sites::which(base, vtable as usize, &candidates).is_none() {
            return Err("market subsidy archetype vtable mismatch");
        }
        row.archetype = read(memory, at(archetype, cequipment_type::TOKEN)?)
            .ok_or("market subsidy archetype token unreadable")?;
        row.kind =
            read(memory, at(entry, SUBSIDY_KIND)?).ok_or("market subsidy condition unreadable")?;
        match row.kind {
            0 => {
                row.trigger.clear();
                let targets = Array::read(memory, at(entry, SUBSIDY_CONDITION)?, 4)
                    .ok_or("market subsidy country array unreadable or invalid")?;
                resize(&mut row.targets, targets.count as usize)?;
                for (index, target) in row.targets.iter_mut().enumerate() {
                    *target = read(
                        memory,
                        targets
                            .element(index as u32, 4)
                            .ok_or("market subsidy country address overflow")?,
                    )
                    .ok_or("market subsidy country unreadable")?;
                }
            }
            1 => {
                row.targets.clear();
                let trigger = pointer(memory, at(entry, SUBSIDY_CONDITION)?)?;
                if engine_sites::at(base, SUBSIDY_TRIGGER_VTABLE_RVA)
                    .is_none_or(|p| read(memory, trigger) != Some(p as u64))
                {
                    return Err("market subsidy trigger vtable mismatch");
                }
                let text =
                    msvc_string::Descriptor::read(memory, at(trigger, SUBSIDY_TRIGGER_NAME)?)?;
                text.copy_buffer(memory, &mut Buffer::Growing(&mut row.trigger), 0)?;
                row.trigger.truncate(text.len);
            }
            _ => return Err("market subsidy condition unsupported"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
