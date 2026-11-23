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

fn sample<'a>(
    links: &'a BTreeMap<u32, u32>,
    relations: &'a Relations,
    units: &[u64],
    primary: &[u32],
) -> Sample<'a> {
    defended(links, relations, units, primary, &[9])
}

fn complete_witness() -> RefreshWitness {
    RefreshWitness::Complete {
        sides: BTreeMap::new(),
        controller_targets: Vec::new(),
        local_viewers: Vec::new(),
        cache_written_viewers: Vec::new(),
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

fn defended<'a>(
    links: &'a BTreeMap<u32, u32>,
    relations: &'a Relations,
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
        links,
        relations: Ok(relations),
        zero_visibility: unused_zero(),
        controllers: Some(Controllers::default()),
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
    let (apart, allied) = (relations(false), relations(true));
    let mut history = History::default();
    let next = Continuity::Next;
    history.advance(
        defended(&links, &apart, &[7], &[2], &[8, 9]),
        Continuity::Load,
    );
    history.advance(defended(&links, &allied, &[7], &[2], &[8, 9]), next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(defended(&links, &allied, &[7], &[2], &[9]), next);
    assert_eq!(history.side(62, 0), Engaged::No);
    assert_eq!(history.side(62, 1), Engaged::No);
    let witnessed = Sample {
        refreshes: RefreshWitness::Complete {
            sides: BTreeMap::from([((62, 0), side_witness(UNITS))]),
            controller_targets: Vec::new(),
            local_viewers: Vec::new(),
            cache_written_viewers: Vec::new(),
        },
        ..defended(&links, &allied, &[7], &[2], &[9])
    };
    history.advance(witnessed, next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(defended(&links, &allied, &[6, 7], &[2], &[9]), next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(defended(&links, &allied, &[6, 7], &[2], &[]), next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn a_viewer_change_across_a_gap_is_not_a_rebuild_now() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    fn viewed<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        viewer: u32,
    ) -> Sample<'a> {
        Sample {
            viewer: Some(viewer),
            ..sample(links, relations, &[7], &[2])
        }
    }
    let mut history = History::default();
    history.advance(viewed(&links, &apart, 5), Continuity::Load);
    history.advance(viewed(&links, &allied, 1), Continuity::Gap);
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let mut history = History::default();
    history.advance(viewed(&links, &apart, 5), Continuity::Load);
    history.advance(viewed(&links, &allied, 1), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    let mut history = History::default();
    history.advance(viewed(&links, &allied, 5), Continuity::Load);
    history.advance(viewed(&links, &allied, 1), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn a_naval_removal_rebuilds_the_other_side() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    fn naval<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        defenders: &[u64],
    ) -> Sample<'a> {
        Sample {
            removal: Removal::BothSides,
            ..defended(links, relations, &[7], &[2], defenders)
        }
    }
    let mut history = History::default();
    history.advance(naval(&links, &apart, &[8, 9]), Continuity::Load);
    history.advance(naval(&links, &allied, &[8, 9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(naval(&links, &allied, &[9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    assert_eq!(history.side(62, 1), Engaged::No);
    history.advance(naval(&links, &apart, &[9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    let mut split = naval(&links, &apart, &[]);
    split.sides.get_mut(&(62, 0)).unwrap().lists = Some((vec![1], vec![]));
    history.advance(split, Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
}

#[test]
fn a_new_added_id_proves_a_rebuild() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    let mut history = History::default();
    history.advance(
        added(&links, &apart, Added::Observed(vec![3])),
        Continuity::Load,
    );
    history.advance(
        added(&links, &allied, Added::Observed(vec![3])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        added(&links, &allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

fn added<'a>(links: &'a BTreeMap<u32, u32>, relations: &'a Relations, added: Added) -> Sample<'a> {
    let mut sample = sample(links, relations, &[7], &[2]);
    sample.sides.get_mut(&(62, 0)).unwrap().added = added;
    sample
}

#[test]
fn unavailable_added_ids_leave_a_possible_rebuild_until_two_observations_compare() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    fn unavailable<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        a: Added,
    ) -> Sample<'a> {
        Sample {
            refreshes: RefreshWitness::Unavailable,
            ..added(links, relations, a)
        }
    }
    let mut history = History::default();
    history.advance(
        unavailable(&links, &apart, Added::Observed(vec![3])),
        Continuity::Load,
    );
    history.advance(
        unavailable(&links, &allied, Added::Unavailable),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(
        unavailable(&links, &allied, Added::Observed(vec![3])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    history.advance(
        unavailable(&links, &allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);

    let mut history = History::default();
    history.advance(
        unavailable(&links, &apart, Added::Observed(vec![3])),
        Continuity::Load,
    );
    history.advance(
        unavailable(&links, &apart, Added::Unavailable),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(
        unavailable(&links, &allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));

    let mut witnessed = History::default();
    witnessed.advance(added(&links, &apart, Added::Observed(vec![3])), Continuity::Load);
    witnessed.advance(added(&links, &allied, Added::Unavailable), Continuity::Next);
    assert_eq!(witnessed.side(62, 0), Engaged::No);
    let transfer = Sample {
        refreshes: RefreshWitness::Complete {
            sides: BTreeMap::from([((62, 0), side_witness(RECORDED))]),
            controller_targets: Vec::new(),
            local_viewers: Vec::new(),
            cache_written_viewers: Vec::new(),
        },
        ..added(&links, &allied, Added::Unavailable)
    };
    witnessed.advance(transfer, Continuity::Next);
    assert_eq!(witnessed.side(62, 0), Engaged::Yes);
}

#[test]
fn a_viewer_change_across_a_gap_rebuilds_nothing() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = relations(true);
    let mut history = History::default();
    history.advance(sample(&links, &allied, &[7], &[2]), Continuity::Load);
    let moved = |viewer| Sample {
        viewer: Some(viewer),
        ..sample(&links, &allied, &[7], &[2])
    };
    history.advance(moved(2), Continuity::Gap);
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let mut next = History::default();
    next.advance(sample(&links, &allied, &[7], &[2]), Continuity::Load);
    next.advance(moved(2), Continuity::Next);
    assert_eq!(next.side(62, 0), Engaged::Yes);
}

#[test]
fn a_viewer_change_is_ordered_against_both_samples_relations() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    fn at<'a>(links: &'a BTreeMap<u32, u32>, viewer: u32, relations: &'a Relations) -> Sample<'a> {
        Sample {
            viewer: Some(viewer),
            ..sample(links, relations, &[7], &[1])
        }
    }
    let run = |hours: &[(u32, &Relations)]| {
        let mut history = History::default();
        history.advance(at(&links, 1, &apart), Continuity::Load);
        assert_eq!(history.side(62, 0), Engaged::Yes);
        for &(viewer, relations) in hours {
            history.advance(at(&links, viewer, relations), Continuity::Next);
        }
        history.side(62, 0)
    };
    assert_eq!(run(&[(2, &apart), (2, &allied)]), Engaged::No);
    assert_eq!(run(&[(1, &allied), (2, &allied)]), Engaged::Yes);
    assert_eq!(run(&[(2, &allied)]), Engaged::Ambiguous(Ambiguity::Order));
    let mut ending = History::default();
    ending.advance(at(&links, 1, &allied), Continuity::Load);
    ending.advance(at(&links, 2, &apart), Continuity::Next);
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
    fn at<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        controllers: Option<Controllers>,
        refreshes: RefreshWitness,
    ) -> Sample<'a> {
        Sample {
            controllers,
            refreshes,
            ..sample(links, relations, &[7], &[2])
        }
    }
    let away = Some(rosters(&links, &[(40, 5)], 1));
    let run = |relations: &Relations, controllers: Option<Controllers>, refreshes: RefreshWitness| {
        let mut history = History::default();
        history.advance(at(&links, &apart, away.clone(), complete_witness()), Continuity::Load);
        history.advance(at(&links, relations, away.clone(), complete_witness()), Continuity::Next);
        history.advance(at(&links, relations, controllers, refreshes), Continuity::Next);
        history.side(62, 0)
    };
    let joined = Some(rosters(&links, &[(40, 1)], 2));
    assert_eq!(run(&allied, away.clone(), complete_witness()), Engaged::No);
    assert_eq!(
        run(
            &allied,
            away.clone(),
            RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: vec![1],
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            }
        ),
        Engaged::Yes
    );
    assert_eq!(
        run(
            &allied,
            away.clone(),
            RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: vec![5],
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            }
        ),
        Engaged::No
    );
    assert_eq!(
        run(
            &allied,
            away.clone(),
            RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: vec![99],
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
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
    let before = BTreeMap::from([(1, 1), (5, 5), (7, 2)]);
    let after = BTreeMap::from([(1, 1), (5, 5), (7, 3)]);
    let mut history = History::default();
    history.advance(sample(&before, &allied, &[7], &[7]), Continuity::Load);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    let mut steady = History::default();
    steady.advance(sample(&after, &allied, &[7], &[7]), Continuity::Load);
    steady.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(steady.side(62, 0), Engaged::No);
    let unlinked = BTreeMap::from([(1, 1), (5, 5)]);
    let mut unplaced = History::default();
    unplaced.advance(sample(&unlinked, &allied, &[7], &[7]), Continuity::Load);
    unplaced.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(unplaced.side(62, 0), Engaged::Unknown);
}

#[test]
fn history_lags_diplomacy_until_the_unit_set_changes() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    let mut history = History::default();
    history.advance(sample(&links, &apart, &[7], &[2]), Continuity::Load);
    assert_eq!(history.combat(62), Engaged::No);
    history.advance(sample(&links, &allied, &[7], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::No);
    history.advance(sample(&links, &allied, &[7, 8], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::Yes);
    history.advance(sample(&links, &apart, &[7], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(sample(&links, &apart, &[8], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::No);
    history.advance(sample(&links, &apart, &[8], &[2]), Continuity::Gap);
    assert_eq!(history.combat(62), Engaged::Unknown);

    let mut started = History::default();
    let empty = Sample {
        sides: BTreeMap::new(),
        ..sample(&links, &allied, &[], &[])
    };
    started.advance(empty, Continuity::Load);
    started.advance(sample(&links, &allied, &[7], &[2]), Continuity::Next);
    assert_eq!(started.combat(62), Engaged::Yes);
}

#[test]
fn without_refresh_witness_diplomacy_change_is_ambiguous() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    let mut history = History::default();
    fn without<'a>(links: &'a BTreeMap<u32, u32>, relations: &'a Relations) -> Sample<'a> {
        Sample {
            refreshes: RefreshWitness::Unavailable,
            ..sample(links, relations, &[7], &[2])
        }
    }
    history.advance(without(&links, &apart), Continuity::Load);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(without(&links, &apart), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(without(&links, &allied), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(without(&links, &allied), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    let joined = Sample {
        refreshes: RefreshWitness::Unavailable,
        ..sample(&links, &allied, &[7, 8], &[2])
    };
    history.advance(joined, Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn witnessed_same_roster_readd_remove_and_local_refresh_drive_engaged_state() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));

    let mut readd = History::default();
    readd.advance(sample(&links, &apart, &[7], &[2]), Continuity::Load);
    assert_eq!(readd.side(62, 0), Engaged::No);
    readd.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([((62, 0), side_witness(RECORDED))]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(readd.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    readd.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([((62, 0), side_witness(RECORDED))]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(readd.side(62, 0), Engaged::Yes);

    let mut bounce = History::default();
    let mut naval_side = side(&[7], &[1]);
    naval_side.unit_lists = Some((vec![2], vec![]));
    fn naval_at<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        naval_side: &Side,
        refreshes: RefreshWitness,
    ) -> Sample<'a> {
        Sample {
            refreshes,
            sides: BTreeMap::from([((62, 0), naval_side.clone()), ((62, 1), side(&[9], &[5]))]),
            removal: Removal::BothSides,
            ..sample(links, relations, &[7], &[1])
        }
    }
    bounce.advance(
        naval_at(&links, &apart, &naval_side, complete_witness()),
        Continuity::Load,
    );
    assert_eq!(bounce.side(62, 0), Engaged::Yes);
    bounce.advance(
        naval_at(
            &links,
            &apart,
            &naval_side,
            RefreshWitness::Complete {
                sides: BTreeMap::from([((62, 0), side_witness(UNITS))]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
        ),
        Continuity::Next,
    );
    assert_eq!(bounce.side(62, 0), Engaged::No);

    let zero_no = ZeroVisibility::for_tests([(2, Ok(false)), (5, Ok(false))]);
    let zero_yes = ZeroVisibility::for_tests([(2, Ok(true)), (5, Ok(false))]);
    let zero_missing = ZeroVisibility::unavailable("missing");
    let mut local = History::default();
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_no,
            ..sample(&links, &apart, &[7], &[2])
        },
        Continuity::Load,
    );
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_no,
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::No);
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_no,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![1],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Yes);
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_no,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![0],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::No);
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_yes,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![0],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_yes,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![0],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Yes);
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_missing,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![0],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(local.side(62, 0), Engaged::Unknown);
    local.advance(
        Sample {
            recording: true,
            zero_visibility: &zero_no,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![99],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
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
        sides: BTreeMap::new(),
        controller_targets: vec![7],
        local_viewers: Vec::new(),
        cache_written_viewers: Vec::new(),
    };

    for (prev_links, next_links) in [
        (&before_links, &after_links),
        (&after_links, &before_links),
        (&unlinked, &before_links),
        (&before_links, &unlinked),
    ] {
        let mut history = History::default();
        history.advance(sample(prev_links, &apart, &[7], &[2]), Continuity::Load);
        history.advance(sample(prev_links, &allied, &[7], &[2]), Continuity::Next);
        assert_eq!(history.side(62, 0), Engaged::No);
        history.advance(
            Sample {
                refreshes: witness_seven(),
                ..sample(next_links, &allied, &[7], &[2])
            },
            Continuity::Next,
        );
        assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    }
}

#[test]
fn partial_previous_census_still_applies_witnessed_side_refresh() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = relations(true);
    let mut history = History::default();
    let partial_empty = Sample {
        sides: BTreeMap::new(),
        complete: false,
        ..sample(&links, &allied, &[], &[])
    };
    history.advance(partial_empty, Continuity::Load);
    history.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([((62, 0), side_witness(RECORDED))]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn partial_previous_naval_census_with_local_sweep_uses_recorded_lists_alone() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = relations(true);
    let mut history = History::default();
    let partial_empty = Sample {
        sides: BTreeMap::new(),
        complete: false,
        removal: Removal::BothSides,
        ..sample(&links, &allied, &[], &[])
    };
    history.advance(partial_empty, Continuity::Load);
    let mut convoy_side = side(&[], &[2]);
    convoy_side.unit_lists = Some((Vec::new(), Vec::new()));
    history.advance(
        Sample {
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![1],
                cache_written_viewers: Vec::new(),
            },
            sides: BTreeMap::from([((62, 0), convoy_side), ((62, 1), side(&[9], &[5]))]),
            complete: true,
            removal: Removal::BothSides,
            ..sample(&links, &allied, &[], &[])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

#[test]
fn alias_change_to_same_slot_does_not_prove_sweep_in_unavailable_interval() {
    let links = BTreeMap::from([(101, 1), (701, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&links, &allied, &[7], &[2])
        },
        Continuity::Load,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&links, &apart, &[7], &[2])
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
            ..sample(&links, &apart, &[7], &[2])
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
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            ..sample(&links_before, &shares_with_one, &[7], &[3])
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
            ..sample(&links_after, &shares_with_one, &[7], &[3])
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
            ..sample(&links_after, &shares_with_one, &[7], &[3])
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
            ..sample(&links_before, &shares_with_one, &[7], &[3])
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
            ..sample(&links_after, &shares_with_one, &[7], &[3])
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
    let mut reverse_remap = History::default();
    reverse_remap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(701),
            recording: true,
            ..sample(&same_before, &shares_with_one, &[7], &[3])
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
            ..sample(&same_before, &apart, &[7], &[3])
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
            ..sample(&split_after, &apart, &[7], &[3])
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
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            zero_visibility: &zero_yes,
            ..sample(&links, &apart, &[7], &[2])
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
            zero_visibility: &zero_no,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![0],
                cache_written_viewers: vec![0],
            },
            ..sample(&links, &apart, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(
        Sample {
            viewer: Some(0),
            viewer_raw: Some(0),
            recording: true,
            zero_visibility: &zero_no,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&links, &apart, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn unidentified_viewer_degrades_unavailable_hour_conservatively() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_three = Relations::factions_for_tests(&[(2, 3)]);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links, &shares_with_three, &[7], &[2])
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
            ..sample(&links, &shares_with_three, &[7], &[2])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
}

#[test]
fn local_all_sides_sweep_preserves_prior_cached_manager_viewer() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_two = Relations::factions_for_tests(&[(2, 3)]);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links, &shares_with_two, &[7], &[3])
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
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![2],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &shares_with_two, &[7], &[3])
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
            ..sample(&links, &shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn unavailable_interval_keeps_secondary_observer_in_manager_cache() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_two = Relations::factions_for_tests(&[(2, 3)]);
    let apart = relations(false);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links, &shares_with_two, &[7], &[3])
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
            ..sample(&links, &apart, &[7], &[3])
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
            ..sample(&links, &apart, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn gap_does_not_establish_recording_manager_cache() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_one = Relations::factions_for_tests(&[(1, 3)]);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links, &shares_with_one, &[7], &[3])
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
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([(
                    (62, 0),
                    SideWitness {
                        lists: RECORDED,
                        recorded_viewers: vec![1],
                        units_viewers: Vec::new(),
                    },
                )]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &shares_with_one, &[7], &[3])
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
            ..sample(&links, &shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
}

#[test]
fn mid_campaign_gap_recovers_recording_manager_cache_after_full_tick() {
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3), (5, 5)]);
    let shares_with_one = Relations::factions_for_tests(&[(1, 3)]);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links, &shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([(
                    (62, 0),
                    SideWitness {
                        lists: RECORDED,
                        recorded_viewers: vec![1],
                        units_viewers: Vec::new(),
                    },
                )]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &shares_with_one, &[7], &[3])
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
            ..sample(&links, &shares_with_one, &[7], &[3])
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
            ..sample(&links, &shares_with_one, &[7], &[3])
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
            ..sample(&links, &shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    local_after_gap.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![2],
                cache_written_viewers: Vec::new(),
            },
            ..sample(&links, &shares_with_one, &[7], &[3])
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
            ..sample(&links, &shares_with_one, &[7], &[3])
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
    let aliased = BTreeMap::from([(101, 1), (202, 1), (3, 3), (5, 5)]);
    let split = BTreeMap::from([(101, 1), (202, 2), (3, 3), (5, 5)]);
    let shares_with_one = Relations::factions_for_tests(&[(1, 3)]);

    let mut from_complete = History::default();
    from_complete.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            ..sample(&aliased, &shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    from_complete.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([(
                    (62, 0),
                    SideWitness {
                        lists: RECORDED,
                        recorded_viewers: vec![101],
                        units_viewers: Vec::new(),
                    },
                )]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&aliased, &shares_with_one, &[7], &[3])
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
            ..sample(&split, &shares_with_one, &[7], &[3])
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
            ..sample(&split, &shares_with_one, &[7], &[3])
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
            ..sample(&aliased, &shares_with_one, &[7], &[3])
        },
        Continuity::Gap,
    );
    from_unavailable.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            refreshes: RefreshWitness::Unavailable,
            ..sample(&aliased, &shares_with_one, &[7], &[3])
        },
        Continuity::Next,
    );
    from_unavailable.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(101),
            recording: true,
            refreshes: RefreshWitness::Complete {
                sides: BTreeMap::from([(
                    (62, 0),
                    SideWitness {
                        lists: RECORDED,
                        recorded_viewers: vec![101],
                        units_viewers: Vec::new(),
                    },
                )]),
                controller_targets: Vec::new(),
                local_viewers: Vec::new(),
                cache_written_viewers: Vec::new(),
            },
            ..sample(&aliased, &shares_with_one, &[7], &[3])
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
            ..sample(&split, &shares_with_one, &[7], &[3])
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
            ..sample(&split, &shares_with_one, &[7], &[3])
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
    let shares_with_two = Relations::factions_for_tests(&[(2, 3)]);
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links, &shares_with_two, &[7], &[3])
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
            ..sample(&links, &shares_with_two, &[7], &[3])
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
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![2],
                cache_written_viewers: vec![2],
            },
            ..sample(&links, &shares_with_two, &[7], &[3])
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
            ..sample(&links, &shares_with_two, &[7], &[3])
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
            ..sample(&links, &shares_with_two, &[7], &[3])
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
            ..sample(&links, &shares_with_two, &[7], &[3])
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
    let mut history = History::default();
    history.advance(
        Sample {
            viewer: Some(1),
            viewer_raw: Some(1),
            recording: true,
            ..sample(&links_init, &shares_with_two, &[7], &[3])
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
            ..sample(&links_init, &shares_with_two, &[7], &[3])
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
                sides: BTreeMap::new(),
                controller_targets: Vec::new(),
                local_viewers: vec![101],
                cache_written_viewers: vec![101],
            },
            ..sample(&links_aliased, &shares_with_two, &[7], &[3])
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
            ..sample(&links_init, &shares_with_two, &[7], &[3])
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
            ..sample(&links_init, &shares_with_two, &[7], &[3])
        },
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::No);
}
