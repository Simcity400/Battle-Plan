use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout},
};

const FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: super::ID,
        destination_offset: 8,
        len: 4,
    },
];
const LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FIELDS, 12).expect("valid province batch layout");
const SLOT_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const SLOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SLOT_FIELDS, 8).expect("valid province slot layout");

pub struct IdentityBatch {
    pointers: [usize; 64],
    identities: [u8; 64 * 12],
    complete: [bool; 64],
    len: usize,
    ids: [u32; 64],
    slots: [u8; 64 * 8],
    slot_complete: [bool; 64],
    slot_len: usize,
}

impl Default for IdentityBatch {
    fn default() -> Self {
        Self {
            pointers: [0; 64],
            identities: [0; 64 * 12],
            complete: [false; 64],
            len: 0,
            ids: [0; 64],
            slots: [0; 64 * 8],
            slot_complete: [false; 64],
            slot_len: 0,
        }
    }
}

impl IdentityBatch {
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        provinces: Array,
        selected: &[usize],
        slot_missing: impl Fn(u32) -> bool,
    ) {
        self.len = 0;
        self.slot_len = 0;
        for &pointer in selected {
            if pointer != 0 && !self.pointers[..self.len].contains(&pointer) {
                self.pointers[self.len] = pointer;
                self.len += 1;
            }
        }
        engine_native_memory::memory::gather_records_once(
            memory,
            &self.pointers[..self.len],
            &LAYOUT,
            &mut self.identities[..self.len * 12],
            &mut self.complete,
            engine_native_memory::batch_fallback::Batch::DivisionProvinces,
        );
        let mut addresses = [0_usize; 64];
        for row in 0..self.len {
            if !self.complete[row] {
                break;
            }
            let (vtable, id) = self.raw(row);
            if Some(vtable) != engine_sites::at(base, super::VTABLE_RVA).map(|at| at as u64)
                || id == 0
                || !slot_missing(id)
                || self.ids[..self.slot_len].contains(&id)
            {
                continue;
            }
            if let Some(at) = provinces.element(id, 8) {
                self.ids[self.slot_len] = id;
                addresses[self.slot_len] = at;
                self.slot_len += 1;
            }
        }
        engine_native_memory::memory::gather_records_once(
            memory,
            &addresses[..self.slot_len],
            &SLOT_LAYOUT,
            &mut self.slots[..self.slot_len * 8],
            &mut self.slot_complete,
            engine_native_memory::batch_fallback::Batch::DivisionProvinces,
        );
    }

    fn raw(&self, row: usize) -> (u64, u32) {
        let bytes = &self.identities[row * 12..][..12];
        (
            u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            u32::from_le_bytes(bytes[8..].try_into().unwrap()),
        )
    }

    pub fn identity(&self, pointer: usize) -> Option<(u64, u32)> {
        self.pointers[..self.len]
            .iter()
            .position(|&at| at == pointer)
            .filter(|&row| self.complete[row])
            .map(|row| self.raw(row))
    }

    pub fn slot(&self, id: u32) -> Option<usize> {
        self.ids[..self.slot_len]
            .iter()
            .position(|&key| key == id)
            .filter(|&row| self.slot_complete[row])
            .map(|row| u64::from_le_bytes(self.slots[row * 8..][..8].try_into().unwrap()) as usize)
    }
}

#[cfg(test)]
mod tests;
