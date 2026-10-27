//! The war domain: a quiet token over every native writer of the set and the
//! fixed fields WarRelations records ([`crate::acquisition_wars`]), which
//! other families patch and forward here. The domain registers no receiver:
//! the Wars job compares the token it took before its latest complete walk
//! with the hour's, and equal tokens prove that no observed writer ran in
//! between ([`WatchRegistry::quiet`]).
//!
//! Forwarded sites ([`Site`]) cover relation replacement, the only writer of a status's war pointer
//! and a war's place in both endpoints' owned (`cdiplomacy_status::OWNED_RELATIONS`), sub- and
//! serializable relation vectors. Its only caller is linked relation replacement, reached by
//! war-link application and clearing for declaration, joining, civil war, load and ending a war.
//! Fifteen typed relation slot replacements also insert owned relations, moving other relations and
//! a war's place in the same vectors. Instigator and defender setters are the only run-time writers
//! of the first-was-instigator byte (`cwar_relation::FIRST_WAS_INSTIGATOR`) after linking. Relation
//! status reset, construction and save reading, and diplomacy reset are forwarded too. The resets
//! reach a war only through relation replacement: redundant for the war set, they name every
//! relation change at its source.
//!
//! Other recorded fields (endpoints, start and end hours, cancel and retired bytes) are written
//! only during construction, by two initializers and by save reading before the war is linked.
//! Linking requires a zero cancel byte; there is no later writer. A linked war is destroyed only
//! inside relation replacement, after it leaves every vector. Hourly combat settlement rewrites the
//! casualty counters, which are read every hour.
//!
//! The domain observes only while every forwarding family's patches are
//! enabled ([`Forwarder`]); anything unobserved (disabled, poisoned, an epoch
//! change, a running writer) yields no token, and the walk reads afresh.
use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static WARS: WatchRegistry = WatchRegistry::new();
pub type ParticipationObserver = fn(Site, usize, usize);
static PARTICIPATION: AtomicUsize = AtomicUsize::new(0);

/// The participation owner installs this before admitting capture and removes
/// it before releasing its state. Shared hook owners retain their callbacks
/// through native quiescence. Notifications never allocate or wait.
pub fn observe_participation(observer: Option<ParticipationObserver>) {
    PARTICIPATION.store(observer.map_or(0, |f| f as usize), Ordering::SeqCst);
}

/// Names the exact status or diplomacy root on both sides of its writer,
/// while the enclosing war-domain notification excludes acquisition.
pub struct ParticipationChange {
    observer: Option<ParticipationObserver>,
    site: Site,
    receiver: usize,
    diplomacy: usize,
}
pub fn participation_change(site: Site, receiver: usize, diplomacy: usize) -> ParticipationChange {
    let pointer = PARTICIPATION.load(Ordering::SeqCst);
    let observer: Option<ParticipationObserver> = if pointer == 0 {
        None
    } else {
        Some(unsafe { std::mem::transmute::<usize, ParticipationObserver>(pointer) })
    };
    if let Some(observer) = observer {
        observer(site, receiver, diplomacy);
    }
    ParticipationChange {
        observer,
        site,
        receiver,
        diplomacy,
    }
}
impl Drop for ParticipationChange {
    fn drop(&mut self) {
        if let Some(observer) = self.observer {
            observer(self.site, self.receiver, self.diplomacy);
        }
    }
}

/// The families whose patches forward here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarder {
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
pub enum Site {
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
/// Quiet and read hours.
static READS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Hook path, allocation-free: a forwarded writer is about to run. Dropping
/// the guard notifies its return.
pub fn forwarded(site: Site) -> Option<Notification<'static>> {
    if !cfg!(feature = "war_changes") {
        return None;
    }
    CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
    Some(WARS.notify(0))
}

/// A forwarding family's patches were enabled or disabled; the domain
/// observes only while every one is enabled, and enabling starts an epoch.
pub fn forwarder(family: Forwarder, enabled: bool) {
    FORWARDERS[family as usize].store(enabled, Ordering::SeqCst);
    let all = FORWARDERS
        .iter()
        .all(|enabled| enabled.load(Ordering::SeqCst));
    WARS.set_enabled(cfg!(feature = "war_changes") && all);
}

/// What the Wars job sees of the domain.
pub trait Changes: Sync {
    /// The domain's token: `(epoch, revision)` while observed and no
    /// forwarded writer runs.
    fn quiet(&self) -> Option<(u64, u64)>;
}

/// The forwarded writers' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WARS.quiet()
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
pub struct Counts {
    /// The held rows were published with fresh casualties, walking nothing.
    pub quiet: bool,
    /// The walk ran.
    pub read: bool,
}

/// Publishes one Wars capture's counts.
pub fn record(counts: &Counts) {
    for (target, value) in READS
        .iter()
        .zip([u64::from(counts.quiet), u64::from(counts.read)])
    {
        target.fetch_add(value, Ordering::Relaxed);
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
        "poisoned": WARS.poisoned(),
        "quiet_hours": reads[0],
        "read_hours": reads[1],
        "events": calls,
        "forwarders": forwarders,
        "scope": "cumulative since attach; WarRelations walks and validates every country's owned relations unless the hour is quiet: when the war domain's token (every forwarded writer of the war set, its vectors and its fixed fields) and the country census equal those taken before the latest complete walk, the held rows are published with each war's casualty counters read afresh (quiet_hours); the domain observes only while every forwarding family is enabled (forwarders)"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participation_lifetime_forwarding_names_the_receiver_before_and_after_without_allocation() {
        static CALLS: [AtomicUsize; Site::ALL.len()] =
            [const { AtomicUsize::new(0) }; Site::ALL.len()];
        fn observe(site: Site, receiver: usize, diplomacy: usize) {
            assert_eq!((receiver, diplomacy), (0x12300, 0x45600));
            CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
        }
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                observe_participation(None);
            }
        }
        let _restore = Restore;
        observe_participation(Some(observe));
        for site in [
            Site::StatusConstruct,
            Site::StatusReset,
            Site::StatusLoad,
            Site::RelationSlot,
            Site::DiplomacyReset,
            Site::ReplaceRelation,
        ] {
            collector_test_support::with_allocations_denied(|| {
                let change = participation_change(site, 0x12300, 0x45600);
                assert_eq!(CALLS[site as usize].load(Ordering::Relaxed), 1);
                drop(change);
                assert_eq!(CALLS[site as usize].load(Ordering::Relaxed), 2);
            });
        }
        observe_participation(None);
        drop(participation_change(Site::StatusLoad, 0, 0));
        assert_eq!(CALLS[Site::StatusLoad as usize].load(Ordering::Relaxed), 2);
    }

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
