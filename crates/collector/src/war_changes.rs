//! The war domain: a quiet token over every native writer of the set and the
//! fixed fields WarRelations records ([`crate::acquisition_wars`]), which
//! other families patch and forward here. The domain registers no receiver:
//! the Wars job compares the token it took before its latest complete walk
//! with the hour's, and equal tokens prove that no observed writer ran in
//! between ([`WatchRegistry::quiet`]).
//!
//! Forwarded sites ([`Site`]) cover relation replacement, the only writer
//! of a status's war pointer `+0x2E8` and a war's place in both endpoints'
//! owned (`CDiplomacyStatus+0x20`), sub- and serializable relation vectors.
//! Its only caller is linked relation replacement, reached by war-link
//! application and clearing for declaration, joining, civil war, load and
//! ending a war. Fifteen typed relation slot replacements also insert owned
//! relations, moving other relations and a war's place in the same vectors.
//! Instigator and defender setters are the only run-time writers of
//! `+0x158` after linking. Relation status reset, construction and save
//! reading, and diplomacy reset are forwarded too. The resets reach a war
//! only through relation replacement: redundant for the war set, they name
//! every relation change at its source.
//!
//! Other recorded fields (endpoints `+0x10`/`+0x14`, start `+0x20` and end
//! `+0x38` hours, cancel `+0x48` and retired `+0x49` bytes) are written only
//! during construction, by two initializers and by save reading before the
//! war is linked. Linking requires a zero cancel byte; there is no later
//! writer. A linked war is destroyed only inside relation replacement,
//! after it leaves every vector. Hourly combat settlement rewrites the
//! casualty counters `+0x50..+0x70`, which are read every hour.
//!
//! The domain observes only while every forwarding family's patches are
//! enabled ([`Forwarder`]); anything unobserved (disabled, poisoned, an epoch
//! change, a running writer) yields no token, and the walk reads afresh.
use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static WARS: WatchRegistry = WatchRegistry::new();

/// The families whose patches forward here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Forwarder {
    /// [`crate::country_input_changes`]: relation replacement, instigator setting,
    /// defender setting.
    CountryInputs,
    /// [`crate::opinion_changes`]: the status reset, constructor and save
    /// reader and the typed relation slot replacements.
    Opinions,
    /// [`crate::relation_changes`]: diplomacy reset.
    Relations,
}

impl Forwarder {
    const ALL: [Self; 3] = [Self::CountryInputs, Self::Opinions, Self::Relations];

    fn name(self) -> &'static str {
        match self {
            Self::CountryInputs => "country_input_changes",
            Self::Opinions => "opinion_changes",
            Self::Relations => "relation_changes",
        }
    }
}

static FORWARDERS: [AtomicBool; Forwarder::ALL.len()] =
    [const { AtomicBool::new(false) }; Forwarder::ALL.len()];

/// The forwarded writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    ReplaceRelation,
    SetInstigator,
    SetDefender,
    StatusReset,
    StatusConstruct,
    StatusLoad,
    RelationSlot,
    DiplomacyReset,
}

impl Site {
    const ALL: [Self; 8] = [
        Self::ReplaceRelation,
        Self::SetInstigator,
        Self::SetDefender,
        Self::StatusReset,
        Self::StatusConstruct,
        Self::StatusLoad,
        Self::RelationSlot,
        Self::DiplomacyReset,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::ReplaceRelation => "replace_relation",
            Self::SetInstigator => "set_instigator",
            Self::SetDefender => "set_defender",
            Self::StatusReset => "status_reset",
            Self::StatusConstruct => "status_construct",
            Self::StatusLoad => "status_load",
            Self::RelationSlot => "relation_slot",
            Self::DiplomacyReset => "diplomacy_reset",
        }
    }
}

static CALLS: [AtomicU64; Site::ALL.len()] = [const { AtomicU64::new(0) }; Site::ALL.len()];
/// Quiet, read and compared hours, and compared wars.
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a quiet hour's held WarRelations rows differ from a fresh walk",
]);

/// Whether the Wars job walks every hour and compares each quiet one with
/// what it would carry, rather than carrying it (the family's burn-in).
pub(crate) const VERIFY: bool = cfg!(feature = "war_verify");

/// Hook path, allocation-free: a forwarded writer is about to run. Dropping
/// the guard notifies its return.
pub(crate) fn forwarded(site: Site) -> Option<Notification<'static>> {
    if !cfg!(feature = "war_changes") {
        return None;
    }
    CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
    Some(WARS.notify(0))
}

/// A forwarding family's patches were enabled or disabled; the domain
/// observes only while every one is enabled, and enabling starts an epoch.
pub(crate) fn forwarder(family: Forwarder, enabled: bool) {
    FORWARDERS[family as usize].store(enabled, Ordering::SeqCst);
    let all = FORWARDERS
        .iter()
        .all(|enabled| enabled.load(Ordering::SeqCst));
    WARS.set_enabled(cfg!(feature = "war_changes") && all);
}

/// What the Wars job sees of the domain.
pub(crate) trait Changes: Sync {
    /// The domain's token: `(epoch, revision)` while observed and no
    /// forwarded writer runs.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// A quiet hour's held rows differed from a fresh walk: nothing is
    /// carried for the rest of the attachment.
    fn mismatch(&self);
}

/// The forwarded writers' domain.
pub(crate) struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WARS.quiet()
    }
    fn mismatch(&self) {
        WARS.poison();
    }
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WARS.boundary(invalidate);
}

/// One Wars capture's use of the domain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// The held rows were published with fresh casualties, walking nothing.
    pub quiet: bool,
    /// The walk ran.
    pub read: bool,
    /// The walk ran on a quiet hour and was compared with the held rows.
    pub compared: bool,
    pub compared_wars: u64,
    pub mismatches: u64,
}

/// The capture already called [`Changes::mismatch`] at a difference.
pub(crate) fn record(counts: &Counts) {
    for (target, value) in READS.iter().zip([
        u64::from(counts.quiet),
        u64::from(counts.read),
        u64::from(counts.compared),
        counts.compared_wars,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if counts.compared {
        SAMPLED.record(counts.compared_wars, counts.mismatches, 0);
    }
}

pub fn status() -> impl serde::Serialize {
    let calls = crate::status::map(
        Site::ALL
            .iter()
            .zip(&CALLS)
            .map(|(site, n)| (site.name(), n.load(Ordering::Relaxed))),
    );
    let forwarders = crate::status::map(
        Forwarder::ALL
            .iter()
            .zip(&FORWARDERS)
            .map(|(family, enabled)| (family.name(), enabled.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "war_changes"),
        "verify": VERIFY,
        "poisoned": WARS.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "quiet_hours": reads[0],
        "read_hours": reads[1],
        "compared_hours": reads[2],
        "compared_wars": reads[3],
        "events": calls,
        "forwarders": forwarders,
        "scope": "cumulative since attach; WarRelations walks and validates every country's owned relations unless the hour is quiet: when the war domain's token (every forwarded writer of the war set, its vectors and its fixed fields) and the country census equal those taken before the latest complete walk, the held rows are published with each war's casualty counters read afresh (quiet_hours); the domain's hour of the rotation (every quiet hour in verify mode) walks instead and compares the walk with what it would carry (compared_*), disabling the carry at the first difference (sampled_*); the domain observes only while every forwarding family is enabled (forwarders)"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The domain needs every forwarder, and each forwarded writer moves the
    /// token at its entry and refuses it until its return.
    #[cfg(feature = "war_changes")]
    #[test]
    fn every_forwarder_enables_and_each_forwarded_call_moves_the_token() {
        for family in Forwarder::ALL {
            forwarder(family, false);
        }
        assert_eq!(Observer.quiet(), None);
        for family in &Forwarder::ALL[..2] {
            forwarder(*family, true);
            assert_eq!(Observer.quiet(), None);
        }
        forwarder(Forwarder::Relations, true);
        let before = Observer.quiet().expect("observed");
        assert_eq!(Observer.quiet(), Some(before));
        let call = forwarded(Site::ReplaceRelation);
        assert_eq!(Observer.quiet(), None, "a running writer certifies nothing");
        drop(call);
        let after = Observer.quiet().expect("observed");
        assert_ne!(after, before);
        forwarder(Forwarder::Opinions, false);
        assert_eq!(Observer.quiet(), None);
    }
}
