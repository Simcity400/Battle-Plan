//! Radar range operands, read only for newly demanded or writer-changed groups.
//! The coverage census already authenticates each group and host. Its worker
//! carries these additional operands under the group-update writer certificate.
use crate::{hook::Hook, watch_registry::WatchRegistry};
use engine_map_air::{radar_group, radar_group::Symbol};
use engine_native::Memory;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

static WATCHES: WatchRegistry = WatchRegistry::new();
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
static CARRIED: AtomicU64 = AtomicU64::new(0);
static READ: AtomicU64 = AtomicU64::new(0);
static REFUSED: AtomicU64 = AtomicU64::new(0);
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Fingerprint of the supported radar-update entry prefix; bytes are read
/// from the installation at run time, never stored in the source tree.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

#[derive(Clone, Copy)]
struct Held {
    host: usize,
    certificate: (u64, u64),
    quiet: (u64, u64),
    value: Option<Symbol>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<usize, Held>,
    charge: crate::raw_memory::Charge,
}

impl Cache {
    fn get(
        &mut self,
        watches: &WatchRegistry,
        quiet: Option<(u64, u64)>,
        group: usize,
        host: usize,
        read: &mut impl FnMut(bool) -> Option<Symbol>,
    ) -> (Option<Symbol>, bool) {
        let Some(quiet) = quiet else {
            return (None, false);
        };
        let held = self
            .entries
            .get(&group)
            .copied()
            .filter(|held| held.host == host);
        if let Some(held) = held.filter(|held| held.quiet == quiet) {
            return (
                if watches.quiet() == Some(quiet) {
                    held.value
                } else {
                    None
                },
                true,
            );
        }
        let Some(certificate) = watches.sequence(group) else {
            return (None, false);
        };
        if let Some(held) = held.filter(|held| held.certificate == certificate) {
            if watches.quiet() != Some(quiet) {
                return (None, false);
            }
            self.entries.get_mut(&group).unwrap().quiet = quiet;
            return (held.value, true);
        }
        let full =
            held.is_none_or(|held| held.certificate.0 != certificate.0 || held.value.is_none());
        let value = read(full);
        if watches.sequence(group) != Some(certificate) || watches.quiet() != Some(quiet) {
            return (None, false);
        }
        self.entries.insert(
            group,
            Held {
                host,
                certificate,
                quiet,
                value,
            },
        );
        (value, false)
    }
}

/// Runs inside the existing radar worker. Sizing and registry housekeeping
/// remain here too; the hourly callback gains no scan, allocation or native read.
pub(crate) fn with_symbols<R>(
    memory: &impl Memory,
    capacity: usize,
    reset: bool,
    capture: impl FnOnce(&mut dyn FnMut(usize, usize) -> Option<Symbol>) -> R,
) -> (R, bool) {
    WATCHES.offer(capacity);
    WATCHES.boundary(reset);
    let quiet = WATCHES.quiet();
    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let cache = cache.get_or_insert_with(Cache::default);
    if reset {
        cache.entries.clear();
    }
    if capacity > cache.entries.capacity() {
        let _ = cache
            .entries
            .try_reserve(capacity.saturating_sub(cache.entries.len()));
    }
    let (mut carried, mut read_count, mut refused) = (0, 0, 0);
    let result = capture(&mut |group, host| {
        let mut demanded = false;
        let (value, reused) = cache.get(&WATCHES, quiet, group, host, &mut |full| {
            demanded = true;
            if full {
                radar_group::symbol(memory, group, host)
            } else {
                radar_group::radius(memory, group).map(|radius| Symbol { radius })
            }
        });
        carried += u64::from(reused);
        read_count += u64::from(demanded);
        refused += u64::from(value.is_none());
        value
    });
    CARRIED.fetch_add(carried, Ordering::Relaxed);
    READ.fetch_add(read_count, Ordering::Relaxed);
    REFUSED.fetch_add(refused, Ordering::Relaxed);
    cache.charge.set(crate::raw_memory::Capacity {
        index_map_entries_lower_bound: (cache.entries.capacity()
            * std::mem::size_of::<(usize, Held)>()) as u64,
        ..Default::default()
    });
    (result, quiet.is_some() && WATCHES.quiet() == quiet)
}

pub(crate) fn active() -> usize {
    WATCHES.active()
}
pub(crate) fn collect() {
    WATCHES.collect();
}

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "events": CALLS.load(Ordering::Relaxed),
        "carried_groups": CARRIED.load(Ordering::Relaxed),
        "read_groups": READ.load(Ordering::Relaxed),
        "refused_groups": REFUSED.load(Ordering::Relaxed),
        "watches": WATCHES.status(),
        "scope": "cumulative since attach; newly encountered or writer-changed radar groups alone read symbol operands in the existing coverage worker; all others are certified carry; missing certification refuses symbols without polling"
    })
}

pub(crate) struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    pub(crate) unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "mission_changes") {
            let prefix = engine_native::read::<[u8; 16]>(memory, base + radar_group::UPDATE_RVA)
                .ok_or_else(|| {
                    crate::mission_changes::hook_failure(
                        "radar_symbols",
                        "verification failed",
                        "group update",
                        "entry prefix unreadable",
                    )
                })?;
            if fingerprint(&prefix) != radar_group::UPDATE_PREFIX_HASH {
                return Err(crate::mission_changes::hook_failure(
                    "radar_symbols",
                    "verification failed",
                    "group update",
                    "entry fingerprint mismatch",
                ));
            }
            let hook = unsafe {
                Hook::new(
                    base + radar_group::UPDATE_RVA,
                    update as *const () as usize,
                    prefix,
                )
            }
            .map_err(|reason| {
                crate::mission_changes::hook_failure(
                    "radar_symbols",
                    "construction failed",
                    "group update",
                    reason,
                )
            })?;
            ORIGINAL.store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(Self { hooks })
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.set_enabled(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled).map_err(|reason| {
            crate::mission_changes::hook_failure(
                "radar_symbols",
                "publication failed",
                "group update",
                reason,
            )
        })? {
            return Ok(false);
        }
        WATCHES.set_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub(crate) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // The enclosing family is disabled and drained before releasing its registries.
        unsafe { WATCHES.release() };
        *CACHE.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

unsafe extern "system" fn update(
    group: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::MissionChanges);
    CALLS.fetch_add(1, Ordering::Relaxed);
    let _notification = WATCHES.notify(group);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(group, second, third, fourth) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_certified_radius_is_read_once_across_hours_and_unrelated_writes() {
        let watches = WatchRegistry::new();
        watches.offer(16);
        watches.boundary(false);
        watches.set_enabled(true);
        let mut cache = Cache::default();
        let mut attempts = 0;
        for hour in 0..2 {
            let (value, reused) = cache.get(&watches, watches.quiet(), 16, 32, &mut |_| {
                attempts += 1;
                None
            });
            assert_eq!(value, None);
            assert_eq!(reused, hour != 0);
        }
        assert_eq!(attempts, 1);
        drop(watches.notify(48));
        assert_eq!(
            cache.get(&watches, watches.quiet(), 16, 32, &mut |_| panic!(
                "unrelated write retried failed radius"
            )),
            (None, true)
        );
        drop(watches.notify(16));
        let (value, reused) = cache.get(&watches, watches.quiet(), 16, 32, &mut |full| {
            assert!(full, "failed initial authentication must be reacquired");
            attempts += 1;
            Some(Symbol { radius: 99 })
        });
        assert_eq!(value, Some(Symbol { radius: 99 }));
        assert!(!reused);
        assert_eq!(attempts, 2);
    }

    #[test]
    fn quiet_and_unrelated_writes_read_no_symbol_fields_and_a_changed_group_reads_only_radius() {
        let watches = WatchRegistry::new();
        watches.offer(5_000);
        watches.boundary(false);
        watches.set_enabled(true);
        let mut cache = Cache::default();
        for group in 1..=5_000 {
            assert!(
                cache
                    .get(
                        &watches,
                        watches.quiet(),
                        group * 16,
                        group * 32,
                        &mut |full| {
                            assert!(full);
                            Some(Symbol {
                                radius: group as i64,
                            })
                        }
                    )
                    .0
                    .is_some()
            );
        }
        let quiet = watches.quiet();
        for group in 1..=5_000 {
            let (value, reused) = cache.get(&watches, quiet, group * 16, group * 32, &mut |_| {
                panic!("quiet read")
            });
            assert!(reused);
            assert_eq!(value.unwrap().radius, group as i64);
        }
        drop(watches.notify(32));
        assert!(
            cache
                .get(&watches, watches.quiet(), 16, 32, &mut |_| panic!(
                    "unrelated read"
                ))
                .1
        );
        let (value, reused) = cache.get(&watches, watches.quiet(), 32, 64, &mut |full| {
            assert!(!full);
            Some(Symbol { radius: 99 })
        });
        assert!(!reused);
        assert_eq!(value, Some(Symbol { radius: 99 }));
        let running = watches.notify(32);
        assert_eq!(
            cache
                .get(&watches, watches.quiet(), 32, 64, &mut |_| panic!(
                    "uncertified read"
                ))
                .0,
            None
        );
        drop(running);
        watches.set_enabled(false);
        assert!(
            cache
                .get(&watches, watches.quiet(), 32, 64, &mut |_| panic!(
                    "disabled read"
                ))
                .0
                .is_none()
        );
    }
}
