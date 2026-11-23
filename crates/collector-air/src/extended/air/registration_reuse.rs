use engine_allocation as allocation;
use engine_native::{
    cair_base::DenseEntry, cair_base::HeldBase, cair_base::RegistrationReuse,
    cair_base::RegistrationRow, cair_base::WalkedBase,
};
use std::collections::HashMap;

use super::HeldSegment;

#[derive(Debug, Default)]
struct Kept {
    pointer: usize,
    segment: u64,
    slots: u32,
    sought_plan: bool,
    plan: Option<Vec<DenseEntry>>,
    rows: Vec<RegistrationRow>,
    wings: Vec<u64>,
    countries: Vec<u32>,
    pools: usize,
    required_wings: usize,
    pass: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodedCounts {
    pub reused: u64,
    pub walked: u64,
}

#[derive(Debug, Default)]
pub struct DecodedRegistrations {
    kept: HashMap<u64, Kept>,
    segments: HashMap<u64, (usize, u64, bool)>,
    walking: Option<(u64, usize, u64)>,
    countries: Vec<u32>,
    pass: u64,
    counts: DecodedCounts,
}

impl DecodedRegistrations {
    pub fn begin(&mut self, segments: &[HeldSegment]) -> bool {
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
            self.segments.insert(
                segment.handle,
                (segment.pointer, segment.id, segment.carried_raw.is_some()),
            );
        }
        true
    }

    pub fn finish(&mut self) {
        let pass = self.pass;
        self.kept.retain(|_, kept| kept.pass == pass);
        self.walking = None;
    }

    pub fn counts(&self) -> DecodedCounts {
        self.counts
    }

    pub fn capacity_bytes(&self) -> u64 {
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
            + self.segments.capacity() * size_of::<(u64, (usize, u64, bool))>()
            + self.countries.capacity() * size_of::<u32>()
            + kept) as u64
    }

    fn reported(&self, handle: u64, pointer: usize) -> Option<u64> {
        self.segments
            .get(&handle)
            .filter(|(at, _, _)| *at == pointer)
            .map(|&(_, segment, _)| segment)
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

    fn trusted_carried(&self, handle: u64, pointer: usize) -> bool {
        let Some(&(at, segment, carried)) = self.segments.get(&handle) else {
            return false;
        };
        carried
            && at == pointer
            && self
                .kept
                .get(&handle)
                .is_some_and(|kept| kept.pointer == pointer && kept.segment == segment)
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
