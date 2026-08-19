use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::executor::{RecipeReport, SnapshotBuild};
use super::view::Snapshot;
use super::{ConsumerId, GapReason, IntervalId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WitnessReport {
    pub witnessed: u32,
    pub changed: Vec<IntervalId>,
    pub invalidated: Vec<IntervalId>,
    pub torn: Vec<IntervalId>,
    pub uncertified: Vec<(IntervalId, ConsumerId)>,
    pub gapped: BTreeMap<ConsumerId, GapReason>,
}

#[derive(Debug)]
pub struct ClosedSnapshot {
    pub snapshot: Snapshot,
    pub reports: Vec<RecipeReport>,
    pub witness: WitnessReport,
    pub forward_waves_run: u32,
    pub phases_run: u32,
    pub custody_held: u64,
    pub custody_table_bytes: u64,
    pub custody_arena_bytes: u64,
    pub final_wave_copies: u64,
    pub closing_order: Vec<IntervalId>,
    pub elisions: Vec<super::executor::Elision>,
}

impl SnapshotBuild {
    pub fn close_witnesses(self) -> ClosedSnapshot {
        let SnapshotBuild {
            mut snapshot,
            reports,
            forward_waves_run,
            phases_run,
            witnessed_intervals,
            closing_copies: _,
            final_wave_copies,
            custody_held,
            custody_table_bytes,
            custody_arena_bytes,
            unreadable_closes,
            closing_order,
            elisions,
        } = self;

        let uncertifiable: BTreeSet<IntervalId> = unreadable_closes.into_iter().collect();

        let mut changed: Vec<IntervalId> = Vec::new();
        for record in snapshot.intervals() {
            if !record.witnessed() {
                continue;
            }
            let differs = match record.closing_offset() {
                None => true,
                Some(closing) => {
                    let len = record.len() as usize;
                    let opening = record.opening_offset();
                    match snapshot.annotations().consumption(record.id()) {
                        Some(consumption) => !consumption.images_agree(
                            record.span(),
                            snapshot.raw(opening, len),
                            snapshot.raw(closing, len),
                        ),
                        None => snapshot.raw(opening, len) != snapshot.raw(closing, len),
                    }
                }
            };
            if differs || uncertifiable.contains(&record.id()) {
                changed.push(record.id());
            }
        }

        let mut torn: Vec<IntervalId> = Vec::new();
        let mut uncertified: Vec<(IntervalId, ConsumerId)> = Vec::new();
        for (id, images) in snapshot.annotations().sampled_intervals() {
            for consumer in images.unmet() {
                uncertified.push((id, consumer));
            }
            if !images.anyone_gaps() {
                continue;
            }
            let Some(position) = snapshot
                .intervals()
                .iter()
                .position(|record| record.id() == id)
            else {
                continue;
            };
            let record = snapshot.intervals()[position];
            let len = record.len() as usize;
            let first = record.opening_offset();
            let mut disagreed = false;
            for &second in &images.extra {
                if disagreed {
                    break;
                }
                let second = second as usize;
                disagreed = match snapshot.annotations().consumption(id) {
                    Some(consumption) => !consumption.images_agree(
                        record.span(),
                        snapshot.raw(first, len),
                        snapshot.raw(second, len),
                    ),
                    None => snapshot.raw(first, len) != snapshot.raw(second, len),
                };
            }
            if disagreed {
                torn.push(id);
            }
        }

        let mut children: BTreeMap<IntervalId, Vec<(IntervalId, ConsumerId)>> = BTreeMap::new();
        let mut dependents: BTreeMap<IntervalId, Vec<ConsumerId>> = BTreeMap::new();
        let links = snapshot.links();
        for record in snapshot.intervals() {
            if links.has_children(record) {
                children.insert(record.id(), links.children(record).collect());
            }
            let consumers: Vec<ConsumerId> = links.consumers(record).collect();
            if !consumers.is_empty() {
                dependents.insert(record.id(), consumers);
            }
        }

        let mut gapped: BTreeMap<ConsumerId, GapReason> = BTreeMap::new();
        let mut invalidated: Vec<IntervalId> = Vec::new();
        let mut invalid_any: BTreeSet<IntervalId> = BTreeSet::new();

        for &root in &changed {
            let mut seen: BTreeSet<(IntervalId, ConsumerId)> = BTreeSet::new();
            let mut queue: VecDeque<(IntervalId, ConsumerId)> = VecDeque::new();
            for consumer in dependents.get(&root).into_iter().flatten() {
                if !snapshot.annotations().close_gaps(root, *consumer) {
                    continue;
                }
                gapped
                    .entry(*consumer)
                    .or_insert(GapReason::WitnessChanged { interval: root });
            }
            queue.extend(children.get(&root).into_iter().flatten().copied());
            while let Some((id, consumer)) = queue.pop_front() {
                if !seen.insert((id, consumer)) {
                    continue;
                }
                if invalid_any.insert(id) {
                    invalidated.push(id);
                }
                if dependents
                    .get(&id)
                    .map(|list| list.contains(&consumer))
                    .unwrap_or(false)
                    && snapshot.annotations().close_gaps(id, consumer)
                {
                    gapped
                        .entry(consumer)
                        .or_insert(GapReason::WitnessDependency {
                            interval: id,
                            changed_parent: root,
                        });
                }
                for (next, label) in children.get(&id).into_iter().flatten() {
                    if *label == consumer {
                        queue.push_back((*next, *label));
                    }
                }
            }
        }
        invalidated.retain(|id| !changed.contains(id));

        for &id in &torn {
            let images = snapshot.annotations().samples(id);
            for consumer in dependents.get(&id).into_iter().flatten() {
                if images
                    .map(|images| images.opted_out_of_gapping(*consumer))
                    .unwrap_or(false)
                {
                    continue;
                }
                gapped
                    .entry(*consumer)
                    .or_insert(GapReason::SamplesDisagreed { interval: id });
            }
        }
        for (id, consumer) in &uncertified {
            gapped
                .entry(*consumer)
                .or_insert(GapReason::SamplesDisagreed { interval: *id });
        }

        for (consumer, reason) in &gapped {
            snapshot.gap_consumer(*consumer, *reason);
        }

        ClosedSnapshot {
            snapshot,
            reports,
            witness: WitnessReport {
                witnessed: witnessed_intervals,
                changed,
                invalidated,
                torn,
                uncertified,
                gapped,
            },
            forward_waves_run,
            phases_run,
            custody_held,
            custody_table_bytes,
            custody_arena_bytes,
            final_wave_copies,
            closing_order,
            elisions,
        }
    }
}
