pub trait PassiveMemory {
    fn read(&self, addr: u64, out: &mut [u8]) -> bool;

    fn module_base(&self) -> u64;
}

pub trait PassiveMemoryExt: PassiveMemory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        let mut buf = [0u8; 1];
        self.read(addr, &mut buf).then(|| buf[0])
    }

    fn read_u32(&self, addr: u64) -> Option<u32> {
        let mut buf = [0u8; 4];
        self.read(addr, &mut buf).then(|| u32::from_le_bytes(buf))
    }

    fn read_u64(&self, addr: u64) -> Option<u64> {
        let mut buf = [0u8; 8];
        self.read(addr, &mut buf).then(|| u64::from_le_bytes(buf))
    }

    fn read_pointer(&self, addr: u64) -> Option<u64> {
        let value = self.read_u64(addr)?;
        (value != 0 && value % 8 == 0).then_some(value)
    }

    fn read_f32_bits(&self, addr: u64) -> Option<u32> {
        self.read_u32(addr)
    }

    fn read_u64_field(&self, base: u64, delta: u64) -> Option<u64> {
        self.read_u64(base.checked_add(delta)?)
    }

    fn read_pointer_field(&self, base: u64, delta: u64) -> Option<u64> {
        self.read_pointer(base.checked_add(delta)?)
    }

    fn read_module_pointer(&self, rva: u64) -> Option<u64> {
        self.read_pointer(self.module_base().checked_add(rva)?)
    }
}

impl<T: PassiveMemory + ?Sized> PassiveMemoryExt for T {}

#[derive(Clone, Debug, Default)]
pub struct SliceMemory {
    module_base: u64,
    regions: Vec<(u64, Vec<u8>)>,
}

impl SliceMemory {
    pub fn new(module_base: u64) -> Self {
        Self {
            module_base,
            regions: Vec::new(),
        }
    }

    pub fn map(&mut self, base: u64, bytes: Vec<u8>) -> &mut Self {
        let end = base + bytes.len() as u64;
        for (other_base, other) in &self.regions {
            let other_end = other_base + other.len() as u64;
            assert!(
                end <= *other_base || base >= other_end,
                "SliceMemory fixture regions overlap at {base:#x}"
            );
        }
        self.regions.push((base, bytes));
        self
    }

    pub fn map_object(&mut self, base: u64, len: usize) -> &mut Self {
        self.map(base, vec![0u8; len])
    }

    fn region_mut(&mut self, addr: u64) -> Option<(&mut Vec<u8>, usize)> {
        for (base, bytes) in &mut self.regions {
            if addr >= *base && addr < *base + bytes.len() as u64 {
                let offset = (addr - *base) as usize;
                return Some((bytes, offset));
            }
        }
        None
    }

    pub fn put_u64(&mut self, addr: u64, value: u64) -> &mut Self {
        let (bytes, offset) = self
            .region_mut(addr)
            .unwrap_or_else(|| panic!("fixture write outside a mapped region at {addr:#x}"));
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        self
    }

    pub fn put_u32(&mut self, addr: u64, value: u32) -> &mut Self {
        let (bytes, offset) = self
            .region_mut(addr)
            .unwrap_or_else(|| panic!("fixture write outside a mapped region at {addr:#x}"));
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        self
    }

    pub fn put_bytes(&mut self, addr: u64, value: &[u8]) -> &mut Self {
        let (bytes, offset) = self
            .region_mut(addr)
            .unwrap_or_else(|| panic!("fixture write outside a mapped region at {addr:#x}"));
        bytes[offset..offset + value.len()].copy_from_slice(value);
        self
    }
}

impl PassiveMemory for SliceMemory {
    fn read(&self, addr: u64, out: &mut [u8]) -> bool {
        let Some(end) = addr.checked_add(out.len() as u64) else {
            return false;
        };
        for (base, bytes) in &self.regions {
            let region_end = base + bytes.len() as u64;
            if addr >= *base && end <= region_end {
                let offset = (addr - base) as usize;
                out.copy_from_slice(&bytes[offset..offset + out.len()]);
                return true;
            }
        }
        false
    }

    fn module_base(&self) -> u64 {
        self.module_base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_crossing_the_end_of_a_region_is_unreadable() {
        let mut memory = SliceMemory::new(0x1_0000_0000);
        memory.map(0x2000, vec![0xAA; 16]);
        let mut buf = [0u8; 8];
        assert!(memory.read(0x2008, &mut buf));
        assert!(!memory.read(0x2009, &mut buf));
        assert!(!memory.read(0x3000, &mut buf));
    }

    #[test]
    fn field_addresses_that_wrap_are_unavailable_not_wrapped() {
        let mut memory = SliceMemory::new(0x1_0000_0000);
        memory.map_object(0x0, 64);
        memory.put_u64(0x8, 0xdead_beef_0000_0000);
        assert_eq!(memory.read_u64_field(u64::MAX - 4, 0x10), None);
        assert_eq!(memory.read_pointer_field(u64::MAX - 4, 0x10), None);
    }

    #[test]
    fn pointer_fields_refuse_null_and_misalignment() {
        let mut memory = SliceMemory::new(0x1_0000_0000);
        memory.map_object(0x2000, 32);
        memory.put_u64(0x2000, 0);
        memory.put_u64(0x2008, 0x3001);
        memory.put_u64(0x2010, 0x3000);
        assert_eq!(memory.read_pointer(0x2000), None);
        assert_eq!(memory.read_pointer(0x2008), None);
        assert_eq!(memory.read_pointer(0x2010), Some(0x3000));
    }
}
