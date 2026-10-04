use super::*;

/// A lane state showing `hour` of branch 1 with the frames of `hours`
/// (recorded gaps: whether a frame exists is all a plan reads).
fn state(hour: Hour, playing: bool, hours: std::ops::RangeInclusive<Hour>) -> State {
    State {
        branch: Some(1),
        shown: Some(hour),
        playing,
        epoch: 1,
        frames: hours.map(|hour| ((1, hour), None)).collect(),
        ..State::default()
    }
}

/// Builds `plan` as the lane would, with an empty cycle standing for
/// its result, `complete` or not.
fn built(chain: &mut Chain, plan: &Plan, complete: bool) {
    let cycle = if complete {
        Cycle::new(1, 0)
    } else {
        Cycle::pending(1, 0)
    };
    chain.after.insert(plan.hour, cycle);
}

/// Playing, each hour is built from the one before as its frame
/// arrives, ahead of the shown hour.
#[test]
fn playing_builds_each_hour_from_the_one_before() {
    let mut chain = Chain::default();
    let mut state = state(10, true, 10..=11);
    let first = chain.plan(&mut state).unwrap();
    assert_eq!(
        (first.hour, first.from.is_none(), first.paused),
        (10, true, false)
    );
    built(&mut chain, &first, false);
    let next = chain.plan(&mut state).unwrap();
    assert_eq!((next.hour, next.from.is_some()), (11, true));
    built(&mut chain, &next, false);
    assert!(chain.plan(&mut state).is_none(), "hour 12 has no frame yet");
    state.frames.insert((1, 12), None);
    assert_eq!(chain.plan(&mut state).unwrap().hour, 12);
}

/// A reset (a seek, a step back, a branch change), or a shown hour
/// before the chain's first, starts the cycle again at the shown hour.
#[test]
fn a_reset_starts_the_cycle_again_at_the_shown_hour() {
    let mut chain = Chain::default();
    let mut state = state(10, true, 8..=13);
    for _ in 0..3 {
        let plan = chain.plan(&mut state).unwrap();
        built(&mut chain, &plan, false);
    }
    state.epoch += 1;
    state.shown = Some(12);
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!((plan.hour, plan.from.is_none()), (12, true));
    built(&mut chain, &plan, false);
    state.shown = Some(11);
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!(
        (plan.hour, plan.from.is_none()),
        (11, true),
        "before the chain"
    );
}

/// Paused, the shown hour is built again from the hour before and
/// stepped per frame until complete; later hours leave the chain.
#[test]
fn paused_the_shown_hour_is_cycled_until_complete() {
    let mut chain = Chain::default();
    let mut state = state(10, true, 10..=12);
    for _ in 0..3 {
        let plan = chain.plan(&mut state).unwrap();
        built(&mut chain, &plan, false);
    }
    state.shown = Some(11);
    state.playing = false;
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!(
        (plan.hour, plan.from.is_some(), plan.paused),
        (11, true, true)
    );
    assert!(!chain.after.contains_key(&12));
    built(&mut chain, &plan, true);
    assert!(chain.plan(&mut state).is_none(), "complete: nothing to do");
    // Played on, the next hour builds from the complete one.
    state.playing = true;
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!((plan.hour, plan.paused), (12, false));
}

/// A capture resets the lane at its hour, paused. Until the hour's frame
/// is fed the lane has nothing to build, and the capture would wait for
/// ever; once it is, the lane builds the hour from row 0 and cycles it
/// until complete, and then has nothing more to do: the capture saves.
#[test]
fn a_cold_capture_cycles_its_hour_once_its_frame_is_fed() {
    let mut chain = Chain::default();
    let mut state = state(10, false, 10..=10);
    state.frames.clear();
    assert!(
        chain.plan(&mut state).is_none(),
        "no frame: nothing to build"
    );
    state.frames.insert((1, 10), None);
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!(
        (plan.hour, plan.from.is_none(), plan.paused),
        (10, true, true)
    );
    built(&mut chain, &plan, true);
    assert!(chain.plan(&mut state).is_none(), "complete");
}

/// Playing, the builders pass hours over to keep up with the clock (a
/// playback skip, not a seek). Once the window shows a later hour, the
/// chain builds the next hour that arrived from the cycle of the last
/// hour it built, in the same reset, instead of waiting for the hours
/// passed over; paused, it does so up to the shown hour.
#[test]
fn a_playback_skip_carries_the_cycle_without_stalling_or_resetting() {
    let mut chain = Chain::default();
    let mut state = state(10, true, 10..=11);
    for _ in 0..2 {
        let plan = chain.plan(&mut state).unwrap();
        built(&mut chain, &plan, false);
    }
    // The cycle after hour 11, told apart by its country.
    chain.after.insert(11, Cycle::pending(1, 7));
    // 12 and 13 are passed over. While the window shows an hour before
    // them they may still come, and the chain waits for 12.
    state.frames.insert((1, 14), None);
    assert!(chain.plan(&mut state).is_none());
    state.shown = Some(14);
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!((plan.hour, plan.epoch), (14, 1), "no reset");
    assert_eq!(plan.from.as_ref().map(Cycle::ours), Some(7), "carried");
    built(&mut chain, &plan, false);
    // An hour passed over that arrives late is not built.
    state.frames.insert((1, 12), None);
    state.frames.insert((1, 15), None);
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!(plan.hour, 15);
    assert!(!state.frames.contains_key(&(1, 12)));
    built(&mut chain, &plan, false);
    // Paused two hours on, with 16 passed over: the shown hour builds
    // from 15 and cycles until complete.
    state.frames.insert((1, 17), None);
    state.shown = Some(17);
    state.playing = false;
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!(
        (plan.hour, plan.paused, plan.from.is_some()),
        (17, true, true)
    );
    built(&mut chain, &plan, true);
    assert!(chain.plan(&mut state).is_none(), "complete");
}

/// A lagging lane keeps the frames it still needs, and drops the rest.
#[test]
fn a_lagging_lane_keeps_the_frames_it_needs() {
    let mut chain = Chain::default();
    let mut state = state(10, true, 9..=20);
    let plan = chain.plan(&mut state).unwrap();
    built(&mut chain, &plan, false);
    state.shown = Some(18);
    let plan = chain.plan(&mut state).unwrap();
    assert_eq!(plan.hour, 11);
    assert!(state.frames.contains_key(&(1, 11)));
    assert!(!state.frames.contains_key(&(1, 9)));
}

fn from(tag: &str) -> perspective::Choice {
    perspective::Choice {
        country: perspective::Country::Tag(tag.into()),
        fog: true,
    }
}

/// Icons of hour 10 on branch 1 for canonical country `ours`.
fn icons_for(ours: u32) -> RaidIcons {
    RaidIcons {
        built_for: Some((1, 10, ours)),
        ..RaidIcons::default()
    }
}

/// After a switch of viewing country, nothing built for the previous
/// one installs, even for one frame: a batch still being built for it is
/// not kept, and a result kept from before is refused for the new
/// resolved country. When the window puts the new country's hour on
/// screen its presentation history resets, and the cycle with it.
#[test]
fn a_country_switch_leaves_no_icon_of_the_previous_country() {
    let (ger, sov) = (from("GER"), from("SOV"));
    let mut state = State::default();
    state.view(true, &ger, (1, 10), false);
    let building = state.stamp().unwrap();
    assert!(state.publish(building.clone(), 10, icons_for(3)));
    assert!(state.icons(1, 10, 3).is_some(), "the view's own icons");
    assert!(state.icons(1, 10, 4).is_none(), "another country's");
    state.view(false, &sov, (1, 10), false);
    // The tables for the Soviet view refuse Germany's icons.
    assert!(state.icons(1, 10, 4).is_none());
    assert!(
        !state.publish(building, 10, icons_for(3)),
        "a German batch finishing after the switch"
    );
    // The Soviet cycle's own results install.
    assert!(state.publish(state.stamp().unwrap(), 10, icons_for(4)));
    assert!(state.icons(1, 10, 4).is_some());
    // A fog change alone keeps the cycle: offers do not read the fog.
    let generation = state.epoch;
    let unfogged = perspective::Choice { fog: false, ..sov };
    state.view(false, &unfogged, (1, 10), false);
    assert_eq!(state.epoch, generation);
    assert!(state.icons(1, 10, 4).is_some());
    // The window's reset for the new resolved country drops them all.
    state.restart(1);
    assert!(state.icons(1, 10, 4).is_none());
    // The installer refuses icons of another country or hour.
    let installed = icons_for(4);
    assert!(installed.drawable_at(1, 10, 4));
    assert!(!installed.drawable_at(1, 10, 3), "the previous country's");
    assert!(!installed.drawable_at(1, 11, 4), "another hour's");
    assert!(!RaidIcons::default().drawable_at(1, 10, 4));
}

/// A country switch while an hour is in flight. The new view and its
/// reset arrive under one lock, so every plan after the reset is the new
/// view's. And in the interleaving the review reproduced (the lane woke
/// on a reset before the new view arrived, and planned the new
/// generation for the old country), the old country's result, complete,
/// is refused at publication and dropped from the chain too: the new
/// view's paused hour is planned again and cycles until complete.
#[test]
fn a_country_switch_with_a_job_in_flight_leaves_the_new_view_to_converge() {
    let (ger, sov) = (from("GER"), from("SOV"));
    let mut chain = Chain::default();
    let mut state = State::default();
    state.present(true, &ger, (1, 10), false, None, true);
    let german = chain.plan(&mut state).unwrap();
    let german = Stamp {
        generation: german.epoch,
        country: ger.country.clone(),
    };
    // The switch: view and reset at once.
    state.present(false, &sov, (1, 10), false, None, true);
    assert_eq!(
        state.stamp().map(|stamp| stamp.country),
        Some(sov.country.clone()),
        "a plan after the reset is the new view's"
    );
    // The German hour finishes, complete, after the switch.
    settle(&mut chain, &state, &german, 10, Cycle::new(1, 3));
    assert!(!state.publish(german, 10, icons_for(3)));
    // The reviewed interleaving: a reset alone, a plan, then the view.
    state.restart(1);
    let planned = chain.plan(&mut state).unwrap();
    let stale = Stamp {
        generation: planned.epoch,
        country: sov.country.clone(),
    };
    state.view(false, &ger, (1, 10), false);
    settle(&mut chain, &state, &stale, 10, Cycle::new(1, 4));
    assert!(!state.publish(stale, 10, icons_for(4)));
    // The German view's paused hour is still to build, and converges.
    let plan = chain.plan(&mut state).expect("not marked complete");
    assert_eq!(
        (plan.hour, plan.from.is_none(), plan.paused),
        (10, true, true)
    );
    let stamp = state.stamp().unwrap();
    settle(&mut chain, &state, &stamp, 10, Cycle::new(1, 3));
    assert!(state.publish(stamp, 10, icons_for(3)));
    assert!(chain.plan(&mut state).is_none(), "complete");
    assert!(state.icons(1, 10, 3).is_some());
}

/// Planned ahead, the lane prunes a frame the window still holds built;
/// a seek back to that hour hands the lane the frame with the reset, and
/// the paused hour cycles until complete. An hour in flight from before
/// the seek is dropped.
#[test]
fn a_seek_back_to_a_cached_built_hour_brings_its_frame() {
    let recorded = perspective::Choice::default();
    let mut chain = Chain::default();
    let mut state = state(8, true, 8..=10);
    state.perspective = Some(recorded.clone());
    for _ in 0..3 {
        let plan = chain.plan(&mut state).unwrap();
        built(&mut chain, &plan, false);
    }
    state.shown = Some(10);
    state.frames.insert((1, 11), None);
    let ahead = chain.plan(&mut state).unwrap();
    assert_eq!(ahead.hour, 11);
    assert!(!state.frames.contains_key(&(1, 8)), "pruned");
    let ahead = Stamp {
        generation: ahead.epoch,
        country: recorded.country.clone(),
    };
    // The window seeks back to 8 from its cache of built hours.
    state.present(false, &recorded, (1, 8), false, None, true);
    assert!(state.frames.contains_key(&(1, 8)));
    settle(&mut chain, &state, &ahead, 11, Cycle::pending(1, 0));
    assert!(!chain.after.contains_key(&11));
    let plan = chain.plan(&mut state).expect("the frame is there");
    assert_eq!(
        (plan.hour, plan.from.is_none(), plan.paused),
        (8, true, true)
    );
    built(&mut chain, &plan, true);
    assert!(chain.plan(&mut state).is_none(), "complete");
}

/// A result stamped with an earlier generation (a reset since) is
/// refused when published, and one kept from it is not installed.
#[test]
fn a_stale_generation_result_is_refused() {
    let ger = from("GER");
    let mut state = State::default();
    state.view(true, &ger, (1, 10), false);
    let stale = state.stamp().unwrap();
    assert!(state.publish(stale.clone(), 10, icons_for(3)));
    state.restart(1);
    assert!(!state.publish(stale.clone(), 10, icons_for(3)));
    assert!(state.icons(1, 10, 3).is_none());
    // Even a stale result that were still held is not installed.
    state.built.insert(
        10,
        Published {
            stamp: stale,
            icons: Arc::new(icons_for(3)),
        },
    );
    assert!(state.icons(1, 10, 3).is_none());
}

/// A new viewing country or branch starts the cycle again.
#[test]
fn a_new_viewing_country_starts_the_cycle_again() {
    assert_eq!(carried(Some(Cycle::pending(1, 3)), 1, 3).ours(), 3);
    assert!(!carried(Some(Cycle::pending(1, 3)), 1, 3).complete());
    assert!(carried(Some(Cycle::pending(1, 3)), 1, 4).complete());
    assert!(carried(Some(Cycle::pending(1, 3)), 2, 3).complete());
    assert!(carried(None, 1, 3).complete());
}
