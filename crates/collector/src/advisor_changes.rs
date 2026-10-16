//! Bounded advisor-map notifications. Only authenticated exact map addresses
//! are watched; generic tree helpers also operate on unrelated template maps.
//! This family also owns the character role-row writer sites
//! ([`engine::character_role_mutations`]) and forwards every advisor-family
//! call to [`crate::character_changes`], which shares these patches.
use crate::{
    extended::characters::advisors::{Changes, Counts},
    hook::Hook,
};
use engine::{Memory, advisor_mutations::MUTATIONS};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const WATCHES: usize = 65536;
static WATCH: [Slot; WATCHES] = [const { Slot::new() }; WATCHES];
static EPOCH: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 11] = [const { AtomicUsize::new(0) }; 11];
const ROLE_SITES: usize = engine::character_role_mutations::MUTATIONS.len();
static ROLE_ORIGINAL: [AtomicUsize; ROLE_SITES] = [const { AtomicUsize::new(0) }; ROLE_SITES];
/// Whether every character role-row site was installed.
static ROLES_INSTALLED: AtomicBool = AtomicBool::new(false);
static EVENTS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];
static COUNTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static SAMPLED: crate::sampled_verify::Sampled =
    crate::sampled_verify::Sampled::burned_in(&["held advisor rows differ from a fresh read"]);
static NOTIFICATION_TICKS: AtomicU64 = AtomicU64::new(0);

struct Slot {
    address: AtomicUsize,
    revision: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            address: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
        }
    }
}

// No removal: reuse of a native address retains its invalidation history.
// A collision or full watch registry can never certify an unrelated address.
fn watch(slots: &[Slot], address: usize, register: bool) -> Option<&AtomicU64> {
    if address == 0 {
        return None;
    }
    let first = ((address >> 3).wrapping_mul(0x9e3779b97f4a7c15) >> 32) & (slots.len() - 1);
    for probe in 0..slots.len().min(16) {
        let slot = &slots[(first + probe) & (slots.len() - 1)];
        let mut current = slot.address.load(Ordering::SeqCst);
        if current == 0 && register {
            current =
                match slot
                    .address
                    .compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => address,
                    Err(current) => current,
                };
        }
        if current == address {
            return Some(&slot.revision);
        }
        if current == 0 {
            return None;
        }
    }
    None
}
fn increment(value: &AtomicU64) {
    if value.fetch_add(1, Ordering::SeqCst) == u64::MAX {
        POISONED.store(true, Ordering::SeqCst);
    }
}
pub fn invalidate() {
    increment(&EPOCH);
}
fn character_map(character: usize) -> usize {
    character
        .checked_add(engine::ccharacter::ADVISOR_MAP_HEAD)
        .unwrap_or(0)
}
fn role_map(role: usize) -> usize {
    // Native receiver is a live advisor, but still guard the metadata read.
    // On failure revoke every certificate rather than guess its owner.
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let owner = role
        .checked_add(engine::cadvisor::CHARACTER)
        .and_then(|address| engine::read::<u64>(&memory, address))
        .filter(|&owner| owner != 0);
    match owner {
        Some(owner) => character_map(owner as usize),
        None => {
            invalidate();
            0
        }
    }
}

struct Mutation {
    revision: Option<&'static AtomicU64>,
    ticks: u64,
    /// The same call as seen by the character role domain; dropped after
    /// this family's own return bookkeeping.
    _character: Option<crate::watch_registry::Notification<'static>>,
    /// The same call as seen by the role census domain.
    _census: Option<crate::watch_registry::Notification<'static>>,
}

/// The role census writers among [`MUTATIONS`] ([`crate::census_changes`]):
/// the advisor map's inserts and range erase, the merge, the parser and the
/// destructor.
fn census_site(site: usize) -> Option<crate::census_changes::Site> {
    use crate::census_changes::Site;
    match MUTATIONS[site].name {
        "insert_map" => Some(Site::AdvisorMapInsert),
        "insert_map_value" => Some(Site::AdvisorMapMove),
        "erase_map" => Some(Site::AdvisorMapErase),
        "merge_character" => Some(Site::Merge),
        "read_character" => Some(Site::Parse),
        "destroy_character" => Some(Site::Destroy),
        _ => None,
    }
}
impl Mutation {
    fn enter(address: impl FnOnce() -> usize, site: usize) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        // Keep callback lifetime protection around QPC too: it can execute in
        // another module while retaining a return address into this DLL.
        let begin = crate::sampling::qpc();
        increment(&REVISION);
        let address = address();
        let character = cfg!(feature = "character_changes")
            .then(|| crate::character_changes::advisor_map(address));
        let revision = watch(&WATCH, address, false);
        if let Some(revision) = revision {
            increment(revision);
        }
        EVENTS[site].fetch_add(1, Ordering::Relaxed);
        let census = census_site(site).and_then(crate::census_changes::forwarded);
        Self {
            revision,
            ticks: crate::sampling::qpc().saturating_sub(begin).max(0) as u64,
            _character: character,
            _census: census,
        }
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        let begin = crate::sampling::qpc();
        if let Some(revision) = self.revision {
            increment(revision);
        }
        increment(&REVISION);
        NOTIFICATION_TICKS.fetch_add(
            self.ticks + crate::sampling::qpc().saturating_sub(begin).max(0) as u64,
            Ordering::Relaxed,
        );
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let value = (
            EPOCH.load(Ordering::SeqCst),
            REVISION.load(Ordering::SeqCst),
        );
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some(value)
    }
    fn revision(&self, map: usize) -> Option<(u64, u64)> {
        let (epoch, _) = self.quiet()?;
        let revision = watch(&WATCH, map, true)?.load(Ordering::SeqCst);
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some((epoch, revision))
    }
    fn check(&self) -> crate::sampled_verify::Check {
        crate::sampled_verify::current_burned_in(cfg!(feature = "advisor_verify"))
    }
}

/// `counts.verified` held characters were read afresh and compared,
/// `counts.mismatches` of them differing; a mismatch disables reuse for the
/// attachment.
pub fn record(counts: Counts) {
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    for (target, value) in COUNTS.iter().zip([
        counts.reused,
        counts.read,
        counts.roles,
        counts.verified,
        counts.mismatches,
        counts.planned,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if counts.mismatches != 0 {
        POISONED.store(true, Ordering::SeqCst);
    }
}
pub fn status() -> impl serde::Serialize {
    let counts = COUNTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let event_counts = EVENTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(event_counts)
            .map(|(site, count)| (site.name, count)),
    );
    crate::status::status!({ "configured": cfg!(feature = "advisor_changes"), "verify": cfg!(feature = "advisor_verify"),
        "poisoned": POISONED.load(Ordering::SeqCst),
        "sampled_verified": SAMPLED.verified(), "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(), "slice": SAMPLED.slice(),
        "reused_characters": counts[0], "read_characters": counts[1],
        "reused_roles": counts[2], "planned_sections": counts[5], "events": events,
        "notification_ticks": NOTIFICATION_TICKS.load(Ordering::Relaxed),
        "scope": "QPC notification bookkeeping ticks including guarded owner lookup, excluding native bodies; capture wall includes all cache work; before each section's capture the held characters of the hour's slice (one in rotation_hours by character id) are read afresh and compared (sampled_*), and a mismatch disables reuse; verify mode reads every proposed hit" })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "advisor_changes") {
            let callbacks = [
                construct as *const () as usize,
                destroy as *const () as usize,
                set_idea as *const () as usize,
                read_role as *const () as usize,
                insert_map as *const () as usize,
                insert_map_value as *const () as usize,
                erase_map as *const () as usize,
                clear_map as *const () as usize,
                merge_character as *const () as usize,
                read_character as *const () as usize,
                destroy_character as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err("advisor hook sentinel differs");
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            // The role sites complete this family's character coverage. A
            // differing sentinel leaves them all uninstalled; the character
            // domain then stays disabled and every hour reads afresh.
            let roles = engine::character_role_mutations::MUTATIONS;
            if cfg!(feature = "character_changes")
                && roles.iter().all(|site| {
                    engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
                })
            {
                let callbacks: [usize; ROLE_SITES] = [
                    country_leader_modifiers as *const () as usize,
                    country_leader_destroy as *const () as usize,
                    country_leader_map_insert as *const () as usize,
                    country_leader_create as *const () as usize,
                    advisor_modifiers as *const () as usize,
                    country_leader_ideology as *const () as usize,
                    character_name as *const () as usize,
                    character_repair as *const () as usize,
                ];
                for (index, site) in roles.iter().enumerate() {
                    let hook =
                        unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                            .map_err(|_| "native hook construction failed")?;
                    ROLE_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                    hooks.push(hook);
                }
                ROLES_INSTALLED.store(true, Ordering::SeqCst);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            crate::character_changes::set_enabled(false);
            crate::roster_changes::set_repair_forward(false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Roles,
                false,
            );
            crate::census_changes::forwarder(crate::census_changes::Forwarder::Advisors, false);
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
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        crate::character_changes::set_enabled(
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        // The roster family observes repair_containers through this patch.
        crate::roster_changes::set_repair_forward(
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        // The stability inputs observe the leader and advisor roles here.
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Roles,
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        // The role census observes the advisor sites and the country-leader
        // map insert here.
        crate::census_changes::forwarder(
            crate::census_changes::Forwarder::Advisors,
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the character watch tables are
        // dropped only once the hooks were never enabled, or were removed and
        // drained after the last capture.
        unsafe { crate::character_changes::release() };
    }
}

// Signatures checked against pinned decompiles and native call sites. The
// callbacks preserve every argument and return, and never call readers/getters
// in the game. Role-owner lookup is guarded metadata, not an engine call.
macro_rules! callback {
    ($name:ident, $site:expr, ($($arg:ident:$ty:ty),*) -> $ret:ty, $map:expr) => {
        unsafe extern "system" fn $name($($arg:$ty),*) -> $ret {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorChanges);
            let _mutation = Mutation::enter(|| $map, $site);
            let original: unsafe extern "system" fn($($ty),*) -> $ret =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($($arg),*) })
        }
    };
}
callback!(construct, 0, (this:usize, owner:usize) -> usize, character_map(owner));
callback!(destroy, 1, (this:usize) -> (), role_map(this));
callback!(set_idea, 2, (this:usize, token:usize) -> (), role_map(this));
callback!(read_role, 3, (this:usize, reader:usize, token:i32) -> (), role_map(this));
callback!(insert_map, 4, (this:usize, result:usize, key:usize, value:usize) -> usize, this);
callback!(insert_map_value, 5, (this:usize, result:usize, key:usize, value:usize) -> usize, this);
callback!(erase_map, 6, (this:usize, range:usize) -> usize, this);
callback!(clear_map, 7, (this:usize, allocator:usize, node:usize) -> (), this);
callback!(merge_character, 8, (this:usize, source:usize, country:usize) -> (), character_map(this));
callback!(read_character, 9, (this:usize, reader:usize, token:i32) -> (), character_map(this));
callback!(destroy_character, 10, (this:usize) -> (), character_map(this));

// Role-row writers ([`engine::character_role_mutations`]): integer and
// pointer arguments only (the asm prologues use at most RCX..R9), and void
// returns pass RAX through unchanged. Each names its character as the audit
// there establishes; a role whose parent does not read revokes every
// certificate. The character domain's notification covers the original in
// flight.
macro_rules! role_callback {
    ($name:ident, $site:expr, ($($arg:ident:$ty:ty),*), $character:expr) => {
        unsafe extern "system" fn $name($($arg:$ty),*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorChanges);
            let character = $character;
            let _notification = crate::character_changes::notify(character, $site);
            let _inputs = crate::country_input_changes::forwarded_role($site, character);
            let _census = ($site == roles::COUNTRY_LEADER_MAP_INSERT)
                .then(|| crate::census_changes::forwarded(crate::census_changes::Site::LeaderMapInsert))
                .flatten();
            let original: unsafe extern "system" fn($($ty),*) -> usize =
                unsafe { std::mem::transmute(ROLE_ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($($arg),*) })
        }
    };
}
use engine::character_role_mutations as roles;
// Country leader modifiers (leader).
role_callback!(country_leader_modifiers, roles::COUNTRY_LEADER_MODIFIERS, (leader: usize),
    crate::character_changes::role_character(leader, engine::ccountry_leader::CHARACTER));
// Country leader destroy (leader): its character link still names the character on entry.
role_callback!(country_leader_destroy, roles::COUNTRY_LEADER_DESTROY, (leader: usize),
    crate::character_changes::role_character(leader, engine::ccountry_leader::CHARACTER));
// Leader-map try_emplace (map = the character's leader map, result, key, value).
role_callback!(country_leader_map_insert, roles::COUNTRY_LEADER_MAP_INSERT,
    (map: usize, result: usize, key: usize, value: usize),
    Some(map.saturating_sub(engine::ccharacter::LEADER_MAP_HEAD)));
// Role builder (out, character, definition).
role_callback!(country_leader_create, roles::COUNTRY_LEADER_CREATE,
    (out: usize, character: usize, definition: usize), Some(character));
// Advisor modifier rebuild (advisor).
role_callback!(advisor_modifiers, roles::ADVISOR_MODIFIERS, (advisor: usize),
    crate::character_changes::role_character(advisor, engine::cadvisor::CHARACTER));
// Name setter (character, name): the token, the name and the unit leader name.
role_callback!(character_name, roles::CHARACTER_NAME, (character: usize, name: usize),
    Some(character));
// Repair containers  (character) rebuilds the name and may give the character a new id
// (`RegisterIdentity`), which the roster family observes through this patch.
unsafe extern "system" fn character_repair(character: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorChanges);
    let _roster = crate::roster_changes::character_repaired();
    let _notification = crate::character_changes::notify(Some(character), roles::CHARACTER_REPAIR);
    let _inputs =
        crate::country_input_changes::forwarded_role(roles::CHARACTER_REPAIR, Some(character));
    let original: unsafe extern "system" fn(usize) -> usize = unsafe {
        std::mem::transmute(ROLE_ORIGINAL[roles::CHARACTER_REPAIR].load(Ordering::Acquire))
    };
    timing.original(|| unsafe { original(character) })
}
// Set_country_leader_ideology (effect, scope): the leader it writes is
// resolved inside the effect, so every certificate is revoked.
role_callback!(country_leader_ideology, roles::COUNTRY_LEADER_IDEOLOGY,
    (effect: usize, scope: usize), None);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_registry_never_certifies_colliding_or_unregistered_maps() {
        let slots = [const { Slot::new() }; 4];
        for address in [8, 16, 24, 32] {
            assert!(watch(&slots, address, false).is_none());
            watch(&slots, address, true)
                .unwrap()
                .store(address as u64, Ordering::SeqCst);
        }
        assert!(watch(&slots, 40, true).is_none());
        assert!(watch(&slots, 0, true).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(
                watch(&slots, address, false)
                    .unwrap()
                    .load(Ordering::SeqCst),
                address as u64
            );
        }
    }
}
