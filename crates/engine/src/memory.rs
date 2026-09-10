use std::mem::size_of;

/// Implementations fill the destination or return false; a failed copy may be partial.
pub trait Memory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool;

    /// Copies fields in order and returns the number fully copied before the first
    /// failure. The failed field may be partial; later destinations are untouched.
    /// Empty fields count as complete without accessing their source address.
    /// The caller owns every buffer; no allocation or retry occurs.
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if destination.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(destination.len()).is_none()
                || !self.copy(address, destination)
            {
                return completed;
            }
        }
        N
    }
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
    /// Use only for foreign game memory under the caller's consistency protocol,
    /// or immutable synthetic storage in tests.
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

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        #[derive(Clone, Copy)]
        #[repr(C)]
        struct Span {
            dst: *mut u8,
            src: *const u8,
            len: usize,
        }
        unsafe extern "C" {
            fn seh_guarded_copy_scattered(spans: *const Span, count: usize) -> usize;
        }
        let mut spans = [Span {
            dst: std::ptr::null_mut(),
            src: std::ptr::null(),
            len: 0,
        }; N];
        let mut count = 0;
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty()
                && (address == 0 || address.checked_add(destination.len()).is_none())
            {
                break;
            }
            spans[index] = Span {
                dst: destination.as_mut_ptr(),
                src: address as *const u8,
                len: destination.len(),
            };
            count = index + 1;
        }
        if count == 0 {
            return 0;
        }
        // SAFETY: the stack metadata and destinations remain valid for this call.
        // C skips empty spans and catches faults only within the current span's
        // ranges, returning the fully copied prefix. It never reads later spans
        // after a fault; address validation also excludes the invalid suffix.
        unsafe { seh_guarded_copy_scattered(spans.as_ptr(), count) }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct TracedMemory {
        calls: RefCell<Vec<usize>>,
        fail_at: usize,
    }

    impl Memory for TracedMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push(address);
            if address == self.fail_at {
                destination[0] = 0x7e;
                false
            } else {
                destination.fill(address as u8);
                true
            }
        }
    }

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

    #[test]
    fn scattered_fields_preserve_unaligned_full_width_values() {
        let mut bytes = [0; 20];
        bytes[1..3].copy_from_slice(&u16::MAX.to_le_bytes());
        bytes[5..9].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
        bytes[11..19].copy_from_slice(&(i64::MIN + 0x1234).to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut narrow = [0; 2];
        let mut medium = [0; 4];
        let mut wide = [0; 8];
        assert_eq!(
            memory.copy_scattered(
                [0x1001, 0x1005, 0x100b],
                &mut [&mut narrow, &mut medium, &mut wide],
            ),
            3
        );
        assert_eq!(u16::from_le_bytes(narrow), u16::MAX);
        assert_eq!(u32::from_le_bytes(medium), u32::MAX - 1);
        assert_eq!(i64::from_le_bytes(wide), i64::MIN + 0x1234);
    }

    #[test]
    fn scattered_failure_returns_only_complete_fields_and_stops_copying() {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: 0x1002,
        };
        let mut first = [0; 2];
        let mut failed = [0; 2];
        let mut later = [0xa5; 2];
        assert_eq!(
            memory.copy_scattered(
                [0x1001, 0x1002, 0x1003],
                &mut [&mut first, &mut failed, &mut later],
            ),
            1
        );
        assert_eq!(*memory.calls.borrow(), [0x1001, 0x1002]);
        assert_eq!(first, [1; 2]);
        assert_eq!(failed, [0x7e, 0]);
        assert_eq!(later, [0xa5; 2]);
    }

    #[test]
    fn scattered_invalid_addresses_leave_the_valid_prefix_copied() {
        for invalid in [0, usize::MAX - 1] {
            let memory = TracedMemory {
                calls: RefCell::new(Vec::new()),
                fail_at: usize::MAX,
            };
            let mut first = [0; 2];
            let mut failed = [0xa5; 2];
            let mut later = [0xa5; 2];
            assert_eq!(
                memory.copy_scattered(
                    [0x1001, invalid, 0x1003],
                    &mut [&mut first, &mut failed, &mut later],
                ),
                1
            );
            assert_eq!(*memory.calls.borrow(), [0x1001]);
            assert_eq!(first, [1; 2]);
            assert_eq!(failed, [0xa5; 2]);
            assert_eq!(later, [0xa5; 2]);
        }
    }

    #[test]
    fn scattered_empty_fields_succeed_without_copying_or_validating_addresses() {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: usize::MAX,
        };
        let mut first_empty = [];
        let mut second_empty = [];
        let mut value = [0; 2];
        assert_eq!(
            memory.copy_scattered(
                [0, usize::MAX, 0x1001],
                &mut [&mut first_empty, &mut second_empty, &mut value],
            ),
            3
        );
        assert_eq!(*memory.calls.borrow(), [0x1001]);
        assert_eq!(value, [1; 2]);
        assert_eq!(memory.copy_scattered([], &mut []), 0);
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_scattered_copy_preserves_the_prefix_before_an_invalid_address() {
        let source = (i64::MIN + 0x1234).to_le_bytes();
        // SAFETY: the only accessed source is immutable, Rust-owned memory that
        // outlives the call. The zero address is rejected before entering C.
        let memory = unsafe { LiveMemory::new() };
        let mut empty = [];
        let mut first = [0; 8];
        let mut failed = [0xa5; 4];
        let mut later = [0xa5; 8];
        assert_eq!(
            memory.copy_scattered(
                [0, source.as_ptr() as usize, 0, source.as_ptr() as usize],
                &mut [&mut empty, &mut first, &mut failed, &mut later],
            ),
            2
        );
        assert_eq!(first, source);
        assert_eq!(failed, [0xa5; 4]);
        assert_eq!(later, [0xa5; 8]);
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_scattered_fault_preserves_completed_fields_and_leaves_later_fields_alone() {
        let source = u64::MAX.to_le_bytes();
        // SAFETY: valid sources are immutable owned bytes. The intentionally
        // inaccessible Windows null-page address faults inside the C SEH shim;
        // it is never dereferenced by Rust or returned as a Rust reference.
        let memory = unsafe { LiveMemory::new() };
        let mut first = [0; 8];
        let mut failed = [0xa5; 4];
        let mut later = [0xa5; 8];
        assert_eq!(
            memory.copy_scattered(
                [source.as_ptr() as usize, 1, source.as_ptr() as usize],
                &mut [&mut first, &mut failed, &mut later],
            ),
            1
        );
        assert_eq!(first, source);
        assert_eq!(later, [0xa5; 8]);
    }
}
