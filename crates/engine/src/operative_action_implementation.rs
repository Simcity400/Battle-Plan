//! NOperativeMissions implementations behind COperativeMission+0x10.

/// Stored raw country tag; GetCountryTag@140FAE550 -> virtual+0x50 ->
/// 141A1DD30 copies this word for all eight concrete implementations.
pub const TARGET_COUNTRY: usize = 0x10;
/// CStateBased's state pointer (not a network); getter 1401F6A80 and
/// serializer 141A1E080. Country-based implementations have no state here.
pub const TARGET_POINTER: usize = 0x18;
/// Operative boost-ideology implementation ideology pointer; old a30_non_operation_action_identity.rs:23, class-specific validity still needs proof.
pub const BOOST_IDEOLOGY: usize = 0x40;

/// Concrete RTTI vtables; dispatch 140FAFAA0 establishes variant order,
/// each vtable's +0x50 is GetCountryTag@141A1DD30. These are executable
/// implementation classes, independent of authored mission definitions.
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

/// Virtual +0x50 for all concrete mission implementations; 140FAE550 calls
/// it and 141A1DD30 copies +0x10. Authenticate each executable vtable once
/// per walk, retaining failures; every object's own vtable is still checked.
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
