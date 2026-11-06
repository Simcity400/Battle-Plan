// Nested capacities: what a buffered fact holds until it is written.
pub trait Resident {
    fn heap_bytes(&self) -> usize;
}
macro_rules! scalar {
    ($($t:ty),*) => {$ (impl Resident for $t {
        fn heap_bytes(&self) -> usize { 0 }
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
}
impl<T: Resident> Resident for Vec<T> {
    fn heap_bytes(&self) -> usize {
        self.capacity()
            .saturating_mul(size_of::<T>())
            .saturating_add(self.iter().fold(0usize, |total, value| {
                total.saturating_add(value.heap_bytes())
            }))
    }
}
impl<T: Resident> Resident for Option<T> {
    fn heap_bytes(&self) -> usize {
        self.as_ref().map_or(0, Resident::heap_bytes)
    }
}
impl<A: Resident, B: Resident> Resident for (A, B) {
    fn heap_bytes(&self) -> usize {
        self.0.heap_bytes().saturating_add(self.1.heap_bytes())
    }
}
