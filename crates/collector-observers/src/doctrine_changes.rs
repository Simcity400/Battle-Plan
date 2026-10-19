//! Doctrine notifications share publication and lifetime with world-input hooks.
//! The hook path notifies its exact receiver, calls the original, then hands
//! that receiver to the bound capture while the native object is still owned.
//! Hourly publication transfers owned events without reading these objects.
use crate::{hook::Hook, watch_registry::WatchRegistry};
pub use engine_mutations::doctrine_mutations::Domain;
use engine_mutations::doctrine_mutations::{DOMAINS, SITES};
use engine_native::Memory;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub const GAIN_SITE_NAMES: [&str;
    engine_mutations::doctrine_mutations::EFFECTIVE_GAIN_STORES.len()] = [
    "doctrine computed daily-mastery store",
    "doctrine base daily-mastery store",
];

static HANDLER: AtomicUsize = AtomicUsize::new(0);
static COST_HANDLER: AtomicUsize = AtomicUsize::new(0);
static GAIN_HANDLER: AtomicUsize = AtomicUsize::new(0);
static CONTEXT: AtomicUsize = AtomicUsize::new(0);
/// # Safety
/// The host retains `context` until all owned native callbacks have quiesced.
pub unsafe fn bind(context: usize, handler: unsafe fn(usize, Domain, usize)) {
    CONTEXT.store(context, Ordering::Release);
    HANDLER.store(handler as *const () as usize, Ordering::Release);
}
/// # Safety
/// Uses the same retained context and quiescence contract as `bind`.
pub unsafe fn bind_gain(handler: unsafe fn(usize, usize, i64)) {
    GAIN_HANDLER.store(handler as *const () as usize, Ordering::Release);
}
/// # Safety
/// Uses the same retained context and quiescence contract as `bind`.
pub unsafe fn bind_cost_consumption(handler: unsafe fn(usize, usize, usize)) {
    COST_HANDLER.store(handler as *const () as usize, Ordering::Release);
}
/// # Safety
/// The host owns these taps together with the enclosing observer family.
pub unsafe fn gain_taps(
    memory: &impl Memory,
    base: usize,
) -> Result<Vec<crate::hook::Tap>, &'static str> {
    let mut taps = engine_allocation::vec(2).map_err(|_| "doctrine gain tap allocation failed")?;
    for (index, &(rva, sentinel)) in engine_mutations::doctrine_mutations::EFFECTIVE_GAIN_STORES
        .iter()
        .enumerate()
    {
        if engine_native::read::<[u8; 16]>(memory, base + rva) != Some(sentinel) {
            return Err("doctrine gain store sentinel differs");
        }
        taps.push(
            unsafe { crate::hook::Tap::new(base + rva, gain_store, index, sentinel) }
                .map_err(|_| "doctrine gain tap construction failed")?,
        );
    }
    Ok(taps)
}
unsafe extern "system-unwind" fn gain_store(registers: &crate::hook::Registers, index: usize) {
    let _notification = WATCHES[Domain::Track as usize].notify(0);
    let handler = GAIN_HANDLER.load(Ordering::Acquire);
    if handler != 0 {
        let handler: unsafe fn(usize, usize, i64) = unsafe { std::mem::transmute(handler) };
        // The exact destination and source of the intercepted native store.
        // Neither operand requires a memory read or a country/track walk.
        let pointer = registers.rsi.wrapping_add(registers.r14);
        let value = if index == 0 {
            registers.rax
        } else {
            registers.r15
        } as i64;
        unsafe { handler(CONTEXT.load(Ordering::Acquire), pointer, value) };
    }
}
static WATCHES: [WatchRegistry; DOMAINS] = [const { WatchRegistry::new() }; DOMAINS];
static ORIGINAL: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];
static CALLS: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];

pub fn active() -> usize {
    WATCHES.iter().map(WatchRegistry::active).sum()
}
pub fn invalidate() {
    for watch in &WATCHES {
        watch.boundary(true);
    }
}
pub fn token(domain: Domain) -> Option<(u64, u64)> {
    WATCHES[domain as usize].quiet()
}
pub fn enabled(enabled: bool) {
    for watch in &WATCHES {
        watch.set_enabled(enabled);
    }
}
/// Free every doctrine certificate table.
///
/// # Safety
/// The world-input hooks and forwarded notifications must be removed and drained,
/// and the acquisition pool must be stopped so no notification or capture can run.
pub unsafe fn release() {
    for watch in &WATCHES {
        unsafe {
            watch.release();
        }
    }
}

pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "events": crate::status::map(SITES.iter().zip(&CALLS).map(|(site, count)| (site.name, count.load(Ordering::Relaxed)))),
        "watches": WATCHES.each_ref().map(WatchRegistry::status),
        "scope": "native writer boundaries capture exact changed receivers; the hourly boundary only hands off owned events; no periodic fallback"
    })
}

/// Constructed and published with the enclosing world-input family. All sites
/// take integer/pointer operands and return only RAX (or no value).
///
/// # Safety
/// The host verified the executable and owns restoration and quiescence of
/// every returned hook and its observer storage before unloading the collector.
pub unsafe fn hooks(memory: &impl Memory, base: usize) -> Result<Vec<Hook>, &'static str> {
    if SITES
        .iter()
        .any(|site| engine_native::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel))
    {
        return Err("doctrine writer sentinel differs");
    }
    let callbacks: [usize; SITES.len()] = [
        reset_status as *const () as usize,
        select_grand as *const () as usize,
        select_sub as *const () as usize,
        reset_track as *const () as usize,
        clear_track as *const () as usize,
        add_mastery as *const () as usize,
        add_bank as *const () as usize,
        refresh_sources as *const () as usize,
        complete_sub as *const () as usize,
        add_cost as *const () as usize,
        consume_cost as *const () as usize,
        add_grant as *const () as usize,
        daily_grants as *const () as usize,
        apply_reward as *const () as usize,
        remove_reward as *const () as usize,
        post_read as *const () as usize,
    ];
    let mut hooks =
        engine_allocation::vec(SITES.len()).map_err(|_| "doctrine hook allocation failed")?;
    for (index, site) in SITES.iter().enumerate() {
        let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
            .map_err(|_| "doctrine hook construction failed")?;
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    Ok(hooks)
}

macro_rules! callback {
    ($name:ident, $site:expr $(, $extra:ident)*) => {
        unsafe extern "system" fn $name(first: usize, second: usize, third: usize, fourth: usize $(, $extra: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
            CALLS[$site].fetch_add(1, Ordering::Relaxed);
            let _notification = WATCHES[SITES[$site].domain as usize].notify(first);
            let original: unsafe extern "system" fn(usize, usize, usize, usize $(, callback!(@type $extra))*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth $(, $extra)*) });
            let handler = HANDLER.load(Ordering::Acquire);
            if handler != 0 {
                let handler: unsafe fn(usize, Domain, usize) = unsafe { std::mem::transmute(handler) };
                unsafe { handler(CONTEXT.load(Ordering::Acquire), SITES[$site].domain, first) };
            }
            result
        }
    };
    (@type $extra:ident) => { usize };
}
callback!(reset_status, 0);
callback!(select_grand, 1);
callback!(select_sub, 2);
callback!(reset_track, 3);
callback!(clear_track, 4);
callback!(add_mastery, 5);
callback!(add_bank, 6);
callback!(refresh_sources, 7);
callback!(complete_sub, 8);
callback!(add_cost, 9, fifth);
unsafe extern "system" fn consume_cost(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    CALLS[10].fetch_add(1, Ordering::Relaxed);
    let _notification = WATCHES[SITES[10].domain as usize].notify(first);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[10].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    let handler = COST_HANDLER.load(Ordering::Acquire);
    if handler != 0 {
        let handler: unsafe fn(usize, usize, usize) = unsafe { std::mem::transmute(handler) };
        unsafe { handler(CONTEXT.load(Ordering::Acquire), first, third) };
    }
    result
}
callback!(add_grant, 11);
callback!(daily_grants, 12);
callback!(apply_reward, 13, fifth, sixth);
callback!(remove_reward, 14, fifth);
callback!(post_read, 15);
