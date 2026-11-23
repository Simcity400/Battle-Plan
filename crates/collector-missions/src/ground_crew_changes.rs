use collector_certificates::watch_registry::WatchRegistry;
use collector_hooks::hook::Hook;
use engine_map_air::{
    map_air_country_region_modifier as native, map_air_country_region_modifier::Region,
};
use engine_native::Memory;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

static WATCHES: WatchRegistry = WatchRegistry::new();
static ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static CACHE: Mutex<Option<Cache<Region>>> = Mutex::new(None);

type Pair = (usize, usize);

fn pair_key((country, region): Pair) -> usize {
    (country.rotate_left(17) ^ region.rotate_left(31)) | 1
}
fn object_key(object: usize) -> usize {
    object & !1
}

#[derive(Clone, Copy)]
struct Read<T> {
    object: Option<(usize, T)>,
    value: Option<u8>,
}

#[derive(Clone, Copy)]
struct Held<T> {
    quiet: (u64, u64),
    pair: (u64, u64),
    object: Option<(u64, u64)>,
    read: Read<T>,
}

struct Cache<T> {
    entries: HashMap<Pair, Held<T>>,
    objects: HashMap<Pair, (u64, Option<T>)>,
    capture: u64,
    watch_offer: usize,
    lifetime: Option<(u64, u64)>,
    charge: collector_primitives::raw_memory::Charge,
}
impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            objects: HashMap::new(),
            capture: 0,
            watch_offer: 0,
            lifetime: None,
            charge: Default::default(),
        }
    }
}

impl<T: Copy> Cache<T> {
    fn get(
        &mut self,
        watches: &WatchRegistry,
        quiet: Option<(u64, u64)>,
        key: Pair,
        read: &mut impl FnMut() -> Option<Read<T>>,
    ) -> Option<Read<T>> {
        let quiet = quiet?;
        let held = self.entries.get(&key).copied();
        if let Some(held) = held.filter(|held| held.quiet == quiet) {
            return (watches.quiet() == Some(quiet)).then_some(held.read);
        }
        let pair = watches.sequence(pair_key(key))?;
        if let Some(held) = held.filter(|held| held.pair == pair) {
            let object = match held.read.object {
                Some((object, _)) => Some(watches.sequence(object_key(object))?),
                None => None,
            };
            if object == held.object && watches.quiet() == Some(quiet) {
                self.entries.get_mut(&key)?.quiet = quiet;
                return Some(held.read);
            }
        }
        let read = read().unwrap_or(Read {
            object: None,
            value: None,
        });
        let object = match read.object {
            Some((object, _)) => Some(watches.sequence(object_key(object))?),
            None => None,
        };
        if watches.sequence(pair_key(key)) != Some(pair) || watches.quiet() != Some(quiet) {
            return None;
        }
        if !self.entries.contains_key(&key) && self.entries.len() == self.entries.capacity() {
            return None;
        }
        self.entries.insert(
            key,
            Held {
                quiet,
                pair,
                object,
                read,
            },
        );
        Some(read)
    }
}

pub struct Flags<'a, M> {
    memory: &'a M,
    base: usize,
    cache: &'a mut Cache<Region>,
    quiet: Option<(u64, u64)>,
    country_map: Option<(usize, native::CountryMap)>,
}

impl<M: Memory> collector_air_sorties::air_sorties::RegionObserver for Flags<'_, M> {
    fn detection_overrides(
        &mut self,
        emit: &mut dyn FnMut(u32, i64) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        crate::air_detection_changes::with_values(self.memory, self.base, emit)
    }

    fn observed(&mut self, country: usize, region: usize, object: Option<Region>) {
        if self.cache.objects.contains_key(&(country, region))
            || self.cache.objects.len() < self.cache.objects.capacity()
        {
            self.cache
                .objects
                .insert((country, region), (self.cache.capture, object));
        }
    }
}

impl<M: Memory> Flags<'_, M> {
    pub fn get(&mut self, country: usize, region: usize) -> Option<u8> {
        let fresh = self
            .cache
            .objects
            .get(&(country, region))
            .filter(|(capture, _)| *capture == self.cache.capture)
            .map(|(_, object)| *object);
        let memory = self.memory;
        let base = self.base;
        let country_map = &mut self.country_map;
        self.cache
            .get(&WATCHES, self.quiet, (country, region), &mut || {
                let object = match fresh {
                    Some(object) => object,
                    None => {
                        let map = match *country_map {
                            Some((owner, map)) if owner == country => map,
                            _ => {
                                let map = native::country_map(memory, base, country).ok()?;
                                *country_map = Some((country, map));
                                map
                            }
                        };
                        map.region(memory, base, region).ok()?
                    }
                };
                Some(match object {
                    Some(object) => Read {
                        object: Some((object.object(), object)),
                        value: object.ground_crews(memory),
                    },
                    None => Read {
                        object: None,
                        value: Some(0),
                    },
                })
            })
            .and_then(|read| read.value)
    }
}

pub fn with_flags<R, M: Memory>(
    memory: &M,
    base: usize,
    demand: usize,
    capture: impl FnOnce(&mut Flags<'_, M>) -> R,
) -> (R, bool) {
    let lifetime = collector_lifetime::world_lifetime::token()
        .zip(collector_observers::raid_changes::air_clear_token());
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = cache.get_or_insert_with(Cache::default);
    let reset = cache.lifetime != lifetime;
    if reset {
        cache.entries.clear();
        cache.objects.clear();
        cache.lifetime = lifetime;
    }
    cache.capture = cache.capture.wrapping_add(1);
    if cache.capture == 0 {
        cache.objects.clear();
    }
    let watches = WATCHES
        .registered()
        .saturating_add(demand.saturating_mul(2));
    let growing = watches > cache.watch_offer;
    if growing {
        cache.watch_offer = watches.checked_next_power_of_two().unwrap_or(usize::MAX);
    }
    WATCHES.offer(cache.watch_offer);
    WATCHES.boundary(reset || growing);
    let storage = if cache.entries.len().saturating_add(demand) > cache.entries.capacity() {
        cache.entries.try_reserve(demand).is_ok()
    } else {
        true
    };
    let object_demand = demand.saturating_mul(2);
    if cache.objects.len().saturating_add(object_demand) > cache.objects.capacity() {
        let _ = cache.objects.try_reserve(object_demand);
    }
    let quiet = (storage && lifetime.is_some())
        .then(|| WATCHES.quiet())
        .flatten();
    let result = capture(&mut Flags {
        memory,
        base,
        cache,
        quiet,
        country_map: None,
    });
    cache
        .charge
        .set(collector_primitives::raw_memory::Capacity {
            index_map_entries_lower_bound: (cache.entries.capacity()
                * std::mem::size_of::<(Pair, Held<Region>)>()
                + cache.objects.capacity() * std::mem::size_of::<(Pair, (u64, Option<Region>))>())
                as u64,
            ..Default::default()
        });
    (
        result,
        quiet.is_some()
            && WATCHES.quiet() == quiet
            && collector_lifetime::world_lifetime::token()
                .zip(collector_observers::raid_changes::air_clear_token())
                == lifetime,
    )
}

pub fn active() -> usize {
    WATCHES.active()
}
pub fn collect() {
    WATCHES.collect();
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        if !cfg!(feature = "mission_changes") {
            return Ok(Self { hooks: Vec::new() });
        }
        let sites = [
            (
                native::GROUND_CREWS_ENABLE_RVA,
                enable as *const () as usize,
            ),
            (
                native::GROUND_CREWS_DISABLE_RVA,
                disable as *const () as usize,
            ),
            (
                native::GROUND_CREWS_CONSTRUCTOR_RVA,
                construct as *const () as usize,
            ),
        ];
        let mut hooks = Vec::new();
        let names = ["enable", "disable", "allocator construction"];
        for (index, (rva, callback)) in sites.into_iter().enumerate() {
            let (at, prefix) = engine_sites::hook(base, rva)
                .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                .map_err(|reason| {
                    crate::mission_changes::hook_failure(
                        "ground_crews",
                        "verification failed",
                        names[index],
                        reason,
                    )
                })?;
            let hook = unsafe { Hook::new(at, callback, prefix) }.map_err(|reason| {
                crate::mission_changes::hook_failure(
                    "ground_crews",
                    "construction failed",
                    names[index],
                    reason,
                )
            })?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            WATCHES.set_enabled(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled).map_err(|reason| {
            crate::mission_changes::hook_failure(
                "ground_crews",
                "publication failed",
                "enable, disable, allocator construction",
                reason,
            )
        })? {
            return Ok(false);
        }
        WATCHES.set_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe { WATCHES.release() };
        *CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

macro_rules! writer {
    ($name:ident, $site:expr, $key:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::MissionChanges,
            );
            let key: fn(usize, usize) -> usize = $key;
            let _notification = WATCHES.notify(key(first, second));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
writer!(enable, 0, |country, region| pair_key((country, region)));
writer!(disable, 1, |country, region| pair_key((country, region)));
writer!(construct, 2, |object, _| object_key(object));

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_certified_flags_and_lookups_carry_until_their_writer_changes() {
        let watches = WatchRegistry::new();
        watches.offer(16);
        watches.boundary(false);
        watches.set_enabled(true);
        let mut cache = Cache::<()>::default();
        cache.entries.try_reserve(2).unwrap();
        for resolved in [false, true] {
            let key = (if resolved { 16 } else { 32 }, 0x100);
            let mut attempts = 0;
            for _hour in 0..2 {
                let value = cache
                    .get(&watches, watches.quiet(), key, &mut || {
                        attempts += 1;
                        resolved.then_some(Read {
                            object: Some((64, ())),
                            value: None,
                        })
                    })
                    .and_then(|read| read.value);
                assert_eq!(value, None);
            }
            assert_eq!(attempts, 1);
            drop(watches.notify(pair_key((48, 0x100))));
            assert_eq!(
                cache
                    .get(&watches, watches.quiet(), key, &mut || panic!(
                        "unrelated write retried failed flag"
                    ))
                    .and_then(|read| read.value),
                None
            );
            let changed = if resolved {
                object_key(64)
            } else {
                pair_key(key)
            };
            drop(watches.notify(changed));
            let value = cache
                .get(&watches, watches.quiet(), key, &mut || {
                    attempts += 1;
                    Some(Read {
                        object: Some((64, ())),
                        value: Some(1),
                    })
                })
                .and_then(|read| read.value);
            assert_eq!(value, Some(1));
            assert_eq!(attempts, 2);
        }
    }

    #[test]
    fn quiet_and_unrelated_writers_carry_but_pair_and_constructor_writers_read() {
        let watches = WatchRegistry::new();
        watches.offer(15_000);
        watches.boundary(false);
        watches.set_enabled(true);
        let mut cache = Cache::<()>::default();
        cache.entries.try_reserve(5000).unwrap();
        for owner in 1..=5000 {
            let key = (owner * 16, 0x100);
            assert!(
                cache
                    .get(&watches, watches.quiet(), key, &mut || Some(Read {
                        object: Some((owner * 32, ())),
                        value: Some(1)
                    }))
                    .is_some()
            );
        }
        let key = (16, 0x100);
        assert_eq!(
            cache
                .get(&watches, watches.quiet(), key, &mut || panic!(
                    "quiet flag read"
                ))
                .unwrap()
                .value,
            Some(1)
        );
        drop(watches.notify(pair_key((32, 0x100))));
        assert_eq!(
            cache
                .get(&watches, watches.quiet(), key, &mut || panic!(
                    "unrelated flag read"
                ))
                .unwrap()
                .value,
            Some(1)
        );
        drop(watches.notify(pair_key(key)));
        assert_eq!(
            cache
                .get(&watches, watches.quiet(), key, &mut || Some(Read {
                    object: Some((32, ())),
                    value: Some(0)
                }))
                .unwrap()
                .value,
            Some(0)
        );
        drop(watches.notify(object_key(32)));
        assert_eq!(
            cache
                .get(&watches, watches.quiet(), key, &mut || Some(Read {
                    object: Some((32, ())),
                    value: Some(1)
                }))
                .unwrap()
                .value,
            Some(1)
        );
        let running = watches.notify(pair_key(key));
        assert!(
            cache
                .get(&watches, watches.quiet(), key, &mut || panic!(
                    "uncertified polling"
                ))
                .is_none()
        );
        drop(running);
        watches.set_enabled(false);
        assert!(
            cache
                .get(&watches, watches.quiet(), key, &mut || panic!(
                    "disabled polling"
                ))
                .is_none()
        );
    }
}
