use std::sync::atomic::{AtomicUsize, Ordering};

static SLOTS_HINT: AtomicUsize = AtomicUsize::new(0);

pub fn note_slots(slots: usize) {
    SLOTS_HINT.fetch_max(slots, Ordering::Relaxed);
}

pub fn census_rows(minimum: usize) -> usize {
    rows(SLOTS_HINT.load(Ordering::Relaxed), 1).max(minimum)
}

pub fn reserve_census<T>(census: &mut Vec<T>) -> bool {
    engine_allocation::advisory(|| reserve_rows(census, census_rows(0)))
}

fn reserve_rows<T>(census: &mut Vec<T>, target: usize) -> bool {
    if census.capacity() >= target {
        return false;
    }
    engine_allocation::reserve_exact(census, target - census.len()).is_ok()
}

pub fn rows(slots: usize, per_country: usize) -> usize {
    slots
        .saturating_mul(per_country)
        .saturating_mul(4)
        .div_ceil(3)
}

pub fn raise(capacity: &mut usize, required: usize) {
    *capacity = (*capacity).max(required);
}

#[cfg(any(test, feature = "test-support"))]
pub fn hint_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    SLOTS_HINT.store(0, Ordering::Relaxed);
    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    const COUNTRIES: usize = 5_000;

    #[test]
    fn rows_keep_growth_headroom_and_never_lower_capacity() {
        assert_eq!(rows(0, 1), 0);
        assert_eq!(rows(15_000, 1), 20_000);
        assert_eq!(rows(15_000, 2), 40_000);
        let mut capacity = 30_000;
        raise(&mut capacity, rows(15_000, 1));
        assert_eq!(capacity, 30_000);
        raise(&mut capacity, rows(30_000, 1));
        assert_eq!(capacity, 40_000);
    }

    #[test]
    fn job_censuses_reserve_one_row_per_slot_with_headroom_only_once() {
        let mut census = Vec::<(usize, u32)>::with_capacity(512);
        assert!(reserve_rows(&mut census, rows(COUNTRIES + 1, 1)));
        assert!(census.capacity() > COUNTRIES);
        assert!(!reserve_rows(&mut census, rows(COUNTRIES + 1, 1)));
        assert!(!reserve_rows(&mut census, rows(10, 1)));
    }
}

pub const MIN_COUNTRY_CELLS: usize = 8192;
