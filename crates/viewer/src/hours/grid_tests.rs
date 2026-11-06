use super::*;

fn state() -> CoordinatorState {
    let (events, _) = mpsc::channel();
    let (outbox, _) = mpsc::sync_channel(1);
    CoordinatorState {
        device: Default::default(),
        playback: Playback::unloaded(Arc::new(crate::playback::tests::fixture())).unwrap(),
        shared_decoder: None,
        lane: None,
        requested: Default::default(),
        profile: Default::default(),
        profile_want: None,
        profile_received: None,
        engaged: Default::default(),
        want: None,
        epoch: 0,
        sent: BTreeMap::new(),
        sends: 0,
        usable_sent: BTreeSet::new(),
        anchor: None,
        slots: BTreeMap::new(),
        store: HourStore::private(),
        reusable: BTreeMap::new(),
        pool: Arc::new(Pool::new(1).unwrap()),
        parallel: 2,
        width: Width::new(2),
        events,
        outbox,
        result_space: Default::default(),
    }
}

fn want(hour: Hour) -> Want {
    Want {
        branch: 1,
        hour,
        ahead: 0,
        behind: 0,
        playing: true,
        perspective: Default::default(),
        map_mode: Default::default(),
        diplomatic_selected: None,
        show_allied_plans: false,
        counters: Default::default(),
        view: super::tests::test_view(),
        have: Vec::new(),
        received: 0,
        grid: None,
    }
}

fn on_grid(mut want: Want, sequence: u64, stride: u32, anchor: Option<Hour>) -> Want {
    want.grid = Some(Grid {
        cadence: Cadence {
            sequence,
            stride,
            count: 2,
            anchor,
            lead: 0,
        },
        installed: None,
        usable: Vec::new(),
    });
    want
}

fn wanted(state: &CoordinatorState) -> Vec<Hour> {
    state.wanted_hours(state.want.as_ref().unwrap(), (0, 10_000))
}

#[test]
fn without_a_grid_the_wanted_hours_and_the_sent_hours_kept_are_todays() {
    let state = state();
    let range = (0, 10_000);
    for (hour, ahead, behind, playing, expected) in [
        (
            100,
            7,
            0,
            true,
            vec![100, 101, 102, 103, 104, 105, 106, 107],
        ),
        (100, 2, 2, false, vec![100, 101, 102, 99, 98]),
        (5, 3, 9, false, vec![5, 6, 7, 8, 4, 3, 2, 1, 0]),
        (9_998, 7, 0, true, vec![9_998, 9_999, 10_000]),
    ] {
        let want = Want {
            ahead,
            behind,
            playing,
            ..want(hour)
        };
        assert_eq!(state.wanted_hours(&want, range), expected);
        let want = Want {
            have: vec![hour, hour + 1],
            received: 3,
            ..want
        };
        for branch in [1, 2] {
            for at in hour - 12..=hour + 12 {
                for order in 0..6 {
                    let key = (branch, at);
                    let today = branch == 1
                        && (hour - behind..=hour + ahead).contains(&at)
                        && (order >= 3 || at == hour || at == hour + 1);
                    assert_eq!(want.keeps_sent(key, order), today, "{key:?} sent {order}");
                }
            }
        }
    }
}

#[test]
fn the_grid_keeps_a_due_fallback_until_a_newer_usable_hour_is_due_or_installed() {
    let mut want = on_grid(want(121), 1, 40, None);
    let wanted: BTreeSet<(BranchId, Hour)> = [(1, 160), (1, 200)].into();
    let facts = |want: &Want, flights: &[((BranchId, Hour), bool)], sent: &[(BranchId, Hour)]| {
        want.grid_facts(flights.iter().copied(), sent.iter().copied())
    };
    want.grid.as_mut().unwrap().installed = Some(80);
    let f = facts(&want, &[((1, 120), true)], &[(1, 160)]);
    assert_eq!(f.newest_usable_due, None, "a future hour is not due");
    assert!(want.keeps_flight((1, 120), true, &wanted, &f));
    let f = facts(
        &want,
        &[((1, 80), false), ((1, 100), false), ((1, 120), false)],
        &[(1, 160)],
    );
    assert_eq!(f.newest_reserved_due, Some(120));
    assert!(want.keeps_flight((1, 120), false, &wanted, &f));
    assert!(
        !want.keeps_flight((1, 100), false, &wanted, &f),
        "an older due target"
    );
    assert!(
        !want.keeps_flight((1, 80), false, &wanted, &f),
        "the hour installed"
    );
    let mut later = Want {
        hour: 170,
        ..want.clone()
    };
    later.grid.as_mut().unwrap().usable = vec![160];
    let f = facts(&later, &[((1, 120), false)], &[]);
    assert_eq!(f.newest_usable_due, Some(160));
    assert!(!later.keeps_flight((1, 120), false, &wanted, &f));
    assert!(later.keeps_flight((1, 120), true, &wanted, &f));
    later.grid.as_mut().unwrap().usable.clear();
    let f = facts(&later, &[((1, 120), false)], &[]);
    assert!(later.keeps_flight((1, 120), false, &wanted, &f));
    let f = facts(&later, &[((1, 120), false)], &[(1, 160)]);
    assert!(!later.keeps_flight((1, 120), false, &wanted, &f));
    later.grid.as_mut().unwrap().installed = Some(160);
    let f = facts(&later, &[((1, 120), true), ((1, 160), true)], &[]);
    assert!(!later.keeps_flight((1, 120), true, &wanted, &f));
    assert!(!later.keeps_flight((1, 160), true, &wanted, &f));
    assert!(later.keeps_flight((1, 200), false, &wanted, &f));
    assert!(!later.keeps_flight((2, 200), true, &wanted, &f));
}

#[test]
fn the_grid_starts_where_the_clock_will_be_when_a_build_lands() {
    let state = state();
    let mut want = on_grid(want(101), 1, 40, None);
    assert_eq!(state.wanted_hours(&want, (0, 10_000)), [120, 160]);
    want.grid.as_mut().unwrap().cadence.lead = 144;
    assert_eq!(state.wanted_hours(&want, (0, 10_000)), [280, 320]);
    assert_eq!(state.wanted_hours(&want, (0, 300)), [280, 300]);
}

#[test]
fn a_strided_target_shares_only_with_settled_neighbours() {
    let settled = Slots::new();
    Fill(Arc::clone(&settled.political)).set(None);
    Fill(Arc::clone(&settled.borders)).set(None);
    Fill(Arc::clone(&settled.rails)).set(None);
    Fill(Arc::clone(&settled.buildings)).set(None);
    let slots: BTreeMap<(BranchId, Hour), Slots> = [
        ((1, 80), settled.clone()),
        ((1, 120), Slots::new()),
        ((1, 200), Slots::new()),
        ((1, 240), settled.clone()),
    ]
    .into();
    let near: Vec<_> = settled_slots(&slots, 1, 160)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(near, [(1, 80), (1, 240)]);
    let near: Vec<_> = neighbour_slots(&slots, 1, 160)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(near, [(1, 120), (1, 200)]);
}

#[test]
fn an_anchor_is_wanted_until_the_window_has_it_and_its_intent_never_wants_it_again() {
    let mut state = state();
    let base = want(107);
    let at = |hour, sequence| {
        on_grid(
            Want {
                hour,
                ..base.clone()
            },
            sequence,
            20,
            Some(101),
        )
    };
    state.set_want(0, at(107, 1));
    assert_eq!(wanted(&state), [101, 120, 140]);
    state.sent.insert((1, 101), 0);
    state.note_sent(0, (1, 101), 0, true);
    state.sends = 1;
    state.set_want(0, at(113, 1));
    assert!(state.sent.contains_key(&(1, 101)));
    assert!(state.usable_sent.contains(&(1, 101)));
    assert_eq!(wanted(&state), [101, 120, 140]);
    let received = Want {
        received: 1,
        ..at(115, 1)
    };
    state.set_want(0, received.clone());
    assert_eq!(wanted(&state), [120, 140]);
    assert!(!state.sent.contains_key(&(1, 101)), "received and not held");
    assert!(!state.usable_sent.contains(&(1, 101)));
    state.set_want(
        0,
        Want {
            received: 0,
            ..received
        },
    );
    assert_eq!(wanted(&state), [120, 140]);
    state.set_want(0, at(115, 2));
    assert_eq!(wanted(&state), [101, 120, 140]);
    state.set_want(
        0,
        Want {
            have: vec![101],
            ..at(115, 2)
        },
    );
    assert_eq!(wanted(&state), [120, 140]);
    state.set_want(0, at(117, 2));
    assert_eq!(wanted(&state), [120, 140]);
    state.set_want(0, at(115, 3));
    assert_eq!(wanted(&state), [101, 120, 140]);
    let mut installed = at(115, 3);
    installed.grid.as_mut().unwrap().installed = Some(110);
    state.set_want(0, installed);
    assert_eq!(wanted(&state), [120, 140]);
    state.set_want(0, at(115, 4));
    state.note_sent(7, (1, 101), 5, true);
    state.set_want(
        0,
        Want {
            received: 9,
            ..at(115, 4)
        },
    );
    assert_eq!(wanted(&state), [101, 120, 140]);
    state.set_want(1, at(115, 4));
    assert_eq!(wanted(&state), [120, 140]);
    state.set_want(1, at(117, 4));
    assert_eq!(wanted(&state), [120, 140]);
}

#[test]
fn an_old_epochs_feedback_never_ends_or_supersedes_a_new_intent() {
    for anchor in [101, 500] {
        let mut state = state();
        let mut old = on_grid(want(500), 1, 20, None);
        old.have = vec![480, 500, 520];
        let grid = old.grid.as_mut().unwrap();
        (grid.installed, grid.usable) = (Some(500), vec![480, 520]);
        state.set_want(0, old.clone());
        let mut next = Want {
            hour: anchor,
            show_allied_plans: true,
            ..old.clone()
        };
        next.grid.as_mut().unwrap().cadence = Cadence {
            sequence: 2,
            stride: 40,
            count: 2,
            anchor: Some(anchor),
            lead: 0,
        };
        let mut submitted = next.clone();
        assert!(submitted.discard_stale_have(Some(&old), false));
        let grid = submitted.grid.as_ref().unwrap();
        assert_eq!(
            (submitted.have.len(), grid.installed, grid.usable.len()),
            (0, None, 0)
        );
        assert_eq!(grid.cadence.anchor, Some(anchor));
        state.set_want(1, next);
        let reduced = state.want.as_ref().unwrap();
        let grid = reduced.grid.as_ref().unwrap();
        assert_eq!(
            (reduced.have.len(), grid.installed, grid.usable.len()),
            (0, None, 0)
        );
        assert_eq!(wanted(&state)[0], anchor, "the old picture ends nothing");
        let facts = reduced.grid_facts([((1, anchor), false)], std::iter::empty());
        assert_eq!((facts.installed, facts.newest_usable_due), (None, None));
        state.note_sent(0, (1, anchor), 0, true);
        let later = Want {
            received: 1,
            ..state.want.clone().unwrap()
        };
        state.set_want(1, later);
        assert_eq!(wanted(&state)[0], anchor);
    }
}

fn ready_flights() -> flow::Ledger<(BranchId, Hour)> {
    let mut ledger = flow::Ledger::new(8);
    let flights = [
        (1, (1, 120), false),
        (1, (1, 140), true),
        (1, (1, 90), false),
        (0, (1, 150), false),
        (1, (2, 150), true),
    ];
    for (ticket, (epoch, key, reused)) in (1..).zip(flights) {
        assert!(ledger.reserve(ticket, epoch, key, BTreeSet::new()));
        assert!(if reused {
            ledger.decoded(ticket, u8::MAX)
        } else {
            ledger.gap(ticket)
        });
    }
    assert!(ledger.reserve(6, 1, (1, 130), BTreeSet::new()));
    ledger
}

fn kept(ledger: &flow::Ledger<(BranchId, Hour)>) -> Vec<(BranchId, Hour)> {
    ledger
        .flights()
        .filter(|(_, flight)| !flight.retired)
        .map(|(_, flight)| flight.key)
        .collect()
}

#[test]
fn ready_results_nothing_accepted_survive_grid_retargeting_for_delivery() {
    let mut state = state();
    state.epoch = 1;
    let mut want = on_grid(want(170), 1, 40, None);
    let grid = want.grid.as_mut().unwrap();
    (grid.installed, grid.usable) = (Some(100), vec![160]);
    let mut ledger = ready_flights();
    let cancelled = coordinator::retarget_ledger(&mut ledger, &state, Some(&want), 1);
    assert_eq!(kept(&ledger), [(1, 120), (1, 140)]);
    assert_eq!(cancelled, [6, 5, 4, 3]);
    let mut ledger = ready_flights();
    let today = Want {
        grid: None,
        ..want.clone()
    };
    let cancelled = coordinator::retarget_ledger(&mut ledger, &state, Some(&today), 1);
    assert_eq!(kept(&ledger), [(1, 120), (1, 140), (1, 90), (1, 130)]);
    assert_eq!(cancelled, [5, 4]);
}

#[test]
fn a_new_fast_play_intent_starts_at_a_seek_the_grids_start_or_a_branch_or_stride_change() {
    assert_eq!(new_intent(None, (1, 40), false), Some("start"));
    assert_eq!(new_intent(Some((1, 40)), (1, 40), false), None);
    assert_eq!(new_intent(Some((1, 40)), (1, 40), true), Some("seek"));
    assert_eq!(new_intent(None, (1, 40), true), Some("seek"));
    assert_eq!(new_intent(Some((1, 40)), (2, 40), false), Some("branch"));
    assert_eq!(new_intent(Some((1, 20)), (1, 40), false), Some("stride"));
}

#[test]
fn retargeting_whole_flights_sees_each_ticket_and_cancels_as_retargeting_keys_does() {
    let mut ledger = flow::Ledger::new(4);
    assert!(ledger.reserve(1, 0, (1, 100), BTreeSet::new()));
    assert!(ledger.decoded(1, 0));
    for part in 0..8 {
        assert!(ledger.accepted(1, part));
    }
    assert!(ledger.reserve(2, 0, (1, 140), BTreeSet::new()));
    let mut seen = Vec::new();
    let cancelled = ledger.retarget_flights(|ticket, flight| {
        seen.push((ticket, flight.key, flight.accepted_any()));
        false
    });
    assert_eq!(seen, [(1, (1, 100), true), (2, (1, 140), false)]);
    assert_eq!(cancelled, [2]);
    let left: Vec<_> = ledger
        .flights()
        .map(|(ticket, flight)| (ticket, flight.retired))
        .collect();
    assert_eq!(left, [(1, true)]);
}
