pub use engine_allocation::credit::{Credit, MemoryBudget};
use std::io;

#[derive(Clone, Copy, Debug)]
pub struct DecodeBudget {
    pub fixed_bytes: usize,
    pub bytes_per_encoded_byte: usize,
}
impl DecodeBudget {
    pub fn for_encoded(self, bytes: usize) -> io::Result<usize> {
        bytes
            .checked_mul(self.bytes_per_encoded_byte)
            .and_then(|n| n.checked_add(self.fixed_bytes))
            .ok_or_else(|| super::invalid("Fact allocation overflow"))
    }
}
// Account nested capacities and bound peak owned allocation, including decoder scratch.
pub trait Resident {
    fn heap_bytes(&self) -> usize;
    fn decode_budget() -> DecodeBudget;
}
macro_rules! scalar {
    ($($t:ty),*) => {$ (impl Resident for $t {
        fn heap_bytes(&self) -> usize { 0 }
        fn decode_budget() -> DecodeBudget { DecodeBudget { fixed_bytes: size_of::<Self>(), bytes_per_encoded_byte: 0 } }
    })*};
}
scalar!(
    (),
    bool,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64
);
impl Resident for String {
    fn heap_bytes(&self) -> usize {
        self.capacity()
    }
    fn decode_budget() -> DecodeBudget {
        DecodeBudget {
            fixed_bytes: size_of::<Self>(),
            bytes_per_encoded_byte: 4,
        }
    }
}
impl<T: Resident> Resident for Vec<T> {
    fn heap_bytes(&self) -> usize {
        self.capacity()
            .saturating_mul(size_of::<T>())
            .saturating_add(self.iter().fold(0usize, |total, value| {
                total.saturating_add(value.heap_bytes())
            }))
    }
    fn decode_budget() -> DecodeBudget {
        let element = T::decode_budget();
        DecodeBudget {
            fixed_bytes: size_of::<Self>(),
            bytes_per_encoded_byte: element
                .fixed_bytes
                .saturating_add(element.bytes_per_encoded_byte)
                .saturating_add(size_of::<T>().saturating_mul(4)),
        }
    }
}
impl<T: Resident> Resident for Option<T> {
    fn heap_bytes(&self) -> usize {
        self.as_ref().map_or(0, Resident::heap_bytes)
    }
    fn decode_budget() -> DecodeBudget {
        let value = T::decode_budget();
        DecodeBudget {
            fixed_bytes: value.fixed_bytes.saturating_add(size_of::<Self>()),
            ..value
        }
    }
}
impl<A: Resident, B: Resident> Resident for (A, B) {
    fn heap_bytes(&self) -> usize {
        self.0.heap_bytes().saturating_add(self.1.heap_bytes())
    }
    fn decode_budget() -> DecodeBudget {
        let a = A::decode_budget();
        let b = B::decode_budget();
        DecodeBudget {
            fixed_bytes: a.fixed_bytes.saturating_add(b.fixed_bytes),
            bytes_per_encoded_byte: a
                .bytes_per_encoded_byte
                .saturating_add(b.bytes_per_encoded_byte),
        }
    }
}
