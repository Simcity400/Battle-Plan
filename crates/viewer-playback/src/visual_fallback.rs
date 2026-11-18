//! The one owner of every map's visual mod fallback: what each recording shows of its visual
//! mods after a map failed with them, decided from what the recording alone refuses.
//!
//! A decision is keyed exactly by the recording and the revision its visual mods held
//! ([`FallbackKey`]) and never applies outside that key. Content records the key and what it was
//! built under ([`Built`]); it is shown and reused only while that is what its key holds now
//! ([`Fallbacks::current`]). Maps report each frame ([`Report`]), and every report of a frame is
//! reduced here in one place ([`Fallbacks::reduce`]), whatever order the maps sent them in.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// A recording and the revision its selected visual mods held
/// ([`installed_content_core::content::visual::revision`]).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FallbackKey {
    /// The recording's identity (its source and revision).
    pub recording: String,
    pub revision: String,
}

/// What a key's maps show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Without visual mods while the recording alone shows whether it refuses `failures` too:
    /// it gets `frames_needed` settled frames, as many as the map with the mods drew before
    /// refusing, of which it has drawn `frames_seen`.
    Probing {
        failures: BTreeSet<String>,
        excused: BTreeSet<String>,
        frames_needed: u32,
        frames_seen: u32,
    },
    /// Without visual mods: the recording alone did not refuse `failures` in its frames. Still
    /// watched; the moment it refuses them too, the mods return.
    Confirmed {
        failures: BTreeSet<String>,
        excused: BTreeSet<String>,
    },
    /// With visual mods, `excused` being refusals the recording alone makes too.
    Restored { excused: BTreeSet<String> },
}

impl Decision {
    /// What content of its key is built under.
    pub fn under(&self) -> Under {
        match self {
            Self::Probing { failures, .. } | Self::Confirmed { failures, .. } => Under::Recording {
                failures: failures.clone(),
            },
            Self::Restored { excused } => Under::Mods {
                excused: excused.clone(),
            },
        }
    }

    /// Why the visual mods are not shown, for the load warnings; `None` when they are.
    pub fn reason(&self) -> Option<String> {
        let (failures, verb) = match self {
            Self::Probing { failures, .. } => (
                failures,
                "the map checks whether the recording alone refuses the same",
            ),
            Self::Confirmed { failures, .. } => {
                (failures, "the recording alone does not refuse the same")
            }
            Self::Restored { .. } => return None,
        };
        Some(format!(
            "{verb}: {}",
            failures.iter().cloned().collect::<Vec<_>>().join("; ")
        ))
    }
}

/// What content was built under: its visual mods with refusals excused, or the recording alone
/// for a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Under {
    Mods { excused: BTreeSet<String> },
    Recording { failures: BTreeSet<String> },
}

/// The key content was built for and what it was built under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    pub key: FallbackKey,
    pub under: Under,
}

/// One map's frame of content built as `built`: what it refused, by stable name, and whether
/// it drew settled (the playhead's hour drawn this frame with nothing still loading for it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub built: Built,
    pub refusals: BTreeSet<String>,
    pub settled: bool,
}

#[derive(Debug, Default)]
struct State {
    decision: Option<Decision>,
    /// Every refusal its content built without visual mods has made.
    seen: BTreeSet<String>,
    /// Settled frames its content built with visual mods has drawn.
    modded: u32,
}

/// Every key's decision, shared by every host of a recording's maps.
#[derive(Debug, Default)]
pub struct Fallbacks {
    states: BTreeMap<FallbackKey, State>,
}

/// The owner as hosts share it.
pub type Shared = Arc<Mutex<Fallbacks>>;

/// A recording's identity in a [`FallbackKey`]: its source and that source's revision.
pub fn recording(source_id: &str, revision: &str) -> String {
    format!("{source_id}@{revision}")
}

/// Locks the shared owner, recovering it if a host panicked holding it.
pub fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Fallbacks> {
    shared.lock().unwrap_or_else(|error| error.into_inner())
}

impl Fallbacks {
    /// The decisions of `recording`'s keys, for a load to look its own up in.
    pub fn decisions(&self, recording: &str) -> BTreeMap<FallbackKey, Decision> {
        self.states
            .iter()
            .filter(|(key, _)| key.recording == recording)
            .filter_map(|(key, state)| Some((key.clone(), state.decision.clone()?)))
            .collect()
    }

    /// What content of `key` is built under now: with its visual mods and nothing excused
    /// when no map has failed with them.
    pub fn under(&self, key: &FallbackKey) -> Under {
        self.states
            .get(key)
            .and_then(|state| state.decision.as_ref())
            .map_or(
                Under::Mods {
                    excused: BTreeSet::new(),
                },
                Decision::under,
            )
    }

    /// Whether content built as `built` is what its key holds now: only such content is
    /// shown, reused or installed.
    pub fn current(&self, built: &Built) -> bool {
        self.under(&built.key) == built.under
    }

    /// Takes on a decision a load made for a scene that could not be built with the visual
    /// mods, unless the key decided otherwise meanwhile.
    pub fn adopt(&mut self, key: FallbackKey, decision: Decision) {
        let state = self.states.entry(key).or_default();
        if state
            .decision
            .as_ref()
            .is_none_or(|held| matches!(held, Decision::Restored { .. }))
        {
            state.decision = Some(decision);
            state.seen.clear();
        }
    }

    /// Reduces one frame's reports from every map, in one place and whatever their order:
    ///
    /// - Content with the visual mods that refused anything not excused fails them: its key
    ///   probes without them, needing as many settled frames as the mods drew.
    /// - Content without them adds what it refused to what its key has seen; the moment that
    ///   includes every failure they are the recording's own and the mods return with them
    ///   excused, whichever map saw them, before anything else is decided. Otherwise a probe that
    ///   has drawn its settled frames is confirmed.
    ///
    /// Reports of content no longer current are ignored. Returns the keys whose content changed,
    /// so their maps load again.
    pub fn reduce(&mut self, reports: Vec<Report>) -> BTreeSet<FallbackKey> {
        let mut changed = BTreeSet::new();
        let mut by_key: BTreeMap<FallbackKey, Vec<Report>> = BTreeMap::new();
        for report in reports {
            if self.current(&report.built) {
                by_key
                    .entry(report.built.key.clone())
                    .or_default()
                    .push(report);
            }
        }
        for (key, reports) in by_key {
            let under = self.under(&key);
            let state = self.states.entry(key.clone()).or_default();
            let settled = reports.iter().any(|report| report.settled);
            let refused: BTreeSet<String> = reports
                .into_iter()
                .flat_map(|report| report.refusals)
                .collect();
            match under {
                Under::Mods { excused } => {
                    let failures: BTreeSet<String> =
                        refused.difference(&excused).cloned().collect();
                    if failures.is_empty() {
                        state.modded = state.modded.saturating_add(u32::from(settled));
                        continue;
                    }
                    state.decision = Some(Decision::Probing {
                        failures,
                        excused,
                        frames_needed: state.modded,
                        frames_seen: 0,
                    });
                    state.seen.clear();
                    state.modded = 0;
                    changed.insert(key);
                }
                Under::Recording { .. } => {
                    state.seen.extend(refused);
                    let Some(decision) = state.decision.as_mut() else {
                        continue;
                    };
                    let (failures, excused) = match decision {
                        Decision::Probing {
                            failures, excused, ..
                        }
                        | Decision::Confirmed { failures, excused } => {
                            (failures.clone(), excused.clone())
                        }
                        Decision::Restored { .. } => continue,
                    };
                    if failures.is_subset(&state.seen) {
                        *decision = Decision::Restored {
                            excused: excused.union(&failures).cloned().collect(),
                        };
                        state.seen.clear();
                        changed.insert(key);
                        continue;
                    }
                    if let Decision::Probing {
                        frames_needed,
                        frames_seen,
                        ..
                    } = decision
                    {
                        *frames_seen = frames_seen.saturating_add(u32::from(settled));
                        if settled && *frames_seen >= (*frames_needed).max(1) {
                            *decision = Decision::Confirmed { failures, excused };
                        }
                    }
                }
            }
        }
        changed
    }
}

/// Whether a frame drew settled, from the viewer's own render result: it drew this frame, the
/// hour it drew and the hour on screen are the hour the playhead asks for, built without
/// error, and nothing it loads lazily is queued or decoding.
pub fn settled(
    drew: bool,
    requested: (replay_data::BranchId, i32),
    rendered: Option<(replay_data::BranchId, i32)>,
    shown: Option<(replay_data::BranchId, i32, bool)>,
    loading: bool,
) -> bool {
    drew && rendered == Some(requested)
        && shown.is_some_and(|(branch, hour, failed)| (branch, hour) == requested && !failed)
        && !loading
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(recording: &str, revision: &str) -> FallbackKey {
        FallbackKey {
            recording: recording.into(),
            revision: revision.into(),
        }
    }

    fn names(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn report(built: &Built, refusals: &[&str], settled: bool) -> Report {
        Report {
            built: built.clone(),
            refusals: names(refusals),
            settled,
        }
    }

    fn built(fallbacks: &Fallbacks, key: &FallbackKey) -> Built {
        Built {
            key: key.clone(),
            under: fallbacks.under(key),
        }
    }

    /// A refusal the recording alone also makes is excused once, the mods coming back; one it
    /// does not make keeps them off after as many settled frames as the mods drew, and the
    /// recording making it later still brings them back.
    #[test]
    fn the_recording_alone_decides_whether_the_mods_caused_a_refusal() {
        let mut fallbacks = Fallbacks::default();
        let a = key("A", "r1");
        let modded = built(&fallbacks, &a);
        for _ in 0..3 {
            assert!(
                fallbacks
                    .reduce(vec![report(&modded, &[], true)])
                    .is_empty()
            );
        }
        // Frames that did not draw settled are not the mods' exposure.
        assert!(
            fallbacks
                .reduce(vec![report(&modded, &[], false)])
                .is_empty()
        );
        assert_eq!(
            fallbacks.reduce(vec![report(&modded, &["oak"], true)]),
            [a.clone()].into()
        );
        assert!(!fallbacks.current(&modded));
        let probe = built(&fallbacks, &a);
        assert_eq!(
            probe.under,
            Under::Recording {
                failures: names(&["oak"])
            }
        );
        // The mods drew 3 settled frames (the unsettled one not counted) before refusing.
        let Some(Decision::Probing { frames_needed, .. }) =
            fallbacks.decisions("A").get(&a).cloned()
        else {
            panic!("no probe");
        };
        assert_eq!(frames_needed, 3);
        for _ in 0..2 {
            assert!(fallbacks.reduce(vec![report(&probe, &[], true)]).is_empty());
        }
        assert!(matches!(
            fallbacks.decisions("A")[&a],
            Decision::Probing { .. }
        ));
        assert!(
            fallbacks
                .reduce(vec![report(&probe, &["oak"], false)])
                .contains(&a)
        );
        assert_eq!(
            fallbacks.under(&a),
            Under::Mods {
                excused: names(&["oak"])
            }
        );
        // The excused refusal no longer fails; another one probes, needing the one settled
        // frame the mods drew since they came back, counting only settled frames.
        let modded = built(&fallbacks, &a);
        assert!(
            fallbacks
                .reduce(vec![report(&modded, &["oak"], true)])
                .is_empty()
        );
        fallbacks.reduce(vec![report(&modded, &["oak", "frame: lost"], true)]);
        let probe = built(&fallbacks, &a);
        let Some(Decision::Probing { frames_needed, .. }) =
            fallbacks.decisions("A").get(&a).cloned()
        else {
            panic!("no probe");
        };
        assert_eq!(frames_needed, 1);
        for _ in 0..10 {
            fallbacks.reduce(vec![report(&probe, &["oak"], false)]);
            assert!(matches!(
                fallbacks.decisions("A")[&a],
                Decision::Probing { .. }
            ));
        }
        assert!(fallbacks.reduce(vec![report(&probe, &[], true)]).is_empty());
        assert!(matches!(
            fallbacks.decisions("A")[&a],
            Decision::Confirmed { .. }
        ));
        assert!(fallbacks.current(&probe));
        assert!(
            fallbacks
                .reduce(vec![report(&probe, &["frame: lost"], true)])
                .contains(&a)
        );
        assert_eq!(
            fallbacks.under(&a),
            Under::Mods {
                excused: names(&["oak", "frame: lost"])
            }
        );
    }

    /// A decision never applies to another recording, even at the same visual mod revision: a
    /// refusal excused in A is a failure in B.
    #[test]
    fn decisions_stay_with_their_recording() {
        let mut fallbacks = Fallbacks::default();
        let a = key("A", "r1");
        let b = key("B", "r1");
        fallbacks.reduce(vec![report(&built(&fallbacks, &a), &["oak"], true)]);
        fallbacks.reduce(vec![report(&built(&fallbacks, &a), &["oak"], true)]);
        assert_eq!(
            fallbacks.under(&a),
            Under::Mods {
                excused: names(&["oak"])
            }
        );
        assert!(fallbacks.decisions("B").is_empty());
        let in_b = built(&fallbacks, &b);
        assert_eq!(
            in_b.under,
            Under::Mods {
                excused: BTreeSet::new()
            }
        );
        assert_eq!(
            fallbacks.reduce(vec![report(&in_b, &["oak"], true)]),
            [b.clone()].into()
        );
        assert!(matches!(
            fallbacks.decisions("B")[&b],
            Decision::Probing { .. }
        ));
    }

    /// Content without visual mods confirmed for one failure is not what a probe of another
    /// failure holds, so it is never lent to it.
    #[test]
    fn content_is_current_only_for_the_failure_it_was_built_for() {
        let mut fallbacks = Fallbacks::default();
        let k = key("A", "r1");
        fallbacks.adopt(
            k.clone(),
            Decision::Confirmed {
                failures: names(&["oak"]),
                excused: BTreeSet::new(),
            },
        );
        let for_oak = built(&fallbacks, &k);
        assert!(fallbacks.current(&for_oak));
        let for_elm = Built {
            key: k.clone(),
            under: Under::Recording {
                failures: names(&["elm"]),
            },
        };
        assert!(!fallbacks.current(&for_elm));
        // Nor is it current for the same failure in another recording.
        let elsewhere = Built {
            key: key("B", "r1"),
            under: for_oak.under.clone(),
        };
        assert!(!fallbacks.current(&elsewhere));
    }

    /// A mod updated while its probe runs is a new revision, another key: the update's maps
    /// show the mods, whatever the old revision's probe decides.
    #[test]
    fn a_mod_update_is_never_held_by_an_old_probe() {
        let mut fallbacks = Fallbacks::default();
        let old = key("A", "r1");
        fallbacks.reduce(vec![report(&built(&fallbacks, &old), &["oak"], true)]);
        assert!(matches!(
            fallbacks.decisions("A")[&old],
            Decision::Probing { .. }
        ));
        let updated = key("A", "r2");
        assert!(!fallbacks.decisions("A").contains_key(&updated));
        assert_eq!(
            fallbacks.under(&updated),
            Under::Mods {
                excused: BTreeSet::new()
            }
        );
    }

    /// Two maps of one probe reporting in the same frame, one whose recording refused the
    /// failure and one that drew its last needed frame without it: the mods come back, whatever
    /// the order.
    #[test]
    fn the_mods_coming_back_beats_keeping_them_off() {
        for order in [false, true] {
            let mut fallbacks = Fallbacks::default();
            let k = key("A", "r1");
            fallbacks.reduce(vec![report(&built(&fallbacks, &k), &["oak"], true)]);
            let probe = built(&fallbacks, &k);
            let refused = report(&probe, &["oak"], false);
            let clean = report(&probe, &[], true);
            let reports = if order {
                vec![refused, clean]
            } else {
                vec![clean, refused]
            };
            assert!(fallbacks.reduce(reports).contains(&k));
            assert_eq!(
                fallbacks.under(&k),
                Under::Mods {
                    excused: names(&["oak"])
                }
            );
        }
    }

    /// A frame is settled only when it drew the hour the playhead asks for, built without
    /// error, with nothing still loading.
    #[test]
    fn a_frame_is_settled_only_when_it_drew_the_requested_hour() {
        let at = (1, 100);
        let shown = Some((1, 100, false));
        assert!(settled(true, at, Some(at), shown, false));
        assert!(!settled(false, at, Some(at), shown, false));
        assert!(!settled(true, at, Some((1, 99)), shown, false));
        assert!(!settled(true, at, Some(at), Some((1, 99, false)), false));
        assert!(!settled(true, at, Some(at), Some((1, 100, true)), false));
        assert!(!settled(true, at, Some(at), shown, true));
    }
}
