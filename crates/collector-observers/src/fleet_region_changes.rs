
use crate::{hook::Hook, watch_registry::WatchRegistry};
use engine_native::Memory;
use engine_sites::Rva;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Certificate {
    Fleet { epoch: u64, sequence: u64 },
    Family { epoch: u64, revision: u64 },
}

#[derive(Clone, Copy)]
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}

#[doc = " Mutation and lifetime entries. Their first argument is always the fleet;"]
#[doc = " each accepts at most four integer arguments and returns through RAX."]
pub const SITES: [Site; 7] = [
    Site {
        name: "append mission region",
        rva: Rva(0x00db72b0),
    },
    Site {
        name: "clear mission regions",
        rva: Rva(0x00dbf900),
    },
    Site {
        name: "remove mission region",
        rva: Rva(0x00dbfb30),
    },
    Site {
        name: "prune mission regions",
        rva: Rva(0x00dbffc0),
    },
    Site {
        name: "load mission regions",
        rva: Rva(0x00dbe690),
    },
    Site {
        name: "construct fleet regions",
        rva: Rva(0x00db6650),
    },
    Site {
        name: "destroy fleet regions",
        rva: Rva(0x00db6ac0),
    },
];

static WATCHES: WatchRegistry = WatchRegistry::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
static CONFIGURED: AtomicBool = AtomicBool::new(false);
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static CALLS: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];
static ORIGINAL: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];

pub fn sequence(fleet: usize) -> Option<Certificate> {
    if !ENABLED.load(Ordering::SeqCst) {
        return None;
    }
    match WATCHES.sequence(fleet) {
        Some((epoch, sequence)) => Some(Certificate::Fleet { epoch, sequence }),
        None => WATCHES
            .quiet()
            .map(|(epoch, revision)| Certificate::Family { epoch, revision }),
    }
}

/// # Safety
/// Run only while fleet-region watches remain live for the active session (or an isolated test)
/// and teardown is excluded, with measured fleet demand for the current session.
pub unsafe fn demand(fleets: usize) {
    RECEIVERS.fetch_max(fleets, Ordering::Relaxed);
}

/// # Safety
/// Run only while fleet-region watches remain live for the active session (or an isolated test)
/// and `WatchRegistry::release` is excluded; `WatchRegistry::offer` atomically swaps offered
/// tables across concurrent frame construction or sizing callers.
pub unsafe fn presize() {
    if !CONFIGURED.load(Ordering::Acquire) {
        return;
    }
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.offer(receivers);
    }
}

pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

pub fn active() -> usize {
    WATCHES.active()
}

pub fn collect() {
    WATCHES.collect();
}


fn hook_error(site: &str, reason: &str) -> String {
    format!("fleet_region_changes {site}: {reason}")
}

pub struct Hooks {
    hooks: Vec<Hook>,
}

impl Hooks {
    pub const fn disabled() -> Self {
        Self { hooks: Vec::new() }
    }
    /// Prepare the complete writer family without publishing any patch.
    ///
    /// # Safety
    /// The enclosing runtime authenticated this executable, owns this family
    /// exclusively and restores its patches and drains native calls before
    /// releasing its storage or unloading.
    pub unsafe fn prepare(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        for site in SITES {
            engine_sites::hook(base, site.rva)
                .and_then(|code| code.sentinel::<16>())
                .map_err(|reason| hook_error(site.name, reason))?;
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
            let code = engine_sites::hook(base, site.rva)?;
            let hook = unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
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
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
        unsafe { Self::prepare(memory, base) }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
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

    pub fn install(&mut self) -> Result<bool, String> {
        self.set_enabled(true)
    }

    pub fn disable(&mut self) -> Result<bool, String> {
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
            let _hook = crate::hook_timing::label();
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

/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
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

/// # Safety
/// `fleet` is the receiver of the authenticated native method running on this thread.
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

    unsafe extern "system-unwind" fn original(
        fleet: usize,
        watches: usize,
        third: usize,
        fourth: usize,
    ) -> usize {
        let watches = unsafe { &*(watches as *const WatchRegistry) };
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
        watches.set_enabled(false);
        assert!(watches.sequence(16).is_none());
        watches.set_enabled(true);
        assert_ne!(watches.sequence(16), Some(before));
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
        unsafe { watches.release() };
    }

    #[test]
    fn a_fleet_the_full_table_cannot_hold_is_certified_by_the_family_quiet_token() {
        WATCHES.offer(1);
        assert!(WATCHES.boundary(true).installed());
        WATCHES.set_enabled(true);
        ENABLED.store(true, Ordering::SeqCst);
        for held in 1..=16_384 {
            let _ = WATCHES.sequence(held * 0x1000);
        }
        let fleet = 0x7fff_0000;
        assert_eq!(WATCHES.sequence(fleet), None, "the table holds no more fleets");
        let quiet = sequence(fleet);
        assert!(quiet.is_some(), "an unheld fleet is still certified");
        assert_eq!(sequence(fleet), quiet, "no writer: the same certificate");
        drop(WATCHES.notify(0x1000));
        assert_ne!(sequence(fleet), quiet, "any fleet's writer revokes it");
        {
            let _running = WATCHES.notify(0x2000);
            assert_eq!(sequence(fleet), None, "nothing is certified while a writer runs");
        }
        ENABLED.store(false, Ordering::SeqCst);
        WATCHES.set_enabled(false);
        assert_eq!(sequence(fleet), None, "disabled observation certifies nothing");
        unsafe { WATCHES.release() };
    }
}
