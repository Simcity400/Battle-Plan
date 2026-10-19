//! Independent production-template notifications. Native writers mark owners;
//! the existing acquisition worker reads only demanded changed owner rosters.
use collector_certificates::dirty_set::DirtySet;
use collector_hooks::hook::Hook;
use engine_native::{
    Memory, ccountry,
    cproduction_status::{
        self,
        design_templates::{MUTATIONS, RECORDS, Receiver},
    },
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

static DIRTY: DirtySet = DirtySet::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static OFFERED: AtomicUsize = AtomicUsize::new(0);
static OBSERVED: AtomicUsize = AtomicUsize::new(0);
static INSTALL: AtomicBool = AtomicBool::new(false);
static INSTALLING: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static UNNAMED: AtomicU64 = AtomicU64::new(0);
static CALLS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static ORIGINAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

pub(super) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}
pub(super) fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
pub(super) fn invalidate() {
    drop(DIRTY.notify_every());
}
pub(super) fn take(visit: impl FnMut(usize)) -> Option<u64> {
    DIRTY.take(visit)
}
/// The worker offers storage from observed owner demand. Installation happens
/// only at attach/reset or an actual native template writer return, never as
/// additional synchronous hourly work. Until installed, writes move the epoch.
pub(super) fn observe_owner(id: u32) {
    OBSERVED.fetch_max(id as usize + 1, Ordering::Relaxed);
}
pub(super) fn prepare() {
    let ids = OBSERVED.load(Ordering::Acquire);
    DIRTY.collect();
    if ids > OFFERED.fetch_max(ids, Ordering::AcqRel) {
        DIRTY.offer(ids);
        INSTALL.store(true, Ordering::Release);
    }
}
fn install_after_writer() {
    if !INSTALL.load(Ordering::Acquire)
        || INSTALLING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    if DIRTY.boundary(true) {
        INSTALL.store(false, Ordering::Release);
    }
    INSTALLING.store(false, Ordering::Release);
}
fn owner(
    memory: &impl Memory,
    base: usize,
    receiver: Receiver,
    first: usize,
    second: usize,
) -> Option<u32> {
    let status = match receiver {
        Receiver::FirstArray => first.checked_sub(RECORDS)?,
        Receiver::SecondArray => second.checked_sub(RECORDS)?,
        Receiver::Status => first,
    };
    if engine_native::read::<u64>(memory, status)?
        != base.checked_add(cproduction_status::VTABLE_RVA)? as u64
    {
        return None;
    }
    let country =
        engine_native::read::<u64>(memory, status.checked_add(cproduction_status::COUNTRY)?)?
            as usize;
    if engine_native::read::<u64>(memory, country)?
        != base.checked_add(ccountry::VTABLE_RVA)? as u64
    {
        return None;
    }
    engine_native::read(memory, country.checked_add(ccountry::ID)?)
}
unsafe extern "system-unwind" fn callback<const SITE: usize>(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    struct Call;
    impl Drop for Call {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let _call = Call;
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::VariantChanges);
    CALLS[SITE].fetch_add(1, Ordering::Relaxed);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let mark = match owner(
        &memory,
        BASE.load(Ordering::Acquire),
        MUTATIONS[SITE].receiver,
        a,
        b,
    ) {
        Some(country) => {
            observe_owner(country);
            DIRTY.notify(country as usize)
        }
        None => {
            UNNAMED.fetch_add(1, Ordering::Relaxed);
            DIRTY.notify_every()
        }
    };
    let source_reset = (SITE == 3).then(|| unsafe {
        super::design_changes::provider_reset(&memory, BASE.load(Ordering::Acquire), a)
    });
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d) });
    if let Some(reset) = source_reset {
        unsafe { reset.finish() };
    }
    drop(mark);
    install_after_writer();
    result
}
const HOOK_ERRORS: [&str; 4] = [
    "design-template AppendDesignTemplate hook construction failed",
    "design-template ReadDesignTemplates hook construction failed",
    "design-template ConsumeDesignTemplate hook construction failed",
    "design-template ResetDesignTemplates hook construction failed",
];
const PUBLICATION_ERROR: &str = "design-template hook publication failed (sites: AppendDesignTemplate, ReadDesignTemplates, ConsumeDesignTemplate, ResetDesignTemplates)";
pub(super) struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    pub(super) unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        BASE.store(base, Ordering::Release);
        // Install an empty prepared bitmap before patch publication, while no
        // callback/consumer can run. A quiet session can then seed its baseline
        // without waiting for the first discount writer.
        DIRTY.offer(0);
        DIRTY.boundary(true);
        let mut hooks = Vec::new();
        let callbacks = [
            callback::<0> as *const () as usize,
            callback::<1> as *const () as usize,
            callback::<2> as *const () as usize,
            callback::<3> as *const () as usize,
        ];
        if MUTATIONS.iter().all(|site| {
            base.checked_add(site.rva)
                .and_then(|address| engine_native::read::<[u8; 16]>(memory, address))
                == Some(site.sentinel)
        }) {
            for (index, site) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| HOOK_ERRORS[index])?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub(super) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::Release);
            DIRTY.set_enabled(false);
        }
        let changed = Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| PUBLICATION_ERROR)?;
        if changed {
            let observed = enabled && self.hooks.len() == MUTATIONS.len();
            ENABLED.store(observed, Ordering::Release);
            DIRTY.set_enabled(observed);
        }
        Ok(changed)
    }
    pub(super) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // The enclosing owner is dropped only after patch restoration and all
        // template callbacks/workers are quiescent, as for the design observer.
        unsafe { DIRTY.release() };
        OFFERED.store(0, Ordering::Release);
        OBSERVED.store(0, Ordering::Release);
        INSTALL.store(false, Ordering::Release);
    }
}
#[derive(serde::Serialize)]
pub(super) struct Status {
    enabled: bool,
    active: usize,
    calls: [u64; 4],
    unnamed: u64,
}
pub(super) fn status() -> Status {
    Status {
        enabled: ENABLED.load(Ordering::Acquire),
        active: active(),
        calls: CALLS.each_ref().map(|value| value.load(Ordering::Relaxed)),
        unnamed: UNNAMED.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_writer_receivers_authenticate_primary_owner_for_both_array_argument_positions() {
        let mut bytes = vec![0; 0x1000];
        bytes[..8].copy_from_slice(&(cproduction_status::VTABLE_RVA as u64).to_le_bytes());
        bytes[cproduction_status::COUNTRY..cproduction_status::COUNTRY + 8]
            .copy_from_slice(&0x1800_u64.to_le_bytes());
        bytes[0x800..0x808].copy_from_slice(&(ccountry::VTABLE_RVA as u64).to_le_bytes());
        bytes[0x808..0x80c].copy_from_slice(&5_000_u32.to_le_bytes());
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(owner(&memory, 0, Receiver::Status, 0x1000, 0), Some(5_000));
        assert_eq!(
            owner(&memory, 0, Receiver::FirstArray, 0x1000 + RECORDS, 0),
            Some(5_000)
        );
        assert_eq!(
            owner(&memory, 0, Receiver::SecondArray, 0, 0x1000 + RECORDS),
            Some(5_000)
        );
        assert_eq!(owner(&memory, 0, Receiver::SecondArray, 0, 0), None);
    }
}
