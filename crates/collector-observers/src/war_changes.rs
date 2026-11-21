use crate::watch_registry::{Notification, WatchRegistry};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static WARS: WatchRegistry = WatchRegistry::new();
pub type ParticipationObserver = fn(Site, usize, usize);
static PARTICIPATION: AtomicUsize = AtomicUsize::new(0);

pub fn observe_participation(observer: Option<ParticipationObserver>) {
    PARTICIPATION.store(observer.map_or(0, |f| f as usize), Ordering::SeqCst);
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarder {
    CountryInputs,
    Opinions,
    Relations,
}

impl Forwarder {
    const ALL: [Self; 3] = [Self::CountryInputs, Self::Opinions, Self::Relations];

}

static FORWARDERS: [AtomicBool; Forwarder::ALL.len()] =
    [const { AtomicBool::new(false) }; Forwarder::ALL.len()];

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

}

static CALLS: [AtomicU64; Site::ALL.len()] = [const { AtomicU64::new(0) }; Site::ALL.len()];
static READS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub fn forwarded(site: Site) -> Option<Notification<'static>> {
    if !cfg!(feature = "war_changes") {
        return None;
    }
    CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
    Some(WARS.notify(0))
}

pub fn forwarder(family: Forwarder, enabled: bool) {
    FORWARDERS[family as usize].store(enabled, Ordering::SeqCst);
    let all = FORWARDERS
        .iter()
        .all(|enabled| enabled.load(Ordering::SeqCst));
    WARS.set_enabled(cfg!(feature = "war_changes") && all);
}

pub trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WARS.quiet()
    }
}

pub fn boundary(invalidate: bool) {
    WARS.boundary(invalidate);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub quiet: bool,
    pub read: bool,
}

pub fn record(counts: &Counts) {
    for (target, value) in READS
        .iter()
        .zip([u64::from(counts.quiet), u64::from(counts.read)])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
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
