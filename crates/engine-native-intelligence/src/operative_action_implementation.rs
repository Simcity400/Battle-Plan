
use engine_sites::Rva;
pub const TARGET_COUNTRY: usize = 0x10;
pub const TARGET_POINTER: usize = 0x18;
pub const BOOST_IDEOLOGY: usize = 0x40;

pub const VTABLE_RVAS: [Rva; 8] = [
    Rva(0x02a9_8bd8),
    Rva(0x02a9_92b8),
    Rva(0x02a9_8f90),
    Rva(0x02a9_9440),
    Rva(0x02a9_8940),
    Rva(0x02a9_8df0),
    Rva(0x02a9_90f0),
    Rva(0x02aa_22e0),
];

pub(crate) const COUNTRY_GETTER_SLOT: usize = 0x50;
pub(crate) const COUNTRY_GETTER_RVA: Rva = Rva(0x01ac_42f0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Targets {
    pub(crate) country: u32,
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
        memory: &impl engine_native_memory::Memory,
        mission: usize,
        variant: u32,
    ) -> Result<u32, &'static str> {
        let pointer = self.implementation(memory, mission, variant)?;
        target_country(memory, pointer)
    }

    pub(crate) fn targets(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        mission: usize,
        variant: u32,
    ) -> Result<Targets, &'static str> {
        let pointer = self.implementation(memory, mission, variant)?;
        let country = target_country(memory, pointer)?;
        let state = if crate::operative_action::targets_state(variant) {
            let raw = pointer
                .checked_add(TARGET_POINTER)
                .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                .ok_or("operative mission target state unreadable")?;
            Some(usize::try_from(raw).map_err(|_| "operative mission target state overflow")?)
                .filter(|&state| state != 0)
        } else {
            None
        };
        Ok(Targets { country, state })
    }

    fn implementation(
        &mut self,
        memory: &impl engine_native_memory::Memory,
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
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .and_then(|raw| usize::try_from(raw).ok())
            .filter(|&at| at != 0)
            .ok_or("operative mission implementation unreadable or null")?;
        let expected = engine_sites::at(self.base, VTABLE_RVAS[index])
            .ok_or("operative mission vtable withheld")?;
        if engine_native_memory::read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("unsupported operative mission implementation vtable");
        }
        let authentication = self.authenticated[index].get_or_insert_with(|| {
            let slot = expected
                .checked_add(COUNTRY_GETTER_SLOT)
                .ok_or("operative mission getter slot overflow")?;
            let getter = engine_sites::at(self.base, COUNTRY_GETTER_RVA)
                .ok_or("operative mission getter withheld")?;
            if engine_native_memory::read::<u64>(memory, slot) != Some(getter as u64) {
                return Err("unsupported operative mission country getter");
            }
            Ok(())
        });
        (*authentication)?;
        Ok(pointer)
    }
}

fn target_country(memory: &impl engine_native_memory::Memory, implementation: usize) -> Result<u32, &'static str> {
    implementation
        .checked_add(TARGET_COUNTRY)
        .and_then(|at| engine_native_memory::read(memory, at))
        .ok_or("operative mission target country unreadable")
}
