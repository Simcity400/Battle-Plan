//! Exact observed direct-war cells borrowed from visibility's SAME sparse pass,
//! augmented only by authenticated native query demand at the fresh boundary.
//! No live pointers leave engine, no country-square matrix scan, no owner-zero
//! capture, no completeness certificate and no alternate relation framework.
//! Worker-owned arenas grow before publication; a refused owner is
//! wholly withheld so missing/conflicting occurrences never become success.
//! No retained query result or value-reuse certificate: omissions stay unknown.

use crate::table_capture::Arena;
use engine::{allocation, crelation_status as native};
use replay::rows::{DirectedWars, WarCell};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capacities {
    pub owners: usize,
    pub cells: usize,
}

impl Capacities {
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        // Linear attach preparation only; sparse header demand may exceed it.
        self.owners = self.owners.max(slots);
        self.cells = self.cells.max(slots);
    }
}

pub(crate) struct Captured {
    owners: Arena<native::DirectedWarOwner>,
    cells: Arena<native::DirectedWarObservation>,
    sample_hour: Option<i32>,
    walk: native::DirectedWarWalk,
}

const PROJECTION_NAMES: [&str; 7] = [
    "directed war sample absent or stale",
    "directed war owner absent from observed pass",
    "directed war owner withheld by discovery or capacity",
    "directed war acquired span invalid",
    "directed war repeated cell observations conflict",
    "directed war projection allocation failed",
    "directed war duplicate native receiver identities",
];
static PROJECTION_REFUSALS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

fn refused<T>(index: usize) -> Option<T> {
    PROJECTION_REFUSALS[index].fetch_add(1, Relaxed);
    None
}

impl Captured {
    pub(crate) fn new(capacity: Capacities) -> Self {
        Self {
            owners: Arena::new(capacity.owners),
            cells: Arena::new(capacity.cells),
            sample_hour: None,
            walk: Default::default(),
        }
    }
    pub(crate) fn reset(&mut self) {
        self.owners.reset();
        self.cells.reset();
        self.sample_hour = None;
        self.walk = Default::default();
    }
    pub(crate) fn fail(&mut self, reason: &'static str) {
        // Preserve captured pressure for growth even when a joined job fails.
        self.sample_hour = None;
        self.walk.pass_reason = Some(reason);
    }
    pub(crate) fn begin(&mut self, hour: i32) {
        self.reset();
        self.sample_hour = Some(hour);
    }
    pub(crate) fn output(&mut self) -> native::DirectedWarOutput<'_> {
        native::DirectedWarOutput {
            owners: &mut self.owners.first,
            cells: &mut self.cells.first,
        }
    }
    pub(crate) fn output_growing(&mut self) -> native::DirectedWarBuffers<'_> {
        native::DirectedWarBuffers {
            owners: engine::walk::Buffer::Growing(&mut self.owners.first),
            cells: engine::walk::Buffer::Growing(&mut self.cells.first),
        }
    }
    pub(crate) fn finish(&mut self, walk: native::DirectedWarWalk) {
        self.walk = walk;
        self.owners.required = walk.required_owners;
        self.cells.required = walk.required_cells;
        self.owners.len = walk.owners_len;
        self.cells.len = walk.cells_len;
        let Some(rows) = self.owners.first.get_mut(..self.owners.len) else {
            self.walk.pass_reason = Some("directed war captured owner length invalid");
            return;
        };
        // Owner indexing only; each native occurrence span remains ordered.
        // sort_unstable allocates nothing and runs once per sample, never per query.
        rows.sort_unstable_by_key(|row| row.country);
        if rows
            .windows(2)
            .any(|pair| pair[0].country == pair[1].country)
        {
            self.walk.pass_reason = Some("directed war duplicate native receiver identities");
            PROJECTION_REFUSALS[6].fetch_add(1, Relaxed);
        }
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.owners.allocated_bytes() + self.cells.allocated_bytes()
    }
    pub(crate) fn grow(&mut self, capacity: &mut Capacities) {
        self.owners.grow(&mut capacity.owners);
        self.cells.grow(&mut capacity.cells);
    }

    /// Off-thread checked projection. Borrow once after both sample jobs join.
    /// The enclosing CountryCounterInputs key must equal the exact native +8
    /// id. Caller checks Ordinary's attempted-source hour separately;
    /// request/frame context and terminal failure override carry association.
    pub(crate) fn for_country(&self, hour: i32, country: u32) -> Option<DirectedWars> {
        if self.sample_hour != Some(hour) || self.walk.pass_reason.is_some() {
            return refused(0);
        }
        let Some(owners) = self.owners.first.get(..self.owners.len) else {
            return refused(3);
        };
        let Ok(index) = owners.binary_search_by_key(&country, |row| row.country) else {
            return refused(1);
        };
        let owner = owners[index];
        if owner.reason.is_some() {
            return refused(2);
        }
        // Use the full canonical slot and the stored signed-i32 matrix extent.
        if owner.country as i32 <= 0
            || owner.canonical_slot == 0
            || owner.matrix_count == 0
            || owner.matrix_count > i32::MAX as u32
            || owner.canonical_slot >= owner.matrix_count
        {
            return refused(3);
        }
        let Some(events) = owner
            .cells_start
            .checked_add(owner.cells_len)
            .filter(|&end| end <= self.cells.len)
            .and_then(|end| self.cells.first.get(owner.cells_start..end))
        else {
            return refused(3);
        };
        if events.iter().any(|event| {
            event.target_slot >= owner.matrix_count
                || event.active.is_some() == event.reason.is_some()
        }) {
            return refused(3);
        }
        let result = (|| {
            let mut ordered = allocation::copy_vec(events)?;
            ordered.sort_unstable_by_key(|event| event.target_slot);
            let mut cells = Vec::new();
            let mut refused_slots = Vec::new();
            allocation::reserve_exact(&mut cells, ordered.len())?;
            allocation::reserve_exact(&mut refused_slots, ordered.len())?;
            let mut first = 0;
            while first < ordered.len() {
                let target = ordered[first].target_slot;
                let mut end = first + 1;
                while end < ordered.len() && ordered[end].target_slot == target {
                    end += 1;
                }
                let active = ordered[first].active;
                let conflict = ordered[first..end]
                    .iter()
                    .any(|event| event.active != active);
                if let Some(active) = active.filter(|_| !conflict) {
                    cells.push(WarCell {
                        target_slot: target,
                        active,
                    });
                } else {
                    refused_slots.push(target);
                    if conflict {
                        PROJECTION_REFUSALS[4].fetch_add(1, Relaxed);
                    }
                }
                first = end;
            }
            Ok::<_, std::io::Error>(DirectedWars {
                sample_hour: hour,
                receiver_raw_id: owner.country as i32,
                receiver_slot: owner.canonical_slot,
                matrix_count: owner.matrix_count,
                complete_active_domain: false,
                cells,
                refused_slots,
            })
        })();
        match result {
            Ok(packet) => Some(packet),
            Err(error) => {
                crate::allocation::observe(&error);
                refused(5)
            }
        }
    }
    pub(crate) fn unassociated() -> Option<DirectedWars> {
        refused(0)
    }

    pub(crate) fn observe(&self, work: &mut Work) {
        if self.sample_hour.is_none() {
            return;
        }
        work.sampled = work.sampled.saturating_add(1);
        work.observed_cells = work.observed_cells.saturating_add(self.walk.observed_cells);
        work.refused_cells = work.refused_cells.saturating_add(self.walk.refused_cells);
        for (sum, count) in work.faults.iter_mut().zip(self.walk.faults) {
            *sum = sum.saturating_add(count);
        }
        work.latest = (
            self.owners.required,
            self.owners.first.len(),
            self.cells.required,
            self.cells.first.len(),
        );
    }
}

#[derive(Default)]
pub(crate) struct Work {
    sampled: u64,
    observed_cells: u64,
    refused_cells: u64,
    faults: [u64; native::DIRECTED_WAR_FAULT_NAMES.len()],
    latest: (usize, usize, usize, usize),
}
impl Work {
    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "sampled":self.sampled,
            "observed_cells":self.observed_cells,
            "refused_cells":self.refused_cells,
            "owners_required":self.latest.0,
            "owners_capacity":self.latest.1,
            "cells_required":self.latest.2,
            "cells_capacity":self.latest.3,
            "refusals":crate::status::map(native::DIRECTED_WAR_FAULT_NAMES.into_iter()
                .zip(self.faults).chain(PROJECTION_NAMES.into_iter()
                .zip(PROJECTION_REFUSALS.iter().map(|count|count.load(Relaxed))))
                .filter(|(_,count)|*count!=0)),
            "complete_active_domain":false,
            "scope":"exact hourly sparse and native-demand predicates; admitted boundary only; missing cells unknown; no value reuse or full-domain certificate",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(target_slot: u32, active: Option<bool>) -> native::DirectedWarObservation {
        native::DirectedWarObservation {
            target_slot,
            active,
            reason: active.is_none().then_some("synthetic guarded cell refusal"),
        }
    }
    fn fixture_capture(events: &[native::DirectedWarObservation]) -> Captured {
        let mut result = Captured::new(Capacities {
            owners: 2,
            cells: events.len(),
        });
        result.begin(100);
        result.owners.first[0] = native::DirectedWarOwner {
            country: 7,
            canonical_slot: 1,
            matrix_count: 8,
            cells_start: 0,
            cells_len: events.len(),
            reason: None,
        };
        result.cells.first.copy_from_slice(events);
        result.finish(native::DirectedWarWalk {
            owners_len: 1,
            cells_len: events.len(),
            required_owners: 1,
            required_cells: events.len(),
            ..Default::default()
        });
        result
    }
    #[test]
    fn partial_packet_compacts_repeats_but_never_certifies_missing_targets() {
        let capture = fixture_capture(&[
            observation(6, Some(false)),
            observation(2, Some(true)),
            observation(2, Some(true)),
        ]);
        let packet = capture.for_country(100, 7).unwrap();
        assert_eq!(
            (
                packet.sample_hour,
                packet.receiver_raw_id,
                packet.receiver_slot
            ),
            (100, 7, 1)
        );
        assert_eq!(
            packet.cells,
            vec![
                WarCell {
                    target_slot: 2,
                    active: true
                },
                WarCell {
                    target_slot: 6,
                    active: false
                }
            ]
        );
        assert!(!packet.complete_active_domain);
        assert!(packet.refused_slots.is_empty());
    }
    #[test]
    fn directed_sparse_projection_admits_native_extent_above_u16() {
        let mut capture = fixture_capture(&[observation(65_536, Some(true))]);
        capture.owners.first[0].matrix_count = 65_537;
        let packet = capture.for_country(100, 7).unwrap();
        assert_eq!(packet.matrix_count, 65_537);
        assert_eq!(
            packet.cells,
            [WarCell {
                target_slot: 65_536,
                active: true
            }]
        );
        assert!(!packet.complete_active_domain);
        let admitted = packet.admit(100, 7, 1).unwrap();
        assert_eq!(admitted.pointer_active(65_536), Ok(true));
        assert!(admitted.pointer_active(0).is_err());
        assert!(admitted.pointer_active(65_537).is_err());
    }
    #[test]
    fn directed_sparse_projection_keeps_the_native_signed_header_domain() {
        let mut capture = fixture_capture(&[observation(65_536, Some(true))]);
        capture.owners.first[0].matrix_count = i32::MAX as u32;
        assert_eq!(
            capture.for_country(100, 7).unwrap().matrix_count,
            i32::MAX as u32
        );
        for invalid in [i32::MAX as u32 + 1, u32::MAX] {
            capture.owners.first[0].matrix_count = invalid;
            assert!(capture.for_country(100, 7).is_none());
        }
    }
    #[test]
    fn any_refused_or_conflicting_repeat_wins_over_success() {
        let capture = fixture_capture(&[
            observation(2, Some(true)),
            observation(2, None),
            observation(3, Some(true)),
            observation(3, Some(false)),
            observation(5, Some(false)),
        ]);
        let packet = capture.for_country(100, 7).unwrap();
        assert_eq!(packet.refused_slots, vec![2, 3]);
        assert_eq!(
            packet.cells,
            vec![WarCell {
                target_slot: 5,
                active: false
            }]
        );
    }
    #[test]
    fn stale_missing_and_reset_samples_are_never_associated_by_projection() {
        let mut capture = fixture_capture(&[]);
        assert!(capture.for_country(99, 7).is_none());
        assert!(capture.for_country(100, 8).is_none());
        assert!(capture.for_country(100, 7).is_some());
        capture.reset();
        assert!(capture.for_country(100, 7).is_none());
    }
    #[test]
    fn capacity_failure_withholds_owner_but_grows_prepared_next_sample() {
        let mut capture = fixture_capture(&[observation(2, Some(true))]);
        capture.owners.first[0].reason = Some("directed war cell arena capacity exceeded");
        capture.cells.required = 9;
        capture.owners.required = 4;
        assert!(capture.for_country(100, 7).is_none());
        let mut capacity = Capacities {
            owners: 2,
            cells: 1,
        };
        capture.grow(&mut capacity);
        assert!(capture.cells.first.len() >= 9);
        assert!(capture.owners.first.len() >= 4);
    }
    #[test]
    fn terminal_job_failure_retains_growth_demand_and_revokes_hour() {
        let mut capture = fixture_capture(&[observation(2, Some(true))]);
        capture.cells.required = 40;
        capture.fail("joined Slow worker panicked");
        assert!(capture.for_country(100, 7).is_none());
        assert_eq!(capture.cells.required, 40);
        assert_eq!(capture.sample_hour, None);
    }
    #[test]
    fn invalid_span_or_domain_and_duplicate_owners_are_refused() {
        let mut capture = fixture_capture(&[observation(2, Some(true))]);
        capture.owners.first[0].cells_start = usize::MAX;
        assert!(capture.for_country(100, 7).is_none());
        let mut capture = fixture_capture(&[observation(8, Some(true))]);
        assert!(capture.for_country(100, 7).is_none());
        capture.owners.first[1] = capture.owners.first[0];
        capture.finish(native::DirectedWarWalk {
            owners_len: 2,
            cells_len: 1,
            ..Default::default()
        });
        assert!(capture.for_country(100, 7).is_none());
    }
    #[test]
    fn statistics_fold_once_per_clocked_sample_without_reset_stale_recount() {
        let mut capture = fixture_capture(&[]);
        let mut work = Work::default();
        capture.observe(&mut work);
        assert_eq!(work.sampled, 1);
        capture.reset();
        capture.observe(&mut work);
        assert_eq!(work.sampled, 1);
    }
}
