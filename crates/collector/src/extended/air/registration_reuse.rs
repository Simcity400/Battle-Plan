//! AirBaseRegistrations rows decoded from a held segment, kept across hours
//! on the decode worker.
//!
//! The acquisition reports each state base whose subtree in the hour's log is
//! a held segment ([`super::RegistrationStore`], [`super::HeldSegment`]): its
//! spliced bytes, or the fresh read the segment was taken from. The walk's
//! output for such a base depends only on those bytes and on the country
//! tables, which the walk checks again ([`engine::cair_base::RegistrationReuse`]),
//! so the output a walk of the segment produced is appended in later hours
//! that report the same segment, without walking its containers, pools or
//! wings. A base whose segment changed, or that reports none, is walked.
//!
//! The cache keeps only bases its latest pass visited. Allocation is advisory:
//! an output that cannot be kept is walked again.
use engine::allocation;
use engine::cair_base::{DenseEntry, HeldBase, RegistrationReuse, RegistrationRow, WalkedBase};
use std::collections::HashMap;

use super::HeldSegment;

#[derive(Debug, Default)]
struct Kept {
    pointer: usize,
    segment: u64,
    slots: u32,
    sought_plan: bool,
    plan: Option<Vec<DenseEntry>>,
    /// Each `wings_start` counts from the base's first wing.
    rows: Vec<RegistrationRow>,
    wings: Vec<u64>,
    countries: Vec<u32>,
    pools: usize,
    required_wings: usize,
    pass: u64,
}

/// Cumulative counts of state bases appended and walked while reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DecodedCounts {
    pub reused: u64,
    pub walked: u64,
}

#[derive(Debug, Default)]
pub(crate) struct DecodedRegistrations {
    kept: HashMap<u64, Kept>,
    /// This pass's reported segments by handle.
    segments: HashMap<u64, (usize, u64)>,
    /// The base being walked and its accepted containers' countries.
    walking: Option<(u64, usize, u64)>,
    countries: Vec<u32>,
    pass: u64,
    counts: DecodedCounts,
}

impl DecodedRegistrations {
    /// Starts a pass over a log that reports `segments`. False when they
    /// cannot be indexed; the pass then walks every base.
    pub(crate) fn begin(&mut self, segments: &[HeldSegment]) -> bool {
        self.pass += 1;
        self.walking = None;
        self.segments.clear();
        let missing = segments.len().saturating_sub(self.kept.len());
        let reserved = allocation::advisory(|| {
            allocation::reserve_map(&mut self.segments, segments.len())?;
            allocation::reserve_map(&mut self.kept, missing)
        });
        if reserved.is_err() {
            return false;
        }
        for segment in segments {
            self.segments
                .insert(segment.handle, (segment.pointer, segment.id));
        }
        true
    }

    /// Ends the pass: keeps the bases it visited.
    pub(crate) fn finish(&mut self) {
        let pass = self.pass;
        self.kept.retain(|_, kept| kept.pass == pass);
        self.walking = None;
    }

    pub(crate) fn counts(&self) -> DecodedCounts {
        self.counts
    }

    pub(crate) fn capacity_bytes(&self) -> u64 {
        let kept = self
            .kept
            .values()
            .map(|kept| {
                kept.plan
                    .as_ref()
                    .map_or(0, |plan| plan.capacity() * size_of::<DenseEntry>())
                    + kept.rows.capacity() * size_of::<RegistrationRow>()
                    + kept.wings.capacity() * size_of::<u64>()
                    + kept.countries.capacity() * size_of::<u32>()
            })
            .sum::<usize>();
        (self.kept.capacity() * size_of::<(u64, Kept)>()
            + self.segments.capacity() * size_of::<(u64, (usize, u64))>()
            + self.countries.capacity() * size_of::<u32>()
            + kept) as u64
    }

    fn reported(&self, handle: u64, pointer: usize) -> Option<u64> {
        self.segments
            .get(&handle)
            .filter(|(at, _)| *at == pointer)
            .map(|&(_, segment)| segment)
    }

    fn keep(&mut self, handle: u64, pointer: usize, segment: u64, output: WalkedBase<'_>) -> bool {
        let pass = self.pass;
        let kept = self.kept.entry(handle).or_default();
        let plan_len = output.plan.map_or(0, <[DenseEntry]>::len);
        let reserved = allocation::advisory(|| {
            let plan = kept.plan.get_or_insert_with(Vec::new);
            plan.clear();
            kept.rows.clear();
            kept.wings.clear();
            kept.countries.clear();
            allocation::reserve(plan, plan_len)?;
            allocation::reserve(&mut kept.rows, output.rows.len())?;
            allocation::reserve(&mut kept.wings, output.wings.len())?;
            allocation::reserve(&mut kept.countries, self.countries.len())
        });
        if reserved.is_err() || self.countries.len() != output.rows.len() {
            self.kept.remove(&handle);
            return false;
        }
        kept.pointer = pointer;
        kept.segment = segment;
        kept.slots = output.slots;
        kept.sought_plan = output.sought_plan;
        match output.plan {
            Some(plan) => kept.plan.as_mut().unwrap().extend_from_slice(plan),
            None => kept.plan = None,
        }
        kept.rows
            .extend(output.rows.iter().map(|row| RegistrationRow {
                wings_start: row.wings_start - output.wings_start,
                ..*row
            }));
        kept.wings.extend_from_slice(output.wings);
        kept.countries.extend_from_slice(&self.countries);
        kept.pools = output.pools;
        kept.required_wings = output.required_wings;
        kept.pass = pass;
        true
    }
}

impl RegistrationReuse for DecodedRegistrations {
    fn previous(&self, handle: u64, pointer: usize) -> Option<HeldBase<'_>> {
        let segment = self.reported(handle, pointer)?;
        let kept = self
            .kept
            .get(&handle)
            .filter(|kept| kept.pointer == pointer && kept.segment == segment)?;
        Some(HeldBase {
            output: WalkedBase {
                slots: kept.slots,
                sought_plan: kept.sought_plan,
                plan: kept.plan.as_deref(),
                rows: &kept.rows,
                wings_start: 0,
                wings: &kept.wings,
                pools: kept.pools,
                required_wings: kept.required_wings,
            },
            countries: &kept.countries,
        })
    }

    fn reused(&mut self, handle: u64, _pointer: usize) {
        if let Some(kept) = self.kept.get_mut(&handle) {
            kept.pass = self.pass;
        }
        self.counts.reused += 1;
    }

    fn begin(&mut self, handle: u64, pointer: usize) {
        self.countries.clear();
        self.walking = self
            .reported(handle, pointer)
            .map(|segment| (handle, pointer, segment));
    }

    fn container(&mut self, country: u32) {
        if self.walking.is_some() {
            if allocation::advisory(|| allocation::reserve(&mut self.countries, 1)).is_ok() {
                self.countries.push(country);
            } else {
                self.walking = None;
            }
        }
    }

    fn walked(&mut self, output: WalkedBase<'_>) {
        if let Some((handle, pointer, segment)) = self.walking.take()
            && self.keep(handle, pointer, segment, output)
        {
            self.counts.walked += 1;
        }
    }
}
