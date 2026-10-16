//! Allocation-free notifications from the native writers of each character's
//! country-leader and advisor role rows ([`engine::character_role_mutations`]
//! and the advisor family's sites, [`crate::advisor_changes`], which owns the
//! patches and forwards every call here). A detour only notifies the shared
//! watch registry ([`WatchRegistry`]) for the character it names, counts its
//! site and calls the original; it never changes game behaviour. Receivers
//! are CCharacter addresses a capture registered. A role writer names its
//! character through the role's guarded parent field; when that read fails,
//! the call revokes every certificate. Anything unobserved (disabled,
//! poisoned, a refused registration, an epoch change, a running writer)
//! makes the capture read afresh.
use crate::watch_registry::{Notification, WatchRegistry};
use engine::{ccharacter, character_role_mutations::MUTATIONS};
use std::sync::atomic::{AtomicU64, Ordering};

static WATCHES: Watches = Watches::new();
/// Reused, read, verified and mismatched characters and quiet hours, per
/// carried table.
static READS: [[AtomicU64; 5]; TABLES.len()] =
    [const { [const { AtomicU64::new(0) }; 5] }; TABLES.len()];
/// The family's sampled comparisons; a reason per carried table.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried CharacterRoleTraits rows differ from a fresh read",
    "carried CountryLeaderRoles rows differ from a fresh read",
    "carried Characters rows differ from a fresh read",
    "carried AdvisorRoles modifier caches or tooltip keys differ from a fresh read",
]);

/// The tables whose rows a capture carries per character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    CharacterRoleTraits,
    CountryLeaderRoles,
    Characters,
    /// Counted per advisor role: its modifier cache and tooltip keys.
    AdvisorRoles,
}
const TABLES: [&str; 4] = [
    "CharacterRoleTraits",
    "CountryLeaderRoles",
    "Characters",
    "AdvisorRoles",
];

/// One observation domain: the watched characters and each site's calls.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; MUTATIONS.len()],
    advisor_calls: AtomicU64,
    unnamed: AtomicU64,
    /// Tests: stands in for the scientist domain ([`crate::roster_changes`]).
    #[cfg(any(test, feature = "test-support"))]
    scientists: WatchRegistry,
    #[cfg(any(test, feature = "test-support"))]
    scientist_verify: std::sync::atomic::AtomicBool,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; MUTATIONS.len()],
            advisor_calls: AtomicU64::new(0),
            unnamed: AtomicU64::new(0),
            #[cfg(any(test, feature = "test-support"))]
            scientists: WatchRegistry::new(),
            #[cfg(any(test, feature = "test-support"))]
            scientist_verify: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Hook path: a writer of `site` ([`MUTATIONS`] index) is about to
    /// change `character`'s role rows; `None` names no character.
    pub fn notify(&self, character: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.receiver(character)
    }

    /// `(epoch, revision)` while observing with no writer running; equal
    /// values around a read prove that no observed writer ran during it.
    pub fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }

    fn receiver(&self, character: Option<usize>) -> Notification<'_> {
        match character {
            Some(character) => self.registry.notify(character),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
        }
    }

    /// An enabled domain with a table for `characters` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(characters: usize) -> Self {
        let watches = Self::new();
        watches.enable(characters);
        watches
    }

    /// Enable a static domain with a table for `characters` receivers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enable(&self, characters: usize) {
        self.registry.offer(characters);
        self.registry.boundary(false);
        self.registry.set_enabled(true);
        self.scientists.set_enabled(true);
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.scientists.boundary(invalidate);
    }

    /// Tests: a scientist role or trait writer runs and returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn scientist_write(&self) {
        drop(self.scientists.notify(0));
    }

    /// Tests: quiet hours read their scientist rows afresh and compare them
    /// (`roster_verify`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_scientist_verify(&self, verify: bool) {
        self.scientist_verify.store(verify, Ordering::Relaxed);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn scientists_poisoned(&self) -> bool {
        self.scientists.poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, revision)` of the whole domain while it is observed and no
    /// writer runs; `None` otherwise. Equal values across hours prove that no
    /// observed writer ran in between, so every certificate taken before
    /// still holds without looking it up.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one character, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched its role rows.
    fn sequence(&self, character: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
    /// The scientist domain's token ([`crate::roster_changes::scientists`]):
    /// equal values prove that no scientist role was built, attached or
    /// cleared and no scientist trait array was written in between.
    fn scientists(&self) -> Option<(u64, u64)>;
    /// Carried scientist rows disagreed with a fresh read.
    fn scientist_mismatch(&self);
    /// A quiet hour reads its scientist rows afresh and compares them with
    /// the held ones instead of carrying them (`roster_verify`).
    fn scientist_verify(&self) -> bool;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, character: usize) -> Option<(u64, u64)> {
        self.registry.sequence(character)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
    #[cfg(any(test, feature = "test-support"))]
    fn scientists(&self) -> Option<(u64, u64)> {
        self.scientists.quiet()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn scientists(&self) -> Option<(u64, u64)> {
        crate::roster_changes::scientists()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn scientist_mismatch(&self) {
        self.scientists.poison();
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn scientist_mismatch(&self) {
        crate::roster_changes::mismatch();
    }
    #[cfg(any(test, feature = "test-support"))]
    fn scientist_verify(&self) -> bool {
        self.scientist_verify.load(Ordering::Relaxed)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn scientist_verify(&self) -> bool {
        crate::roster_changes::VERIFY
    }
}

/// Tests: a domain that counts its per-character lookups.
#[cfg(any(test, feature = "test-support"))]
pub struct Counting {
    pub watches: Watches,
    lookups: AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Counting {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Counting {
    pub const fn new() -> Self {
        Self {
            watches: Watches::new(),
            lookups: AtomicU64::new(0),
        }
    }

    /// Lookups since the last call.
    pub fn lookups(&self) -> u64 {
        self.lookups.swap(0, Ordering::Relaxed)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for Counting {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.watches.quiet()
    }
    fn sequence(&self, character: usize) -> Option<(u64, u64)> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.watches.sequence(character)
    }
    fn mismatch(&self) {
        self.watches.mismatch();
    }
    fn scientists(&self) -> Option<(u64, u64)> {
        self.watches.scientists()
    }
    fn scientist_mismatch(&self) {
        self.watches.scientist_mismatch();
    }
    fn scientist_verify(&self) -> bool {
        self.watches.scientist_verify()
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.registry.quiet()
    }
    fn sequence(&self, character: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(character)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
    fn scientists(&self) -> Option<(u64, u64)> {
        crate::roster_changes::scientists()
    }
    fn scientist_mismatch(&self) {
        crate::roster_changes::mismatch();
    }
    fn scientist_verify(&self) -> bool {
        crate::roster_changes::VERIFY
    }
}

/// The hooks' domain's [`Watches::quiet`].
pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.quiet()
}

/// Whether captures read every certified character afresh and compare,
/// rather than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "character_verify");

/// Hook path: a writer of `site` ([`MUTATIONS`]) is about to change the
/// character it names.
pub fn notify(character: Option<usize>, site: usize) -> Notification<'static> {
    WATCHES.notify(character, site)
}

/// Hook path: an advisor-family writer touches the advisor map at `map`
/// (`CCharacter+0xB0`), or names no map (`0`), whose writer revokes every
/// certificate. Generic tree helpers also reach unrelated maps; their
/// address never names a registered character.
pub fn advisor_map(map: usize) -> Notification<'static> {
    WATCHES.advisor_calls.fetch_add(1, Ordering::Relaxed);
    if map == 0 {
        return WATCHES.receiver(None);
    }
    WATCHES
        .registry
        .notify(map.saturating_sub(ccharacter::ADVISOR_MAP_HEAD))
}

/// The guarded parent field of a live role at `role + offset`; `None` when
/// it does not read, so the writer revokes every certificate.
pub fn role_character(role: usize, offset: usize) -> Option<usize> {
    let memory = unsafe { engine::memory::LiveMemory::new() };
    role.checked_add(offset)
        .and_then(|address| engine::read::<u64>(&memory, address))
        .and_then(|character| usize::try_from(character).ok())
        .filter(|&character| character != 0)
}

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

/// Off the game thread: size the watches for `characters` roster
/// occurrences. Nothing is offered before the live roster is known: a later
/// larger table waits for an invalidating hour, while the first installs at
/// once.
pub fn presize(characters: usize) {
    if characters != 0 {
        WATCHES.registry.offer(characters);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// Observation follows the hooks that cover every writer.
pub fn set_enabled(enabled: bool) {
    WATCHES.registry.set_enabled(enabled);
}

/// Free the watch tables.
///
/// # Safety
/// The hooks were never enabled, or were removed and drained after the last
/// capture.
pub unsafe fn release() {
    unsafe { WATCHES.registry.release() };
}

/// One capture's characters. `verified` carried characters were read afresh
/// for the sampled comparison, `mismatches` of them differing. `quiet` marks
/// a capture that carried its whole table from a quiet domain and an
/// unchanged census, reading only its sampled slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub quiet: bool,
}

/// The capture already poisoned reuse at a mismatch ([`Changes::mismatch`]).
/// Carried, read, verified and mismatched CharacterOperatives links
/// ([`crate::extended::character_inputs`]).
static OPERATIVES: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// One CharacterInputs walk's operative links.
pub fn record_operatives(carried: u64, read: u64, verified: u64, mismatches: u64) {
    for (target, value) in OPERATIVES.iter().zip([carried, read, verified, mismatches]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn record(table: Table, counts: &Counts) {
    for (target, value) in READS[table as usize].iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, table as usize);
}

pub fn status() -> impl serde::Serialize {
    let [carried, read, verified, mismatches] = OPERATIVES
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let operatives = crate::status::status!({
        "carried": carried,
        "read": read,
        "verified": verified,
        "mismatches": mismatches,
        "scope": "CharacterOperatives links: carried without reading their leader while both of its certificates (unit leader and trait domains) hold and the slice does not cover the character; read otherwise, and verified when a carryable link was read in the slice"
    });
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|(site, n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let tables = crate::status::map(TABLES.iter().zip(&READS).map(|(&name, reads)| {
        let reads = reads.each_ref().map(|value| value.load(Ordering::Relaxed));
        (
            name,
            crate::status::status!({
                "reused_characters": reads[0],
                "read_characters": reads[1],
                "sampled_verified": reads[2],
                "sampled_mismatches": reads[3],
                "quiet_hours": reads[4],
            }),
        )
    }));
    crate::status::status!({
        "configured": cfg!(feature = "character_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "tables": tables,
        "events": events,
        "advisor_family_events": WATCHES.advisor_calls.load(Ordering::Relaxed),
        "unnamed_receivers": WATCHES.unnamed.load(Ordering::Relaxed),
        "operative_links": operatives,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; CharacterRoleTraits carries a character's country-leader and advisor rows, and CountryLeaderRoles its country-leader roles with the census country, and Characters its names, gender and template with this hour's identity, country and roster position, when no hooked writer touched it since its read and its census map headers are unchanged, and AdvisorRoles each role's modifier cache and tooltip keys (counted per role) when the role, its character and the character's certificate repeat at the same position; scientist rows and CharacterOperatives are read every hour; every hour also reads the carried characters (and AdvisorRoles roles) of its slice afresh (one in rotation_hours by character id; every carried one in verify mode) and compares them with the held rows: CharacterRoleTraits, CountryLeaderRoles and AdvisorRoles publish the fresh rows, and a Characters difference drops the carried set so the hour reads every name afresh; the first mismatch disables reuse (sampled_*); quiet_hours counts captures certified by the domain's unchanged quiet token alone, without a per-character lookup: CharacterRoleTraits, CountryLeaderRoles and AdvisorRoles then carry the whole table (after a capture that certified every occurrence, with its census generation), reading only the slice, while Characters still reads every identity, country and roster position"
    })
}
