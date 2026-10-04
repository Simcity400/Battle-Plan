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
    // An unresolved reference blocks only a negative answer.
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

/// One side's hour as fixed test answers per list kind, [now, before].
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

/// [`super::step`] over fixed answers, counting the evaluations it asks.
fn step(held: Engaged, hour: SideHour) -> Engaged {
    super::step(held, hour.refresh, |lists, when| match lists {
        Tested::Recorded => hour.recorded[when],
        Tested::Units => hour.units[when],
    })
}

/// A held side evaluates nothing, a load one list under one relation.
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
    // A relation change alone does not rebuild.
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

/// A rebuild the hour only allows keeps the held value as a candidate; a
/// join and a removal in one hour test both list kinds.
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
    // A proven rebuild drops the held value.
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
    // Two ambiguous sides report one cause, whichever side it is on.
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

/// The defending side (country 5) is foreign to the viewer throughout.
fn defended<'a>(
    links: &'a BTreeMap<u32, u32>,
    relations: &'a Relations,
    units: &[u64],
    primary: &[u32],
    defenders: &[u64],
) -> Sample<'a> {
    Sample {
        viewer: 1,
        links,
        relations: Ok(relations),
        controllers: Some(Controllers::default()),
        sides: BTreeMap::from([
            ((62, 0), side(units, primary)),
            ((62, 1), side(defenders, &[5])),
        ]),
        complete: true,
        removal: Removal::BothSidesUnlessRoundRetreat,
    }
}

/// RemoveUnitFromCombat (1413CE250) rebuilds both sides, so a unit
/// leaving the defenders may let the attackers pick up a faction joined
/// earlier. Land cannot tell it from a round retreat (1413CE670), which
/// rebuilds only the retreating side, so the attackers are ambiguous when
/// the two disagree and decided when they agree.
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
    // The faction join alone rebuilds nothing.
    history.advance(defended(&links, &allied, &[7], &[2], &[8, 9]), next);
    assert_eq!(history.side(62, 0), Engaged::No);
    history.advance(defended(&links, &allied, &[7], &[2], &[9]), next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    // The defenders lost the unit themselves: rebuilt either way.
    assert_eq!(history.side(62, 1), Engaged::No);
    // A join decides the attackers again; a later defender removal whose
    // rebuild agrees with the held value keeps it.
    history.advance(defended(&links, &allied, &[6, 7], &[2], &[9]), next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    history.advance(defended(&links, &allied, &[6, 7], &[2], &[]), next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

/// A viewer change after unobserved hours proves no rebuild in this
/// sample: the player change rebuilt at an unknown time, before the
/// alliance this sample holds, so the flag stays unknown. Between
/// consecutive samples it is a proven every-side refresh the samples
/// cannot order against the alliance, so both relations are candidates.
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
            viewer,
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
    // With the alliance already in the previous sample, the refresh is Yes either way.
    let mut history = History::default();
    history.advance(viewed(&links, &allied, 5), Continuity::Load);
    history.advance(viewed(&links, &allied, 1), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
}

/// A naval removal is always `RemoveUnitFromCombat` (1413CE250), so a unit
/// leaving the defenders proves the attackers rebuilt from their own
/// units: the faction joined earlier engages them, where land's round
/// retreat leaves the same hour ambiguous.
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
    // The alliance ending rebuilds nothing on its own.
    history.advance(naval(&links, &apart, &[9]), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Yes);
    // Only the rebuilt lists decide: the attackers' remaining owners, not their recorded
    // lists, which can hold convoy owners no rebuild tested.
    let mut split = naval(&links, &apart, &[]);
    split.sides.get_mut(&(62, 0)).unwrap().lists = Some((vec![1], vec![]));
    history.advance(split, Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
}

/// An id in [`Side::added`] that the previous sample lacked proves a
/// rebuild of the recorded lists, with no unit change.
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

/// The attackers' sample with their [`Side::added`] ids.
fn added<'a>(links: &'a BTreeMap<u32, u32>, relations: &'a Relations, added: Added) -> Sample<'a> {
    let mut sample = sample(links, relations, &[7], &[2]);
    sample.sides.get_mut(&(62, 0)).unwrap().added = added;
    sample
}

/// An hour whose ids are unavailable may hide a join (`AddTransferToGroup@1416084E0`
/// refreshes the flag), so it cannot hold a value the recorded lists contradict; and the
/// first observation after it proves a join only against the previous hour, which it cannot
/// compare with, so an id already seen before the gap proves nothing and a new one only
/// allows a rebuild.
#[test]
fn unavailable_added_ids_leave_a_possible_rebuild_until_two_observations_compare() {
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
    // A join would test the allied lists: not a definite No.
    history.advance(added(&links, &allied, Added::Unavailable), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    // Slot 3 was already there before the gap: no join is invented.
    history.advance(
        added(&links, &allied, Added::Observed(vec![3])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Scope));
    // Two comparable observations prove the join again.
    history.advance(
        added(&links, &allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Yes);

    let mut history = History::default();
    history.advance(
        added(&links, &apart, Added::Observed(vec![3])),
        Continuity::Load,
    );
    // Every candidate agrees: whether or not a convoy joined, the flag is No.
    history.advance(added(&links, &apart, Added::Unavailable), Continuity::Next);
    assert_eq!(history.side(62, 0), Engaged::No);
    // Slot 4 is new against the last observation, but may have joined in the unavailable
    // hour, before the alliance: only possible, so the held No and the allied Yes disagree,
    // and the samples cannot order that join against the alliance.
    history.advance(
        added(&links, &allied, Added::Observed(vec![3, 4])),
        Continuity::Next,
    );
    assert_eq!(history.side(62, 0), Engaged::Ambiguous(Ambiguity::Order));
    // A kind without the trigger (land) is unaffected by any of this.
    let mut land = History::default();
    land.advance(added(&links, &apart, Added::NoTrigger), Continuity::Load);
    land.advance(added(&links, &allied, Added::NoTrigger), Continuity::Next);
    assert_eq!(land.side(62, 0), Engaged::No);
}

/// A viewer change inside unobserved hours is not placed at the hour the
/// samples resume: the sides stay unknown until they rebuild.
#[test]
fn a_viewer_change_across_a_gap_rebuilds_nothing() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let allied = relations(true);
    let mut history = History::default();
    history.advance(sample(&links, &allied, &[7], &[2]), Continuity::Load);
    let moved = |viewer| Sample {
        viewer,
        ..sample(&links, &allied, &[7], &[2])
    };
    history.advance(moved(2), Continuity::Gap);
    assert_eq!(history.side(62, 0), Engaged::Unknown);
    let mut next = History::default();
    next.advance(sample(&links, &allied, &[7], &[2]), Continuity::Load);
    next.advance(moved(2), Continuity::Next);
    assert_eq!(next.side(62, 0), Engaged::Yes);
}

/// A viewer change between samples refreshes every side (140BA9890, or
/// 14113E4B0 through 140BABC00) at its own moment, never as a load. The
/// side lists the old viewer (1); the new viewer (2) shares vision with it
/// only while allied. Viewer change then alliance: the refresh saw no
/// alliance and the alliance refreshes nothing, so native keeps No.
/// Alliance then viewer change: the refresh sees it, Yes. In one interval
/// the samples are the same for both orders.
#[test]
fn a_viewer_change_is_ordered_against_both_samples_relations() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    fn at<'a>(links: &'a BTreeMap<u32, u32>, viewer: u32, relations: &'a Relations) -> Sample<'a> {
        Sample {
            viewer,
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
    // Apart samples: the viewer changed first.
    assert_eq!(run(&[(2, &apart), (2, &allied)]), Engaged::No);
    // Apart samples: the alliance came first.
    assert_eq!(run(&[(1, &allied), (2, &allied)]), Engaged::Yes);
    // One interval holds both, in an order the samples cannot tell.
    assert_eq!(run(&[(2, &allied)]), Engaged::Ambiguous(Ambiguity::Order));
    // And the other way: the alliance ends as the viewer changes.
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

/// 14113E4B0 refreshes every combat through 140BABC00 when it gives any
/// controller the local player's country. The hourly rosters can only
/// allow that: a stale flag that the refresh would change is ambiguous,
/// never asserted, and a roster change elsewhere refreshes nothing.
#[test]
fn a_controller_joining_the_viewer_may_refresh_a_stale_flag() {
    let links = BTreeMap::from([(1, 1), (2, 2), (5, 5)]);
    let (apart, allied) = (relations(false), relations(true));
    fn at<'a>(
        links: &'a BTreeMap<u32, u32>,
        relations: &'a Relations,
        controllers: Option<Controllers>,
    ) -> Sample<'a> {
        Sample {
            controllers,
            ..sample(links, relations, &[7], &[2])
        }
    }
    let away = Some(rosters(&links, &[(40, 5)], 1));
    let run = |relations: &Relations, controllers: Option<Controllers>| {
        let mut history = History::default();
        history.advance(at(&links, &apart, away.clone()), Continuity::Load);
        // The alliance alone refreshes nothing: the flag goes stale.
        history.advance(at(&links, relations, away.clone()), Continuity::Next);
        history.advance(at(&links, relations, controllers), Continuity::Next);
        history.side(62, 0)
    };
    let joined = Some(rosters(&links, &[(40, 1)], 2));
    assert_eq!(run(&allied, away.clone()), Engaged::No);
    assert_eq!(
        run(&allied, joined.clone()),
        Engaged::Ambiguous(Ambiguity::Scope)
    );
    // A new player key, or only a higher count, allows it too.
    for rosters in [
        rosters(&links, &[(40, 5), (41, 1)], 1),
        rosters(&links, &[(40, 5)], 2),
    ] {
        assert_eq!(
            run(&allied, Some(rosters)),
            Engaged::Ambiguous(Ambiguity::Scope)
        );
    }
    // Rosters that are not whole cannot rule it out.
    assert_eq!(run(&allied, None), Engaged::Ambiguous(Ambiguity::Scope));
    // A refresh that agrees with the held flag keeps it.
    assert_eq!(run(&apart, joined), Engaged::No);
    // A controller leaving for another country refreshes nothing.
    assert_eq!(
        run(&allied, Some(rosters(&links, &[(40, 2)], 1))),
        Engaged::No
    );
}

/// Raw reference 7 moves from country 2 (the viewer's ally) to country 3
/// between two samples in which a unit joins. A refresh before the move
/// resolved 7 as 2 (Yes), one after it as 3 (No): the samples cannot order
/// them, so the flag is ambiguous, never the current links' No for both.
/// A reference the previous links lack leaves that candidate unknown.
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
    // The same links in both samples keep one answer.
    let mut steady = History::default();
    steady.advance(sample(&after, &allied, &[7], &[7]), Continuity::Load);
    steady.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(steady.side(62, 0), Engaged::No);
    // A reference the earlier links cannot place stays unresolved there.
    let unlinked = BTreeMap::from([(1, 1), (5, 5)]);
    let mut unplaced = History::default();
    unplaced.advance(sample(&unlinked, &allied, &[7], &[7]), Continuity::Load);
    unplaced.advance(sample(&after, &allied, &[7, 8], &[7]), Continuity::Next);
    assert_eq!(unplaced.side(62, 0), Engaged::Unknown);
}

/// Joining a faction does not engage a combat until a unit joins or
/// leaves it; a combat that starts after an earlier sample rebuilds.
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
    // A unit change in the hour the alliance ends cannot be ordered.
    history.advance(sample(&links, &apart, &[7], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::Ambiguous(Ambiguity::Order));
    history.advance(sample(&links, &apart, &[8], &[2]), Continuity::Next);
    assert_eq!(history.combat(62), Engaged::No);
    // After unobserved hours only a proven rebuild decides.
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
