//! Allocation-free certificates for a fleet's mission-region multiset.
//!
//! A running guard excludes certificates during each native writer. Append,
//! clear, remove and prune notify only when their before/after counts differ;
//! these operations change membership only by changing the count. Pure
//! permutations need no observation. Construction, loading and destruction
//! revoke observations of a reused address. The existing fleet
//! census reads a vector only on its first observation or a changed
//! certificate. Disabled or unavailable observation refuses the field rather
//! than introducing a periodic native vector walk.

use crate::{hook::Hook, watch_registry::WatchRegistry};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

/// A receiver's observation epoch and writer sequence.
pub type Certificate = (u64, u64);

/// A verified native entry owned by this family.
#[derive(Clone, Copy)]
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Mutation and lifetime entries. Their first argument is always the fleet;
/// each accepts at most four integer arguments and returns through RAX.
pub const SITES: [Site; 7] = [
    Site {
        name: "append mission region",
        rva: 0x00d3c6e0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24,
            0x10, 0x57,
        ],
    },
    Site {
        name: "clear mission regions",
        rva: 0x00d44cf0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x30, 0x8b, 0x41, 0x54, 0x0f,
            0xb6, 0xda,
        ],
    },
    Site {
        name: "remove mission region",
        rva: 0x00d44f20,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x41, 0x48,
            0x41, 0x0f,
        ],
    },
    Site {
        name: "prune mission regions",
        rva: 0x00d453b0,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x41, 0x54, 0x48, 0x8b, 0xd9, 0x83, 0xe8,
            0x01, 0x78,
        ],
    },
    Site {
        name: "load mission regions",
        rva: 0x00d43a80,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
    },
    Site {
        name: "construct fleet regions",
        rva: 0x00d3bba0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x48,
        ],
    },
    Site {
        name: "destroy fleet regions",
        rva: 0x00d3c010,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54,
            0x41, 0x56,
        ],
    },
];

static WATCHES: WatchRegistry = WatchRegistry::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
static CONFIGURED: AtomicBool = AtomicBool::new(false);
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static CALLS: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];
static ORIGINAL: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];

/// Register a fleet reached by the authenticated census. Observation being
/// disabled refuses even an otherwise retained registry certificate.
pub fn sequence(fleet: usize) -> Option<Certificate> {
    ENABLED
        .load(Ordering::SeqCst)
        .then(|| WATCHES.sequence(fleet))
        .flatten()
}

/// Demand measured by the fleet census, used only for off-thread sizing.
pub fn demand(fleets: usize) {
    RECEIVERS.fetch_max(fleets, Ordering::Relaxed);
}

/// Prepare watch storage off the native thread.
pub fn presize() {
    if !CONFIGURED.load(Ordering::Acquire) {
        return;
    }
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.offer(receivers);
    }
}

/// Begin an hourly boundary; a load, gap or recovery revokes prior values.
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

/// Native notifications still in flight.
pub fn active() -> usize {
    WATCHES.active()
}

/// Release retired watch storage off-thread.
pub fn collect() {
    WATCHES.collect();
}

pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "configured": CONFIGURED.load(Ordering::Acquire),
        "enabled": ENABLED.load(Ordering::SeqCst),
        "receivers": RECEIVERS.load(Ordering::Relaxed),
        "events": crate::status::map(SITES.iter().zip(&CALLS).map(|(site, calls)| (site.name, calls.load(Ordering::Relaxed)))),
        "watches": WATCHES.status(),
    })
}

fn hook_error(site: &str, reason: &str) -> &'static str {
    Box::leak(format!("fleet_region_changes {site}: {reason}").into_boxed_str())
}

pub struct Hooks {
    hooks: Vec<Hook>,
}

impl Hooks {
    /// The owning feature is disabled: no native verification, patches or
    /// observer storage are prepared.
    pub const fn disabled() -> Self {
        Self { hooks: Vec::new() }
    }
    /// Prepare the complete writer family without publishing any patch.
    ///
    /// # Safety
    /// The enclosing runtime authenticated this executable, owns this family
    /// exclusively and restores its patches and drains native calls before
    /// releasing its storage or unloading.
    pub unsafe fn prepare(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        for site in SITES {
            if engine_native::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                return Err(hook_error(site.name, "entry verification failed"));
            }
        }
        let callbacks = [
            append as *const () as usize,
            clear as *const () as usize,
            remove as *const () as usize,
            prune as *const () as usize,
            load as *const () as usize,
            construct as *const () as usize,
            destroy as *const () as usize,
        ];
        let mut hooks = Vec::with_capacity(SITES.len());
        for (index, site) in SITES.into_iter().enumerate() {
            let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                .map_err(|reason| hook_error(site.name, &reason))?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        CONFIGURED.store(true, Ordering::Release);
        Ok(Self { hooks })
    }

    /// Alias matching the other writer families' construction API.
    ///
    /// # Safety
    /// The requirements of [`Self::prepare`] apply.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        unsafe { Self::prepare(memory, base) }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            WATCHES.set_enabled(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|reason| hook_error("writer family", &reason))?
        {
            return Ok(false);
        }
        WATCHES.set_enabled(enabled && self.hooks.len() == SITES.len());
        ENABLED.store(enabled && self.hooks.len() == SITES.len(), Ordering::SeqCst);
        Ok(true)
    }

    pub fn install(&mut self) -> Result<bool, &'static str> {
        self.set_enabled(true)
    }

    pub fn disable(&mut self) -> Result<bool, &'static str> {
        self.set_enabled(false)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        CONFIGURED.store(false, Ordering::Release);
        ENABLED.store(false, Ordering::SeqCst);
        WATCHES.set_enabled(false);
        // SAFETY: the runtime removes and drains patches before release.
        unsafe { WATCHES.release() };
    }
}

macro_rules! writer {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            fleet: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            CALLS[$site].fetch_add(1, Ordering::Relaxed);
            unsafe {
                invoke(
                    &WATCHES,
                    &ORIGINAL[$site],
                    $site < 4,
                    [fleet, second, third, fourth],
                )
            }
        }
    };
}

unsafe fn invoke(
    watches: &WatchRegistry,
    trampoline: &AtomicUsize,
    count_changes: bool,
    arguments: [usize; 4],
) -> usize {
    let [fleet, second, third, fourth] = arguments;
    let _running = watches.running();
    let before = count_changes
        .then(|| unsafe { region_count(fleet) })
        .flatten();
    let _notification = (!count_changes).then(|| watches.notify(fleet));
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(trampoline.load(Ordering::Acquire)) };
    let result = unsafe { original(fleet, second, third, fourth) };
    if count_changes {
        let after = unsafe { region_count(fleet) };
        if !matches!((before, after), (Some(before), Some(after)) if before == after) {
            drop(watches.notify(fleet));
        }
    }
    result
}

unsafe fn region_count(fleet: usize) -> Option<i32> {
    // SAFETY: the authenticated native method owns its live receiver for
    // this call. The guarded copy contains no allocation or game call.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    engine_native::read(
        &memory,
        fleet.checked_add(engine_native::cfleet::MISSION_REGION_COUNT)?,
    )
}

writer!(append, 0);
writer!(clear, 1);
writer!(remove, 2);
writer!(prune, 3);
writer!(load, 4);
writer!(construct, 5);
writer!(destroy, 6);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_family_has_no_patch_or_executable_storage() {
        let hooks = Hooks::disabled();
        assert!(hooks.hooks.is_empty());
        assert_eq!(hooks.executable_ranges().count(), 0);
    }

    unsafe extern "system-unwind" fn original(
        fleet: usize,
        watches: usize,
        third: usize,
        fourth: usize,
    ) -> usize {
        let watches = unsafe { &*(watches as *const WatchRegistry) };
        // The notification must span the original, including nested work.
        assert!(watches.sequence(fleet).is_none());
        third ^ fourth
    }

    #[test]
    fn writer_spans_original_preserves_arguments_and_invalidates_only_its_fleet() {
        let watches = WatchRegistry::new();
        watches.offer(2);
        watches.boundary(true);
        watches.set_enabled(true);
        let before = watches.sequence(16).unwrap();
        let other = watches.sequence(32).unwrap();
        let trampoline = AtomicUsize::new(original as *const () as usize);
        let result = unsafe {
            invoke(
                &watches,
                &trampoline,
                false,
                [16, &watches as *const _ as usize, 123, 456],
            )
        };
        assert_eq!(result, 123 ^ 456);
        assert_ne!(watches.sequence(16), Some(before));
        assert_eq!(watches.sequence(32), Some(other));
        assert_eq!(watches.active(), 0);
        // Disabled observation never authorizes an hourly payload reread.
        watches.set_enabled(false);
        assert!(watches.sequence(16).is_none());
        watches.set_enabled(true);
        assert_ne!(watches.sequence(16), Some(before));
        // SAFETY: the synthetic original returned and holds no notification.
        unsafe { watches.release() };
    }

    unsafe extern "system-unwind" fn counted_original(
        fleet: usize,
        watches: usize,
        count: usize,
        result: usize,
    ) -> usize {
        let watches = unsafe { &*(watches as *const WatchRegistry) };
        assert!(watches.sequence(fleet).is_none());
        unsafe {
            std::ptr::write_unaligned(
                (fleet + engine_native::cfleet::MISSION_REGION_COUNT) as *mut i32,
                count as i32,
            );
        }
        result
    }

    #[test]
    fn unchanged_membership_keeps_certificate_and_a_count_change_revokes_it() {
        let watches = WatchRegistry::new();
        watches.offer(1);
        watches.boundary(true);
        watches.set_enabled(true);
        let mut fleet_bytes = [0_u8; engine_native::cfleet::MISSION_REGION_COUNT + 4];
        let fleet = fleet_bytes.as_mut_ptr() as usize;
        let before = watches.sequence(fleet).unwrap();
        let trampoline = AtomicUsize::new(counted_original as *const () as usize);
        let arguments = [fleet, &watches as *const _ as usize, 0, 123];
        assert_eq!(
            unsafe { invoke(&watches, &trampoline, true, arguments) },
            123
        );
        assert_eq!(watches.sequence(fleet), Some(before));
        let arguments = [fleet, &watches as *const _ as usize, 2, 456];
        assert_eq!(
            unsafe { invoke(&watches, &trampoline, true, arguments) },
            456
        );
        assert_ne!(watches.sequence(fleet), Some(before));
        assert_eq!(watches.active(), 0);
        // SAFETY: no synthetic writer remains in flight.
        unsafe { watches.release() };
    }
}
