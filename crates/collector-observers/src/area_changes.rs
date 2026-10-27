//! The controller and owner areas of the Provinces rows
//! ([`engine_native::controller_areas`]), held between hours and read again only
//! where a native writer may have changed them.
//!
//! The Provinces job gathers every province's two area pointers with its
//! movement fields; after a complete walk it runs [`capture`] over the
//! territory family's province marks ([`crate::territory_changes`]), which
//! the job drained once before its walk ([`crate::province_changes::take`]):
//! the province controller writer,
//! the only path to a controller area change, marks its
//! province; the owner-area rebuild marks every province and
//! moves the epoch. A state's owner, reset or reader writes no province's
//! areas. An hour with no mark publishes the held areas; a marked hour
//! compares every province's pointers with the held ones and reads again the
//! areas of the marked and re-pointed provinces, the areas they left and
//! their neighbours' areas. A refused or moved take, a load, gap or
//! recovery, a requested refresh or a failed walk reads every area.
//!
//! A row the job carried ([`crate::province_changes`]) arrives with its held
//! pointers: an hour that compares every province's pointers first reads
//! every carried row's pointers again, a bounded marked hour those it
//! compares, and a quiet carry none, so the areas see what a full walk would
//! gather.
use engine_native::{cprovince::Capture, walk::Outcome};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

/// Full, quiet and marked hours.
static HOURS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// Re-pointed provinces, read areas and carried areas.
static WORK: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);

/// The Provinces job, after its walk: labels the walked rows' areas and
/// writes the hour's controller-area nodes, under the hour's `take` of the
/// province marks. Without the family's store the rows keep no labels and
/// the fields are refused. Returns the Provinces outcome, which refuses as a
/// full walk would when a carried row's pointers no longer read.
pub fn capture(
    capture: &mut Capture,
    outcome: Outcome,
    memory: &impl engine_native::Memory,
    base: usize,
    take: Option<[u64; 2]>,
) -> Outcome {
    capture.area_nodes.clear();
    let Some(store) = capture.area_store.as_mut() else {
        return outcome;
    };
    let Outcome::Rows { len } = outcome else {
        // Nothing is held; the take's marks no longer matter.
        store.forget();
        refuse("the province walk did not complete");
        return outcome;
    };
    let quiet = store.carries_quietly(take, len);
    // A marked hour compares only the provinces a controller change can
    // re-point ([`engine_native::controller_areas::Store::repointable`]);
    // anything it cannot bound compares every province.
    let mut scope = std::mem::take(&mut store.scope);
    let bounded = !quiet
        && take.is_some()
        && matches!(
            store.repointable(memory, &capture.rows[..len], &mut scope),
            Ok(true)
        );
    let refreshed = capture.refresh_carried_areas(memory, &|id| {
        if quiet {
            false
        } else if bounded {
            scope.binary_search(&id).is_ok()
        } else {
            true
        }
    });
    if let Some(store) = capture.area_store.as_mut() {
        store.scope = scope;
    }
    if let Err(reason) = refreshed {
        return fail(capture, reason);
    }
    let mut result = run(capture, memory, base, len, take);
    // A quiet or bounded carry that fell back to a full read (a refused
    // area) read some carried rows' held pointers: read them all again and
    // capture afresh.
    let fell_back = match result {
        Ok(counts) => counts.full || (quiet && !counts.quiet),
        Err(_) => true,
    };
    if (quiet || bounded) && fell_back && capture.carried_rows() != 0 {
        if let Err(reason) = capture.refresh_carried_areas(memory, &|_| true) {
            return fail(capture, reason);
        }
        result = run(capture, memory, base, len, take);
    }
    match result {
        Ok(counts) => record(counts),
        Err(reason) => refuse(reason),
    }
    outcome
}

fn run(
    capture: &mut Capture,
    memory: &impl engine_native::Memory,
    base: usize,
    len: usize,
    take: Option<[u64; 2]>,
) -> Result<engine_native::controller_areas::Counts, &'static str> {
    let Capture {
        rows,
        areas,
        area_nodes,
        area_store,
        ..
    } = capture;
    let Some(store) = area_store.as_mut() else {
        return Err("controller area store absent");
    };
    store.capture(memory, base, &mut rows[..len], areas, area_nodes, take)
}

/// A carried row's pointers did not read again: the Provinces table
/// refuses as a full walk would, and neither store holds the walk.
fn fail(capture: &mut Capture, reason: &'static str) -> Outcome {
    if let Some(store) = capture.area_store.as_mut() {
        store.forget();
    }
    if let Some(store) = capture.province_store.as_mut() {
        store.forget();
    }
    capture.conflict_walk = engine_native::cprovince::ConflictWalk {
        outcome: Outcome::Unavailable { reason },
        groups_len: 0,
        required_groups: capture.conflict_walk.required_groups,
    };
    refuse(reason);
    Outcome::Unavailable { reason }
}

fn record(counts: engine_native::controller_areas::Counts) {
    let hour = if counts.full {
        0
    } else if counts.quiet {
        1
    } else {
        2
    };
    HOURS[hour].fetch_add(1, Ordering::Relaxed);
    for (total, value) in WORK
        .iter()
        .zip([counts.changed, counts.read, counts.carried])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

fn refuse(reason: &'static str) {
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    *LAST_REFUSAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
}

pub fn status() -> impl serde::Serialize {
    let hours = HOURS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let work = WORK.each_ref().map(|value| value.load(Ordering::Relaxed));
    let last_refusal = *LAST_REFUSAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::status::status!({
        "configured": cfg!(feature = "area_changes"),
        "full_hours": hours[0],
        "quiet_hours": hours[1],
        "marked_hours": hours[2],
        "repointed_provinces": work[0],
        "read_areas": work[1],
        "carried_areas": work[2],
        "refusals": REFUSALS.load(Ordering::Relaxed),
        "last_refusal": last_refusal,
        "scope": "cumulative since attach; the Provinces job labels each province's controller and owner areas (gathered with its movement fields) by their smallest member and records each controller area's representative and ordered contacts once; an hour with no territory province mark publishes the held areas, a marked hour reads again the areas of the marked and re-pointed provinces, the areas they left and their neighbours' areas, and a refused or moved take reads every area"
    })
}

#[cfg(test)]
mod tests {
    use engine_native::{
        controller_areas::Node, controller_areas::Pointers, controller_areas::Store,
        cprovince::Capture, cprovince::Row, memory::SliceMemory, walk::Outcome,
    };

    fn walked(store: bool) -> Capture {
        let mut capture = Capture::try_with_capacity(2, 0, 0).unwrap();
        for (index, id) in [1, 2].into_iter().enumerate() {
            capture.rows[index] = Row {
                id,
                ..Default::default()
            };
            capture.areas[index] = Pointers {
                province: 0x100 * id as usize,
                definition: 0x1000 * id as usize,
                controller: 0,
                owner: 0,
            };
        }
        // A previous hour's node in this recycled frame.
        capture.area_nodes.nodes.push(Node {
            label: 9,
            ..Default::default()
        });
        capture.area_store = store.then(|| Box::new(Store::new()));
        capture
    }

    fn labels(capture: &Capture) -> Vec<(Option<u32>, Option<u32>, Option<u32>)> {
        capture
            .rows
            .iter()
            .map(|row| (row.controller_area, row.owner_area, row.area_node))
            .collect()
    }

    /// The Provinces job labels only a complete walk, and only with the
    /// family's store; a frame never keeps another hour's nodes.
    #[test]
    fn the_province_job_labels_complete_walks_only() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut without = walked(false);
        super::capture(&mut without, Outcome::Rows { len: 2 }, &memory, 0, None);
        assert!(without.area_nodes.nodes.is_empty());
        assert_eq!(labels(&without), [(None, None, None); 2]);
        let mut truncated = walked(true);
        super::capture(
            &mut truncated,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
            &memory,
            0,
            None,
        );
        assert_eq!(labels(&truncated), [(None, None, None); 2]);
        // Sea provinces have null areas: labelled zero, with no area to read.
        let mut complete = walked(true);
        super::capture(&mut complete, Outcome::Rows { len: 2 }, &memory, 0, None);
        assert_eq!(labels(&complete), [(Some(0), Some(0), None); 2]);
        assert!(complete.area_nodes.nodes.is_empty());
    }
}
