//! World text draw data shared by name and ribbon producers.
pub use map_display_data::landscape::{Placements, WorldName};

/// The next [`Placements::generation`].
pub fn next_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
