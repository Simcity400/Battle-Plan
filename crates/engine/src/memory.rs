use std::mem::size_of;

/// Implementations fill the destination or return false; a failed copy may be partial.
pub trait Memory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool;
}

mod sealed {
    pub trait Sealed {}
}

/// Padding-free types for which every bit pattern is valid. Sealed to prevent
/// decoding pointers, references, bools, or enums with invalid representations.
pub trait Raw: sealed::Sealed + Copy {
    const ZERO: Self;
}

macro_rules! raw_integers {
    ($($ty:ty),* $(,)?) => { $(
        impl sealed::Sealed for $ty {}
        impl Raw for $ty { const ZERO: Self = 0; }
    )* };
}
raw_integers!(u8, i8, u16, i16, u32, i32, u64, i64);
impl<const N: usize> sealed::Sealed for [u8; N] {}
impl<const N: usize> Raw for [u8; N] {
    const ZERO: Self = [0; N];
}

pub fn read<T: Raw>(memory: &impl Memory, address: usize) -> Option<T> {
    let mut value = T::ZERO;
    // SAFETY: Raw is sealed to initialized, padding-free, all-bit-pattern types.
    let bytes = unsafe {
        std::slice::from_raw_parts_mut((&mut value as *mut T).cast::<u8>(), size_of::<T>())
    };
    read_slice(memory, address, bytes).then_some(value)
}

/// The caller owns the buffer. No allocation occurs, including on a read failure.
pub fn read_slice(memory: &impl Memory, address: usize, destination: &mut [u8]) -> bool {
    if destination.is_empty() {
        return true;
    }
    address != 0
        && address.checked_add(destination.len()).is_some()
        && memory.copy(address, destination)
}

/// An immutable, synthetic address space for pure walker tests.
pub struct SliceMemory<'a> {
    pub base: usize,
    pub bytes: &'a [u8],
}

impl Memory for SliceMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some(start) = address.checked_sub(self.base) else {
            return false;
        };
        let Some(end) = start.checked_add(destination.len()) else {
            return false;
        };
        let Some(source) = self.bytes.get(start..end) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
pub struct LiveMemory(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
impl LiveMemory {
    /// # Safety
    /// Use only for foreign game memory under the caller's consistency protocol.
    /// SEH protects against invalid addresses, not concurrent writes or Rust aliasing violations.
    /// This handle is deliberately neither Send nor Sync.
    pub unsafe fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
impl Memory for LiveMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        unsafe extern "C" {
            fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
        }
        if destination.is_empty() {
            return true;
        }
        if address == 0 || address.checked_add(destination.len()).is_none() {
            return false;
        }
        // SAFETY: destination is writable; C catches faults in the foreign source range.
        unsafe {
            seh_guarded_copy(
                destination.as_mut_ptr(),
                address as *const u8,
                destination.len(),
            ) != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unaligned_raw_values_preserve_all_bits_and_refuse_missing_bytes() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[0, 255, 255, 255, 255],
        };
        assert_eq!(read::<i32>(&memory, 0x1001), Some(-1));
        assert_eq!(read::<u32>(&memory, 0x1002), None);
        assert_eq!(read::<u64>(&memory, usize::MAX - 3), None);
        assert_eq!(read::<u32>(&memory, 0), None);
    }
}
