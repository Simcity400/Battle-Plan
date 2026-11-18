use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct AfterUnlock {
    shared: Shared,
    observed: Arc<AtomicBool>,
    action: Option<Box<dyn FnOnce(&mut State) + Send>>,
}
impl Drop for AfterUnlock {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .0
            .try_lock()
            .expect("retirement must not hold presentation's raid mutex");
        if let Some(action) = self.action.take() {
            action(&mut state);
        }
        self.observed.store(true, Ordering::Relaxed);
        self.shared.1.notify_all();
    }
}

fn chain() -> Chain {
    Chain {
        epoch: 1,
        start: Some(10),
        after: BTreeMap::from([(10, Cycle::pending(1, 6)), (11, Cycle::pending(1, 7))]),
        ..Default::default()
    }
}

#[test]
fn expired_raid_values_drop_after_unlock_and_before_the_same_snapshot() {
    let mut state = state(14, true, 9..=13);
    state.frames.insert((0, 20), None);
    state.frames.insert((2, 10), None);
    let shared = Arc::new((Mutex::new(state), Condvar::new()));
    let observed = Arc::new(AtomicBool::new(false));
    let mut chain = chain();
    let plan = wait_for_plan(
        &shared,
        &mut chain,
        |chain, state, retired| {
            let planned = chain.prepare(state, retired);
            retired.push(AfterUnlock {
                shared: shared.clone(),
                observed: observed.clone(),
                action: None,
            });
            planned
        },
        |chain, planned| {
            assert!(
                observed.load(Ordering::Relaxed),
                "old credit frees before snapshot allocation"
            );
            assert!(
                shared.0.try_lock().is_ok(),
                "snapshot copy must also be outside the mutex"
            );
            chain.snapshot(planned)
        },
    )
    .unwrap();
    assert_eq!((plan.epoch, plan.hour, plan.paused), (1, 12, false));
    assert_eq!(plan.from.as_ref().map(Cycle::ours), Some(7));
    assert_eq!(chain.after.keys().copied().collect::<Vec<_>>(), [10, 11]);
    assert_eq!(
        chain.after[&11].ours(),
        7,
        "the native paused predecessor is retained"
    );
    assert_eq!(
        lock(&shared).frames.keys().copied().collect::<Vec<_>>(),
        [(1, 11), (1, 12), (1, 13)]
    );
}

#[test]
fn a_retirement_without_a_ready_frame_frees_before_the_lane_sleeps() {
    let shared = Arc::new((Mutex::new(state(14, true, 9..=9)), Condvar::new()));
    let observed = Arc::new(AtomicBool::new(false));
    let mut chain = chain();
    let mut calls = 0;
    let plan = wait_for_plan(
        &shared,
        &mut chain,
        |chain, state, retired| {
            calls += 1;
            let planned = chain.prepare(state, retired);
            if calls == 1 {
                assert!(planned.is_none());
                retired.push(AfterUnlock {
                    shared: shared.clone(),
                    observed: observed.clone(),
                    action: Some(Box::new(|state| {
                        state.frames.insert((1, 12), None);
                    })),
                });
            }
            planned
        },
        |chain, planned| chain.snapshot(planned),
    )
    .unwrap();
    assert_eq!(calls, 2);
    assert!(observed.load(Ordering::Relaxed));
    assert_eq!(plan.hour, 12);
    assert_eq!(plan.from.as_ref().map(Cycle::ours), Some(7));
    assert_eq!(
        lock(&shared).frames.keys().copied().collect::<Vec<_>>(),
        [(1, 12)]
    );
}

#[test]
fn a_view_reset_during_unlocked_retirement_still_refuses_the_old_result() {
    let ger = from("GER");
    let mut state = state(14, true, 9..=13);
    state.perspective = Some(ger);
    let shared = Arc::new((Mutex::new(state), Condvar::new()));
    let observed = Arc::new(AtomicBool::new(false));
    let mut chain = chain();
    let (stamp, plan) = wait_for_plan(
        &shared,
        &mut chain,
        |chain, state, retired| {
            let stamp = state.stamp().unwrap();
            let plan = chain.prepare(state, retired)?;
            retired.push(AfterUnlock {
                shared: shared.clone(),
                observed: observed.clone(),
                action: Some(Box::new(|state| {
                    state.present(false, &from("SOV"), (1, 20), false, None, true);
                })),
            });
            Some((stamp, plan))
        },
        |chain, (stamp, planned)| (stamp, chain.snapshot(planned)),
    )
    .unwrap();
    assert!(observed.load(Ordering::Relaxed));
    assert_eq!((plan.epoch, plan.hour), (1, 12));
    assert_eq!(plan.from.as_ref().map(Cycle::ours), Some(7));
    let mut state = lock(&shared);
    assert_eq!((state.epoch, state.shown), (2, Some(20)));
    settle(&mut chain, &state, &stamp, plan.hour, Cycle::new(1, 3));
    assert!(!chain.after.contains_key(&12));
    assert!(!state.publish(stamp, plan.hour, icons_for(3)));
}
