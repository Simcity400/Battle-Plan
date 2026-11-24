
pub mod labels;
mod shared_box;
pub mod spill;
#[cfg(feature = "zstd")]
pub mod zstd;

use std::alloc::{Layout, alloc};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash};
use std::io;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};

pub use shared_box::{SharedBox, shared};

thread_local! {
    static ADVISORY_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CONTAINED_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CONTAINED_REFUSALS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static FINISHING_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static GROWTHS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static COLLECTED: std::cell::Cell<Option<std::time::Instant>> =
        const { std::cell::Cell::new(None) };
}

static COLLECT: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
const COLLECT_EVERY: std::time::Duration = std::time::Duration::from_millis(25);

pub fn set_collect(collect: Option<fn()>) {
    COLLECT.store(
        collect.map_or(std::ptr::null_mut(), |f| f as *mut ()),
        Ordering::Release,
    );
}

pub fn collect() {
    let callback = COLLECT.load(Ordering::Acquire);
    if callback.is_null() {
        return;
    }
    let now = std::time::Instant::now();
    if COLLECTED
        .get()
        .is_some_and(|at| now.duration_since(at) < COLLECT_EVERY)
    {
        return;
    }
    COLLECTED.set(Some(now));
    unsafe { std::mem::transmute::<*mut (), fn()>(callback)() };
}

pub fn growths() -> u64 {
    GROWTHS.get()
}

pub fn advisory<T>(read: impl FnOnce() -> T) -> T {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            ADVISORY_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
    ADVISORY_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _scope = Scope;
    read()
}

pub fn contained<T>(work: impl FnOnce() -> T) -> (T, bool) {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            CONTAINED_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
    let before = CONTAINED_REFUSALS.get();
    CONTAINED_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let scope = Scope;
    let value = work();
    drop(scope);
    (value, CONTAINED_REFUSALS.get() != before)
}

fn contained_active() -> bool {
    CONTAINED_DEPTH.with(|depth| depth.get() != 0)
}

pub fn finishing<T>(work: impl FnOnce() -> T) -> T {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            FINISHING_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
    FINISHING_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _scope = Scope;
    work()
}

fn finishing_active() -> bool {
    FINISHING_DEPTH.with(|depth| depth.get() != 0)
}

static FAILED: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static STOPPED: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
#[cfg(feature = "alloc-fault-injection")]
static BEFORE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

pub fn set_hooks(before: Option<fn() -> bool>, failed: Option<fn()>) {
    #[cfg(feature = "alloc-fault-injection")]
    BEFORE.store(
        before.map_or(std::ptr::null_mut(), |f| f as *mut ()),
        Ordering::Release,
    );
    #[cfg(not(feature = "alloc-fault-injection"))]
    let _ = before;
    FAILED.store(
        failed.map_or(std::ptr::null_mut(), |f| f as *mut ()),
        Ordering::Release,
    );
}

pub fn set_stop_hook(stopped: Option<fn() -> bool>) {
    STOPPED.store(
        stopped.map_or(std::ptr::null_mut(), |f| f as *mut ()),
        Ordering::Release,
    );
}

pub fn capture_active() -> bool {
    !FAILED.load(Ordering::Acquire).is_null()
}

pub fn capture_stopped() -> bool {
    let stopped = STOPPED.load(Ordering::Acquire);
    !stopped.is_null() && unsafe { std::mem::transmute::<*mut (), fn() -> bool>(stopped)() }
}

pub fn oom() -> io::Error {
    if contained_active() && !advisory_active() {
        CONTAINED_REFUSALS.set(CONTAINED_REFUSALS.get().wrapping_add(1));
    } else {
        let callback = FAILED.load(Ordering::Acquire);
        if !callback.is_null() && !advisory_active() {
            unsafe { std::mem::transmute::<*mut (), fn()>(callback)() };
        }
    }
    io::Error::from(io::ErrorKind::OutOfMemory)
}

fn advisory_active() -> bool {
    ADVISORY_DEPTH.with(|depth| depth.get() != 0)
}

pub fn allocation_attempt() -> io::Result<()> {
    if capture_stopped() && !finishing_active() {
        return Err(io::Error::from(io::ErrorKind::OutOfMemory));
    }
    #[cfg(feature = "alloc-fault-injection")]
    {
        let callback = BEFORE.load(Ordering::Acquire);
        if !advisory_active() && !callback.is_null()
            && unsafe { std::mem::transmute::<*mut (), fn() -> bool>(callback)() }
        {
            return Err(oom());
        }
    }
    GROWTHS.set(GROWTHS.get().saturating_add(1));
    Ok(())
}

pub fn add(left: usize, right: usize) -> io::Result<usize> {
    left.checked_add(right).ok_or_else(oom)
}

pub fn mul(left: usize, right: usize) -> io::Result<usize> {
    left.checked_mul(right).ok_or_else(oom)
}

pub fn is_allocation_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::OutOfMemory
        || matches!(error.raw_os_error(), Some(8 | 14 | 1450..=1455 | 1816))
}

pub fn allocation_error(error: io::Error) -> io::Error {
    if is_allocation_error(&error) {
        oom()
    } else {
        error
    }
}

pub fn owned_vec<T>(result: io::Result<Vec<T>>) -> Vec<T> {
    match result {
        Ok(values) => values,
        Err(error) => {
            let advisory = advisory_active() || contained_active();
            assert!(is_allocation_error(&error) && (advisory || capture_active()));
            let _ = oom();
            assert!(advisory || capture_stopped());
            Vec::new()
        }
    }
}

pub fn boxed<T>(value: T) -> io::Result<Box<T>> {
    let layout = Layout::new::<T>();
    let pointer = if layout.size() == 0 {
        NonNull::<T>::dangling().as_ptr()
    } else {
        allocation_attempt()?;
        unsafe { alloc(layout).cast::<T>() }
    };
    if pointer.is_null() {
        return Err(oom());
    }
    // SAFETY: storage is aligned and writable for T (or dangling for a ZST).
    // The value moves exactly once and Box owns the matching allocation.
    unsafe {
        pointer.write(value);
        Ok(Box::from_raw(pointer))
    }
}

pub fn boxed_uninit<T>() -> io::Result<Box<std::mem::MaybeUninit<T>>> {
    let layout = Layout::new::<std::mem::MaybeUninit<T>>();
    let pointer = if layout.size() == 0 {
        NonNull::<std::mem::MaybeUninit<T>>::dangling().as_ptr()
    } else {
        allocation_attempt()?;
        // SAFETY: layout has non-zero size matching MaybeUninit<T>.
        unsafe { alloc(layout).cast::<std::mem::MaybeUninit<T>>() }
    };
    if pointer.is_null() {
        return Err(oom());
    }
    // SAFETY: storage matches the global allocator layout for MaybeUninit<T>
    // (or is dangling for a ZST), and uninitialized bytes are valid MaybeUninit<T>.
    Ok(unsafe { Box::from_raw(pointer) })
}

pub fn reserve<T>(values: &mut Vec<T>, additional: usize) -> io::Result<()> {
    if additional > values.capacity() - values.len() {
        allocation_attempt()?;
        values.try_reserve(additional).map_err(|_| oom())?;
    }
    Ok(())
}

pub fn reserve_exact<T>(values: &mut Vec<T>, additional: usize) -> io::Result<()> {
    if additional > values.capacity() - values.len() {
        allocation_attempt()?;
        values.try_reserve_exact(additional).map_err(|_| oom())?;
    }
    Ok(())
}

pub fn reserve_push_batch<T>(values: &mut Vec<T>, additional: usize) -> io::Result<()> {
    let required = values.len().checked_add(additional).ok_or_else(oom)?;
    if required <= values.capacity() {
        return Ok(());
    }
    let size = std::mem::size_of::<T>();
    let minimum = if size == 1 {
        8
    } else if size <= 1024 {
        4
    } else {
        1
    };
    let mut capacity = values.capacity();
    while capacity < required {
        capacity = capacity.checked_mul(2).ok_or_else(oom)?.max(minimum);
    }
    reserve_exact(values, capacity - values.len())
}

pub fn vec<T>(capacity: usize) -> io::Result<Vec<T>> {
    let mut values = Vec::new();
    reserve_exact(&mut values, capacity)?;
    Ok(values)
}

pub fn filled<T: Copy>(len: usize, value: T) -> io::Result<Vec<T>> {
    let mut values = vec(len)?;
    values.resize_with(len, || value);
    Ok(values)
}

pub fn default_vec<T: Default>(len: usize) -> io::Result<Vec<T>> {
    let mut values = vec(len)?;
    values.resize_with(len, T::default);
    Ok(values)
}

pub fn resize_default<T: Default>(values: &mut Vec<T>, len: usize) -> io::Result<()> {
    reserve(values, len.saturating_sub(values.len()))?;
    values.resize_with(len, T::default);
    Ok(())
}

pub fn copy_vec<T: Copy>(values: &[T]) -> io::Result<Vec<T>> {
    let mut copy = vec(values.len())?;
    copy.extend_from_slice(values);
    Ok(copy)
}

pub fn map_slice<T, U>(values: &[T], map: impl FnMut(&T) -> U) -> io::Result<Vec<U>> {
    let mut output = vec(values.len())?;
    output.extend(values.iter().map(map));
    Ok(output)
}

pub fn collect_vec<T>(values: impl IntoIterator<Item = T>) -> io::Result<Vec<T>> {
    let values = values.into_iter();
    let (minimum, maximum) = values.size_hint();
    let mut output = vec(maximum.unwrap_or(minimum))?;
    if maximum.is_some() {
        output.extend(values);
    } else {
        for value in values {
            if output.len() == output.capacity() {
                reserve_push_batch(&mut output, 1)?;
            }
            output.push(value);
        }
    }
    Ok(output)
}

pub fn resize<T: Copy>(values: &mut Vec<T>, len: usize, value: T) -> io::Result<()> {
    reserve(values, len.saturating_sub(values.len()))?;
    values.resize_with(len, || value);
    Ok(())
}

pub fn string(value: &str) -> io::Result<String> {
    let mut copy = String::new();
    reserve_string(&mut copy, value.len())?;
    copy.push_str(value);
    Ok(copy)
}

pub fn reserve_string(value: &mut String, additional: usize) -> io::Result<()> {
    if additional > value.capacity() - value.len() {
        allocation_attempt()?;
        value.try_reserve(additional).map_err(|_| oom())?;
    }
    Ok(())
}

pub fn reserve_map<K: Eq + Hash, V, S: BuildHasher>(
    values: &mut HashMap<K, V, S>,
    additional: usize,
) -> io::Result<()> {
    if additional > values.capacity() - values.len() {
        allocation_attempt()?;
        values.try_reserve(additional).map_err(|_| oom())?;
    }
    Ok(())
}

pub fn reserve_set<T: Eq + Hash, S: BuildHasher>(
    values: &mut HashSet<T, S>,
    additional: usize,
) -> io::Result<()> {
    if additional > values.capacity() - values.len() {
        allocation_attempt()?;
        values.try_reserve(additional).map_err(|_| oom())?;
    }
    Ok(())
}

pub fn boxed_slice<T>(mut values: Vec<T>) -> io::Result<Box<[T]>> {
    let len = values.len();
    if len == 0 {
        return Ok(unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                NonNull::<T>::dangling().as_ptr(),
                0,
            ))
        });
    }
    if len == values.capacity() || std::mem::size_of::<T>() == 0 {
        let pointer = values.as_mut_ptr();
        std::mem::forget(values);
        return Ok(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(pointer, len)) });
    }
    let layout = Layout::array::<T>(len).map_err(|_| oom())?;
    allocation_attempt()?;
    let pointer = unsafe { alloc(layout).cast::<T>() };
    if pointer.is_null() {
        return Err(oom());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(values.as_ptr(), pointer, len);
        values.set_len(0);
        Ok(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            pointer, len,
        )))
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn growths_count_real_growth_but_not_reserves_that_fit() {
        let mut values = Vec::<u64>::new();
        let before = growths();
        reserve(&mut values, 1).unwrap();
        assert_eq!(growths() - before, 1);
        values.push(7);
        let spare = values.capacity() - values.len();
        reserve(&mut values, spare).unwrap();
        reserve_exact(&mut values, spare).unwrap();
        reserve_push_batch(&mut values, spare).unwrap();
        assert_eq!(growths() - before, 1);
        reserve_exact(&mut values, spare + 1).unwrap();
        assert_eq!(growths() - before, 2);
    }

    #[global_allocator]
    static ALLOCATOR: super::test_support::DeniedAllocator = super::test_support::DeniedAllocator;

    #[cfg(feature = "alloc-fault-injection")]
    use super::test_support::with_allocations_denied;

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    #[ignore = "run alone: allocation hooks are process-global"]
    fn advisory_owned_constructor_denial_does_not_require_terminal_state() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        static STOP: AtomicBool = AtomicBool::new(false);
        static FAILURES: AtomicUsize = AtomicUsize::new(0);
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        fn before() -> bool {
            ATTEMPTS.fetch_add(1, Ordering::SeqCst);
            true
        }
        fn failed() {
            FAILURES.fetch_add(1, Ordering::SeqCst);
            STOP.store(true, Ordering::SeqCst);
        }
        fn stopped() -> bool {
            STOP.load(Ordering::SeqCst)
        }
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                set_hooks(None, None);
                set_stop_hook(None);
            }
        }
        let _reset = Reset;
        STOP.store(false, Ordering::SeqCst);
        FAILURES.store(0, Ordering::SeqCst);
        ATTEMPTS.store(0, Ordering::SeqCst);
        set_hooks(Some(before), Some(failed));
        set_stop_hook(Some(stopped));
        advisory(|| {
            struct Tracked;
            impl Drop for Tracked {
                fn drop(&mut self) {
                    DROPS.fetch_add(1, Ordering::SeqCst);
                }
            }
            static DROPS: AtomicUsize = AtomicUsize::new(0);
            let denied = with_allocations_denied(|| boxed((Tracked, 1_u64)));
            assert!(denied.is_err());
            assert_eq!(DROPS.load(Ordering::SeqCst), 1);
            let values =
                with_allocations_denied(|| advisory(|| owned_vec(default_vec::<u64>(5_001))));
            assert!(values.is_empty());
            assert!(!capture_stopped());
            assert_eq!(FAILURES.load(Ordering::SeqCst), 0);
            assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 0);
            assert_eq!(boxed(53_u64).unwrap().as_ref(), &53);
        });
        let values = owned_vec(default_vec::<u64>(5_001));
        assert!(values.is_empty());
        assert!(capture_stopped());
        assert!(FAILURES.load(Ordering::SeqCst) > 0);
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    fn real_allocator_denial_preserves_retained_storage_and_advisory_owned_fallback() {
        let mut retained = vec::<u64>(2).unwrap();
        retained.push(53);
        let capacity = retained.capacity();
        let denied = with_allocations_denied(|| {
            advisory(|| {
                assert!(reserve(&mut retained, 5_001).is_err());
                assert!(boxed(53_u64).is_err());
                owned_vec(default_vec::<u64>(5_001))
            })
        });
        assert!(denied.is_empty());
        assert_eq!(retained, [53]);
        assert_eq!(retained.capacity(), capacity);
    }

    #[test]
    fn contained_work_reports_its_own_refusals_not_advisory_or_another_threads() {
        let mut values = vec::<u64>(1).unwrap();
        let (result, refused) = contained(|| reserve(&mut values, usize::MAX));
        assert!(result.is_err() && refused);
        assert!(!contained(|| reserve(&mut values, 1)).1);
        let ((), refused) = contained(|| {
            assert!(advisory(|| reserve(&mut values, usize::MAX)).is_err());
        });
        assert!(!refused);
        let (empty, refused) = contained(|| owned_vec(vec::<u64>(usize::MAX)));
        assert!(empty.is_empty() && refused);
        assert!(contained(|| contained(|| reserve(&mut values, usize::MAX)).1).1);
        let ((), refused) = contained(|| {
            std::thread::spawn(|| {
                assert!(!contained_active(), "containment does not cross threads");
                assert!(reserve(&mut vec::<u64>(1).unwrap(), usize::MAX).is_err());
            })
            .join()
            .unwrap();
        });
        assert!(!refused);
    }

    #[test]
    fn checked_push_batches_preserve_the_sequential_capacity_trace() {
        fn compare<T: Copy>(value: T) {
            for initial in [0, 1, 3, 8, 19] {
                let mut previous = Vec::<T>::with_capacity(initial);
                let mut checked = vec::<T>(initial).unwrap();
                for count in [1, 5, 0, 17, 3, 64] {
                    reserve_push_batch(&mut checked, count).unwrap();
                    for _ in 0..count {
                        previous.push(value);
                        checked.push(value);
                    }
                    assert_eq!(checked.capacity(), previous.capacity());
                    assert_eq!(checked.len(), previous.len());
                }
            }
        }
        compare(0_u8);
        compare(0_u64);
        compare([0_u8; 1025]);
        compare(());
    }

    #[test]
    fn checked_growth_reports_capacity_overflow_without_allocating_error() {
        let mut bytes = Vec::<u8>::new();
        assert_eq!(
            reserve(&mut bytes, usize::MAX).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        assert!(bytes.is_empty());
    }

    #[test]
    fn checked_collection_uses_bounds_and_handles_unknown_lengths() {
        let copied = map_slice(&[3_u64, 5, 7], |value| value * 2).unwrap();
        assert_eq!(copied, [6, 10, 14]);
        let filtered = collect_vec((0..8).filter(|value| value % 2 == 0)).unwrap();
        assert_eq!(filtered, [0, 2, 4, 6]);
        let mut next = 0;
        let unknown = std::iter::from_fn(|| {
            next += 1;
            (next <= 9).then_some(next)
        });
        assert_eq!(collect_vec(unknown).unwrap(), (1..=9).collect::<Vec<_>>());
        struct Oversized;
        impl Iterator for Oversized {
            type Item = u64;
            fn next(&mut self) -> Option<u64> {
                panic!("must fail before iteration")
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (usize::MAX, Some(usize::MAX))
            }
        }
        assert_eq!(
            collect_vec(Oversized).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
    }

    #[test]
    #[ignore = "run alone: allocation hooks are process-global"]
    fn advisory_failure_preserves_hooks_nesting_unwind_and_other_threads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FAILURES: AtomicUsize = AtomicUsize::new(0);
        fn failed() {
            FAILURES.fetch_add(1, Ordering::SeqCst);
        }
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                set_hooks(None, None);
            }
        }
        let _reset = Reset;
        FAILURES.store(0, Ordering::SeqCst);
        set_hooks(None, Some(failed));
        let mut retained = vec::<u8>(8).unwrap();
        retained.extend_from_slice(b"kept");
        let capacity = retained.capacity();
        advisory(|| {
            advisory(|| assert!(reserve(&mut retained, usize::MAX).is_err()));
            assert!(reserve(&mut retained, usize::MAX).is_err());
            assert_eq!(FAILURES.load(Ordering::SeqCst), 0);
            std::thread::spawn(|| {
                assert!(vec::<u8>(usize::MAX).is_err());
            })
            .join()
            .unwrap();
            assert_eq!(FAILURES.load(Ordering::SeqCst), 1);
        });
        assert_eq!(retained, b"kept");
        assert_eq!(retained.capacity(), capacity);
        assert!(std::panic::catch_unwind(|| advisory(|| panic!("synthetic unwind"))).is_err());
        assert!(vec::<u8>(usize::MAX).is_err());
        assert_eq!(FAILURES.load(Ordering::SeqCst), 2);
        assert_eq!(vec::<u8>(16).unwrap().capacity(), 16);
    }

    #[test]
    fn finishing_work_grows_after_capture_stopped() {
        use std::sync::atomic::{AtomicBool, Ordering};
        const ALONE: &str = "ENGINE_ALLOCATION_FINISHING_ALONE";
        if std::env::var_os(ALONE).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::finishing_work_grows_after_capture_stopped", "--nocapture"])
                .env(ALONE, "1")
                .status()
                .unwrap();
            assert!(status.success(), "{status}");
            return;
        }
        static STOP: AtomicBool = AtomicBool::new(false);
        fn stopped() -> bool {
            STOP.load(Ordering::SeqCst)
        }
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                set_stop_hook(None);
            }
        }
        let _reset = Reset;
        set_stop_hook(Some(stopped));
        STOP.store(true, Ordering::SeqCst);
        assert!(vec::<u8>(16).is_err());
        let finished = finishing(|| finishing(|| vec::<u8>(16)).and_then(|_| vec::<u8>(32)));
        assert_eq!(finished.unwrap().capacity(), 32);
        assert!(vec::<u8>(16).is_err());
        STOP.store(false, Ordering::SeqCst);
        assert!(vec::<u8>(16).is_ok());
    }
}
