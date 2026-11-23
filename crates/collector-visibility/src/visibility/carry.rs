use crate::country_input_changes::{self as changes, Changes, Counts, Table};
use engine_allocation as allocation;
use engine_native_memory::{
    Memory,
    country_identity::Country,
    modifier_cache::Value,
    walk::{Buffer, Outcome},
};
use engine_native_parts::modifier_accumulator;
use engine_visibility::visibility_inputs as native;

#[derive(Clone, Copy)]
struct HeldCountry {
    country: Country,
    raw: Value,
}

pub(super) struct Store {
    epoch: Option<u64>,
    held: Vec<Option<HeldCountry>>,
    fresh: Vec<HeldCountry>,
    marked: Vec<u32>,
    taken: Option<u64>,
    certified: bool,
    untracked: bool,
    counts: Counts,
}

impl Store {
    pub(super) const fn new() -> Self {
        Self {
            epoch: None,
            held: Vec::new(),
            fresh: Vec::new(),
            marked: Vec::new(),
            taken: None,
            certified: false,
            untracked: false,
            counts: Counts {
                marked: 0,
                read: 0,
                full: false,
            },
        }
    }

    pub(super) fn forget(&mut self) {
        self.epoch = None;
        self.held.fill(None);
    }

    fn begin(&mut self, changes: &impl Changes, force_full: bool) {
        if force_full {
            self.forget();
        }
        self.marked.clear();
        self.fresh.clear();
        self.untracked = false;
        let mut overflow = false;
        let epoch = changes.take(Table::Visibility, &mut |id| match u32::try_from(id) {
            Ok(id)
                if allocation::advisory(|| allocation::reserve_push_batch(&mut self.marked, 1))
                    .is_ok() =>
            {
                self.marked.push(id)
            }
            _ => overflow = true,
        });
        self.marked.sort_unstable();
        self.marked.dedup();
        self.taken = if overflow { None } else { epoch };
        self.certified = self.taken.is_some() && self.taken == self.epoch;
        self.counts = Counts {
            marked: self.marked.len() as u64,
            full: !self.certified,
            ..Counts::default()
        };
    }

    fn lookup(
        &mut self,
        memory: &impl Memory,
        country: Country,
        accumulator: usize,
        carried: &mut u64,
    ) -> Result<Value, &'static str> {
        let held = self
            .held
            .get(country.canonical_slot as usize)
            .copied()
            .flatten()
            .filter(|held| {
                held.country.pointer == country.pointer && held.country.id == country.id
            });
        let certified = self.certified
            && !self.untracked
            && held.is_some()
            && self.marked.binary_search(&country.id).is_err();
        let raw = if certified {
            *carried += 1;
            held.unwrap().raw
        } else {
            self.counts.read += 1;
            modifier_accumulator::lookup_at(memory, accumulator, native::UNIT_MODIFIER)?
        };
        if !self.untracked {
            if allocation::advisory(|| allocation::reserve_push_batch(&mut self.fresh, 1)).is_ok() {
                self.fresh.push(HeldCountry { country, raw });
            } else {
                self.untracked = true;
            }
        }
        Ok(raw)
    }

    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        output: Buffer<'_, native::CountryRow>,
        changes: &impl Changes,
        force_full: bool,
    ) -> ((Outcome, usize), Counts, u64) {
        self.begin(changes, force_full);
        let mut carried = 0;
        let result = native::countries_buffer_with(
            memory,
            base,
            game_state,
            output,
            &mut |country, accumulator| {
                self.lookup(memory, country, accumulator, &mut carried)
            },
        );
        let complete = matches!(result.0, Outcome::Rows { .. }) && !self.untracked;
        let ids = complete.then(|| {
            self.fresh
                .iter()
                .map(|held| held.country.id as usize + 1)
                .max()
                .unwrap_or(0)
        });
        if complete
            && self.taken.is_some()
            && allocation::advisory(|| allocation::resize_default(&mut self.held, result.1)).is_ok()
        {
            self.held.fill(None);
            for fresh in &self.fresh {
                self.held[fresh.country.canonical_slot as usize] = Some(*fresh);
            }
            self.epoch = self.taken;
        } else {
            self.forget();
        }
        changes::record(Table::Visibility, &self.counts, ids);
        changes::record_carried(Table::Visibility, carried);
        (result, self.counts, carried)
    }
}

#[cfg(test)]
mod tests;
