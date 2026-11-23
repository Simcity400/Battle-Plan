
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccountry, cgame_state};
use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};

pub const MANAGER: usize = 0x60;

const CONSUMED: &str = "naval movement input already consumed";

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("naval movement address overflow")
}

fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("naval movement pointer unreadable")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, stride)
        .ok_or("naval movement array unreadable or invalid")
}

fn vtable(
    memory: &impl Memory,
    base: usize,
    object: usize,
    key: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, key).ok_or("naval movement module overflow")? as u64;
    if object == 0 || read::<u64>(memory, object) != Some(expected) {
        return Err("naval movement identity mismatch");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub navy_slots: Result<usize, &'static str>,
}

#[derive(Debug, Clone, Copy)]
struct Manager {
    pointer: usize,
    navies: Array,
}

pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    manager: Result<Option<Manager>, &'static str>,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, state: usize) -> Self {
        let manager = (|| {
            let manager = pointer(memory, at(state, cgame_state::STRATEGIC_NAVY_MANAGER)?)?;
            if manager == 0 {
                return Ok(None);
            }
            vtable(memory, base, manager, cstrategic_navy_manager::VTABLE_RVA)?;
            Ok(Some(Manager {
                pointer: manager,
                navies: array(memory, manager, cstrategic_navy_manager::NAVIES, 8)?,
            }))
        })();
        Self {
            memory,
            base,
            manager,
        }
    }

    pub fn bounds(&self) -> Bounds {
        Bounds {
            navy_slots: self
                .manager
                .map(|manager| manager.map_or(0, |manager| manager.navies.count as usize)),
        }
    }

    pub fn clear(&mut self) {
        self.manager = Err("naval movement pass cleared");
    }

    pub fn observe_navy(&self, slot: u32) -> Result<NavyInputs, &'static str> {
        let Some(manager) = self.manager? else {
            return Ok(NavyInputs::absent(slot));
        };
        if manager.navies.count == 0 {
            return Ok(NavyInputs::absent(slot));
        }
        let entry = manager
            .navies
            .element(slot, 8)
            .ok_or("naval movement physical navy slot outside manager")?;
        let navy = pointer(self.memory, entry)?;
        if navy == 0 {
            return Ok(NavyInputs::absent(slot));
        }
        let mut primary = [0; 8];
        let mut secondary = [0; 8];
        let mut backlink = [0; 8];
        if self.memory.copy_scattered(
            [
                navy,
                at(navy, cstrategic_navy::SECONDARY)?,
                at(navy, MANAGER)?,
            ],
            &mut [&mut primary, &mut secondary, &mut backlink],
        ) != 3
        {
            return Err("naval movement navy identity unreadable");
        }
        let expected = engine_sites::at(self.base, cstrategic_navy::VTABLE_RVA)
            .ok_or("naval movement module overflow")? as u64;
        let secondary_expected = engine_sites::at(self.base, cstrategic_navy::SECONDARY_VTABLE_RVA)
            .ok_or("naval movement module overflow")? as u64;
        if u64::from_le_bytes(primary) != expected
            || u64::from_le_bytes(secondary) != secondary_expected
            || u64::from_le_bytes(backlink) != manager.pointer as u64
        {
            return Err("naval movement navy identity or manager mismatch");
        }
        Ok(NavyInputs {
            slot,
            present: true,
            launch_navy: Some(navy),
            ledger_navy: Some(navy),
            ledgers: Some(array(
                self.memory,
                navy,
                cstrategic_navy::REGION_MISSIONS,
                cstrategic_navy::REGION_MISSIONS_STRIDE,
            )),
        })
    }
}

impl<M: Memory> Drop for Reader<'_, M> {
    fn drop(&mut self) {
        self.clear();
    }
}

pub struct NavyInputs {
    pub slot: u32,
    pub present: bool,
    launch_navy: Option<usize>,
    ledger_navy: Option<usize>,
    ledgers: Option<Result<Array, &'static str>>,
}

impl NavyInputs {
    fn absent(slot: u32) -> Self {
        Self {
            slot,
            present: false,
            launch_navy: Some(0),
            ledger_navy: Some(0),
            ledgers: Some(Ok(Array::default())),
        }
    }

    pub fn clear(&mut self) {
        self.launch_navy = None;
        self.ledger_navy = None;
        self.ledgers = None;
    }

    pub fn ledger_navy(&self) -> Result<usize, &'static str> {
        self.ledger_navy.ok_or(CONSUMED)
    }

    pub fn take_ledger_regions(&mut self) -> Result<Array, &'static str> {
        self.ledgers.take().ok_or(CONSUMED)?
    }

    pub fn capture_launch(
        &mut self,
        memory: &impl Memory,
        base: usize,
        scratch: &mut Vec<ccountry::launch_sources::Occurrence>,
        output: &mut Vec<ccountry::launch_sources::LaunchNavalBase>,
    ) -> ccountry::launch_sources::NavalCapture {
        self.capture_launch_buffer(
            memory,
            base,
            engine_native_memory::walk::Buffer::Growing(scratch),
            engine_native_memory::walk::Buffer::Growing(output),
        )
    }

    pub fn capture_launch_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        scratch: engine_native_memory::walk::Buffer<'_, ccountry::launch_sources::Occurrence>,
        output: engine_native_memory::walk::Buffer<'_, ccountry::launch_sources::LaunchNavalBase>,
    ) -> ccountry::launch_sources::NavalCapture {
        match self.launch_navy.take() {
            Some(navy) => {
                ccountry::launch_sources::capture_naval_buffer(memory, base, navy, scratch, output)
            }
            None => ccountry::launch_sources::NavalCapture {
                required: 0,
                provider: Err("reserved launch navy control consumed"),
            },
        }
    }
}

impl Drop for NavyInputs {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests;
