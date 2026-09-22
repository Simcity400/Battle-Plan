//! Bounded advisor-map notifications. Only authenticated exact map addresses
//! are watched; generic tree helpers also operate on unrelated template maps.
use crate::{
    extended::characters::advisors::{Changes, Counts},
    hook::Hook,
};
use engine::{Memory, advisor_mutations::MUTATIONS};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const WATCHES: usize = 65536;
static WATCH: [Slot; WATCHES] = [const { Slot::new() }; WATCHES];
static EPOCH: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 11] = [const { AtomicUsize::new(0) }; 11];
static EVENTS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];
static COUNTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static NOTIFICATION_TICKS: AtomicU64 = AtomicU64::new(0);

struct Slot {
    address: AtomicUsize,
    revision: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            address: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
        }
    }
}

// No removal: reuse of a native address retains its invalidation history.
// A collision or full watch registry can never certify an unrelated address.
fn watch(slots: &[Slot], address: usize, register: bool) -> Option<&AtomicU64> {
    if address == 0 {
        return None;
    }
    let first = ((address >> 3).wrapping_mul(0x9e3779b97f4a7c15) >> 32) & (slots.len() - 1);
    for probe in 0..slots.len().min(16) {
        let slot = &slots[(first + probe) & (slots.len() - 1)];
        let mut current = slot.address.load(Ordering::SeqCst);
        if current == 0 && register {
            current =
                match slot
                    .address
                    .compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => address,
                    Err(current) => current,
                };
        }
        if current == address {
            return Some(&slot.revision);
        }
        if current == 0 {
            return None;
        }
    }
    None
}
fn increment(value: &AtomicU64) {
    if value.fetch_add(1, Ordering::SeqCst) == u64::MAX {
        POISONED.store(true, Ordering::SeqCst);
    }
}
pub fn invalidate() {
    increment(&EPOCH);
}
fn character_map(character: usize) -> usize {
    character
        .checked_add(engine::ccharacter::ADVISOR_MAP_HEAD)
        .unwrap_or(0)
}
fn role_map(role: usize) -> usize {
    // Native receiver is a live CAdvisor, but still guard the metadata read.
    // On failure revoke every certificate rather than guess its owner.
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let owner = role
        .checked_add(engine::cadvisor::CHARACTER)
        .and_then(|address| engine::read::<u64>(&memory, address))
        .filter(|&owner| owner != 0);
    match owner {
        Some(owner) => character_map(owner as usize),
        None => {
            invalidate();
            0
        }
    }
}

struct Mutation {
    revision: Option<&'static AtomicU64>,
    ticks: u64,
}
impl Mutation {
    fn enter(address: impl FnOnce() -> usize, site: usize) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        // Keep callback lifetime protection around QPC too: it can execute in
        // another module while retaining a return address into this DLL.
        let begin = crate::sampling::qpc();
        increment(&REVISION);
        let revision = watch(&WATCH, address(), false);
        if let Some(revision) = revision {
            increment(revision);
        }
        EVENTS[site].fetch_add(1, Ordering::Relaxed);
        Self {
            revision,
            ticks: crate::sampling::qpc().saturating_sub(begin).max(0) as u64,
        }
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        let begin = crate::sampling::qpc();
        if let Some(revision) = self.revision {
            increment(revision);
        }
        increment(&REVISION);
        NOTIFICATION_TICKS.fetch_add(
            self.ticks + crate::sampling::qpc().saturating_sub(begin).max(0) as u64,
            Ordering::Relaxed,
        );
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let value = (
            EPOCH.load(Ordering::SeqCst),
            REVISION.load(Ordering::SeqCst),
        );
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some(value)
    }
    fn revision(&self, map: usize) -> Option<(u64, u64)> {
        let (epoch, _) = self.quiet()?;
        let revision = watch(&WATCH, map, true)?.load(Ordering::SeqCst);
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some((epoch, revision))
    }
    fn verify(&self) -> bool {
        cfg!(feature = "advisor_verify")
    }
}

pub fn record(counts: Counts) {
    for (target, value) in COUNTS.iter().zip([
        counts.reused,
        counts.read,
        counts.roles,
        counts.verified,
        counts.mismatches,
        counts.planned,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if counts.mismatches != 0 {
        POISONED.store(true, Ordering::SeqCst);
    }
}
pub fn status() -> serde_json::Value {
    let counts = COUNTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events: serde_json::Map<String, serde_json::Value> = MUTATIONS
        .iter()
        .zip(&EVENTS)
        .map(|(site, count)| (site.name.to_owned(), count.load(Ordering::Relaxed).into()))
        .collect();
    serde_json::json!({ "configured": cfg!(feature = "advisor_changes"), "verify": cfg!(feature = "advisor_verify"),
        "poisoned": POISONED.load(Ordering::SeqCst), "reused_characters": counts[0], "read_characters": counts[1],
        "reused_roles": counts[2], "verified_characters": counts[3], "mismatches": counts[4], "planned_sections": counts[5], "events": events,
        "notification_ticks": NOTIFICATION_TICKS.load(Ordering::Relaxed),
        "scope": "QPC notification bookkeeping ticks including guarded owner lookup, excluding native bodies; capture wall includes all cache work; verify reads every proposed hit" })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "advisor_changes") {
            let callbacks = [
                construct as *const () as usize,
                destroy as *const () as usize,
                set_idea as *const () as usize,
                read_role as *const () as usize,
                insert_map as *const () as usize,
                insert_map_value as *const () as usize,
                erase_map as *const () as usize,
                clear_map as *const () as usize,
                merge_character as *const () as usize,
                read_character as *const () as usize,
                destroy_character as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err(format!("advisor {} sentinel differs", site.name));
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        for hook in &mut self.hooks {
            if !hook.set_enabled(enabled)? {
                return Ok(false);
            }
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .collect()
    }
}

// Signatures checked against pinned decompiles and native call sites. The
// callbacks preserve every argument and return, and never call readers/getters
// in the game. Role-owner lookup is guarded metadata, not an engine call.
macro_rules! callback {
    ($name:ident, $site:expr, ($($arg:ident:$ty:ty),*) -> $ret:ty, $map:expr) => {
        unsafe extern "system" fn $name($($arg:$ty),*) -> $ret {
            let _mutation = Mutation::enter(|| $map, $site);
            let original: unsafe extern "system" fn($($ty),*) -> $ret =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            unsafe { original($($arg),*) }
        }
    };
}
callback!(construct, 0, (this:usize, owner:usize) -> usize, character_map(owner));
callback!(destroy, 1, (this:usize) -> (), role_map(this));
callback!(set_idea, 2, (this:usize, token:usize) -> (), role_map(this));
callback!(read_role, 3, (this:usize, reader:usize, token:i32) -> (), role_map(this));
callback!(insert_map, 4, (this:usize, result:usize, key:usize, value:usize) -> usize, this);
callback!(insert_map_value, 5, (this:usize, result:usize, key:usize, value:usize) -> usize, this);
callback!(erase_map, 6, (this:usize, range:usize) -> usize, this);
callback!(clear_map, 7, (this:usize, allocator:usize, node:usize) -> (), this);
callback!(merge_character, 8, (this:usize, source:usize, country:usize) -> (), character_map(this));
callback!(read_character, 9, (this:usize, reader:usize, token:i32) -> (), character_map(this));
callback!(destroy_character, 10, (this:usize) -> (), character_map(this));

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_registry_never_certifies_colliding_or_unregistered_maps() {
        let slots = [const { Slot::new() }; 4];
        for address in [8, 16, 24, 32] {
            assert!(watch(&slots, address, false).is_none());
            watch(&slots, address, true)
                .unwrap()
                .store(address as u64, Ordering::SeqCst);
        }
        assert!(watch(&slots, 40, true).is_none());
        assert!(watch(&slots, 0, true).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(
                watch(&slots, address, false)
                    .unwrap()
                    .load(Ordering::SeqCst),
                address as u64
            );
        }
    }
}
