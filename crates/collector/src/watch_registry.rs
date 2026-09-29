//! A clearable registry of the receiver addresses a mutation observer
//! watches, for the observers' hooks ([`crate::relation_changes`]).
//!
//! A capture registers each receiver it certifies and takes its
//! `(epoch, sequence)`. A hooked writer bumps the sequence of a registered
//! receiver on entry and on return, and the domain-wide revision either way.
//! Equal values across hours prove that no observed writer touched it.
//!
//! Registrations are dropped only together with an epoch change, so no
//! certificate outlives a notification it could have missed:
//! - [`WatchRegistry::boundary`] runs on the game thread in the hourly
//!   callback, before any capture. It clears the table (on an invalidation,
//!   or once dead receivers fill half of it) or installs a larger table
//!   offered off-thread, each between two epoch changes, and only while no
//!   notification is running (see below for when each waits).
//! - An epoch change makes every receiver be read afresh, so a clear or an
//!   install waits for an invalidating hour (attach, load, gap or recovery:
//!   the recorder's full hours), where that happens anyway. One a running
//!   notification blocked there waits for the next invalidating hour too:
//!   that hour's epoch already revoked every certificate, and dead
//!   registrations only take room. Only the first table installs at once:
//!   nothing is certified without one.
//! - Enabling observation starts an epoch: writers ran unobserved before it.
//!
//! The hook path never allocates or frees. Tables are allocated off-thread
//! ([`WatchRegistry::offer`]); a replaced table is freed off-thread
//! ([`WatchRegistry::collect`]) once no notification can still hold it.
//! Bounded probing refuses a registration when the table is full; that
//! receiver is then read afresh every hour, and the refusal is counted.

use std::{
    ptr,
    sync::atomic::{
        AtomicBool, AtomicPtr, AtomicU64, AtomicUsize,
        Ordering::{Relaxed, SeqCst},
    },
};

/// Slots probed from a receiver's hash before its registration is refused.
const PROBES: usize = 16;
/// Table slots per expected watch: probing stays short, and receivers that
/// died since the last clear have room to accumulate.
const SLOTS_PER_WATCH: usize = 4;
const MIN_SLOTS: usize = 1024;

#[derive(Default)]
struct Slot {
    address: AtomicUsize,
    sequence: AtomicU64,
}

struct Table {
    slots: Box<[Slot]>,
}

enum Lookup<'a> {
    Watched(&'a AtomicU64),
    Registered(&'a AtomicU64),
    Unwatched,
    Full,
}

impl Table {
    /// Off the hook path; `None` when the allocation fails.
    fn new(len: usize) -> Option<Box<Self>> {
        debug_assert!(len.is_power_of_two());
        let mut slots = Vec::new();
        slots.try_reserve_exact(len).ok()?;
        slots.resize_with(len, Slot::default);
        Some(Box::new(Self {
            slots: slots.into_boxed_slice(),
        }))
    }

    fn find(&self, address: usize, register: bool) -> Lookup<'_> {
        if address == 0 {
            return Lookup::Unwatched;
        }
        let mask = self.slots.len() - 1;
        let first = ((address >> 3).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) & mask;
        for probe in 0..PROBES.min(self.slots.len()) {
            let slot = &self.slots[(first + probe) & mask];
            let mut current = slot.address.load(SeqCst);
            if current == 0 && register {
                match slot.address.compare_exchange(0, address, SeqCst, SeqCst) {
                    Ok(_) => return Lookup::Registered(&slot.sequence),
                    Err(winner) => current = winner,
                }
            }
            if current == address {
                return Lookup::Watched(&slot.sequence);
            }
            if current == 0 {
                return Lookup::Unwatched;
            }
        }
        if register {
            Lookup::Full
        } else {
            Lookup::Unwatched
        }
    }

    /// Between two epoch changes, which order these stores.
    fn clear(&self) {
        for slot in &self.slots {
            slot.sequence.store(0, Relaxed);
            slot.address.store(0, Relaxed);
        }
    }
}

/// Frees a table no notification or capture can reach any more.
unsafe fn free(table: *mut Table) {
    if !table.is_null() {
        drop(unsafe { Box::from_raw(table) });
    }
}

/// One observation domain: its watches, epoch, revision and running
/// notifications.
pub struct WatchRegistry {
    /// The installed table; null until a boundary installs one.
    table: AtomicPtr<Table>,
    /// A larger table allocated off-thread, awaiting a boundary.
    offered: AtomicPtr<Table>,
    /// A replaced table that a notification begun before its replacement
    /// may still hold.
    draining: AtomicPtr<Table>,
    /// A replaced table nothing holds, awaiting [`Self::collect`].
    retired: AtomicPtr<Table>,
    /// Slots of the installed table.
    capacity: AtomicUsize,
    /// Slots of the largest table allocated, installed or offered.
    requested: AtomicUsize,
    /// Registrations since the table was last cleared or installed.
    registered: AtomicUsize,
    epoch: AtomicU64,
    revision: AtomicU64,
    active: AtomicUsize,
    enabled: AtomicBool,
    poisoned: AtomicBool,
    refused: AtomicU64,
    clears: AtomicU64,
    installs: AtomicU64,
    deferred: AtomicU64,
    /// Boundaries outside an invalidation that left a wanted clear or an
    /// offered table for the next invalidating hour.
    held: AtomicU64,
    failed_offers: AtomicU64,
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchRegistry {
    pub const fn new() -> Self {
        Self {
            table: AtomicPtr::new(ptr::null_mut()),
            offered: AtomicPtr::new(ptr::null_mut()),
            draining: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
            capacity: AtomicUsize::new(0),
            requested: AtomicUsize::new(0),
            registered: AtomicUsize::new(0),
            epoch: AtomicU64::new(1),
            revision: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            enabled: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            refused: AtomicU64::new(0),
            clears: AtomicU64::new(0),
            installs: AtomicU64::new(0),
            deferred: AtomicU64::new(0),
            held: AtomicU64::new(0),
            failed_offers: AtomicU64::new(0),
        }
    }

    fn table(&self) -> Option<&Table> {
        // SAFETY: a table is freed only after a boundary replaced it and saw
        // no notification running; captures run only after the boundary.
        unsafe { self.table.load(SeqCst).as_ref() }
    }

    fn increment(&self, value: &AtomicU64) {
        if value.fetch_add(1, SeqCst) == u64::MAX {
            self.poison();
        }
    }

    fn new_epoch(&self) {
        self.increment(&self.epoch);
    }

    fn observing(&self) -> bool {
        self.enabled.load(SeqCst) && !self.poisoned.load(SeqCst)
    }

    /// Hook path, allocation-free: a writer is about to change `receiver`.
    /// Dropping the guard notifies its return.
    pub fn notify(&self, receiver: usize) -> Notification<'_> {
        self.active.fetch_add(1, SeqCst);
        let sequence = match self.table().map(|table| table.find(receiver, false)) {
            Some(Lookup::Watched(sequence)) => Some(sequence),
            _ => None,
        };
        self.increment(&self.revision);
        if let Some(sequence) = sequence {
            self.increment(sequence);
        }
        Notification {
            registry: self,
            sequence,
        }
    }

    /// `(epoch, revision)` while observing with no writer running; equal
    /// values across hours prove that no observed writer ran.
    pub fn quiet(&self) -> Option<(u64, u64)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let revision = self.revision.load(SeqCst);
        (self.active.load(SeqCst) == 0).then_some((epoch, revision))
    }

    /// `(epoch, sequence)` of `receiver`, registering it; `None` when it
    /// cannot be observed. Captures only, after the hour's boundary.
    pub fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let sequence = match self.table()?.find(receiver, true) {
            Lookup::Watched(sequence) => sequence,
            Lookup::Registered(sequence) => {
                self.registered.fetch_add(1, Relaxed);
                sequence
            }
            Lookup::Unwatched => return None,
            Lookup::Full => {
                self.refused.fetch_add(1, Relaxed);
                return None;
            }
        }
        .load(SeqCst);
        (self.epoch.load(SeqCst) == epoch && self.active.load(SeqCst) == 0)
            .then_some((epoch, sequence))
    }

    /// Game thread, in the hourly callback before any capture: start an
    /// epoch when `invalidate`, then install an offered table or clear the
    /// installed one while no notification is running. Outside an
    /// invalidation both wait, counted as held, unless no table is installed
    /// yet; so does one a running notification blocks.
    pub fn boundary(&self, invalidate: bool) {
        if invalidate {
            self.new_epoch();
        }
        let full = self.registered.load(Relaxed) * 2 > self.capacity.load(Relaxed);
        let first = self.table.load(SeqCst).is_null();
        let restart = invalidate || first;
        if !restart {
            if full || !self.offered.load(SeqCst).is_null() {
                self.held.fetch_add(1, Relaxed);
            }
            if self.active.load(SeqCst) == 0 {
                self.retire_drained();
            }
            return;
        }
        if self.active.load(SeqCst) != 0 {
            if full
                || invalidate
                || !self.offered.load(SeqCst).is_null()
                || !self.draining.load(SeqCst).is_null()
            {
                self.deferred.fetch_add(1, Relaxed);
            }
            // Only a missing first table retries at the next hour; the rest
            // waits for the next invalidating hour.
            return;
        }
        // No notification is running, so none holds a replaced table.
        self.retire_drained();
        if self.draining.load(SeqCst).is_null() && self.retired.load(SeqCst).is_null() {
            let next = self.offered.swap(ptr::null_mut(), SeqCst);
            // SAFETY: an offered table is owned here once swapped out.
            if let Some(len) = unsafe { next.as_ref() }.map(|table| table.slots.len()) {
                self.new_epoch();
                let previous = self.table.swap(next, SeqCst);
                self.new_epoch();
                self.capacity.store(len, Relaxed);
                self.registered.store(0, Relaxed);
                self.installs.fetch_add(1, Relaxed);
                // A notification begun since the check above may hold it.
                self.draining.store(previous, SeqCst);
                if self.active.load(SeqCst) == 0 {
                    self.retire_drained();
                }
                return;
            }
        }
        if full || invalidate {
            // A notification beginning during the clear either bumped its
            // receiver before it (a certificate taken while it runs is
            // refused) or finds it unregistered; the epoch changes both ways.
            if let Some(table) = self.table() {
                self.new_epoch();
                table.clear();
                self.new_epoch();
                self.registered.store(0, Relaxed);
                self.clears.fetch_add(1, Relaxed);
            }
        }
    }

    /// Hand a drained table to [`Self::collect`] once the previous one is
    /// gone. Only while no notification is running.
    fn retire_drained(&self) {
        let drained = self.draining.load(SeqCst);
        if !drained.is_null()
            && self
                .retired
                .compare_exchange(ptr::null_mut(), drained, SeqCst, SeqCst)
                .is_ok()
        {
            self.draining.store(ptr::null_mut(), SeqCst);
        }
    }

    /// Off the game thread: allocate a table for `watches` receivers when
    /// the installed and offered ones are smaller. A boundary installs it.
    pub fn offer(&self, watches: usize) {
        if let Some(len) = watches
            .saturating_mul(SLOTS_PER_WATCH)
            .max(MIN_SLOTS)
            .checked_next_power_of_two()
        {
            self.offer_slots(len);
        }
    }

    fn offer_slots(&self, len: usize) {
        if len <= self.requested.load(SeqCst) {
            return;
        }
        let Some(table) = Table::new(len) else {
            self.failed_offers.fetch_add(1, Relaxed);
            return;
        };
        self.requested.store(len, SeqCst);
        let previous = self.offered.swap(Box::into_raw(table), SeqCst);
        // SAFETY: an offered table was never installed; the swap owns it.
        unsafe { free(previous) };
    }

    /// Off the game thread: free a replaced table nothing holds any more.
    pub fn collect(&self) {
        let retired = self.retired.swap(ptr::null_mut(), SeqCst);
        // SAFETY: only a drained table is retired, and the swap owns it.
        unsafe { free(retired) };
    }

    /// Free every table.
    ///
    /// # Safety
    /// No notification or capture may run: the hooks are removed and
    /// drained, and the acquisition pool has stopped.
    pub unsafe fn release(&self) {
        for table in [&self.table, &self.offered, &self.draining, &self.retired] {
            unsafe { free(table.swap(ptr::null_mut(), SeqCst)) };
        }
        self.capacity.store(0, Relaxed);
        self.requested.store(0, SeqCst);
        self.registered.store(0, Relaxed);
        self.new_epoch();
    }

    /// Enabling starts an epoch: writers ran unobserved while disabled.
    pub fn set_enabled(&self, enabled: bool) {
        if enabled {
            self.new_epoch();
        }
        self.enabled.store(enabled, SeqCst);
    }

    /// Disable reuse for the attachment.
    pub fn poison(&self) {
        self.poisoned.store(true, SeqCst);
    }

    pub fn poisoned(&self) -> bool {
        self.poisoned.load(SeqCst)
    }

    /// Notifications running now.
    pub fn active(&self) -> usize {
        self.active.load(SeqCst)
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "capacity": self.capacity.load(Relaxed),
            "registered": self.registered.load(Relaxed),
            "refused_registrations": self.refused.load(Relaxed),
            "clears": self.clears.load(Relaxed),
            "installs": self.installs.load(Relaxed),
            "deferred_boundaries": self.deferred.load(Relaxed),
            "held_for_full_hour": self.held.load(Relaxed),
            "failed_offers": self.failed_offers.load(Relaxed),
            "scope": "cumulative since attach; a refused receiver is read afresh every hour; a clear or install starts an epoch, so every receiver is read afresh once, and waits for an invalidating hour (attach, load, gap or recovery)"
        })
    }
}

impl Drop for WatchRegistry {
    fn drop(&mut self) {
        // SAFETY: exclusive access; nothing else can reach the tables.
        unsafe { self.release() };
    }
}

/// A running writer; its drop notifies the writer's return.
pub struct Notification<'a> {
    registry: &'a WatchRegistry,
    sequence: Option<&'a AtomicU64>,
}

impl Drop for Notification<'_> {
    fn drop(&mut self) {
        if let Some(sequence) = self.sequence {
            self.registry.increment(sequence);
        }
        self.registry.increment(&self.registry.revision);
        self.registry.active.fetch_sub(1, SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An enabled registry with a `len`-slot table installed.
    fn registry(len: usize) -> WatchRegistry {
        let registry = WatchRegistry::new();
        registry.offer_slots(len);
        registry.boundary(false);
        registry.set_enabled(true);
        assert_eq!(registry.capacity.load(Relaxed), len);
        registry
    }

    #[test]
    fn watches_are_exact_and_a_full_table_refuses_and_counts() {
        let registry = registry(4);
        assert!(registry.sequence(0).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(registry.sequence(address).map(|(_, s)| s), Some(0));
        }
        assert!(registry.sequence(40).is_none());
        assert_eq!(registry.refused.load(Relaxed), 1);
        drop(registry.notify(8));
        assert_eq!(registry.sequence(8).map(|(_, s)| s), Some(2));
        assert_eq!(registry.sequence(16).map(|(_, s)| s), Some(0));
        // An unregistered receiver moves only the revision.
        let quiet = registry.quiet().unwrap();
        drop(registry.notify(40));
        assert_ne!(registry.quiet(), Some(quiet));
        assert_eq!(registry.sequence(24).map(|(_, s)| s), Some(0));
    }

    #[test]
    fn a_clear_starts_an_epoch_so_no_certificate_survives_it() {
        let registry = registry(64);
        let certified = registry.sequence(8).unwrap();
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert_eq!(registry.registered.load(Relaxed), 0);
        // Unregistered now: this writer's notification is not recorded,
        // but the epoch moved, so the old certificate cannot match.
        drop(registry.notify(8));
        let fresh = registry.sequence(8).unwrap();
        assert_eq!(fresh.1, 0);
        assert_ne!(fresh, certified);
    }

    #[test]
    fn a_running_notification_defers_the_clear_but_not_the_epoch() {
        let registry = registry(64);
        let certified = registry.sequence(8).unwrap();
        let running = registry.notify(8);
        registry.boundary(true);
        assert_eq!(registry.deferred.load(Relaxed), 1);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert!(registry.sequence(8).is_none());
        assert!(registry.quiet().is_none());
        drop(running);
        assert_ne!(registry.sequence(8), Some(certified));
        // The next ordinary hour keeps the new certificates: the clear waits
        // for the next invalidating hour, whose epoch revokes them anyway.
        let fresh = registry.sequence(8).unwrap();
        registry.boundary(false);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert_eq!(registry.sequence(8), Some(fresh));
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert_eq!(registry.registered.load(Relaxed), 0);
    }

    #[test]
    fn dead_receivers_filling_half_the_table_clear_it_at_the_next_full_hour() {
        let registry = registry(64);
        for address in (1..=33).map(|n| n * 8) {
            registry.sequence(address).unwrap();
        }
        // An ordinary hour keeps every certificate: the clear is held.
        let certified = registry.sequence(8).unwrap();
        registry.boundary(false);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert_eq!(registry.held.load(Relaxed), 1);
        assert_eq!(registry.sequence(8), Some(certified));
        let before = registry.epoch.load(SeqCst);
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert!(registry.epoch.load(SeqCst) > before);
    }

    #[test]
    fn a_larger_table_is_installed_at_a_quiet_full_hour_and_freed_off_thread() {
        let registry = registry(4);
        let certified = registry.sequence(8).unwrap();
        // Already large enough: nothing is allocated.
        registry.offer_slots(4);
        assert!(registry.offered.load(SeqCst).is_null());
        registry.offer(1_000);
        // An ordinary hour holds it and keeps every certificate.
        registry.boundary(false);
        assert_eq!(registry.capacity.load(Relaxed), 4);
        assert_eq!(registry.held.load(Relaxed), 1);
        assert_eq!(registry.sequence(8), Some(certified));
        let running = registry.notify(8);
        registry.boundary(true);
        assert_eq!(registry.capacity.load(Relaxed), 4);
        drop(running);
        // Blocked by the running writer, it waits for the next full hour.
        registry.boundary(false);
        assert_eq!(registry.capacity.load(Relaxed), 4);
        registry.boundary(true);
        assert_eq!(registry.capacity.load(Relaxed), 4096);
        assert_eq!(registry.installs.load(Relaxed), 2);
        assert_ne!(registry.sequence(8), Some(certified));
        // Nothing ran across the swap, so the old table is retired at once.
        assert!(registry.draining.load(SeqCst).is_null());
        assert!(!registry.retired.load(SeqCst).is_null());
        registry.collect();
        assert!(registry.retired.load(SeqCst).is_null());
    }

    #[test]
    fn observation_is_off_until_enabled_and_enabling_starts_an_epoch() {
        let registry = WatchRegistry::new();
        registry.offer(1);
        registry.boundary(false);
        assert!(registry.sequence(8).is_none() && registry.quiet().is_none());
        let before = registry.epoch.load(SeqCst);
        registry.set_enabled(true);
        assert!(registry.epoch.load(SeqCst) > before);
        assert!(registry.sequence(8).is_some());
        registry.poison();
        assert!(registry.sequence(8).is_none() && registry.quiet().is_none());
    }
}
