//! The read primitive every law in this crate is written against.
//!
//! The crate deliberately does NOT contain the collector's SEH-protected
//! `read_u64`/`read_u8`. A producer law is a statement about offsets and
//! guards; the mechanism that turns an address into bytes without faulting is
//! the collector's, and mixing the two is what makes read laws untestable.
//! The collector supplies one adapter implementing [`PassiveMemory`]; every
//! test in this crate supplies [`SliceMemory`].
//!
//! # Every failure is `None`, and every `None` becomes a named refusal
//!
//! [`PassiveMemory::read`] returns `false` for an unreadable span. It never
//! returns partially-filled bytes: a half-read pointer is indistinguishable
//! from a torn one, and both must refuse.

/// Read-only access to another process's (or this process's) address space.
///
/// Implementations must be side-effect free and must never execute engine code.
pub trait PassiveMemory {
    /// Fill `out` from `addr`, or report that the whole span was not readable.
    ///
    /// A partial read MUST return `false` and leave `out` unspecified.
    fn read(&self, addr: u64, out: &mut [u8]) -> bool;

    /// The base address the module's RVAs are relative to.
    fn module_base(&self) -> u64;
}

/// Helpers shared by every law. Blanket-implemented; not overridable, so a
/// reader family cannot quietly widen one of them.
pub trait PassiveMemoryExt: PassiveMemory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        let mut buf = [0u8; 1];
        self.read(addr, &mut buf).then(|| buf[0])
    }

    fn read_u32(&self, addr: u64) -> Option<u32> {
        let mut buf = [0u8; 4];
        self.read(addr, &mut buf)
            .then(|| u32::from_le_bytes(buf))
    }

    fn read_u64(&self, addr: u64) -> Option<u64> {
        let mut buf = [0u8; 8];
        self.read(addr, &mut buf)
            .then(|| u64::from_le_bytes(buf))
    }

    /// A pointer field, with the two structural checks every chain in this
    /// crate applies before following it: non-null and 8-byte aligned.
    ///
    /// Returns `None` for null so the caller can name *which* link was null;
    /// there is no variant that turns a null pointer into a zero value.
    fn read_pointer(&self, addr: u64) -> Option<u64> {
        let value = self.read_u64(addr)?;
        (value != 0 && value % 8 == 0).then_some(value)
    }

    fn read_f32_bits(&self, addr: u64) -> Option<u32> {
        self.read_u32(addr)
    }

    /// `*(u64*)(base + delta)`, with the address arithmetic CHECKED.
    ///
    /// `base` is engine-supplied, so an unchecked `base + delta` near the top
    /// of the address space would wrap and read a low address instead — a
    /// wrapped read that happens to land on mapped memory is the worst kind of
    /// failure, because it succeeds. Overflow is folded into `None` on purpose:
    /// every caller already names WHICH link was unavailable, and "this link is
    /// unavailable" is the same fact whether the cause was a wrap or an
    /// unmapped page.
    fn read_u64_field(&self, base: u64, delta: u64) -> Option<u64> {
        self.read_u64(base.checked_add(delta)?)
    }

    /// A pointer field at `base + delta`, checked and validated as in
    /// [`PassiveMemoryExt::read_pointer`].
    fn read_pointer_field(&self, base: u64, delta: u64) -> Option<u64> {
        self.read_pointer(base.checked_add(delta)?)
    }

    /// Read at `module_base + rva`.
    fn read_module_pointer(&self, rva: u64) -> Option<u64> {
        self.read_pointer(self.module_base().checked_add(rva)?)
    }
}

impl<T: PassiveMemory + ?Sized> PassiveMemoryExt for T {}

/// A flat, explicitly mapped address space for tests.
///
/// Regions are `(base, bytes)`. A read is served only when the WHOLE span
/// falls inside one region, which is what makes a test able to prove that a
/// law refuses on an unmapped page instead of reading adjacent memory.
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

    /// Map `bytes` at `base`. Panics on an overlap, because an overlapping
    /// fixture is a fixture bug that would otherwise silently pick a winner.
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

    /// Map a zeroed object of `len` bytes at `base` and return a writer for it.
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

    /// kills: serving a read that straddles the end of a mapped region from
    /// the region's tail (i.e. dropping the `end <= region_end` bound).
    #[test]
    fn a_span_crossing_the_end_of_a_region_is_unreadable() {
        let mut memory = SliceMemory::new(0x1_0000_0000);
        memory.map(0x2000, vec![0xAA; 16]);
        let mut buf = [0u8; 8];
        assert!(memory.read(0x2008, &mut buf));
        assert!(!memory.read(0x2009, &mut buf));
        assert!(!memory.read(0x3000, &mut buf));
    }

    /// kills: dropping the checked address arithmetic in the `*_field`
    /// helpers, which would let `base + delta` wrap past `u64::MAX` and read a
    /// low address that may well be mapped.
    #[test]
    fn field_addresses_that_wrap_are_unavailable_not_wrapped() {
        let mut memory = SliceMemory::new(0x1_0000_0000);
        memory.map_object(0x0, 64);
        memory.put_u64(0x8, 0xdead_beef_0000_0000);
        assert_eq!(memory.read_u64_field(u64::MAX - 4, 0x10), None);
        assert_eq!(memory.read_pointer_field(u64::MAX - 4, 0x10), None);
    }

    /// kills: accepting a misaligned or null pointer field in `read_pointer`.
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
