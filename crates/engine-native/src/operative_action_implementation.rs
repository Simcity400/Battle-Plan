//! Polymorphic operative-action implementations selected by a mission.

/// Stored raw country tag shared by all eight action implementations.
pub const TARGET_COUNTRY: usize = 0x10;
/// State-based action state pointer; country-based forms have no state here.
pub const TARGET_POINTER: usize = 0x18;
/// Boost-ideology action ideology pointer; class-specific validity is not established here.
pub const BOOST_IDEOLOGY: usize = 0x40;

/// Concrete action vtables in dispatch order, independent of authored mission definitions.
pub const VTABLE_RVAS: [usize; 8] = [
    0x02a9_8128,
    0x02a9_8808,
    0x02a9_84e0,
    0x02a9_8990,
    0x02a9_7e90,
    0x02a9_8340,
    0x02a9_8640,
    0x02aa_1790,
];

/// Country-tag virtual dispatch shared by concrete action implementations.
/// Authenticate each executable vtable once per walk, retaining failures, while
/// checking every object's own vtable.
pub(crate) const COUNTRY_GETTER_SLOT: usize = 0x50;
pub(crate) const COUNTRY_GETTER_RVA: usize = 0x01ac_3fd0;

/// A mission implementation's stored targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Targets {
    pub(crate) country: u32,
    /// Nonnull state pointer of a state-targeted variant.
    pub(crate) state: Option<usize>,
}

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
        let pointer = self.implementation(memory, mission, variant)?;
        target_country(memory, pointer)
    }

    /// The stored raw country and, for the state-targeted variants, the
    /// stored state pointer, null as `None`; the implementation pointer and
    /// its vtable are authenticated once for both.
    pub(crate) fn targets(
        &mut self,
        memory: &impl crate::Memory,
        mission: usize,
        variant: u32,
    ) -> Result<Targets, &'static str> {
        let pointer = self.implementation(memory, mission, variant)?;
        let country = target_country(memory, pointer)?;
        let state = if crate::operative_action::targets_state(variant) {
            let raw = pointer
                .checked_add(TARGET_POINTER)
                .and_then(|at| crate::read::<u64>(memory, at))
                .ok_or("operative mission target state unreadable")?;
            Some(usize::try_from(raw).map_err(|_| "operative mission target state overflow")?)
                .filter(|&state| state != 0)
        } else {
            None
        };
        Ok(Targets { country, state })
    }

    /// The authenticated implementation of the mission's variant.
    fn implementation(
        &mut self,
        memory: &impl crate::Memory,
        mission: usize,
        variant: u32,
    ) -> Result<usize, &'static str> {
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
        Ok(pointer)
    }
}

fn target_country(memory: &impl crate::Memory, implementation: usize) -> Result<u32, &'static str> {
    implementation
        .checked_add(TARGET_COUNTRY)
        .and_then(|at| crate::read(memory, at))
        .ok_or("operative mission target country unreadable")
}
