use collector_hooks::hook::Hook;
use engine_map_air::map_air_sortie_inputs as native;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

struct Domain {
    enabled: AtomicBool,
    sequence: AtomicU64,
    active: AtomicUsize,
}
impl Domain {
    const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            active: AtomicUsize::new(0),
        }
    }
    fn token(&self) -> Option<u64> {
        if !self.enabled.load(Ordering::SeqCst) || self.active.load(Ordering::SeqCst) != 0 {
            return None;
        }
        let sequence = self.sequence.load(Ordering::SeqCst);
        (self.active.load(Ordering::SeqCst) == 0
            && self.sequence.load(Ordering::SeqCst) == sequence)
            .then_some(sequence)
    }
    fn set_enabled(&self, enabled: bool) {
        if self.enabled.load(Ordering::SeqCst) != enabled {
            if !enabled {
                self.enabled.store(false, Ordering::SeqCst);
            }
            self.sequence.fetch_add(1, Ordering::SeqCst);
            self.enabled.store(enabled, Ordering::SeqCst);
        }
    }
    fn begin(&self) {
        self.active.fetch_add(1, Ordering::SeqCst);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    fn end(&self) {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
    fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
}
static DOMAIN: Domain = Domain::new();
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

pub fn active() -> usize {
    DOMAIN.active()
}

#[derive(Default)]
struct Cache {
    token: Option<u64>,
    failure: Option<&'static str>,
    entries: Vec<(u32, i64)>,
    charge: collector_primitives::raw_memory::Charge,
}
impl Cache {
    fn refresh(
        &mut self,
        domain: &Domain,
        read: impl FnOnce(&mut Vec<(u32, i64)>) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let token = domain
            .token()
            .ok_or("Air detection override certificate unavailable")?;
        if self.token != Some(token) {
            self.failure = read(&mut self.entries).err();
            if domain.token() != Some(token) {
                self.token = None;
                self.entries.clear();
                return Err("Air detection overrides changed during acquisition");
            }
            self.token = Some(token);
        }
        self.failure.map_or(Ok(()), Err)
    }
}

pub fn with_values(
    memory: &impl Memory,
    base: usize,
    emit: &mut dyn FnMut(u32, i64) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut held = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = held.get_or_insert_with(Cache::default);
    let result = cache.refresh(&DOMAIN, |entries| {
        native::detection_overrides(memory, base, entries)
    });
    cache
        .charge
        .set(collector_primitives::raw_memory::Capacity {
            auxiliary_vectors: (cache.entries.capacity() * size_of::<(u32, i64)>()) as u64,
            ..Default::default()
        });
    result?;
    for &(slot, value) in &cache.entries {
        emit(slot, value)?;
    }
    if DOMAIN.token() != cache.token {
        return Err("Air detection overrides changed during publication");
    }
    Ok(())
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "mission_changes") {
            let (at, prefix) = engine_sites::hook(base, native::DETECTION_OVERRIDE_WRITER_RVA)
                .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                .map_err(|reason| {
                    crate::mission_changes::hook_failure(
                        "air_detection_overrides",
                        "verification failed",
                        "console vector writer",
                        reason,
                    )
                })?;
            let hook = unsafe { Hook::new(at, writer as *const () as usize, prefix) }.map_err(
                |error| {
                    crate::mission_changes::hook_failure(
                        "air_detection_overrides",
                        "construction failed",
                        "console vector writer",
                        error,
                    )
                },
            )?;
            ORIGINAL.store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            DOMAIN.set_enabled(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled).map_err(|error| {
            crate::mission_changes::hook_failure(
                "air_detection_overrides",
                "publication failed",
                "console vector writer",
                error,
            )
        })? {
            return Ok(false);
        }
        DOMAIN.set_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        DOMAIN.set_enabled(false);
        *CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}
unsafe extern "system" fn writer(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::MissionChanges);
    DOMAIN.begin();
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    DOMAIN.end();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabling_observation_does_not_hide_an_original_still_in_flight() {
        let domain = Domain::new();
        domain.set_enabled(true);
        assert_eq!(domain.active(), 0);
        domain.begin();
        assert_eq!(domain.active(), 1);
        domain.set_enabled(false);
        assert_eq!(domain.active(), 1);
        assert!(domain.token().is_none());
        domain.end();
        assert_eq!(domain.active(), 0);
    }
    #[test]
    fn quiet_and_failed_observations_are_held_until_the_console_writer_changes() {
        let domain = Domain::new();
        domain.enabled.store(true, Ordering::SeqCst);
        let mut cache = Cache::default();
        cache
            .refresh(&domain, |rows| {
                rows.push((7, -123_456));
                Ok(())
            })
            .unwrap();
        cache
            .refresh(&domain, |_| panic!("quiet vector was polled"))
            .unwrap();
        domain.begin();
        assert!(
            cache
                .refresh(&domain, |_| panic!("active writer was read"))
                .is_err()
        );
        domain.end();
        assert_eq!(
            cache.refresh(&domain, |_| Err("synthetic read refusal")),
            Err("synthetic read refusal")
        );
        assert_eq!(
            cache.refresh(&domain, |_| panic!("failed witness was polled")),
            Err("synthetic read refusal")
        );
        domain.begin();
        domain.end();
        cache
            .refresh(&domain, |rows| {
                rows.clear();
                Ok(())
            })
            .unwrap();
        assert!(cache.entries.is_empty());
    }
    #[test]
    fn a_writer_during_read_never_certifies_the_partial_vector() {
        let domain = Domain::new();
        domain.enabled.store(true, Ordering::SeqCst);
        let mut cache = Cache::default();
        assert!(
            cache
                .refresh(&domain, |rows| {
                    rows.push((7, 123));
                    domain.begin();
                    domain.end();
                    Ok(())
                })
                .is_err()
        );
        assert!(cache.token.is_none());
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn disabling_observation_revokes_a_previously_held_vector() {
        let domain = Domain::new();
        domain.set_enabled(true);
        let mut cache = Cache::default();
        cache
            .refresh(&domain, |rows| {
                rows.push((7, 1));
                Ok(())
            })
            .unwrap();
        domain.set_enabled(false);
        assert!(
            cache
                .refresh(&domain, |_| panic!("disabled vector was read"))
                .is_err()
        );
        domain.set_enabled(true);
        cache
            .refresh(&domain, |rows| {
                rows.clear();
                rows.push((7, 2));
                Ok(())
            })
            .unwrap();
        assert_eq!(cache.entries, [(7, 2)]);
        cache
            .refresh(&domain, |_| panic!("re-certified vector was polled"))
            .unwrap();
    }
}
