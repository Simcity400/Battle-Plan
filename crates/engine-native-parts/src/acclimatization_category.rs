
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice};
use engine_sites::Rva;

pub const REGISTRY_RVA: Rva = Rva(0x033d_e940);
pub const NAME: usize = 0x00;
pub const TOKEN: usize = 0x20;

#[derive(Debug, Default)]
pub struct Categories {
    entries: Vec<(usize, u32)>,
}

impl Categories {
    pub fn try_with_capacity(capacity: usize) -> std::io::Result<Self> {
        Ok(Self {
            entries: engine_allocation::vec(capacity)?,
        })
    }

    #[cfg(test)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::try_with_capacity(capacity).unwrap()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<(usize, u32)>()
    }

    pub fn load(
        &mut self,
        memory: &impl Memory,
        base: usize,
        required: &mut usize,
    ) -> Result<(), &'static str> {
        self.load_buffer(memory, base, required)
    }

    fn load_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        required: &mut usize,
    ) -> Result<(), &'static str> {
        self.entries.clear();
        let registry = engine_sites::at(base, REGISTRY_RVA)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|at| usize::try_from(at).ok())
            .filter(|at| *at != 0)
            .ok_or("acclimatization category registry unreadable")?;
        let array =
            Array::read(memory, registry, 8).ok_or("acclimatization category registry invalid")?;
        let count = array.count as usize;
        *required = (*required).max(count);
        engine_allocation::reserve(&mut self.entries, count)
            .map_err(|_| "acclimatization category allocation failed")?;
        
        if count > self.entries.capacity() {
            return Err("acclimatization category capacity exceeded");
        }
        const SPAN: usize = 64;
        let mut pointers = [0_u8; SPAN * 8];
        for first in (0..count).step_by(SPAN) {
            let len = (count - first).min(SPAN);
            let at = array
                .element(first as u32, 8)
                .ok_or("acclimatization category registry index invalid")?;
            if !read_slice(memory, at, &mut pointers[..len * 8]) {
                return Err("acclimatization category registry unreadable");
            }
            for bytes in pointers[..len * 8].chunks_exact(8) {
                let pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .ok()
                    .filter(|pointer| *pointer != 0)
                    .ok_or("acclimatization category registry entry null")?;
                let token = pointer
                    .checked_add(TOKEN)
                    .and_then(|at| read::<u32>(memory, at))
                    .filter(|token| *token != 0)
                    .ok_or("acclimatization category token unreadable")?;
                self.entries.push((pointer, token));
            }
        }
        self.entries.sort_unstable_by_key(|(pointer, _)| *pointer);
        Ok(())
    }

    pub fn token(&self, pointer: usize) -> Option<u32> {
        self.entries
            .binary_search_by_key(&pointer, |(pointer, _)| *pointer)
            .ok()
            .map(|at| self.entries[at].1)
    }
}
