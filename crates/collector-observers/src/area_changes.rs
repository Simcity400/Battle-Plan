use engine_native::cprovince::Capture;
use engine_native_memory::walk::Outcome;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

static HOURS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static WORK: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);

pub fn capture(
    capture: &mut Capture,
    outcome: Outcome,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    take: Option<[u64; 2]>,
) -> Outcome {
    capture.area_nodes.clear();
    let Some(store) = capture.area_store.as_mut() else {
        return outcome;
    };
    let Outcome::Rows { len } = outcome else {
        store.forget();
        refuse("the province walk did not complete");
        return outcome;
    };
    let quiet = store.carries_quietly(take, len);
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
    memory: &impl engine_native_memory::Memory,
    base: usize,
    len: usize,
    take: Option<[u64; 2]>,
) -> Result<engine_native::cprovince::controller_areas::Counts, &'static str> {
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

fn record(counts: engine_native::cprovince::controller_areas::Counts) {
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


#[cfg(test)]
mod tests {
    use engine_native::cprovince::{
        controller_areas::{Node, Pointers, Store},
        Capture,
        Row,
    };
    use engine_native_memory::{memory::SliceMemory, walk::Outcome};

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
        let mut complete = walked(true);
        super::capture(&mut complete, Outcome::Rows { len: 2 }, &memory, 0, None);
        assert_eq!(labels(&complete), [(Some(0), Some(0), None); 2]);
        assert!(complete.area_nodes.nodes.is_empty());
    }
}
