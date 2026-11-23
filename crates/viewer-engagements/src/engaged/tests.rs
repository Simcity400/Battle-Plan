use super::*;

fn resolve(viewer: u32, shares: &'static [u32]) -> impl Fn(u32) -> Option<(bool, bool)> {
    move |raw| (raw < 100).then(|| (raw == viewer, raw == viewer || shares.contains(&raw)))
}

#[test]
fn primary_members_share_vision_and_secondary_owners_must_be_the_viewer() {
    let r = resolve(1, &[2]);
    assert_eq!(test(&[5, 2], &[], &r), Some(true));
    assert_eq!(test(&[1], &[], &r), Some(true));
    assert_eq!(test(&[5], &[1], &r), Some(true));
    assert_eq!(test(&[5], &[2], &r), Some(false));
    assert_eq!(test(&[], &[], &r), Some(false));
    assert_eq!(test(&[500], &[], &r), None);
    assert_eq!(test(&[500, 2], &[], &r), Some(true));
}

const RECORDED: Lists = Lists {
    recorded: true,
    units: false,
};
const UNITS: Lists = Lists {
    recorded: false,
    units: true,
};
const NONE: Lists = Lists {
    recorded: false,
    units: false,
};

#[derive(Clone, Copy)]
struct SideHour {
    refresh: Option<Refresh>,
    recorded: [Option<bool>; 2],
    units: [Option<bool>; 2],
}

fn hour(refresh: Option<Refresh>, now: Option<bool>, before: Option<bool>) -> SideHour {
    SideHour {
        refresh,
        recorded: [now, before],
        units: [now, before],
    }
}

fn step(held: Engaged, hour: SideHour) -> Engaged {
    super::step(held, hour.refresh, |lists, when| match lists {
        Tested::Recorded => hour.recorded[when],
        Tested::Units => hour.units[when],
    })
}

#[test]
fn only_the_candidates_a_refresh_allows_are_evaluated() {
    let count = |refresh| {
        let mut calls = 0;
        super::step(Engaged::No, refresh, |_, _| {
            calls += 1;
            Some(true)
        });
        calls
    };
    assert_eq!(count(rebuilt(NONE, NONE)), 0);
    assert_eq!(count(None), 0);
    assert_eq!(count(Some(Refresh::Load)), 1);
    assert_eq!(count(rebuilt(RECORDED, NONE)), 2);
    assert_eq!(count(rebuilt(RECORDED, UNITS)), 4);
}

fn rebuilt(proven: Lists, possible: Lists) -> Option<Refresh> {
    Some(Refresh::Hour { proven, possible })
}

#[test]
fn a_rebuild_holds_until_the_next_and_a_split_hour_is_ambiguous() {
    let load = step(
        Engaged::Unknown,
        hour(Some(Refresh::Load), Some(false), None),
    );
    assert_eq!(load, Engaged::No);
    let held = step(load, hour(rebuilt(NONE, NONE), Some(true), Some(false)));
    assert_eq!(held, Engaged::No);
    let joined = step(held, hour(rebuilt(RECORDED, NONE), Some(true), Some(true)));
    assert_eq!(joined, Engaged::Yes);
    let split = step(
        joined,
        hour(rebuilt(RECORDED, NONE), Some(false), Some(true)),
    );
    assert_eq!(split, Engaged::Ambiguous(Ambiguity::Order));
    assert_eq!(
        step(split, hour(rebuilt(NONE, NONE), Some(false), Some(false))),
        Engaged::Ambiguous(Ambiguity::Order)
    );
    assert_eq!(
        step(Engaged::Yes, hour(None, Some(true), Some(true))),
        Engaged::Unknown
    );
    assert_eq!(
        step(
            Engaged::Yes,
            hour(rebuilt(RECORDED, NONE), Some(true), None)
        ),
        Engaged::Unknown
    );
}

#[test]
fn possible_rebuilds_and_both_list_kinds_must_agree() {
    let possible = |held, now, before| step(held, hour(rebuilt(NONE, UNITS), now, before));
    assert_eq!(possible(Engaged::Yes, Some(true), Some(true)), Engaged::Yes);
    assert_eq!(
        possible(Engaged::No, Some(true), Some(true)),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
    assert_eq!(
        possible(Engaged::Unknown, Some(true), Some(true)),
        Engaged::Unknown
    );
    assert_eq!(
        possible(Engaged::Ambiguous(Ambiguity::Order), Some(true), Some(true)),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
    let proven = step(
        Engaged::No,
        hour(rebuilt(UNITS, UNITS), Some(true), Some(true)),
    );
    assert_eq!(proven, Engaged::Yes);
    let kinds = |recorded, units| SideHour {
        refresh: rebuilt(RECORDED, UNITS),
        recorded,
        units,
    };
    assert_eq!(
        step(Engaged::No, kinds([Some(true); 2], [Some(true); 2])),
        Engaged::Yes
    );
    assert_eq!(
        step(Engaged::No, kinds([Some(true); 2], [Some(false); 2])),
        Engaged::Ambiguous(Ambiguity::Order)
    );
    assert_eq!(
        step(Engaged::No, kinds([Some(true); 2], [None, Some(true)])),
        Engaged::Unknown
    );
}

#[test]
fn either_side_engages_the_combat() {
    use Engaged::*;
    assert_eq!(No.either(Yes), Yes);
    assert_eq!(Unknown.either(Yes), Yes);
    assert_eq!(Ambiguous(Ambiguity::Order).either(Unknown), Unknown);
    assert_eq!(
        No.either(Ambiguous(Ambiguity::Order)),
        Ambiguous(Ambiguity::Order)
    );
    assert_eq!(
        Ambiguous(Ambiguity::Order).either(Ambiguous(Ambiguity::Scope)),
        Ambiguous(Ambiguity::Scope)
    );
    assert_eq!(No.either(No), No);
}

fn relations(allied: bool) -> Relations {
    Relations::factions_for_tests(if allied { &[(1, 2)] } else { &[][..] })
}

fn side(units: &[u64], primary: &[u32]) -> Side {
    let lists = Some((primary.to_vec(), vec![]));
    Side {
        units: Some(units.to_vec()),
        added: Added::NoTrigger,
        unit_lists: lists.clone(),
        lists,
    }
}

fn empty_sides() -> &'static BTreeMap<(u64, u8), SideWitness> {
    static EMPTY: std::sync::OnceLock<BTreeMap<(u64, u8), SideWitness>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(BTreeMap::new)
}

fn complete_witness() -> RefreshWitness<'static> {
    RefreshWitness::Complete {
        sides: empty_sides(),
        controller_targets: &[],
        local_viewers: &[],
        cache_written_viewers: &[],
    }
}

fn side_witness(lists: Lists) -> SideWitness {
    SideWitness {
        lists,
        recorded_viewers: Vec::new(),
        units_viewers: Vec::new(),
    }
}

fn unused_zero() -> &'static ZeroVisibility {
    static UNUSED: std::sync::OnceLock<ZeroVisibility> = std::sync::OnceLock::new();
    UNUSED.get_or_init(|| ZeroVisibility::unavailable("unused"))
}

fn snap_with(
    links: &BTreeMap<u32, u32>,
    relations: &Relations,
    zero_visibility: &ZeroVisibility,
    controllers: Option<Controllers>,
) -> Arc<Snapshot> {
    Arc::new(Snapshot {
        links: links.clone(),
        relations: Ok(relations.clone()),
        zero_visibility: zero_visibility.clone(),
        controllers,
    })
}

fn snap(links: &BTreeMap<u32, u32>, relations: &Relations) -> Arc<Snapshot> {
    snap_with(links, relations, unused_zero(), Some(Controllers::default()))
}

fn sample<'a>(snapshot: &'a Arc<Snapshot>, units: &[u64], primary: &[u32]) -> Sample<'a> {
    defended(snapshot, units, primary, &[9])
}

fn defended<'a>(
    snapshot: &'a Arc<Snapshot>,
    units: &[u64],
    primary: &[u32],
    defenders: &[u64],
) -> Sample<'a> {
    Sample {
        viewer: Some(1),
        viewer_raw: None,
        alternate_viewer: None,
        alternate_viewer_raw: None,
        recording: false,
        snapshot,
        refreshes: complete_witness(),
        sides: BTreeMap::from([
            ((62, 0), side(units, primary)),
            ((62, 1), side(defenders, &[5])),
        ]),
        complete: true,
        removal: Removal::BothSidesUnlessRoundRetreat,
    }
}

#[test]
fn a_land_removal_may_rebuild_the_other_side() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    let mut history = History::default();
    let next = Continuity::Next;
    history.advance(defended(&apart, &[7], &[2], &[8, 9]), Continuity::Load);
    history.advance(defended(&allied, &[7], &[2], &[8, 9]), next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(defended(&allied, &[7], &[2], &[9]), next);
    assert_eq!(history.side(62, 0), Engaged::No);
    assert_eq!(history.side(62, 1), Engaged::No);
    let sides = BTreeMap::from([((62, 0), side_witness(UNITS))]);
    let witnessed = Sample {
        refreshes: RefreshWitness::Complete {
            sides: &sides,
            controller_targets: &[],
            local_viewers: &[],
            cache_written_viewers: &[],
        },
        ..defended(&allied, &[7], &[2], &[9])
    };
    history.advance(witnessed, next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(defended(&allied, &[6, 7], &[2], &[9]), next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(defended(&allied, &[6, 7], &[2], &[]), next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn a_viewer_change_across_a_gap_is_not_a_rebuild_now() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    fn viewed<'a>(snapshot: &'a Arc<Snapshot>, viewer: u32) -> Sample<'a> {
        Sample {
            viewer: Some(viewer),
            ..sample(snapshot, &[7], &[2])
        }
    }
    let mut history = History::default();
    history.advance(viewed(&apart, 5), Continuity::Load);
    history.advance(viewed(&allied, 1), Continuity::Gap);
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let mut history = History::default();
    history.advance(viewed(&apart, 5), Continuity::Load);
    history.advance(viewed(&allied, 1), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    let mut history = History::default();
    history.advance(viewed(&allied, 5), Continuity::Load);
    history.advance(viewed(&allied, 1), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn a_naval_removal_rebuilds_the_other_side() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    fn naval<'a>(snapshot: &'a Arc<Snapshot>, defenders: &[u64]) -> Sample<'a> {
        Sample {
            removal: Removal::BothSides,
            ..defended(snapshot, &[7], &[2], defenders)
        }
    }
    let mut history = History::default();
    history.advance(naval(&apart, &[8, 9]), Continuity::Load);
    history.advance(naval(&allied, &[8, 9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(naval(&allied, &[9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    assert_eq!(history.side(62, 1), Engaged::No);
    history.advance(naval(&apart, &[9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    let mut split = naval(&apart, &[]);
    split.sides.get_mut(&(62, 0)).unwrap().lists = Some((vec![1], vec![]));
    history.advance(split, Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
}

#[test]
fn a_new_added_id_proves_a_rebuild() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    let mut history = History::default();
    history.advance(added(&apart, Added::Observed(vec![3])), Continuity::Load);
    history.advance(added(&allied, Added::Observed(vec![3])), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        added(&allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

fn added<'a>(snapshot: &'a Arc<Snapshot>, added: Added) -> Sample<'a> {
    let mut sample = sample(snapshot, &[7], &[2]);
    sample.sides.get_mut(&(62, 0)).unwrap().added = added;
    sample
}

#[test]
fn unavailable_added_ids_leave_a_possible_rebuild_until_two_observations_compare() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    fn unavailable<'a>(snapshot: &'a Arc<Snapshot>, a: Added) -> Sample<'a> {
        Sample {
            refreshes: RefreshWitness::Unavailable,
            ..added(snapshot, a)
        }
    }
    let mut history = History::default();
    history.advance(
        unavailable(&apart, Added::Observed(vec![3])),
        Continuity::Load,
    );
    history.advance(unavailable(&allied, Added::Unavailable), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(
        unavailable(&allied, Added::Observed(vec![3])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    history.advance(
        unavailable(&allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);

    let mut history = History::default();
    history.advance(
        unavailable(&apart, Added::Observed(vec![3])),
        Continuity::Load,
    );
    history.advance(unavailable(&apart, Added::Unavailable), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        unavailable(&allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));

    let mut witnessed = History::default();
    witnessed.advance(added(&apart, Added::Observed(vec![3])), Continuity::Load);
    witnessed.advance(added(&allied, Added::Unavailable), Continuity::Next);
    assert_eq!(witnessed.side(62, 0), Engaged::No);
    let sides = BTreeMap::from([((62, 0), side_witness(RECORDED))]);
    let transfer = Sample {
        refreshes: RefreshWitness::Complete {
            sides: &sides,
            controller_targets: &[],
            local_viewers: &[],
            cache_written_viewers: &[],
        },
        ..added(&allied, Added::Unavailable)
    };
    witnessed.advance(transfer, Continuity::Next);
    assert_eq!(witnessed.side(62, 0), Engaged::Yes);
}

#[test]
fn a_viewer_change_across_a_gap_rebuilds_nothing() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = snap(&links, &relations(true));
    let mut history = History::default();
    history.advance(sample(&allied, &[7], &[2]), Continuity::Load);
    let moved = |viewer| Sample {
        viewer: Some(viewer),
        ..sample(&allied, &[7], &[2])
    };
    history.advance(moved(2), Continuity::Gap);
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let mut next = History::default();
    next.advance(sample(&allied, &[7], &[2]), Continuity::Load);
    next.advance(moved(2), Continuity::Next);
    assert_eq!(next.side(62, 0), Engaged::Yes);
}

#[test]
fn a_viewer_change_is_ordered_against_both_samples_relations() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    fn at<'a>(snapshot: &'a Arc<Snapshot>, viewer: u32) -> Sample<'a> {
        Sample {
            viewer: Some(viewer),
            ..sample(snapshot, &[7], &[1])
        }
    }
    let run = |hours: &[(u32, &Arc<Snapshot>)]| {
        let mut history = History::default();
        history.advance(at(&apart, 1), Continuity::Load);
        assert_eq!(history.side(62, 0), Engaged::Yes);
        for &(viewer, snapshot) in hours {
            history.advance(at(snapshot, viewer), Continuity::Next);
        }
        history.side(62, 0)
    };
    assert_eq!(run(&[(2, &apart), (2, &allied)]), Engaged::No);
    assert_eq!(run(&[(1, &allied), (2, &allied)]), Engaged::Yes);
    assert_eq!(run(&[(2, &allied)]), Engaged::Ambiguous(Ambiguity::Order));
    let mut ending = History::default();
    ending.advance(at(&allied, 1), Continuity::Load);
    ending.advance(at(&apart, 2), Continuity::Next);
    assert_eq!(ending.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
}

fn rosters(links: &BTreeMap<u32, u32>, players: &[(i32, u32)], count: i64) -> Controllers {
    Controllers {
        players: players
            .iter()
            .map(|&(key, raw)| (key, (raw, links.get(&raw).copied())))
            .collect(),
        counts: BTreeMap::from([(1, count)]),
    }
}

#[test]
fn a_controller_joining_the_viewer_may_refresh_a_stale_flag() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    let away = Some(rosters(&links, &[(40, 5)], 1));
    let run = |relations: &Relations,
               controllers: Option<Controllers>,
               refreshes: RefreshWitness<'_>| {
        let mut history = History::default();
        let s0 = snap_with(&links, &apart, unused_zero(), away.clone());
        let s1 = snap_with(&links, relations, unused_zero(), away.clone());
        let s2 = snap_with(&links, relations, unused_zero(), controllers);
        history.advance(sample(&s0, &[7], &[2]), Continuity::Load);
        history.advance(sample(&s1, &[7], &[2]), Continuity::Next);
        history.advance(
            Sample {
                refreshes,
                ..sample(&s2, &[7], &[2])
            },
            Continuity::Next,
        );
        history.side(62, 0)
    };
    let joined = Some(rosters(&links, &[(40, 1)], 2));
    assert_eq!(run(&allied, away.clone(), complete_witness()), Engaged::No);
    assert_eq!(
        run(
            &allied,
            away.clone(),
            RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[1],
                local_viewers: &[],
                cache_written_viewers: &[],
            }
        ),
        Engaged::Yes
    );
    assert_eq!(
        run(
            &allied,
            away.clone(),
            RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[5],
                local_viewers: &[],
                cache_written_viewers: &[],
            }
        ),
        Engaged::No
    );
    assert_eq!(
        run(
            &allied,
            away.clone(),
            RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[99],
                local_viewers: &[],
                cache_written_viewers: &[],
            }
        ),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
    assert_eq!(
        run(&allied, joined.clone(), RefreshWitness::Unavailable),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
    assert_eq!(
        run(&allied, None, RefreshWitness::Unavailable),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
    assert_eq!(run(&apart, joined, RefreshWitness::Unavailable), Engaged::No);
}

#[test]
fn each_candidate_resolves_references_through_its_own_sample_links() {
    let allied = relations(true);
    let before = snap(&BTreeMap::from([(1, 1), (5, 5), (7, 2)]), &allied);
    let after = snap(&BTreeMap::from([(1, 1), (5, 5), (7, 3)]), &allied);
    let mut history = History::default();
    history.advance(sample(&before, &[7], &[7]), Continuity::Load);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(sample(&after, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    let mut steady = History::default();
    steady.advance(sample(&after, &[7], &[7]), Continuity::Load);
    steady.advance(sample(&after, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(steady.side(62, 0), Engaged::No);
    let unlinked = snap(&BTreeMap::from([(1, 1), (5, 5)]), &allied);
    let mut unplaced = History::default();
    unplaced.advance(sample(&unlinked, &[7], &[7]), Continuity::Load);
    unplaced.advance(sample(&after, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(unplaced.side(62, 0), Engaged::Unknown);
}

#[test]
fn history_lags_diplomacy_until_the_unit_set_changes() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    let mut history = History::default();
    history.advance(sample(&apart, &[7], &[2]), Continuity::Load);
    assert_eq!(history.combat(62), Engaged::No);
    history.advance(sample(&allied, &[7], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::No);
    history.advance(sample(&allied, &[7, 8], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::Yes);
    history.advance(sample(&apart, &[7], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(sample(&apart, &[8], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::No);
    history.advance(sample(&apart, &[8], &[2]), Continuity::Gap);
    assert_eq!(history.combat(62), Engaged::Unknown);

    let mut started = History::default();
    let empty = Sample {
        sides: BTreeMap::new(),
        ..sample(&allied, &[], &[])
    };
    started.advance(empty, Continuity::Load);
    started.advance(sample(&allied, &[7], &[2]), Continuity::Next);
    assert_eq!(started.combat(62), Engaged::Yes);
}

#[test]
fn without_refresh_witness_diplomacy_change_is_ambiguous() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    let mut history = History::default();
    fn without<'a>(snapshot: &'a Arc<Snapshot>) -> Sample<'a> {
        Sample {
            refreshes: RefreshWitness::Unavailable,
            ..sample(snapshot, &[7], &[2])
        }
    }
    history.advance(without(&apart), Continuity::Load);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(without(&apart), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(without(&allied), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(without(&allied), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    let joined = Sample {
        refreshes: RefreshWitness::Unavailable,
        ..sample(&allied, &[7, 8], &[2])
    };
    history.advance(joined, Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn witnessed_same_roster_readd_remove_and_local_refresh_drive_engaged_state() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart_rel, allied_rel) = (relations(false), relations(true));
    let (apart, allied) = (snap(&links, &apart_rel), snap(&links, &allied_rel));

    let mut readd = History::default();
    readd.advance(sample(&apart, &[7], &[2]), Continuity::Load);
    assert_eq!(readd.side(62, 0), Engaged::No);
    let rec_sides = BTreeMap::from([((62, 0), side_witness(RECORDED))]);
    readd.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: &rec_sides,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(readd.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    readd.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: &rec_sides,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(readd.side(62, 0), Engaged::Yes);

    let mut bounce = History::default();
    let mut naval_side = side(&[7], &[1]);
    naval_side.unit_lists = Some((vec![2], vec![]));
    fn naval_at<'a>(
        snapshot: &'a Arc<Snapshot>,
        naval_side: &Side,
        refreshes: RefreshWitness<'a>,
    ) -> Sample<'a> {
        Sample {
            refreshes,
            sides: BTreeMap::from([((62, 0), naval_side.clone()), ((62, 1), side(&[9], &[5]))]),
            removal: Removal::BothSides,
            ..sample(snapshot, &[7], &[1])
        }
    }
    bounce.advance(
        naval_at(&apart, &naval_side, complete_witness()),
        Continuity::Load,
    );
    assert_eq!(bounce.side(62, 0), Engaged::Yes);
    let unit_sides = BTreeMap::from([((62, 0), side_witness(UNITS))]);
    bounce.advance(
        naval_at(
            &apart,
            &naval_side,
            RefreshWitness::Complete {
                sides: &unit_sides,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
        ),
        Continuity::Next,
    );
    assert_eq!(bounce.side(62, 0), Engaged::No);

    let zero_no = ZeroVisibility::for_tests([(2, Ok(false)), (5, Ok(false))]);
    let zero_yes = ZeroVisibility::for_tests([(2, Ok(true)), (5, Ok(false))]);
    let zero_missing = ZeroVisibility::unavailable("missing");
    let apart_zero_no = snap_with(&links, &apart_rel, &zero_no, Some(Controllers::default()));
    let allied_zero_no = snap_with(&links, &allied_rel, &zero_no, Some(Controllers::default()));
    let allied_zero_yes = snap_with(&links, &allied_rel, &zero_yes, Some(Controllers::default()));
    let allied_zero_missing = snap_with(
        &links,
        &allied_rel,
        &zero_missing,
        Some(Controllers::default()),
    );
    let mut local = History::default();
    local.advance(
        Sample {
            recording: true,
            ..sample(&apart_zero_no, &[7], &[2])
        },
        Continuity::Load,
    );
    local.advance(
        Sample {
            recording: true,
            ..sample(&allied_zero_no, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::No);
    local.advance(
        Sample {
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[1],
                cache_written_viewers: &[],
            },
            ..sample(&allied_zero_no, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Yes);
    local.advance(
        Sample {
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[0],
                cache_written_viewers: &[],
            },
            ..sample(&allied_zero_no, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::No);
    local.advance(
        Sample {
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[0],
                cache_written_viewers: &[],
            },
            ..sample(&allied_zero_yes, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    local.advance(
        Sample {
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[0],
                cache_written_viewers: &[],
            },
            ..sample(&allied_zero_yes, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Yes);
    local.advance(
        Sample {
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[0],
                cache_written_viewers: &[],
            },
            ..sample(&allied_zero_missing, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Unknown);
    local.advance(
        Sample {
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[99],
                cache_written_viewers: &[],
            },
            ..sample(&allied_zero_no, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Unknown);
}

#[test]
fn remapped_controller_target_is_scope_ambiguous_not_proven() {
    let (apart, allied) = (relations(false), relations(true));
    let before_links = BTreeMap::from([(1, 1), (2, 2), (5, 5), (7, 1)]);
    let after_links = BTreeMap::from([(1, 1), (2, 2), (5, 5), (7, 3)]);
    let unlinked = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let witness_seven = || RefreshWitness::Complete {
        sides: empty_sides(),
        controller_targets: &[7],
        local_viewers: &[],
        cache_written_viewers: &[],
    };

    for (prev_links, next_links) in [
        (&before_links, &after_links),
        (&after_links, &before_links),
        (&unlinked, &before_links),
        (&before_links, &unlinked),
    ] {
        let prev_apart = snap(prev_links, &apart);
        let prev_allied = snap(prev_links, &allied);
        let next_allied = snap(next_links, &allied);
        let mut history = History::default();
        history.advance(sample(&prev_apart, &[7], &[2]), Continuity::Load);
        history.advance(sample(&prev_allied, &[7], &[2]), Continuity::Next);
        assert_eq!(history.side(62, 0), Engaged::No);
        history.advance(
            Sample {
                refreshes: witness_seven(),
                ..sample(&next_allied, &[7], &[2])
            },
            Continuity::Next,
        );
        assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    }
}

#[test]
fn partial_previous_census_still_applies_witnessed_side_refresh() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = snap(&links, &relations(true));
    let mut history = History::default();
    let partial_empty = Sample {
        sides: BTreeMap::new(),
        complete: false,
        ..sample(&allied, &[], &[])
    };
    history.advance(partial_empty, Continuity::Load);
    let sides = BTreeMap::from([((62, 0), side_witness(RECORDED))]);
    history.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: &sides,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn partial_previous_naval_census_with_local_sweep_uses_recorded_lists_alone() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = snap(&links, &relations(true));
    let mut history = History::default();
    let partial_empty = Sample {
        sides: BTreeMap::new(),
        complete: false,
        removal: Removal::BothSides,
        ..sample(&allied, &[], &[])
    };
    history.advance(partial_empty, Continuity::Load);
    let mut convoy_side = side(&[], &[2]);
    convoy_side.unit_lists = Some((Vec::new(), Vec::new()));
    history.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[1],
                cache_written_viewers: &[],
            },
            sides: BTreeMap::from([((62, 0), convoy_side), ((62, 1), side(&[9], &[5]))]),
            complete: true,
            removal: Removal::BothSides,
            ..sample(&allied, &[], &[])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn alias_change_to_same_slot_does_not_prove_sweep_in_unavailable_interval() {
    let links = BTreeMap::from([(101, 1), (701, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (snap(&links, &relations(false)), snap(&links, &relations(true)));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&allied, &[7], &[2])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&apart, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&apart, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn remapped_tag_compares_within_same_links_table_without_proving_false_sweep() {
    let links_before = BTreeMap::from([(701, 1), (202, 2), (3, 3), (5, 5)]);
    let links_after = BTreeMap::from([(701, 2), (202, 2), (3, 3), (5, 5)]);
    let shares_with_one = Relations::factions_for_tests(&[(1, 3)]);
    let snap_before = snap(&links_before, &shares_with_one);
    let snap_after = snap(&links_after, &shares_with_one);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            ..sample(&snap_before, &[7], &[3])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(202),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&snap_after, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&snap_after, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));

    let mut direct_remap = History::default();
    direct_remap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            ..sample(&snap_before, &[7], &[3])
        },
        Continuity::Load,
    );
    assert_eq!(direct_remap.side(62, 0), Engaged::Yes);
    direct_remap.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&snap_after, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(
        direct_remap.side(62, 0),
        Engaged::Ambiguous(Ambiguity::Scope)
    );

    let same_before = BTreeMap::from([(701, 1), (202, 1), (3, 3), (5, 5)]);
    let split_after = BTreeMap::from([(701, 2), (202, 1), (3, 3), (5, 5)]);
    let apart = relations(false);
    let same_shared = snap(&same_before, &shares_with_one);
    let same_apart = snap(&same_before, &apart);
    let split_apart = snap(&split_after, &apart);
    let mut reverse_remap = History::default();
    reverse_remap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            ..sample(&same_shared, &[7], &[3])
        },
        Continuity::Load,
    );
    assert_eq!(reverse_remap.side(62, 0), Engaged::Yes);
    reverse_remap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&same_apart, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(reverse_remap.side(62, 0), Engaged::Yes);
    reverse_remap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&split_apart, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(
        reverse_remap.side(62, 0),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
}

#[test]
fn witnessed_zero_sweep_prevents_subsequent_unavailable_hour_from_proving_second_sweep() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let apart = relations(false);
    let zero_yes = ZeroVisibility::for_tests([(2, Ok(true)), (5, Ok(false))]);
    let zero_no = ZeroVisibility::for_tests([(2, Ok(false)), (5, Ok(false))]);
    let snap_yes = snap_with(&links, &apart, &zero_yes, Some(Controllers::default()));
    let snap_no = snap_with(&links, &apart, &zero_no, Some(Controllers::default()));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&snap_yes, &[7], &[2])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            alternate_viewer: Some(0),
            alternate_viewer_raw: Some(0),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[0],
                cache_written_viewers: &[0],
            },
            ..sample(&snap_no, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(
        Sample {
            viewer: Some(0),
            viewer_raw: Some(0),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&snap_no, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn unidentified_viewer_degrades_unavailable_hour_conservatively() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_three = snap(&links, &Relations::factions_for_tests(&[(2, 3)]));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_three, &[7], &[2])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        Sample {
            viewer: None,
            viewer_raw: None,
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_three, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
}

#[test]
fn local_all_sides_sweep_preserves_prior_cached_manager_viewer() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_two = snap(&links, &Relations::factions_for_tests(&[(2, 3)]));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[2],
                cache_written_viewers: &[],
            },
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn unavailable_interval_keeps_secondary_observer_in_manager_cache() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_two = snap(&links, &Relations::factions_for_tests(&[(2, 3)]));
    let apart = snap(&links, &relations(false));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(2),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&apart, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(2),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&apart, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn gap_does_not_establish_recording_manager_cache() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_one = snap(&links, &Relations::factions_for_tests(&[(1, 3)]));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let rec_one = BTreeMap::from([(
        (62, 0),
        SideWitness {
            lists: RECORDED,
            recorded_viewers: vec![1],
            units_viewers: Vec::new(),
        },
    )]);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(2),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: &rec_one,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(2),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn mid_campaign_gap_recovers_recording_manager_cache_after_full_tick() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_one = snap(&links, &Relations::factions_for_tests(&[(1, 3)]));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let rec_one = BTreeMap::from([(
        (62, 0),
        SideWitness {
            lists: RECORDED,
            recorded_viewers: vec![1],
            units_viewers: Vec::new(),
        },
    )]);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: &rec_one,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(2),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(2),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);

    let mut local_after_gap = History::default();
    local_after_gap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    local_after_gap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[2],
                cache_written_viewers: &[],
            },
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(local_after_gap.side(62, 0), Engaged::No);
    local_after_gap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(
        local_after_gap.side(62, 0),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
}

#[test]
fn gap_recovery_preserves_raw_aliases_across_later_remap() {
    let aliased_links = BTreeMap::from([(101, 1), (202, 1), (3, 3), (5, 5)]);
    let split_links = BTreeMap::from([(101, 1), (202, 2), (3, 3), (5, 5)]);
    let shares_with_one = Relations::factions_for_tests(&[(1, 3)]);
    let aliased = snap(&aliased_links, &shares_with_one);
    let split = snap(&split_links, &shares_with_one);
    let rec_101 = BTreeMap::from([(
        (62, 0),
        SideWitness {
            lists: RECORDED,
            recorded_viewers: vec![101],
            units_viewers: Vec::new(),
        },
    )]);

    let mut from_complete = History::default();
    from_complete.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&aliased, &[7], &[3])
        },
        Continuity::Gap,
    );
    from_complete.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: &rec_101,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&aliased, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(from_complete.side(62, 0), Engaged::Yes);
    from_complete.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(202),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&split, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(from_complete.side(62, 0), Engaged::Yes);
    from_complete.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&split, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(
        from_complete.side(62, 0),
        Engaged::Ambiguous(Ambiguity::Scope)
    );

    let mut from_unavailable = History::default();
    from_unavailable.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&aliased, &[7], &[3])
        },
        Continuity::Gap,
    );
    from_unavailable.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&aliased, &[7], &[3])
        },
        Continuity::Next,
    );
    from_unavailable.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: &rec_101,
                controller_targets: &[],
                local_viewers: &[],
                cache_written_viewers: &[],
            },
            ..sample(&aliased, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(from_unavailable.side(62, 0), Engaged::Yes);
    from_unavailable.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(202),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&split, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(from_unavailable.side(62, 0), Engaged::Yes);
    from_unavailable.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&split, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(
        from_unavailable.side(62, 0),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
}

#[test]
fn narrowing_recovers_exact_manager_cache_after_gap_and_observer_switch() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_two = snap(&links, &Relations::factions_for_tests(&[(2, 3)]));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Gap,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(2),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(2),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[2],
                cache_written_viewers: &[2],
            },
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(2),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(2),
            alternate_viewer: Some(1),
            alternate_viewer_raw: Some(1),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
}

#[test]
fn complete_witness_cache_write_does_not_admit_unwritten_alias_tag() {
    let links_init = BTreeMap::from([(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (101, 2), (202, 4)]);
    let links_aliased =
        BTreeMap::from([(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (101, 2), (202, 2)]);
    let shares_with_two = Relations::factions_for_tests(&[(2, 3)]);
    let snap_init = snap(&links_init, &shares_with_two);
    let snap_aliased = snap(&links_aliased, &shares_with_two);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&snap_init, &[7], &[3])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(101),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&snap_init, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        Sample {
            viewer: Some(2),
            viewer_raw: Some(101),
            alternate_viewer: Some(2),
            alternate_viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[101],
                cache_written_viewers: &[101],
            },
            ..sample(&snap_aliased, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(4),
            viewer_raw: Some(202),
            recording: true,
            refreshes: complete_witness(),
            ..sample(&snap_init, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(4),
            viewer_raw: Some(202),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&snap_init, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
}

#[test]
fn gap_preserves_directly_witnessed_cache_write_for_next_hour() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    let shares_with_three = snap(&links, &Relations::factions_for_tests(&[(2, 3)]));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(3),
            viewer_raw: Some(3),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: empty_sides(),
                controller_targets: &[],
                local_viewers: &[4],
                cache_written_viewers: &[4],
            },
            ..sample(&shares_with_three, &[7], &[2])
        },
        Continuity::Gap,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    history.advance(
        Sample {
            viewer: Some(3),
            viewer_raw: Some(3),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&shares_with_three, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn shared_snapshot_fast_paths_match_distinct_snapshots() {
    let links = BTreeMap::from([(1, 1), (101, 1), (2, 2), (5, 5)]);
    let apart_rel = relations(false);
    let allied_rel = relations(true);
    let with_controllers = snap_with(
        &links,
        &allied_rel,
        unused_zero(),
        Some(rosters(&links, &[(40, 5)], 1)),
    );
    let no_controllers = snap_with(&links, &allied_rel, unused_zero(), None);
    let apart_none = snap_with(&links, &apart_rel, unused_zero(), None);
    let apart_some = snap_with(
        &links,
        &apart_rel,
        unused_zero(),
        Some(rosters(&links, &[(40, 5)], 1)),
    );
    let rec_sides = BTreeMap::from([((62, 0), side_witness(RECORDED))]);
    let unit_sides = BTreeMap::from([((62, 0), side_witness(UNITS))]);

    let run_pair = |steps: &[(
        &Arc<Snapshot>,
        Continuity,
        Option<u32>,
        Option<u32>,
        Option<u32>,
        Option<u32>,
        bool,
        RefreshWitness<'_>,
        &[u64],
    )]| {
        let mut shared = History::default();
        let mut distinct = History::default();
        let mut out = Vec::new();
        for &(
            base_snap,
            continuity,
            viewer,
            viewer_raw,
            alternate_viewer,
            alternate_viewer_raw,
            recording,
            refreshes,
            units,
        ) in steps
        {
            let cloned_snap = Arc::new((**base_snap).clone());
            shared.advance(
                Sample {
                    viewer,
                    viewer_raw,
                    alternate_viewer,
                    alternate_viewer_raw,
                    recording,
                    refreshes,
                    ..sample(base_snap, units, &[2])
                },
                continuity,
            );
            distinct.advance(
                Sample {
                    viewer,
                    viewer_raw,
                    alternate_viewer,
                    alternate_viewer_raw,
                    recording,
                    refreshes,
                    ..sample(&cloned_snap, units, &[2])
                },
                continuity,
            );
            assert_eq!(shared.side(62, 0), distinct.side(62, 0));
            assert_eq!(shared.side(62, 1), distinct.side(62, 1));
            out.push(shared.side(62, 0));
        }
        out
    };

    assert_eq!(
        run_pair(&[
            (&apart_none, Continuity::Load, Some(1), None, None, None, false, complete_witness(), &[7]),
            (&no_controllers, Continuity::Next, Some(1), None, None, None, false, complete_witness(), &[7]),
            (&no_controllers, Continuity::Next, Some(1), None, None, None, false, RefreshWitness::Unavailable, &[7]),
        ]),
        vec![Engaged::No, Engaged::No, Engaged::Ambiguous(Ambiguity::Scope)]
    );

    assert_eq!(
        run_pair(&[
            (&apart_some, Continuity::Load, Some(1), Some(1), None, None, true, complete_witness(), &[7]),
            (&with_controllers, Continuity::Next, Some(1), Some(1), None, None, true, complete_witness(), &[7]),
            (&with_controllers, Continuity::Next, Some(1), Some(999), None, None, true, RefreshWitness::Unavailable, &[7]),
        ]),
        vec![Engaged::No, Engaged::No, Engaged::Unknown]
    );

    assert_eq!(
        run_pair(&[
            (&apart_some, Continuity::Load, Some(1), Some(1), None, None, true, complete_witness(), &[7]),
            (&with_controllers, Continuity::Next, Some(1), Some(1), Some(1), Some(101), true, complete_witness(), &[7]),
            (&with_controllers, Continuity::Next, Some(1), Some(1), Some(1), Some(101), true, RefreshWitness::Unavailable, &[7]),
            (
                &with_controllers,
                Continuity::Next,
                Some(1),
                Some(1),
                None,
                None,
                false,
                RefreshWitness::Complete {
                    sides: &rec_sides,
                    controller_targets: &[1],
                    local_viewers: &[],
                    cache_written_viewers: &[],
                },
                &[7],
            ),
            (
                &with_controllers,
                Continuity::Next,
                Some(1),
                Some(1),
                None,
                None,
                false,
                RefreshWitness::Complete {
                    sides: &unit_sides,
                    controller_targets: &[],
                    local_viewers: &[],
                    cache_written_viewers: &[],
                },
                &[7],
            ),
        ]),
        vec![Engaged::No, Engaged::No, Engaged::Ambiguous(Ambiguity::Scope), Engaged::Yes, Engaged::Yes]
    );

    assert_eq!(
        run_pair(&[
            (&with_controllers, Continuity::Gap, Some(1), Some(1), None, None, true, complete_witness(), &[7]),
            (&with_controllers, Continuity::Next, Some(1), Some(1), None, None, true, RefreshWitness::Unavailable, &[7]),
            (&with_controllers, Continuity::Next, None, None, None, None, true, RefreshWitness::Unavailable, &[7]),
        ]),
        vec![Engaged::Unknown, Engaged::Unknown, Engaged::Unknown]
    );
}
