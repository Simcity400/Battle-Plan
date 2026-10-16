//! The Provinces rows carried between hours ([`engine::cprovince::carry`]):
//! the Provinces job reads only the provinces a native writer may have
//! changed, the provinces in a border conflict, the `fort_bunker` operand of
//! every fortified province and its sampled slice.
//!
//! The job first drains the territory family's province marks
//! ([`crate::territory_changes`]) once for the hour: the marked province ids
//! (controller and victory-point writers, the province reader and building
//! writes forwarded by location) and the state ids whose provinces a writer
//! reached (owner setter, state reset and reader, border-conflict insertion
//! and removal). Province initialization, the owner-area rebuild and a
//! building write naming no status move the epoch. The
//! row carry expands a state mark through the provinces its held rows link
//! to the state, the membership before a reset emptied the state's list,
//! and the [area family](crate::area_changes) takes the same province marks.
//!
//! Each hour reads the carryable provinces of its slice
//! ([`crate::sampled_verify::current`], by province id) afresh and compares
//! them with the held rows; the first difference poisons the carry and every
//! later hour reads every province. `province_verify`, on by default during
//! the family's burn-in, reads every province every hour and compares each
//! carryable one.
use crate::{sampled_verify::Sampled, territory_changes::Changes};
use engine::{
    cprovince::{Capture, carry::Counts},
    walk::Outcome,
};
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Every province is read and every carryable one compared every hour
/// (`province_verify`).
pub const VERIFY: bool = cfg!(feature = "province_verify");

static POISONED: AtomicBool = AtomicBool::new(false);
/// Full, quiet and marked hours.
static HOURS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// Carried rows, read rows and refreshed `fort_bunker` operands.
static ROWS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static SAMPLED: Sampled = Sampled::burned_in(&["carried Provinces rows differ from a fresh read"]);

/// The Provinces job: one drain of the hour's province marks, the walk
/// carrying unmarked provinces, and the area step over the same marks.
pub fn walk(
    capture: &mut Capture,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) -> Outcome {
    let take = take(&crate::territory_changes::Observer, capture);
    if POISONED.load(Ordering::Relaxed)
        && let Some(store) = capture.province_store.as_mut()
    {
        store.poison();
    }
    let check = crate::sampled_verify::current_burned_in(VERIFY);
    let outcome = capture.walk_carrying(
        memory,
        base,
        game_state,
        take,
        &|id| check.covers(id),
        VERIFY,
    );
    if let Outcome::Rows { len } = outcome {
        if let Some(store) = capture.province_store.as_ref() {
            record(store.counts());
        }
        if (capture.province_store.is_some() || capture.area_store.is_some())
            && let Some(last) = capture.rows[..len].last()
        {
            crate::territory_changes::walked_provinces(last.id as usize + 1);
        }
    }
    crate::area_changes::capture(capture, outcome, memory, base, take)
}

/// Drains the hour's province marks once into the stores that consume
/// them: the province ids into both, the state ids into the row carry.
/// `None` when the take refused or a mark could not be kept; then neither
/// store carries anything. Without either store nothing is taken.
pub fn take(changes: &dyn Changes, capture: &mut Capture) -> Option<[u64; 2]> {
    let Capture {
        province_store,
        area_store,
        ..
    } = capture;
    if province_store.is_none() && area_store.is_none() {
        return None;
    }
    let (mut provinces, mut states) = match province_store.as_deref_mut() {
        Some(store) => {
            let (provinces, states) = store.marks_mut();
            (Some(provinces), Some(states))
        }
        None => (None, None),
    };
    let mut areas = area_store.as_deref_mut().map(|store| store.marks());
    for marks in [&mut provinces, &mut states, &mut areas]
        .into_iter()
        .flatten()
    {
        marks.clear();
    }
    let overflow = Cell::new(false);
    let take = changes.take_provinces(
        &mut |id| match u32::try_from(id) {
            Ok(id) => {
                push(&mut provinces, id, &overflow);
                push(&mut areas, id, &overflow);
            }
            Err(_) => overflow.set(true),
        },
        &mut |id| match u32::try_from(id) {
            Ok(id) => push(&mut states, id, &overflow),
            Err(_) => overflow.set(true),
        },
    );
    if overflow.get() { None } else { take }
}

fn push(marks: &mut Option<&mut Vec<u32>>, id: u32, overflow: &Cell<bool>) {
    let Some(marks) = marks else {
        return;
    };
    if overflow.get() {
        return;
    }
    if engine::allocation::advisory(|| engine::allocation::reserve(marks, 1)).is_err() {
        overflow.set(true);
        return;
    }
    marks.push(id);
}

fn record(counts: Counts) {
    let hour = if counts.full {
        0
    } else if counts.quiet {
        1
    } else {
        2
    };
    HOURS[hour].fetch_add(1, Ordering::Relaxed);
    for (total, value) in ROWS
        .iter()
        .zip([counts.carried, counts.read, counts.refreshed])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
    if !counts.full && SAMPLED.record(counts.verified, counts.mismatches, 0) {
        POISONED.store(true, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let hours = HOURS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let rows = ROWS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "province_changes"),
        "verify": VERIFY,
        "poisoned": POISONED.load(Ordering::Relaxed),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "full_hours": hours[0],
        "quiet_hours": hours[1],
        "marked_hours": hours[2],
        "carried_rows": rows[0],
        "read_rows": rows[1],
        "refreshed_fort_bunkers": rows[2],
        "scope": "cumulative since attach; the Provinces job carries a province's row when the hour's take of the territory family's province marks returned the epochs of the held complete walk, the province table header and the province's slot pointer are the held ones, neither the province nor its held state is marked and its held row is in no border conflict; carried rows read only fort_bunker again (positive fort sum and controller), every other province is read as a full walk reads it; every hour also reads its slice of carryable provinces (by id) afresh and compares it with the held rows (one in rotation_hours; every one in verify mode) and the first difference makes every later hour read every province (sampled_*)"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::territory_changes::Domain;
    use engine::{controller_areas, cprovince::carry::RowStore};

    fn stores(provinces: bool, areas: bool) -> Capture {
        let mut capture = Capture::try_with_capacity(2, 0, 0).unwrap();
        capture.province_store = provinces.then(|| Box::new(RowStore::new()));
        capture.area_store = areas.then(|| Box::new(controller_areas::Store::new()));
        capture
    }

    /// One drain feeds both consumers: the row carry receives the province
    /// and state marks, the area family the same province marks, and a
    /// second take finds them consumed.
    #[test]
    fn one_take_feeds_the_row_carry_and_the_areas() {
        use crate::territory_changes::Target;
        let domain = Domain::enabled(64, 1024);
        let mut capture = stores(true, true);
        let epochs = take(&domain, &mut capture).unwrap();
        drop(domain.notify(0, Target::Province(Some(700))));
        drop(domain.notify(
            7,
            Target::State {
                id: Some(9),
                state: true,
                provinces: true,
            },
        ));
        assert_eq!(take(&domain, &mut capture), Some(epochs));
        let (provinces, states) = capture.province_store.as_mut().unwrap().marks_mut();
        assert_eq!((provinces.clone(), states.clone()), (vec![700], vec![9]));
        assert_eq!(capture.area_store.as_mut().unwrap().marks().clone(), [700]);
        assert_eq!(take(&domain, &mut capture), Some(epochs));
        assert!(capture.area_store.as_mut().unwrap().marks().is_empty());
        // The area family alone still takes its marks.
        let mut areas = stores(false, true);
        drop(domain.notify(0, Target::Province(Some(5))));
        assert_eq!(take(&domain, &mut areas), Some(epochs));
        assert_eq!(areas.area_store.as_mut().unwrap().marks().clone(), [5]);
        // Without a consumer the marks stay for the next take.
        drop(domain.notify(0, Target::Province(Some(6))));
        assert_eq!(take(&domain, &mut stores(false, false)), None);
        assert_eq!(take(&domain, &mut capture), Some(epochs));
        assert_eq!(capture.area_store.as_mut().unwrap().marks().clone(), [6]);
    }
}
