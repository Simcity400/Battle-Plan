//! The Provinces rows carried between hours ([`engine_native::cprovince::carry`]):
//! the Provinces job reads only the provinces a native writer may have
//! changed, the provinces in a border conflict and the `fort_bunker` operand
//! of every fortified province whose state's modifiers a writer rebuilt.
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
//! It then drains the state ids whose modifier accumulator the raid family's
//! forwarded writers rebuilt, for the row carry's `fort_bunker` terms.
use crate::territory_changes::Changes;
use engine_native::{cprovince::Capture, cprovince::carry::Counts, walk::Outcome};
use std::{
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering},
};

/// Full, quiet and marked hours.
static HOURS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// Carried rows, read rows and refreshed `fort_bunker` operands.
static ROWS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// The Provinces job: one drain of the hour's province marks, the walk
/// carrying unmarked provinces, and the area step over the same marks.
pub fn walk(
    capture: &mut Capture,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
) -> Outcome {
    let take = take(&crate::territory_changes::Observer, capture);
    let outcome = capture.walk_carrying(memory, base, game_state, take);
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
    if let Some(store) = province_store.as_deref_mut() {
        let (marks, modifiers) = store.modifier_marks_mut();
        marks.clear();
        let refused = Cell::new(false);
        let mut marks = Some(marks);
        let epoch = changes.take_modifier_states(&mut |id| match u32::try_from(id) {
            Ok(id) => push(&mut marks, id, &refused),
            Err(_) => refused.set(true),
        });
        *modifiers = epoch.filter(|_| !refused.get());
    }
    if overflow.get() { None } else { take }
}

fn push(marks: &mut Option<&mut Vec<u32>>, id: u32, overflow: &Cell<bool>) {
    let Some(marks) = marks else {
        return;
    };
    if overflow.get() {
        return;
    }
    if engine_allocation::advisory(|| engine_allocation::reserve(marks, 1)).is_err() {
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
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::territory_changes::Domain;
    use engine_native::{controller_areas, cprovince::carry::RowStore};

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
