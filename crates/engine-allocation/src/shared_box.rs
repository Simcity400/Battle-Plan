
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering, fence};

struct Inner<T> {
    owners: AtomicUsize,
    value: T,
}

pub struct SharedBox<T> {
    inner: NonNull<Inner<T>>,
    _owns: PhantomData<Inner<T>>,
}

// SAFETY: owners on several threads share `&T`, which `T: Sync` allows, and whichever owner is
// last drops `T` on its own thread, which `T: Send` allows.
unsafe impl<T: Send + Sync> Send for SharedBox<T> {}
// SAFETY: a shared `SharedBox<T>` yields only `&T` and new owners, which `T: Send + Sync` allow
// on any thread.
unsafe impl<T: Send + Sync> Sync for SharedBox<T> {}

pub fn shared<T>(value: T) -> io::Result<SharedBox<T>> {
    let inner = crate::boxed(Inner {
        owners: AtomicUsize::new(1),
        value,
    })?;
    Ok(SharedBox {
        inner: NonNull::from(Box::leak(inner)),
        _owns: PhantomData,
    })
}

impl<T> SharedBox<T> {
    pub const BYTES: usize = std::mem::size_of::<Inner<T>>();

    fn inner(&self) -> &Inner<T> {
        // SAFETY: the allocation lives while any owner does, and self is one.
        unsafe { self.inner.as_ref() }
    }

    pub fn get_mut(this: &mut Self) -> Option<&mut T> {
        if this.inner().owners.load(Ordering::Acquire) == 1 {
            // SAFETY: the only owner, borrowed mutably: no other reference exists or can be made.
            Some(unsafe { &mut this.inner.as_mut().value })
        } else {
            None
        }
    }

    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        this.inner == other.inner
    }

    pub fn as_ptr(this: &Self) -> *const T {
        &raw const this.inner().value
    }

    pub fn strong_count(this: &Self) -> usize {
        this.inner().owners.load(Ordering::Acquire)
    }
}

impl<T> Clone for SharedBox<T> {
    fn clone(&self) -> Self {
        if self.inner().owners.fetch_add(1, Ordering::Relaxed) > isize::MAX as usize {
            std::process::abort();
        }
        Self {
            inner: self.inner,
            _owns: PhantomData,
        }
    }
}

impl<T> Drop for SharedBox<T> {
    fn drop(&mut self) {
        if self.inner().owners.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        // SAFETY: the last owner; `shared` made the allocation as a Box.
        drop(unsafe { Box::from_raw(self.inner.as_ptr()) });
    }
}

impl<T> Deref for SharedBox<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner().value
    }
}

impl<T: fmt::Debug> fmt::Debug for SharedBox<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl<T: PartialEq> PartialEq for SharedBox<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq> Eq for SharedBox<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Tracked<'a>(&'a AtomicUsize, u64);
    impl Drop for Tracked<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn clones_share_one_value_and_only_the_last_drop_frees_it() {
        let drops = AtomicUsize::new(0);
        let first = shared(Tracked(&drops, 53)).unwrap();
        let second = first.clone();
        assert!(SharedBox::ptr_eq(&first, &second));
        assert_eq!(SharedBox::strong_count(&first), 2);
        drop(first);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(second.1, 53);
        assert_eq!(SharedBox::strong_count(&second), 1);
        drop(second);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let other = shared(Tracked(&drops, 53)).unwrap();
        assert!(!SharedBox::ptr_eq(
            &other,
            &shared(Tracked(&drops, 53)).unwrap()
        ));
    }

    #[test]
    fn the_value_is_mutable_only_while_one_owner_holds_it() {
        let mut only = shared(vec![1_u64]).unwrap();
        SharedBox::get_mut(&mut only).unwrap().push(2);
        let held = only.clone();
        assert!(SharedBox::get_mut(&mut only).is_none());
        drop(held);
        SharedBox::get_mut(&mut only).unwrap().push(3);
        assert_eq!(*only, [1, 2, 3]);
    }

    #[test]
    fn a_refused_allocation_is_an_error_and_drops_the_value() {
        let drops = AtomicUsize::new(0);
        let refused =
            crate::test_support::with_allocations_denied(|| shared(Tracked(&drops, 53)).is_err());
        assert!(refused);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(*shared(53_u64).unwrap(), 53);
    }

    #[test]
    fn owners_on_many_threads_free_the_value_exactly_once() {
        fn shareable<T: Send + Sync>() {}
        shareable::<SharedBox<std::sync::Mutex<u64>>>();
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        let value = shared(Tracked(&DROPS, 53)).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let value = &value;
                scope.spawn(move || {
                    for _ in 0..1_000 {
                        let held = value.clone();
                        assert_eq!(held.1, 53);
                    }
                });
            }
        });
        assert_eq!(SharedBox::strong_count(&value), 1);
        let last = value.clone();
        drop(value);
        assert_eq!(DROPS.load(Ordering::SeqCst), 0);
        drop(last);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }
}
