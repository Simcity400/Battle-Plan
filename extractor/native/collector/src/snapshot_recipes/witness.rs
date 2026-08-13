//! A/B witnesses: proving the bytes did not move under the copier.
//!
//! The opening copies drive discovery. The closing wave re-copies the SAME
//! addresses — it does not chase the pointer graph again, because it does not
//! have to: if a parent's bytes changed, every address decoded out of them is
//! suspect, and invalidating the dependent subtree is both cheaper than a
//! reverse walk and strictly more conservative than one.
//!
//! Comparison happens HERE, post-bracket. Nothing in this file runs while the
//! observation window is open.
//!
//! # Invalidation is per consumer
//!
//! A changed interval gaps the consumers that depend on it and the consumers
//! that depend on anything below it. Deduplication is what makes that precise:
//! a shared extent knows its dependency list, so one family's bad witness does
//! not cost another family its hour. There is no global bracket to fail.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::executor::{RecipeReport, SnapshotBuild};
use super::view::Snapshot;
use super::{ConsumerId, GapReason, IntervalId};

/// What the closing comparison found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WitnessReport {
    /// Intervals enrolled in the closing wave.
    pub witnessed: u32,
    /// Intervals whose opening and closing copies differ, or whose closing
    /// copy could not be taken at all.
    pub changed: Vec<IntervalId>,
    /// Intervals reached from a changed one through discovery edges. Disjoint
    /// from `changed`.
    pub invalidated: Vec<IntervalId>,
    /// Intervals whose back-to-back images disagreed: the copy caught a writer
    /// mid-update. A statement about the BYTES.
    pub torn: Vec<IntervalId>,
    /// `(interval, consumer)` where the consumer asked for more images than
    /// were taken. A statement about a REQUEST, not about the bytes — which is
    /// why it is a separate list and gaps only the consumer named in it.
    pub uncertified: Vec<(IntervalId, ConsumerId)>,
    /// Consumers gapped by this comparison, with the interval that did it.
    pub gapped: BTreeMap<ConsumerId, GapReason>,
}

/// A snapshot whose witnesses have been compared. This is what a post-bracket
/// reader is handed.
#[derive(Debug)]
pub struct ClosedSnapshot {
    pub snapshot: Snapshot,
    pub reports: Vec<RecipeReport>,
    pub witness: WitnessReport,
    pub forward_waves_run: u32,
    pub phases_run: u32,
    /// Everything the observation held against its 512 MiB ceiling: copied
    /// bytes plus the tables that index them. Carried through from
    /// [`SnapshotBuild`] because the post-bracket reporting path is where a
    /// near-ceiling observation is worth knowing about.
    pub custody_held: u64,
    pub custody_table_bytes: u64,
    pub custody_arena_bytes: u64,
    /// Copies the FINAL closing wave issued, carried through from
    /// [`SnapshotBuild::final_wave_copies`] for the same reason `custody_held`
    /// is: the caller that has to say WHERE the final wave began holds a
    /// `ClosedSnapshot` and not the build it came from.
    pub final_wave_copies: u64,
    /// RULING B. The order the closing wave ran in, and the closes a later
    /// stage covered. Carried through because a declared closure order that
    /// silently did not happen is a defect nothing else would show.
    pub closing_order: Vec<IntervalId>,
    pub elisions: Vec<super::executor::Elision>,
}

impl SnapshotBuild {
    /// Compare the A and B images and gap exactly the affected consumers.
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
        // RULING B. The comparison is over the bytes a consumer CONSUMES, not
        // over every byte of the interval. d81's rail header has four dead
        // bytes inside the window it copies, and d47's map header has a LIVE
        // pointer field sitting between two compared words; under a whole-
        // interval comparison either of those manufactures a torn gap out of a
        // write the reader never looks at. An interval nobody declared a mask
        // for still compares whole, which is what every interval did before.
        for record in snapshot.intervals() {
            if !record.witnessed() {
                continue;
            }
            let differs = match record.closing_offset() {
                // A witness that could not be re-copied is a FAILED witness,
                // never an absent one. Treating it as unchanged would let a
                // region that became unreadable mid-bracket certify itself.
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

        // The ADJACENT pair, compared on the same terms. This is a different
        // instrument from the opening/closing witness — nanoseconds apart
        // rather than a bracket apart — and it answers a different question:
        // not "did anything move while we worked" but "did we copy this object
        // mid-write". An interval whose two back-to-back images disagree is
        // torn, and a torn image is never issued.
        // Two outcomes, and they are not the same statement. Images that DIFFER
        // prove the object was written under the copier, which is true for
        // everyone who reads it. An image that could not be taken proves
        // nothing about the bytes — it means the certification one consumer
        // asked for is unavailable — so it costs that consumer its hour and
        // nobody else's.
        let mut torn: Vec<IntervalId> = Vec::new();
        let mut uncertified: Vec<(IntervalId, ConsumerId)> = Vec::new();
        // THE LOOKUP BELOW IS A LINEAR SCAN INSIDE THIS LOOP, and lane HL
        // (task #185) left it there deliberately after measuring it.
        //
        // `intervals().iter().position(..)` walks every interval in the
        // snapshot to find one, so the pair is quadratic in the interval count
        // — d81's production shape is 21,056 intervals. It is NOT reached on
        // any shape this repo currently runs: the scan sits behind
        // `images.anyone_gaps()`, and the only production recipe with this many
        // intervals is d81's copy plan, whose two `SampleSpec`s both declare
        // `DisagreeAction::Record` (`d81_copy_bracket.rs`, the `samples`
        // assignments) and therefore set `gap_on_disagree` false on every ask.
        // An indexed spelling was written and measured against the base at four
        // shapes up to 84,226 transactions and moved nothing, which is what
        // "never reached" predicts; it was reverted rather than kept as an
        // unpriced change (`runs/hl_d81_span/AB_SUBSTRATE.txt`).
        //
        // WHAT WOULD MAKE IT HOT, stated so this is a measurement and not a
        // shrug: one recipe declaring `DisagreeAction::Gap` at d81's interval
        // count. Residual HL-2, owner: the S4 substrate lane.
        for (id, images) in snapshot.annotations().sampled_intervals() {
            // SHORTNESS IS PER CONSUMER. A recipe that asked for three images
            // and got two is short; a recipe that asked for two and got them is
            // not — on the same interval, at the same time. Asking the question
            // of the interval instead of the request is how one recipe's
            // stricter requirement takes its neighbour's hour.
            for consumer in images.unmet() {
                uncertified.push((id, consumer));
            }
            // The comparison itself is only worth running if somebody wants to
            // be gapped by it.
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
            // An image that could not be taken cannot certify the one that was.
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

        // Discovery edges, keyed by interval id, each carrying the consumer
        // whose chain walked it.
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

        // The walk is per (interval, consumer). A changed interval invalidates
        // a consumer's subtree only along edges that consumer's own recipe
        // walked: another consumer that reached the same bytes by its own route
        // did not depend on the parent that moved, and its hour survives.
        for &root in &changed {
            let mut seen: BTreeSet<(IntervalId, ConsumerId)> = BTreeSet::new();
            let mut queue: VecDeque<(IntervalId, ConsumerId)> = VecDeque::new();
            for consumer in dependents.get(&root).into_iter().flatten() {
                // `CloseAction::Record`: this consumer takes the closing image
                // and renders its own verdict, so a change here is recorded in
                // `changed` and costs it nothing. It is opt-in per (op,
                // consumer) and it is what lets a converted reader close over
                // exactly the fields its live form closed over -- see
                // `executor::CloseAction`.
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
                // Only gap the consumer that walked the edge, and only if it
                // actually reads these bytes.
                if dependents
                    .get(&id)
                    .map(|list| list.contains(&consumer))
                    .unwrap_or(false)
                    // A consumer that judges this interval's own closing image
                    // judges its parent's too: the subtree walk exists to gap a
                    // reader whose pointer chain may be stale, and a reader that
                    // re-reads the chain from the closing image is exactly the
                    // reader that does not need it.
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

        // A torn interval gaps its own dependents. It deliberately does NOT
        // invalidate their subtrees the way a changed witness does: a torn copy
        // says this object was written while it was read, which says nothing
        // about whether the addresses decoded out of it were stale — and the
        // closing witness above already answers that question for every one of
        // them.
        // A real disagreement is a property of the BYTES: the object was
        // written while it was being copied, and that is true for everyone who
        // reads it. So it gaps every dependent — except a consumer that
        // explicitly elected to resolve a disagreement itself, which is what
        // `DisagreeAction::Record` means and the only reason it exists.
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
        // An UNCERTIFIABLE pair says nothing about the bytes — only that the
        // certification one consumer asked for is not available. Its blast
        // radius is that consumer, not the interval's dependent set.
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
