//! The sole entry patch owner for division-pool membership primitives.
use crate::hook::Hook;
use collector_observers::division_panel_changes as changes;
use engine_mutations::division_panel_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};
static ORIGINAL: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
pub struct Hooks {
    hooks: Vec<Hook>,
}
fn failure(site: Option<usize>, reason: &str) -> &'static str {
    let sites = site.map_or_else(
        || {
            MUTATIONS
                .iter()
                .map(|site| site.name)
                .collect::<Vec<_>>()
                .join(", ")
        },
        |index| MUTATIONS[index].name.into(),
    );
    Box::leak(format!("division_panel_changes [{sites}]: {reason}").into_boxed_str())
}
impl Hooks {
    /// # Safety
    /// The enclosing family owns the authenticated executable and patch lifetime.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        let callbacks = [
            add_pool as *const () as usize,
            add_variant as *const () as usize,
            set_variant as *const () as usize,
            scale_pool as *const () as usize,
            clear_pool as *const () as usize,
            assign_pool as *const () as usize,
        ];
        if cfg!(feature = "army_stat_changes") {
            for (index, site) in MUTATIONS.iter().enumerate() {
                if engine_native::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err(failure(Some(index), "instruction sentinel mismatch"));
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|why| failure(Some(index), &why))?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            changes::patched(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|why| failure(None, &why))?
        {
            return Ok(false);
        }
        changes::patched(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            changes::release();
        }
    }
}
macro_rules! callback {
    ($name:ident,$index:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
            fifth: usize,
            sixth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
            let _mark = changes::notify(first);
            let original: unsafe extern "system" fn(
                usize,
                usize,
                usize,
                usize,
                usize,
                usize,
            ) -> usize = unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth, fifth, sixth) })
        }
    };
}
callback!(add_pool, 0);
callback!(add_variant, 1);
callback!(set_variant, 2);
callback!(scale_pool, 3);
callback!(clear_pool, 4);
callback!(assign_pool, 5);
