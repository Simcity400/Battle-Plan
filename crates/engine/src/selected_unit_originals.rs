//! Original class-0 division observations selected transport borrows: each
//! division's authenticated stat block (CArmy+0x138, a CSubUnitDefinition) and
//! standard template wrapper (CArmy+0x3B8, type-52 registry identity). Shared
//! stat blocks and templates are read once per pass, including failures, so
//! no borrower retries them. The pointers are callback-local: the owner calls
//! [`Packet::finish_native`] once its borrowers return. No unit selection, path
//! cache, UI clock or displayed text is read.

use crate::{
    Memory, carmy, creference_registry, creferenced_division_template as template,
    csub_unit_definition as stats, read, walk::Outcome,
};
use std::{collections::HashMap, mem::size_of};

pub type Field<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Debug)]
pub struct StatObservation {
    pub pointer: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct TemplateObservation {
    /// Null wrapper has handle zero; nonnull requires type52 registry equality.
    pub wrapper: usize,
    pub handle: u64,
}
impl TemplateObservation {
    /// The coastal predicate consumes the contained composition, not wrapper+A8.
    pub fn composition(self) -> Field<usize> {
        at(self.wrapper, template::COMPOSITION)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OriginalObservation {
    pub division: u64,
    pub pointer: usize,
    /// The hour this observation was read for.
    pub sample_hour: i32,
    pub stat: Field<StatObservation>,
    pub template: Field<TemplateObservation>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub rows: usize,
}

pub struct Packet {
    originals: Vec<OriginalObservation>,
    stat_cache: Vec<Field<StatObservation>>,
    template_cache: Vec<Field<TemplateObservation>>,
    stat_indices: HashMap<usize, usize>,
    /// Wrapper pointer -> its `template_cache` entry, so each division finds
    /// its template's first observation without scanning every template.
    template_indices: HashMap<usize, usize>,
    /// Cache entries examined by template lookups since the last clear.
    #[cfg(test)]
    template_probes: usize,
    outcome: Outcome,
    required: Capacity,
    growing: bool,
}

const UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "selected unit originals not acquired",
};

fn at(pointer: usize, offset: usize) -> Field<usize> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("selected unit original pointer null or overflow")
}
fn field<T: crate::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Field<T> {
    read(memory, at(pointer, offset)?).ok_or("selected unit original field unreadable")
}
fn expected(base: usize, rva: usize) -> Field<u64> {
    base.checked_add(rva)
        .map(|value| value as u64)
        .ok_or("selected unit original module address overflow")
}
fn reserve<T>(values: &mut Vec<T>, count: usize) -> std::io::Result<()> {
    if values.capacity() < count {
        crate::allocation::reserve_exact(values, count.saturating_sub(values.len()))?;
    }
    Ok(())
}
fn admitted<T>(values: &mut Vec<T>, value: T) -> Field<()> {
    if values.len() == values.capacity() {
        return Err("selected unit original arena capacity exhausted");
    }
    values.push(value);
    Ok(())
}

pub fn authenticate_original(seed: &carmy::DivisionSeed, base: usize) -> Field<()> {
    if seed.pointer == 0
        || seed.division == 0
        || seed.handle != Some(seed.division)
        || seed.primary_vtable != Some(expected(base, carmy::VTABLE_RVA)?)
        || seed.secondary_vtable != Some(expected(base, carmy::SECONDARY_VTABLE_RVA)?)
        || seed.class != Some(0)
    {
        return Err("selected unit original class0 seed identity unavailable");
    }
    Ok(())
}

impl Packet {
    pub fn vacant() -> Self {
        Self {
            originals: Vec::new(),
            stat_cache: Vec::new(),
            template_cache: Vec::new(),
            stat_indices: HashMap::new(),
            template_indices: HashMap::new(),
            #[cfg(test)]
            template_probes: 0,
            outcome: UNCAPTURED,
            required: Capacity::default(),
            growing: false,
        }
    }
    /// Worker-only growth continues from the original census.
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn try_new(capacity: Capacity) -> std::io::Result<Self> {
        let mut packet = Self::vacant();
        packet.reserve(capacity)?;
        Ok(packet)
    }
    pub fn reserve(&mut self, capacity: Capacity) -> std::io::Result<()> {
        reserve(&mut self.originals, capacity.rows)?;
        reserve(&mut self.stat_cache, capacity.rows)?;
        reserve(&mut self.template_cache, capacity.rows)?;
        let additional = capacity.rows.saturating_sub(self.stat_indices.len());
        crate::allocation::reserve_map(&mut self.stat_indices, additional)?;
        let additional = capacity.rows.saturating_sub(self.template_indices.len());
        crate::allocation::reserve_map(&mut self.template_indices, additional)
    }
    pub fn clear(&mut self) {
        self.finish_native();
        #[cfg(test)]
        {
            self.template_probes = 0;
        }
        self.outcome = UNCAPTURED;
        self.required = Capacity::default();
    }
    /// End the native lifetime once every borrower (selected transport) has
    /// returned. Nothing here is re-authenticated or followed afterwards.
    pub fn finish_native(&mut self) {
        self.originals.clear();
        self.stat_cache.clear();
        self.template_cache.clear();
        self.stat_indices.clear();
        self.template_indices.clear();
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.finish_native();
        self.outcome = Outcome::Unavailable { reason };
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn required(&self) -> Capacity {
        self.required
    }
    pub fn originals(&self) -> &[OriginalObservation] {
        &self.originals
    }
    /// The observation of this division, from any hour; the borrower checks
    /// its sample hour and original pointer.
    pub fn original(&self, division: u64) -> Option<&OriginalObservation> {
        self.originals
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|index| self.originals.get(index))
    }
    /// Completed original observations for consumer fixtures, as a capture
    /// leaves them: sorted by division.
    #[cfg(test)]
    pub(crate) fn with_observations(observations: &[OriginalObservation]) -> Self {
        let mut packet = Self::vacant();
        packet.originals.extend_from_slice(observations);
        packet.originals.sort_unstable_by_key(|row| row.division);
        packet.outcome = Outcome::Rows {
            len: observations.len(),
        };
        packet
    }
    pub fn allocated_bytes(&self) -> usize {
        self.originals.capacity() * size_of::<OriginalObservation>()
            + self.stat_cache.capacity() * size_of::<Field<StatObservation>>()
            + self.template_cache.capacity() * size_of::<Field<TemplateObservation>>()
            + self.stat_indices.capacity() * (size_of::<(usize, usize)>() + 1)
            + self.template_indices.capacity() * (size_of::<(usize, usize)>() + 1)
    }
    fn template(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: Field<usize>,
        resolver: &mut creference_registry::ScopedResolver,
    ) -> Field<TemplateObservation> {
        let pointer = pointer?;
        if pointer == 0 {
            return Ok(TemplateObservation {
                wrapper: 0,
                handle: 0,
            });
        }
        if let Some(&index) = self.template_indices.get(&pointer) {
            #[cfg(test)]
            {
                self.template_probes += 1;
            }
            return self.template_cache[index];
        }
        let observed = (|| {
            if field::<u64>(memory, pointer, 0)? != expected(base, template::VTABLE_RVA)? {
                return Err("selected unit original template vtable mismatch");
            }
            let handle = field::<u64>(memory, pointer, template::TYPE)?;
            if handle >> 32 == 0 || handle as u32 != template::REFERENCE_TYPE {
                return Err("selected unit original template full handle invalid");
            }
            if resolver.resolve(memory, handle)? != Some(pointer) {
                return Err(
                    "selected unit original template registry does not identify original wrapper",
                );
            }
            Ok(TemplateObservation {
                wrapper: pointer,
                handle,
            })
        })();
        let index = self.template_cache.len();
        admitted(&mut self.template_cache, observed)?;
        // Census admission reserved one index entry per division.
        self.template_indices.insert(pointer, index);
        observed
    }
    /// The stat block's vtable is its only read; an alias shares the first
    /// observation, including a failure.
    fn stat(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: Field<usize>,
    ) -> Field<StatObservation> {
        let pointer = pointer?;
        if let Some(&index) = self.stat_indices.get(&pointer) {
            return self.stat_cache[index];
        }
        if self.stat_indices.len() == self.stat_indices.capacity() {
            return Err("selected unit original stat index capacity exhausted");
        }
        let observation = (|| {
            if field::<u64>(memory, pointer, 0)? != expected(base, stats::VTABLE_RVA)? {
                return Err("selected unit original stat block identity mismatch");
            }
            Ok(StatObservation { pointer })
        })();
        let index = self.stat_cache.len();
        admitted(&mut self.stat_cache, observation)?;
        self.stat_indices.insert(pointer, index);
        observation
    }
    /// The complete owned class-0 census of this pass, as the selected army's
    /// attack list visits it. The first unauthenticated seed ends the pass; the
    /// completed prefix stays sorted for its borrowers and the outcome names
    /// the refusal.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        source: Outcome,
    ) {
        self.clear();
        self.required.rows = source.len();
        let result = (|| {
            let count = match source {
                Outcome::Rows { len } => len,
                Outcome::Truncated { .. } => {
                    return Err("selected unit original division census incomplete");
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            let seeds = seeds
                .get(..count)
                .ok_or("selected unit original division census length invalid")?;
            if count == 0 {
                return Ok(Outcome::Rows { len: 0 });
            }
            if self.growing {
                self.reserve(Capacity { rows: count })
                    .map_err(|_| "selected unit original allocation failed")?;
            }
            if count > self.originals.capacity()
                || count > self.stat_cache.capacity()
                || count > self.template_cache.capacity()
                || count > self.stat_indices.capacity()
                || count > self.template_indices.capacity()
            {
                return Err("selected unit original census arena exhausted");
            }
            let mut resolver = creference_registry::ScopedResolver::new(base);
            for seed in seeds {
                authenticate_original(seed, base)?;
                let stat = self.stat(
                    memory,
                    base,
                    seed.stat_block
                        .ok_or("selected unit original stat selector unobserved"),
                );
                let template = self.template(
                    memory,
                    base,
                    seed.standard_template
                        .ok_or("selected unit original template selector unobserved"),
                    &mut resolver,
                );
                self.originals.push(OriginalObservation {
                    division: seed.division,
                    pointer: seed.pointer,
                    sample_hour: hour,
                    stat,
                    template,
                });
            }
            Ok(Outcome::Rows { len: count })
        })();
        // Sort every completed prefix, including failures, before transport
        // borrows it; a missed prefix must not authorize a native reread.
        self.originals.sort_unstable_by_key(|row| row.division);
        self.outcome = result
            .and_then(|outcome| {
                if self
                    .originals
                    .windows(2)
                    .any(|pair| pair[0].division == pair[1].division)
                {
                    Err("selected unit original identity duplicated")
                } else {
                    Ok(outcome)
                }
            })
            .unwrap_or_else(|reason| Outcome::Unavailable { reason });
    }
}

#[cfg(test)]
mod tests;
