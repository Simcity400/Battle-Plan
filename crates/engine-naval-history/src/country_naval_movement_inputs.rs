
use engine_sites::Rva;
use crate::{
    Memory, ccountry, cgame_state, cpdx_array::Array, cstrategic_navy, cstrategic_navy_manager,
    memory::{RecordField, RecordLayout, RecordSources},
    read, read_slice,
};

pub const MANAGER: usize = 0x60;

const NAVY_BATCH: usize = 64;
const NAVY_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cstrategic_navy::SECONDARY,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: MANAGER,
        destination_offset: 16,
        len: 8,
    },
    RecordField {
        source_offset: cstrategic_navy::REGION_MISSIONS,
        destination_offset: 24,
        len: 16,
    },
];
const NAVY_BYTES: usize = 40;
const NAVY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NAVY_FIELDS, NAVY_BYTES).expect("valid navy layout");

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

    pub fn observe_navies_range(
        &self,
        count: usize,
        mut each: impl FnMut(u32, Result<NavyInputs, &'static str>),
    ) {
        let Ok(Some(manager)) = self.manager else {
            for index in 0..count {
                if let Ok(slot) = u32::try_from(index) {
                    each(slot, self.observe_navy(slot));
                }
            }
            return;
        };
        if manager.navies.count == 0 {
            for index in 0..count {
                if let Ok(slot) = u32::try_from(index) {
                    each(slot, Ok(NavyInputs::absent(slot)));
                }
            }
            return;
        }
        let expected = engine_sites::at(self.base, cstrategic_navy::VTABLE_RVA).map(|v| v as u64);
        let secondary_expected =
            engine_sites::at(self.base, cstrategic_navy::SECONDARY_VTABLE_RVA).map(|v| v as u64);
        let (Some(expected), Some(secondary_expected)) = (expected, secondary_expected) else {
            for index in 0..count {
                if let Ok(slot) = u32::try_from(index) {
                    each(slot, self.observe_navy(slot));
                }
            }
            return;
        };
        let mut ptr_bytes = [0_u8; NAVY_BATCH * 8];
        let mut pointers = [0_usize; NAVY_BATCH];
        let mut records = [0_u8; NAVY_BATCH * NAVY_BYTES];
        let mut first = 0_usize;
        while first < count {
            let n = (count - first).min(NAVY_BATCH);
            let contiguous = u32::try_from(first)
                .ok()
                .filter(|_| (first + n) as u64 <= u64::from(manager.navies.count))
                .and_then(|start_slot| manager.navies.element(start_slot, 8))
                .filter(|&start| {
                    start.checked_add(n * 8).is_some()
                        && read_slice(self.memory, start, &mut ptr_bytes[..n * 8])
                })
                .is_some();
            if !contiguous {
                for offset in 0..n {
                    if let Ok(slot) = u32::try_from(first + offset) {
                        each(slot, self.observe_navy(slot));
                    }
                }
                first += n;
                continue;
            }
            let mut live = 0_usize;
            let mut map = [usize::MAX; NAVY_BATCH];
            let mut raw_ptrs = [0_usize; NAVY_BATCH];
            for offset in 0..n {
                let navy =
                    u64::from_le_bytes(ptr_bytes[offset * 8..(offset + 1) * 8].try_into().unwrap())
                        as usize;
                raw_ptrs[offset] = navy;
                if navy != 0 {
                    map[offset] = live;
                    pointers[live] = navy;
                    live += 1;
                }
            }
            let copied = if live != 0 {
                self.memory.copy_records(
                    RecordSources::Pointers(&pointers[..live]),
                    &NAVY_LAYOUT,
                    &mut records[..live * NAVY_BYTES],
                )
            } else {
                0
            };
            let complete = copied / NAVY_FIELDS.len();
            for offset in 0..n {
                let slot = (first + offset) as u32;
                let navy = raw_ptrs[offset];
                if navy == 0 {
                    each(slot, Ok(NavyInputs::absent(slot)));
                    continue;
                }
                let at = map[offset];
                if at < complete {
                    let rec = &records[at * NAVY_BYTES..(at + 1) * NAVY_BYTES];
                    let primary = u64::from_le_bytes(rec[0..8].try_into().unwrap());
                    let secondary = u64::from_le_bytes(rec[8..16].try_into().unwrap());
                    let backlink = u64::from_le_bytes(rec[16..24].try_into().unwrap());
                    if primary != expected
                        || secondary != secondary_expected
                        || backlink != manager.pointer as u64
                    {
                        each(slot, Err("naval movement navy identity or manager mismatch"));
                    } else {
                        let ledgers = Array::from_header(
                            rec[24..40].try_into().unwrap(),
                            cstrategic_navy::REGION_MISSIONS_STRIDE,
                        )
                        .ok_or("naval movement array unreadable or invalid");
                        each(
                            slot,
                            Ok(NavyInputs {
                                slot,
                                present: true,
                                launch_navy: Some(navy),
                                ledger_navy: Some(navy),
                                ledgers: Some(ledgers),
                            }),
                        );
                    }
                } else {
                    each(slot, self.observe_navy(slot));
                }
            }
            first += n;
        }
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
            crate::walk::Buffer::Growing(scratch),
            crate::walk::Buffer::Growing(output),
        )
    }

    pub fn capture_launch_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        scratch: crate::walk::Buffer<'_, ccountry::launch_sources::Occurrence>,
        output: crate::walk::Buffer<'_, ccountry::launch_sources::LaunchNavalBase>,
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
