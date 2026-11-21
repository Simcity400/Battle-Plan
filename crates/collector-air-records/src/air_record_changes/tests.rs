use super::*;
use std::sync::Arc;

fn record(wing_id: u32, date: i32) -> AirCombatRecord {
    AirCombatRecord {
        wing_id,
        date,
        planes: 1,
        ..Default::default()
    }
}

fn touched_with(holders: usize) -> Touched {
    let touched = Touched::new();
    unsafe { touched.replace(Some(Set::new(holders).unwrap())) };
    touched.observing.store(true, Ordering::SeqCst);
    touched
}

fn free(touched: &Touched) {
    unsafe { touched.replace(None) };
    assert_eq!(touched.capacity(), 0);
}

#[test]
fn a_take_names_each_touched_holder_once() {
    let touched = touched_with(0);
    let mut offered = None;
    let mut out = Vec::new();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(false));
    for holder in [0x1000, 0x2000, 0x1000, 0x3000] {
        touched.notify(holder);
    }
    out.clear();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(true));
    out.sort_unstable();
    assert_eq!(out, [0x1000, 0x2000, 0x3000]);
    out.clear();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(true));
    assert!(out.is_empty());
    assert_eq!(touched.calls.load(Ordering::Relaxed), 4);
    free(&touched);
}

#[test]
fn a_never_installed_observer_certifies_no_interval() {
    let touched = Touched::new();
    unsafe { touched.replace(Some(Set::new(0).unwrap())) };
    let mut offered = None;
    let mut out = Vec::new();
    for _ in 0..3 {
        out.clear();
        assert_eq!(touched.take(&mut out, &mut offered), Ok(false));
        assert!(out.is_empty());
    }
    free(&touched);
}

#[test]
fn an_unrecorded_write_loses_the_interval() {
    let touched = touched_with(0);
    let mut offered = None;
    let mut out = Vec::new();
    touched.take(&mut out, &mut offered).unwrap();
    touched.notify(0x1000);
    touched.active.fetch_add(1, Ordering::SeqCst);
    out.clear();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(false));
    assert!(out.is_empty(), "a running write leaves the set alone");
    touched.active.fetch_sub(1, Ordering::SeqCst);
    assert_eq!(touched.take(&mut out, &mut offered), Ok(false));
    assert_eq!(out, [0x1000]);
    out.clear();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(true));
    touched.closed.store(true, Ordering::SeqCst);
    touched.notify(0x2000);
    touched.closed.store(false, Ordering::SeqCst);
    assert_eq!(touched.take(&mut out, &mut offered), Ok(false));
    let slots = touched.capacity();
    for holder in 1..=slots {
        touched.notify(holder * 16);
    }
    assert_eq!(touched.lost_calls.load(Ordering::Relaxed), 1);
    touched.notify(usize::MAX - 15);
    out.clear();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(false));
    assert_eq!(out.len(), slots);
    free(&touched);
}

#[test]
fn an_offered_set_installs_after_the_drain() {
    let touched = touched_with(0);
    let mut out = Vec::new();
    let mut offered = None;
    touched.take(&mut out, &mut offered).unwrap();
    touched.notify(0x1000);
    let mut offered = Set::new(5_000);
    let larger = offered.as_ref().unwrap().slots.len();
    assert!(larger >= 10_000 && larger > touched.capacity());
    out.clear();
    assert_eq!(touched.take(&mut out, &mut offered), Ok(true));
    assert_eq!(out, [0x1000]);
    assert!(offered.is_none());
    assert_eq!(touched.capacity(), larger);
    free(&touched);
}

#[test]
fn a_touched_vector_splices_the_tail_after_the_pruned_held_records() {
    let since = 100;
    let mut held = Arc::new(vec![
        record(1, 70),
        record(2, 80),
        record(3, 100),
        record(4, 100),
    ]);
    let tail = vec![record(3, 100), record(4, 100), record(5, 101)];
    assert_eq!(
        splice(&mut held, since, 1, tail.clone().into_iter()),
        Some(true)
    );
    assert_eq!(
        held.iter().map(|r| r.wing_id).collect::<Vec<_>>(),
        [2, 3, 4, 5]
    );
    let mut held = Arc::new(vec![record(1, 120), record(2, 80), record(3, 100)]);
    let shared = Arc::clone(&held);
    assert_eq!(
        splice(&mut held, since, 2, [record(3, 100)].into_iter()),
        Some(false)
    );
    assert!(Arc::ptr_eq(&held, &shared));
    assert_eq!(
        held.iter().map(|r| r.wing_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let mut held = Arc::new(vec![record(1, 70), record(3, 100)]);
    assert_eq!(splice(&mut held, since, 2, tail.into_iter()), None);
}

#[test]
fn touched_tail_reuses_equal_damage_and_matches_replacement_changes() {
    let mut original = vec![record(1, 70), record(2, 80), record(3, 100), record(4, 100)];
    original[2].first_damage = vec![AirCombatDamage {
        cause: 2,
        amount: 3,
    }];
    for first in 0..=3 {
        for tail in [
            vec![],
            vec![original[2].clone()],
            original[2..].to_vec(),
            vec![record(9, 101)],
        ] {
            let mut held = Arc::new(original.clone());
            let expected = 2usize.checked_sub(first).map(|pruned| {
                let mut expected = original[pruned..].to_vec();
                expected.truncate(first);
                expected.extend(tail.iter().cloned());
                expected
            });
            let result = splice(&mut held, 100, first, tail.into_iter());
            assert_eq!(
                result,
                expected.as_ref().map(|expected| *expected != original)
            );
            assert_eq!(*held, expected.unwrap_or_else(|| original.clone()));
        }
    }
    let mut held = Arc::new(original.clone());
    let damage = held[2].first_damage.as_ptr();
    assert_eq!(
        splice(&mut held, 100, 2, original[2..].iter().cloned()),
        Some(false)
    );
    assert_eq!(held[2].first_damage.as_ptr(), damage);
}

#[test]
fn suffix_start_finds_the_records_the_updater_can_reach() {
    let records = [record(1, 70), record(2, 120), record(3, 80), record(4, 100)];
    assert_eq!(suffix_start(&records, 100), 3);
    assert_eq!(suffix_start(&records, 50), 0);
    assert_eq!(suffix_start(&records, 200), 4);
    assert_eq!(suffix_start(&[], 100), 0);
}
