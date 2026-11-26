use collector_hooks::hook::Hook;
use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native_air::cair_wing as wing;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        OnceLock, RwLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use wing::replacement::{COMMAND_WINGS, MUTATIONS, SETTING};

#[derive(Clone, Copy)]
struct Value {
    id: u64,
    epoch: u64,
    setting: Option<u8>,
}
#[derive(Default)]
struct Cache {
    values: HashMap<usize, Value>,
    identities: HashMap<u64, usize>,
    epoch: u64,
}
impl Cache {
    fn read(&mut self, memory: &impl Memory, pointer: usize, id: u64, epoch: u64) -> Option<u8> {
        if self.epoch != epoch {
            self.values.clear();
            self.identities.clear();
            self.epoch = epoch;
        }
        if let Some(value) = self
            .values
            .get(&pointer)
            .filter(|value| value.id == id && value.epoch == epoch)
        {
            return value.setting;
        }
        engine_allocation::reserve_map(&mut self.values, 1).ok()?;
        engine_allocation::reserve_map(&mut self.identities, 1).ok()?;
        let setting = pointer
            .checked_add(SETTING)
            .and_then(|address| engine_native_memory::read(memory, address));
        self.retire(pointer);
        if let Some(previous) = self.identities.get(&id).copied() {
            self.retire(previous);
        }
        self.identities.insert(id, pointer);
        self.values.insert(pointer, Value { id, epoch, setting });
        setting
    }
    fn retire(&mut self, pointer: usize) {
        if let Some(value) = self.values.remove(&pointer) {
            self.identities.remove(&value.id);
        }
    }
    fn retire_identity(&mut self, id: u64) {
        if let Some(pointer) = self.identities.get(&id).copied() {
            self.retire(pointer);
        }
    }
}
static CACHE: OnceLock<RwLock<Cache>> = OnceLock::new();
static OWN: AtomicBool = AtomicBool::new(false);
static PUBLICATION: AtomicBool = AtomicBool::new(false);
static LIFETIME: AtomicBool = AtomicBool::new(false);
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(1);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
pub(crate) fn active() -> usize {
    ACTIVE.load(Ordering::Acquire)
}
fn refresh() {
    let next = OWN.load(Ordering::Acquire)
        && PUBLICATION.load(Ordering::Acquire)
        && LIFETIME.load(Ordering::Acquire);
    if AVAILABLE.swap(next, Ordering::AcqRel) != next {
        invalidate();
    }
}
pub(crate) fn publication_enabled(enabled: bool) {
    PUBLICATION.store(enabled, Ordering::Release);
    refresh();
}
pub(crate) fn lifetime_enabled(enabled: bool) {
    LIFETIME.store(enabled, Ordering::Release);
    refresh();
}
pub(crate) fn invalidate() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
}
fn retire(pointer: usize) {
    let Some(cache) = CACHE.get() else {
        return;
    };
    match cache.try_write() {
        Ok(mut cache) => {
            cache.retire(pointer);
        }
        Err(_) => invalidate(),
    }
}
pub fn quiet_token() -> Option<(u64, u64)> {
    if !AVAILABLE.load(Ordering::Acquire) || ACTIVE.load(Ordering::Acquire) != 0 {
        return None;
    }
    let epoch = EPOCH.load(Ordering::Acquire);
    let revision = REVISION.load(Ordering::Acquire);
    if !AVAILABLE.load(Ordering::Acquire)
        || ACTIVE.load(Ordering::Acquire) != 0
        || EPOCH.load(Ordering::Acquire) != epoch
        || REVISION.load(Ordering::Acquire) != revision
    {
        return None;
    }
    Some((epoch, revision))
}

pub fn read(memory: &impl Memory, pointer: usize, id: u64) -> Option<u8> {
    if !AVAILABLE.load(Ordering::Acquire) || ACTIVE.load(Ordering::Acquire) != 0 {
        return None;
    }
    let epoch = EPOCH.load(Ordering::Acquire);
    let revision = REVISION.load(Ordering::Acquire);
    let lock = CACHE.get_or_init(|| RwLock::new(Cache::default()));
    if let Ok(cache) = lock.read()
        && cache.epoch == epoch
        && let Some(value) = cache
            .values
            .get(&pointer)
            .filter(|value| value.id == id && value.epoch == epoch)
    {
        let setting = value.setting;
        drop(cache);
        if ACTIVE.load(Ordering::Acquire) != 0
            || EPOCH.load(Ordering::Acquire) != epoch
            || REVISION.load(Ordering::Acquire) != revision
        {
            if let Ok(mut cache) = lock.write() {
                cache.retire(pointer);
            } else {
                invalidate();
            }
            return None;
        }
        return setting;
    }
    let mut cache = lock.write().ok()?;
    let value = cache.read(memory, pointer, id, epoch);
    if ACTIVE.load(Ordering::Acquire) != 0
        || EPOCH.load(Ordering::Acquire) != epoch
        || REVISION.load(Ordering::Acquire) != revision
    {
        cache.retire(pointer);
        return None;
    }
    value
}

pub(crate) struct Scope {
    split: bool,
    complete: bool,
}
impl Scope {
    fn begin(split: bool) -> Self {
        REVISION.fetch_add(1, Ordering::AcqRel);
        ACTIVE.fetch_add(1, Ordering::AcqRel);
        Self {
            split,
            complete: false,
        }
    }
    pub(crate) fn finish(mut self, result: usize) {
        if self.split && result != 0 {
            retire(result);
        }
        self.complete = true;
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if !self.complete {
            invalidate();
        }
        REVISION.fetch_add(1, Ordering::AcqRel);
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
#[inline]
pub(crate) fn base_site(site: usize, args: &[usize]) -> Option<Scope> {
    if !matches!(site, 4 | 6 | 7) || !AVAILABLE.load(Ordering::Acquire) {
        return None;
    }
    let scope = Scope::begin(site == 7);
    if site == 4 {
        retire(args[1]);
    }
    Some(scope)
}
pub(crate) fn destruction(pointer: usize) -> Option<Scope> {
    if !AVAILABLE.load(Ordering::Acquire) {
        return None;
    }
    let scope = Scope::begin(false);
    retire(pointer);
    Some(scope)
}
fn command(memory: &impl Memory, pointer: usize) -> Result<(), &'static str> {
    let Some(cache) = CACHE.get() else {
        return Ok(());
    };
    let mut cache = cache
        .try_write()
        .map_err(|_| "wing preference watch storage busy")?;
    if cache.values.is_empty() || !AVAILABLE.load(Ordering::Acquire) {
        return Ok(());
    }
    let references = Array::read(memory, pointer + COMMAND_WINGS, 8)
        .ok_or("wing preference command roster unreadable")?;
    for index in 0..references.count {
        let handle: u64 = engine_native_memory::read(
            memory,
            references
                .element(index, 8)
                .ok_or("wing preference command address overflow")?,
        )
        .ok_or("wing preference command handle unreadable")?;
        if handle == 0 {
            continue;
        }
        if handle as u32 != engine_native_parts::cair_wing_layout::TYPE {
            return Err("wing preference command reference type mismatch");
        }
        cache.retire_identity(handle);
    }
    Ok(())
}
unsafe extern "system-unwind" fn setting(a: usize, b: usize, c: usize, d: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WingChanges);
    let scope = Scope::begin(false);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    if command(&memory, a).is_err() {
        invalidate();
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d) });
    scope.finish(result);
    result
}
unsafe extern "system-unwind" fn deploy(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WingChanges);
    let scope = Scope::begin(false);
    let original: unsafe extern "system-unwind" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize = unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d, e, f, g, h) });
    scope.finish(result);
    result
}
pub(crate) struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub(crate) unsafe fn new(base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        let callbacks = [deploy as *const () as usize, setting as *const () as usize];
        if cfg!(feature = "wing_changes")
            && MUTATIONS
                .iter()
                .all(|site| crate::verified(base, site.rva).is_ok())
        {
            for (index, site) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| {
                            if index == 0 {
                                "wing-preference DeployWing hook construction failed"
                            } else {
                                "wing-preference SetWingReplacement hook construction failed"
                            }
                        })?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            OWN.store(false, Ordering::Release);
            refresh();
        }
        let changed = Hook::set_family_enabled_shared(&mut self.hooks, enabled).map_err(
            |_| "wing-preference hook publication failed (DeployWing, SetWingReplacement)",
        )?;
        if !changed {
            return Ok(false);
        }
        OWN.store(
            enabled && self.hooks.len() == MUTATIONS.len(),
            Ordering::Release,
        );
        refresh();
        Ok(changed)
    }
    pub(crate) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        OWN.store(false, Ordering::Release);
        refresh();
        if let Some(cache) = CACHE.get() {
            *cache
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Cache::default();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    static SERIAL: Mutex<()> = Mutex::new(());
    #[test]
    fn complete_wing_hook_batch_admits_initial_preference_and_withdrawal_revokes_it() {
        let _serial = SERIAL.lock().unwrap();
        struct Byte;
        impl Memory for Byte {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                assert_eq!((address, out.len()), (0x5000 + SETTING, 1));
                out[0] = 255;
                true
            }
        }
        struct Quiet;
        impl Memory for Quiet {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("uncertified or carried preference read native memory")
            }
        }
        let previous = [
            OWN.swap(true, Ordering::AcqRel),
            PUBLICATION.load(Ordering::Acquire),
            LIFETIME.load(Ordering::Acquire),
        ];
        publication_enabled(true);
        let installed = engine_mutations::air_wing_equipment_mutations::MUTATIONS.len()
            + engine_mutations::air_wing_equipment_mutations::MAP_MUTATIONS.len();
        crate::wing_changes::publish_preference_lifetime(true, installed);
        invalidate();
        assert_eq!(read(&Byte, 0x5000, 100), Some(255));
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(read(&Quiet, 0x5000, 100), Some(255));
        });
        for missing in 0..installed {
            crate::wing_changes::publish_preference_lifetime(true, missing);
            assert_eq!(read(&Quiet, 0x5000, 100), None);
        }
        crate::wing_changes::publish_preference_lifetime(true, installed);
        assert_eq!(read(&Byte, 0x5000, 100), Some(255));
        crate::wing_changes::publish_preference_lifetime(false, installed);
        assert_eq!(read(&Quiet, 0x5000, 100), None);
        OWN.store(previous[0], Ordering::Release);
        PUBLICATION.store(previous[1], Ordering::Release);
        LIFETIME.store(previous[2], Ordering::Release);
        refresh();
    }
    #[test]
    fn quiet_preferences_read_nothing_and_address_or_epoch_reuse_requires_one_new_byte() {
        struct Byte(u8);
        impl Memory for Byte {
            fn copy(&self, _: usize, bytes: &mut [u8]) -> bool {
                if bytes.len() != 1 {
                    return false;
                }
                bytes[0] = self.0;
                true
            }
        }
        struct Quiet;
        impl Memory for Quiet {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("quiet carry read native memory")
            }
        }
        let mut cache = Cache::default();
        assert_eq!(cache.read(&Byte(255), 0x1000, 69, 1), Some(255));
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(cache.read(&Quiet, 0x1000, 69, 1), Some(255))
        });
        assert_eq!(cache.read(&Byte(2), 0x1000, 70, 1), Some(2));
        assert_eq!(cache.read(&Byte(3), 0x1000, 70, 2), Some(3));
        collector_test_support::with_allocations_denied(|| {
            cache.retire_identity(70);
            assert!(!cache.values.contains_key(&0x1000));
            assert!(!cache.identities.contains_key(&70));
        });
        assert_eq!(cache.read(&Byte(1), 0x1000, 70, 2), Some(1));
        struct Refused;
        impl Memory for Refused {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        assert_eq!(cache.read(&Refused, 0x2000, 80, 2), None);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(cache.read(&Quiet, 0x2000, 80, 2), None)
        });
    }
    #[test]
    fn unrelated_base_sites_need_no_arguments_or_preference_scope() {
        let _serial = SERIAL.lock().unwrap();
        let previous = AVAILABLE.swap(true, Ordering::AcqRel);
        let epoch = EPOCH.load(Ordering::Acquire);
        let revision = REVISION.load(Ordering::Acquire);
        let active = ACTIVE.load(Ordering::Acquire);
        collector_test_support::with_allocations_denied(|| {
            for site in [0, 1, 2, 3, 5, 8, 9] {
                assert!(base_site(site, &[]).is_none());
            }
        });
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert_eq!(REVISION.load(Ordering::Acquire), revision);
        assert_eq!(ACTIVE.load(Ordering::Acquire), active);
        AVAILABLE.store(previous, Ordering::Release);
    }

    #[test]
    fn split_late_write_publication_and_unwind_cannot_reuse_an_intermediate_setting() {
        let _serial = SERIAL.lock().unwrap();
        struct Byte(u8);
        impl Memory for Byte {
            fn copy(&self, _: usize, out: &mut [u8]) -> bool {
                out.fill(self.0);
                true
            }
        }
        let previous = AVAILABLE.swap(true, Ordering::AcqRel);
        invalidate();
        assert_eq!(read(&Byte(1), 0x3000, 90), Some(1));
        let split = base_site(7, &[1, 2, 3, 4]).unwrap();
        let publication = base_site(4, &[1, 0x3000, 0]).unwrap();
        publication.finish(0);
        assert_eq!(read(&Byte(2), 0x3000, 90), None);
        split.finish(0x3000);
        assert_eq!(read(&Byte(2), 0x3000, 90), Some(2));
        let epoch = EPOCH.load(Ordering::Acquire);
        let _ = std::panic::catch_unwind(|| {
            let _scope = Scope::begin(false);
            panic!("synthetic unwinding original");
        });
        assert_ne!(EPOCH.load(Ordering::Acquire), epoch);
        assert_eq!(read(&Byte(3), 0x3000, 90), Some(3));
        let destruction = destruction(0x3000).unwrap();
        assert!(
            !CACHE
                .get()
                .unwrap()
                .read()
                .unwrap()
                .values
                .contains_key(&0x3000)
        );
        destruction.finish(0);
        AVAILABLE.store(previous, Ordering::Release);
    }
    #[test]
    fn deployment_preserves_all_eight_arguments_and_return_value() {
        let _serial = SERIAL.lock().unwrap();
        unsafe extern "system-unwind" fn original(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
            f: usize,
            g: usize,
            h: usize,
        ) -> usize {
            assert_eq!([a, b, c, d, e, f, g, h], [11, 22, 33, 44, 55, 66, 77, 88]);
            99
        }
        let old = ORIGINAL[0].swap(original as *const () as usize, Ordering::AcqRel);
        assert_eq!(unsafe { deploy(11, 22, 33, 44, 55, 66, 77, 88) }, 99);
        ORIGINAL[0].store(old, Ordering::Release);
    }

    #[test]
    fn quiet_token_tracks_availability_active_scopes_and_revisions() {
        let _serial = SERIAL.lock().unwrap();
        let previous = AVAILABLE.swap(false, Ordering::AcqRel);
        assert_eq!(quiet_token(), None);
        AVAILABLE.store(true, Ordering::Release);
        let before = quiet_token().unwrap();
        assert_eq!(quiet_token(), Some(before));
        let scope = Scope::begin(false);
        assert_eq!(quiet_token(), None);
        scope.finish(0);
        let after_scope = quiet_token().unwrap();
        assert_ne!(after_scope, before);
        invalidate();
        let after_invalidate = quiet_token().unwrap();
        assert_ne!(after_invalidate, after_scope);
        AVAILABLE.store(previous, Ordering::Release);
    }
}
