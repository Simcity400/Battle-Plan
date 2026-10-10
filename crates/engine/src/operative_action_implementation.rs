//! Polymorphic operative-action implementations selected by a mission.

/// Stored raw country tag shared by all eight action implementations.
pub const TARGET_COUNTRY: usize = 0x10;
/// State-based action state pointer; country-based forms have no state here.
pub const TARGET_POINTER: usize = 0x18;
/// Boost-ideology action ideology pointer; class-specific validity is not established here.
pub const BOOST_IDEOLOGY: usize = 0x40;

/// Concrete action vtables in dispatch order, independent of authored mission definitions.
pub const VTABLE_RVAS: [usize; 8] = [
    0x02a0_8b18,
    0x02a0_9268,
    0x02a0_8f40,
    0x02a0_9458,
    0x02a0_8888,
    0x02a0_8da0,
    0x02a0_90a0,
    0x02a1_2848,
];

/// Country-tag virtual dispatch shared by concrete action implementations.
/// Authenticate each executable vtable once per walk, retaining failures, while
/// checking every object's own vtable.
const COUNTRY_GETTER_SLOT: usize = 0x50;
const COUNTRY_GETTER_RVA: usize = 0x01a1_dd30;

pub(crate) struct Vtables {
    base: usize,
    authenticated: [Option<Result<(), &'static str>>; VTABLE_RVAS.len()],
}

#[cfg(test)]
mod tests;

impl Vtables {
    pub(crate) fn new(base: usize) -> Self {
        Self {
            base,
            authenticated: [None; VTABLE_RVAS.len()],
        }
    }

    pub(crate) fn country(
        &mut self,
        memory: &impl crate::Memory,
        mission: usize,
        variant: u32,
    ) -> Result<u32, &'static str> {
        let index = variant
            .checked_sub(1)
            .map(|index| index as usize)
            .filter(|&index| index < VTABLE_RVAS.len())
            .ok_or("unsupported operative mission variant")?;
        let pointer = mission
            .checked_add(crate::operative_action::IMPLEMENTATION)
            .and_then(|at| crate::read::<u64>(memory, at))
            .and_then(|raw| usize::try_from(raw).ok())
            .filter(|&at| at != 0)
            .ok_or("operative mission implementation unreadable or null")?;
        let expected = self
            .base
            .checked_add(VTABLE_RVAS[index])
            .ok_or("operative mission vtable overflow")?;
        if crate::read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("unsupported operative mission implementation vtable");
        }
        let authentication = self.authenticated[index].get_or_insert_with(|| {
            let slot = expected
                .checked_add(COUNTRY_GETTER_SLOT)
                .ok_or("operative mission getter slot overflow")?;
            let getter = self
                .base
                .checked_add(COUNTRY_GETTER_RVA)
                .ok_or("operative mission getter address overflow")?;
            if crate::read::<u64>(memory, slot) != Some(getter as u64) {
                return Err("unsupported operative mission country getter");
            }
            Ok(())
        });
        (*authentication)?;
        pointer
            .checked_add(TARGET_COUNTRY)
            .and_then(|at| crate::read(memory, at))
            .ok_or("operative mission target country unreadable")
    }
}
