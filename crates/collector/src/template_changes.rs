//! Allocation-free notifications from the native writers of a division
//! template's fallback name key ([`engine::template_mutations`]). A detour
//! only notifies the shared watch registry ([`WatchRegistry`]) for the
//! `CReferencedDivisionTemplate` its first argument names, counts its site
//! and calls the original; it never changes game behaviour. Receivers are the
//! templates whose keys a DivisionTemplates capture registered; a staging copy
//! the designer assigns is never registered. Anything unobserved (disabled,
//! poisoned, a refused registration, an epoch change, a running writer) makes
//! the capture read that template's names afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{
    Memory,
    template_mutations::{DATA, MUTATIONS, Receiver},
};
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read and verified keys, mismatches and quiet captures.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a carried DivisionTemplates name key differs from a fresh read",
]);
/// Templates the latest complete capture held, for off-thread sizing.
static TEMPLATES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched templates and each writer site's
/// calls.
pub(crate) struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
}

impl Watches {
    pub(crate) const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `template`'s names. Dropping the guard notifies its
    /// return.
    pub(crate) fn notify(&self, template: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(template)
    }

    /// An enabled domain with a table for `objects` receivers installed.
    #[cfg(test)]
    pub(crate) fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(test)]
    pub(crate) fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a walk sees of the notifications.
pub(crate) trait Changes: Sync {
    /// `(epoch, revision)` of the whole domain while it is observed and no
    /// writer runs; `None` otherwise. Equal values across hours prove that no
    /// observed writer ran in between, so every certificate taken after the
    /// earlier value still holds without looking it up.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one template, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched it.
    fn sequence(&self, template: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with a carried key: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, template: usize) -> Option<(u64, u64)> {
        self.registry.sequence(template)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub(crate) struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, template: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(template)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every carryable key afresh and compare, rather than
/// the hour's sampled slice ([`crate::sampled_verify`]).
pub(crate) const VERIFY: bool = cfg!(feature = "template_verify");

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off the game thread: size the watches for the templates the latest
/// capture held. Nothing is offered before a capture held them: a later
/// larger table waits for an invalidating hour, while the first installs at
/// once.
pub fn presize() {
    let templates = TEMPLATES.load(Ordering::Relaxed);
    if templates != 0 {
        WATCHES.registry.offer(templates);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's name keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    /// The capture's certificates held by the domain's unchanged quiet token,
    /// without a per-template lookup.
    pub quiet: bool,
}

/// A worker publishes one capture's counts and, when it held its templates,
/// their number for sizing.
pub(crate) fn record(counts: &Counts, templates: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(templates) = templates {
        TEMPLATES.store(templates, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "template_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_keys": reads[0],
        "read_keys": reads[1],
        "verified_keys": reads[2],
        "mismatches": reads[3],
        "quiet_hours": reads[4],
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; DivisionTemplates is derived every hour from the template icon walk, which reads each template's handle, owner, literal name and unit-type operands; a template's fallback name key, read by that walk only behind an empty literal, is carried (reused_keys) when the template object and handle are those of the held capture and either the domain's quiet token is unchanged since before that capture (quiet_hours) or no hooked writer touched the template since it registered; every other key is read afresh with its literal (read_keys); a capture is held only when no hooked writer ran during it; every hour also reads the carryable keys of its slice afresh (one in rotation_hours by template id; every one in verify mode), publishes the fresh names and, after an acquire fence, withdraws the key capture if its final certificate moved; a key difference under an unchanged certificate disables key reuse for the attachment (sampled_*); failed key reads refuse the table and forget the store"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; DivisionTemplates
    /// then reads every template's names afresh.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "template_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                assign_editable_template_data as *const () as usize,
                assign_template_data as *const () as usize,
                read_template_data as *const () as usize,
                destroy_template as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

/// The template a site's first argument names.
fn receiver(receiver: Receiver, first: usize) -> usize {
    match receiver {
        Receiver::Data => first.saturating_sub(DATA),
        Receiver::Template => first,
    }
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the detour forwards all four. The
// notification covers the original in flight.
// Foreign unwind may cross a detour. Keep the notification active on escape:
// only a normal native return proves that its flight finished.
type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::TemplateChanges);
            let notification =
                ManuallyDrop::new(WATCHES.notify(receiver(MUTATIONS[$site].1, first), $site));
            let original: NativeFunction =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            drop(ManuallyDrop::into_inner(notification));
            result
        }
    };
}
// Editable-data assignment (dest data, source data).
callback!(assign_editable_template_data, 0);
// Full-data assignment (dest data, source data).
callback!(assign_template_data, 1);
// Read template data (data, reader, token).
callback!(read_template_data, 2);
// Division template deleting destructor(template, flags).
callback!(destroy_template, 3);

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer moves only the template it names (an assignment names it
    /// through its contained data), and every writer moves the quiet token.
    #[test]
    fn a_writer_moves_only_its_templates_certificate_and_the_quiet_token() {
        let watches = Watches::enabled(16);
        let [first, second] =
            [0x1000, 0x2000].map(|template| Changes::sequence(&watches, template).unwrap());
        let quiet = Changes::quiet(&watches).unwrap();
        let named = receiver(MUTATIONS[0].1, 0x2000 + DATA);
        assert_eq!(named, 0x2000);
        drop(watches.notify(named, 0));
        assert_eq!(Changes::sequence(&watches, 0x1000), Some(first));
        assert_ne!(Changes::sequence(&watches, 0x2000), Some(second));
        assert_ne!(Changes::quiet(&watches), Some(quiet));
        assert_eq!(watches.calls[0].load(Ordering::Relaxed), 1);
        assert_eq!(receiver(MUTATIONS[3].1, 0x1000), 0x1000);
        // A running writer certifies nothing.
        let running = watches.notify(0x1000, 3);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        assert!(Changes::quiet(&watches).is_none());
        drop(running);
        assert!(!watches.poisoned());
    }
}
