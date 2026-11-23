use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::order_membership_mutations::{MUTATIONS, Receiver};
use engine_native_memory::{Memory, memory::LiveMemory};
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

static GROUPS: WatchRegistry = WatchRegistry::new();
static PATCHED: AtomicBool = AtomicBool::new(false);
static FORWARDED: AtomicBool = AtomicBool::new(false);
static ORIGINAL: [AtomicUsize; MUTATIONS.len()] = [const { AtomicUsize::new(0) }; MUTATIONS.len()];
#[cfg(feature = "order_membership_oracle")]
static HOOK_READS: AtomicU64 = AtomicU64::new(0);
static CALLS: [AtomicU64; MUTATIONS.len()] = [const { AtomicU64::new(0) }; MUTATIONS.len()];
static GLOBAL_REVOCATIONS: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_OWNERS: AtomicU64 = AtomicU64::new(0);

fn observe() {
    let enabled = PATCHED.load(Ordering::SeqCst) && FORWARDED.load(Ordering::SeqCst);
    GROUPS.set_enabled(enabled);
}
pub fn forwarder(enabled: bool) {
    FORWARDED.store(enabled, Ordering::SeqCst);
    observe();
}
pub fn group_changing(group: usize) -> Notification<'static> {
    GROUPS.notify(group)
}
pub fn boundary(invalidate: bool) {
    GROUPS.boundary(invalidate);
}
pub fn active() -> usize {
    GROUPS.active()
}
pub fn collect() {
    GROUPS.collect();
}

pub struct Observer;
impl engine_native::corders_group_member::MembershipSignals for Observer {
    fn available(&self) -> bool {
        GROUPS.quiet().is_some()
    }
    fn prepare(&self, members: usize) {
        if !PATCHED.load(Ordering::SeqCst) || !FORWARDED.load(Ordering::SeqCst) {
            return;
        }
        engine_allocation::advisory(|| {
            GROUPS.offer(members.saturating_mul(2));
        });
    }
    fn group(&self, receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
        GROUPS.sequence_at(receiver, slot)
    }
}

fn failure(index: usize, detail: &str) -> String {
    format!(
        "order_membership_changes [{}]: {detail}",
        MUTATIONS[index].0.name
    )
}
pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The runtime owns the authenticated image and this patch family's lifetime.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "order_membership_changes") {
            let callbacks = [
                invalidate as *const () as usize,
                add_root as *const () as usize,
                remove_root as *const () as usize,
                remove_fallback as *const () as usize,
                create_fallback as *const () as usize,
                repair as *const () as usize,
                set_parent as *const () as usize,
                dissolve as *const () as usize,
                delete_connections as *const () as usize,
                add_member as *const () as usize,
                remove_member as *const () as usize,
                remove_keep as *const () as usize,
                remove_subtree as *const () as usize,
                append_member as *const () as usize,
                clear_members as *const () as usize,
                destroy_order as *const () as usize,
                load_order as *const () as usize,
                load_group as *const () as usize,
                remove_deleted as *const () as usize,
            ];
            for (index, (site, receiver)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva).map_err(|why| failure(index, why))?;
                let sentinel = code.sentinel::<16>().map_err(|why| failure(index, why))?;
                let (hook, original) = if matches!(receiver, Receiver::AssignedMembersCall) {
                    let displacement = i32::from_le_bytes(sentinel[1..5].try_into().unwrap());
                    let target = code
                        .address
                        .checked_add(5)
                        .and_then(|at| at.checked_add_signed(displacement as isize))
                        .ok_or_else(|| failure(index, "call target address overflow"))?;
                    let hook = unsafe { Hook::new_call(code.address, callbacks[index], sentinel) }
                        .map_err(|why| failure(index, &why))?;
                    (hook, target)
                } else {
                    let hook = unsafe { Hook::new(code.address, callbacks[index], sentinel) }
                        .map_err(|why| failure(index, &why))?;
                    let original = hook.trampoline();
                    (hook, original)
                };
                ORIGINAL[index].store(original, Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            PATCHED.store(false, Ordering::SeqCst);
            observe();
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|why| failure(0, &why))?
        {
            return Ok(false);
        }
        PATCHED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        PATCHED.store(false, Ordering::SeqCst);
        observe();
        unsafe {
            GROUPS.release();
        }
    }
}

fn receiver_group(
    receiver: Receiver,
    first: usize,
    mut owner: impl FnMut(usize) -> Option<u64>,
) -> Option<usize> {
    match receiver {
        Receiver::Group => Some(first),
        receiver @ (Receiver::Order | Receiver::AssignedMembersCall) => {
            let order = if matches!(receiver, Receiver::AssignedMembersCall) {
                first.checked_sub(engine_native::corder_instance::ASSIGNED_MEMBER_DATA)
            } else {
                Some(first)
            };
            order
                .and_then(|order| order.checked_add(engine_native::corder_instance::OWNER))
                .and_then(&mut owner)
                .filter(|&group| group != 0)
                .map(|group| group as usize)
        }
        Receiver::Every => None,
    }
}

fn mark(index: usize, first: usize, _second: usize) -> Notification<'static> {
    CALLS[index].fetch_add(1, Ordering::Relaxed);
    let memory = unsafe { LiveMemory::new() };
    match receiver_group(MUTATIONS[index].1, first, |address| {
        #[cfg(feature = "order_membership_oracle")]
        HOOK_READS.fetch_add(1, Ordering::Relaxed);
        engine_native_memory::read::<u64>(&memory, address)
    }) {
        Some(group) => GROUPS.notify(group),
        None => {
            GLOBAL_REVOCATIONS.fetch_add(1, Ordering::Relaxed);
            if !matches!(MUTATIONS[index].1, Receiver::Every) {
                UNKNOWN_OWNERS.fetch_add(1, Ordering::Relaxed);
            }
            GROUPS.notify_every()
        }
    }
}
macro_rules! callback {
    ($name:ident, $index:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::OrderMembershipChanges);
            let notification = ManuallyDrop::new(mark($index, first, second));
            let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            drop(ManuallyDrop::into_inner(notification));
            result
        }
    };
}
callback!(invalidate, 0);
callback!(add_root, 1);
callback!(remove_root, 2);
callback!(remove_fallback, 3);
callback!(create_fallback, 4);
callback!(repair, 5);
callback!(set_parent, 6);
callback!(dissolve, 7);
callback!(delete_connections, 8);
callback!(add_member, 9);
callback!(remove_member, 10);
callback!(remove_keep, 11);
callback!(remove_subtree, 12);
callback!(append_member, 13);
callback!(clear_members, 14);
callback!(destroy_order, 15);
callback!(load_order, 16);
callback!(load_group, 17);
callback!(remove_deleted, 18);

pub fn status() -> impl serde::Serialize {
    use collector_status::status::{map, status};
    status!({
        "enabled": PATCHED.load(Ordering::SeqCst) && FORWARDED.load(Ordering::SeqCst),
        "active": active(),
        "global_revocations": GLOBAL_REVOCATIONS.load(Ordering::Relaxed),
        "unknown_owner_revocations": UNKNOWN_OWNERS.load(Ordering::Relaxed),
        "calls": map(MUTATIONS.iter().zip(&CALLS).map(|((site, _), count)| (site.name, count.load(Ordering::Relaxed)))),
    })
}

#[cfg(feature = "order_membership_oracle")]
pub fn hook_reads() -> u64 {
    HOOK_READS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_vector_recovers_the_actual_order_owner() {
        let order = 0x1000;
        let vector = order + engine_native::corder_instance::ASSIGNED_MEMBER_DATA;
        for (receiver, first) in [
            (Receiver::Order, order),
            (Receiver::AssignedMembersCall, vector),
        ] {
            let mut reads = Vec::new();
            assert_eq!(
                receiver_group(receiver, first, |address| {
                    reads.push(address);
                    Some(0x2000)
                }),
                Some(0x2000)
            );
            assert_eq!(reads, [order + engine_native::corder_instance::OWNER]);
            for owner in [None, Some(0)] {
                assert_eq!(receiver_group(receiver, first, |_| owner), None);
            }
        }
    }

    #[test]
    fn known_groups_and_unrecoverable_receivers_do_not_read_an_owner() {
        for (receiver, first, expected) in [
            (Receiver::Group, 0x2000, Some(0x2000)),
            (Receiver::Every, 0x2000, None),
            (Receiver::AssignedMembersCall, 0, None),
            (Receiver::Order, usize::MAX, None),
        ] {
            assert_eq!(
                receiver_group(receiver, first, |_| {
                    panic!("receiver has no readable owner address")
                }),
                expected
            );
        }
    }
}
