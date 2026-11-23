
use engine_native_memory::{Memory, memory::Raw, read};
use engine_sites::Rva;

mod air;
mod capture;
mod demand;
pub use capture::{BufferRows, Buffers, capture};
pub use demand::demand;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Name {
    pub bytes: Option<Span>,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub country: u32,
    pub instance: Option<u32>,
    pub divisions: Span,
    pub army_reason: Option<&'static str>,
    pub air_reason: Option<&'static str>,
    pub sites: Span,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CachedUnit {
    pub country: u32,
    pub reference: u64,
    pub parent: u64,
    pub ordinal: u32,
    pub owners: Option<[u32; 3]>,
    pub rate: Option<i64>,
    pub reason: Option<&'static str>,
    pub name: Name,
    pub group: Option<u64>,
    pub group_reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fleet {
    pub country: u32,
    pub reference: u64,
    pub name: Name,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub list: u8,
    pub ordinal: u32,
    pub base: u64,
    pub container_owner: Option<u32>,
    pub capacity: Option<i64>,
    pub demilitarized: Option<u8>,
    pub wings: Span,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wing {
    pub reference: u64,
    pub owner: Option<u32>,
    pub country_override: Option<i32>,
    pub owner_reason: Option<&'static str>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub name: Name,
    pub inputs: Option<AirInputs>,
    pub weather: Span,
    pub context: Option<usize>,
    pub reason: Option<&'static str>,
}

pub use engine_native_parts::modifier_accumulator::RequestedOperand as Modifier;

pub use game_model::fuel_demand::{AirInputs, Weather};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub region: u32,
    pub pointer: usize,
    pub weather: Span,
    pub counts: [i32; 24],
    pub thresholds: [i64; 24],
    pub reason: Option<&'static str>,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("fuel demand address overflow")
}
fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("fuel demand operand unreadable")
}
fn module_at(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("fuel demand address overflow")
}
fn global<T: Raw>(memory: &impl Memory, base: usize, key: Rva) -> Result<T, &'static str> {
    read(memory, module_at(base, key)?).ok_or("fuel demand operand unreadable")
}

pub use game_model::fuel_demand::{capacity_gate, wing_rate, context_sums, wing_rate_with_context, daily_required};

#[cfg(test)]
mod tests;
