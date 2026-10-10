//! Selected-attack map and country operands.
use super::*;

/// Own-country modifier keys 0x103/0x272 and region keys 0xAD/0x1DD.
pub const LAND_NIGHT_ATTACK: u32 = crate::modifier_keys::LAND_NIGHT_ATTACK;
pub const LAND_BUNKER_FACTOR: u32 = crate::modifier_keys::LAND_BUNKER_EFFECTIVENESS_FACTOR;
pub const WEATHER_ATTACK: u32 = crate::modifier_keys::ARMY_ATTACK_FACTOR;
pub const WEATHER_BREAKTHROUGH: u32 = crate::modifier_keys::BREAKTHROUGH_FACTOR;

#[derive(Clone, Copy, Debug)]
pub struct CountryRow {
    pub country: u32,
    pub canonical_slot: u32,
    pub sample_hour: i32,
    /// Undemanded scalars carry a named undemanded Result, never numerical0.
    pub land_night_attack: Field<i64>,
    pub land_bunker_effectiveness_factor: Field<i64>,
}

#[derive(Clone, Copy, Debug)]
pub struct ProvinceRow {
    pub province: u32,
    pub sample_hour: i32,
    pub fort_sum: Field<i32>,
    pub land_bunker_effectiveness_factor: Field<i64>,
}

#[derive(Clone, Copy, Debug)]
pub struct WeatherAttack {
    pub sample_hour: i32,
    pub attack: Field<i64>,
    pub breakthrough: Field<i64>,
}
impl Default for WeatherAttack {
    fn default() -> Self {
        Self::refused(0, UNSUPPLIED)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapCapacity {
    pub countries: usize,
    pub links: usize,
    pub provinces: usize,
}

#[derive(Clone, Copy)]
struct StateScalar {
    value: Field<i64>,
}

#[derive(Clone, Copy)]
struct CountryScalar {
    owner: ccountry::Country,
    demanded: u8,
    night: Field<i64>,
    bunker: Field<i64>,
}

/// Caller-local fixed-key metadata observations include refusal, unlike a
/// success-only cache. Never retained across a sample or content reload.
#[derive(Debug)]
pub struct ScalarMetadata {
    guard: Field<()>,
    operands: [Option<Field<crate::modifier_definition::Operands>>; 4],
    roots: Vec<RawRoot>,
    root_indices: HashMap<usize, usize>,
    required: usize,
    growing: bool,
}
#[derive(Debug)]
struct RawRoot {
    values: Field<[crate::modifier_accumulator::Value; 4]>,
}
impl Default for ScalarMetadata {
    fn default() -> Self {
        Self {
            guard: Err("selected attack modifier layout unverified"),
            operands: [None; 4],
            roots: Vec::new(),
            root_indices: HashMap::new(),
            required: 0,
            growing: false,
        }
    }
}
impl ScalarMetadata {
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn try_new(roots: usize) -> std::io::Result<Self> {
        let mut memo = Self::default();
        memo.reserve(roots)?;
        Ok(memo)
    }
    pub fn reserve(&mut self, roots: usize) -> std::io::Result<()> {
        reserve(&mut self.roots, roots)?;
        let additional = roots.saturating_sub(self.root_indices.len());
        crate::allocation::reserve_map(&mut self.root_indices, additional)
    }
    pub fn clear(&mut self) {
        self.finish_native();
        self.required = 0;
    }
    pub fn set_guard(&mut self, guard: Field<()>) {
        self.guard = guard;
    }
    /// End only native observation lifetime; caller growth demand survives.
    pub fn finish_native(&mut self) {
        self.operands.fill(None);
        self.roots.clear();
        self.root_indices.clear();
        self.guard = Err("selected attack modifier layout unverified");
    }
    pub fn required(&self) -> usize {
        self.required
    }
    pub fn allocated_bytes(&self) -> usize {
        self.roots.capacity() * size_of::<RawRoot>()
            + self.root_indices.capacity() * (size_of::<(usize, usize)>() + 1)
    }
    pub fn value(
        &mut self,
        memory: &impl Memory,
        base: usize,
        accumulator: usize,
        key: u32,
    ) -> Field<i64> {
        self.guard?;
        let slot = match key {
            LAND_NIGHT_ATTACK => 0,
            LAND_BUNKER_FACTOR => 1,
            WEATHER_ATTACK => 2,
            WEATHER_BREAKTHROUGH => 3,
            _ => return Err("selected attack unsupported modifier key"),
        };
        // lookup_effective_at's supported raw path scans the sorted collection
        // whole. Select the four fixed narrow keys in that SAME scan once,
        // retaining source faults. No tree or modifier list is serialized.
        let raw_slot = match self.root_indices.get(&accumulator).copied() {
            Some(index) => index,
            None => {
                self.required = self.required.saturating_add(1);
                if self.growing {
                    self.reserve(self.required)
                        .map_err(|_| "selected attack allocation failed")?;
                }
                if self.roots.len() == self.roots.capacity()
                    || self.root_indices.len() == self.root_indices.capacity()
                {
                    return Err("selected attack scalar root memo arena exhausted");
                }
                let mut values = [crate::modifier_accumulator::Value::default(); 4];
                let keys = [
                    WEATHER_ATTACK,
                    LAND_NIGHT_ATTACK,
                    WEATHER_BREAKTHROUGH,
                    LAND_BUNKER_FACTOR,
                ];
                let cached = keys.map(|key| memory.modifier_accumulator_value(accumulator, key));
                let values = if cached.iter().all(Option::is_some) {
                    (|| {
                        for (target, observed) in values.iter_mut().zip(cached) {
                            *target = observed.ok_or(UNSUPPLIED)??;
                        }
                        Ok(values)
                    })()
                } else {
                    // A partial owned fault may never authorize source retry.
                    match cached
                        .iter()
                        .flatten()
                        .find_map(|value| value.as_ref().err().copied())
                    {
                        Some(reason) => Err(reason),
                        None => crate::modifier_accumulator::lookup_many_at(
                            memory,
                            accumulator,
                            &keys,
                            &mut values,
                        )
                        .map(|_| values),
                    }
                };
                self.roots.push(RawRoot { values });
                self.root_indices.insert(accumulator, self.roots.len() - 1);
                self.roots.len() - 1
            }
        };
        let raw_index = match key {
            WEATHER_ATTACK => 0,
            LAND_NIGHT_ATTACK => 1,
            WEATHER_BREAKTHROUGH => 2,
            LAND_BUNKER_FACTOR => 3,
            _ => unreachable!(),
        };
        let value = self.roots[raw_slot].values?[raw_index];
        if !value.present {
            return Ok(0);
        }
        let operands = *self.operands[slot]
            .get_or_insert_with(|| crate::modifier_definition::operands(memory, base, key));
        Ok(operands?.effective(value.raw))
    }
}

impl WeatherAttack {
    pub fn refused(hour: i32, reason: &'static str) -> Self {
        Self {
            sample_hour: hour,
            attack: Err(reason),
            breakthrough: Err(reason),
        }
    }
    /// The caller's original dense region gather already authenticated this
    /// record's vtable/id. No vtable/count or row selector is retried here.
    pub fn capture_authenticated(
        memory: &impl Memory,
        base: usize,
        record: usize,
        hour: i32,
        metadata: &mut ScalarMetadata,
    ) -> Self {
        let root = at(record, crate::sweather_per_region::MODIFIERS);
        Self {
            sample_hour: hour,
            attack: root.and_then(|root| metadata.value(memory, base, root, WEATHER_ATTACK)),
            breakthrough: root
                .and_then(|root| metadata.value(memory, base, root, WEATHER_BREAKTHROUGH)),
        }
    }
}

pub struct MapPacket {
    countries: Vec<CountryRow>,
    provinces: Vec<ProvinceRow>,
    owners: Vec<Option<CountryScalar>>,
    raw: Vec<Option<Field<ccountry::Country>>>,
    states: Vec<StateScalar>,
    state_indices: HashMap<(usize, usize, u32), usize>,
    metadata: ScalarMetadata,
    required: MapCapacity,
    growing: bool,
    country_outcome: Outcome,
    province_outcome: Outcome,
    failures: [Option<&'static str>; 3],
}

const NIGHT: u8 = 1;
const BUNKER: u8 = 2;
const UNDEMANDED: &str = "selected attack modifier undemanded";

impl MapPacket {
    pub fn vacant() -> Self {
        Self {
            countries: Vec::new(),
            provinces: Vec::new(),
            owners: Vec::new(),
            raw: Vec::new(),
            states: Vec::new(),
            state_indices: HashMap::new(),
            metadata: ScalarMetadata::default(),
            required: MapCapacity::default(),
            growing: false,
            country_outcome: UNCAPTURED,
            province_outcome: UNCAPTURED,
            failures: [None; 3],
        }
    }
    pub fn enable_growth(&mut self) {
        self.growing = true;
        self.metadata.enable_growth();
    }
    pub fn try_new(capacity: MapCapacity) -> std::io::Result<Self> {
        let mut packet = Self::vacant();
        packet.reserve(capacity)?;
        Ok(packet)
    }
    pub fn reserve(&mut self, capacity: MapCapacity) -> std::io::Result<()> {
        reserve(&mut self.countries, capacity.countries)?;
        reserve(&mut self.provinces, capacity.provinces)?;
        reserve(&mut self.states, capacity.provinces)?;
        let additional = capacity.provinces.saturating_sub(self.state_indices.len());
        crate::allocation::reserve_map(&mut self.state_indices, additional)?;
        self.metadata.reserve(
            capacity
                .countries
                .checked_add(capacity.provinces)
                .ok_or_else(crate::allocation::oom)?,
        )?;
        // These are indexed scratch, lengths admitted off the acquisition.
        if self.owners.len() < capacity.countries {
            crate::allocation::resize(&mut self.owners, capacity.countries, None)?;
        }
        if self.raw.len() < capacity.links {
            crate::allocation::resize(&mut self.raw, capacity.links, None)?;
        }
        Ok(())
    }
    pub fn clear(&mut self) {
        self.countries.clear();
        self.provinces.clear();
        self.owners.fill(None);
        self.raw.fill(None);
        self.states.clear();
        self.state_indices.clear();
        self.metadata.clear();
        self.required = MapCapacity::default();
        self.country_outcome = UNCAPTURED;
        self.province_outcome = UNCAPTURED;
        self.failures.fill(None);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.clear();
        self.country_outcome = Outcome::Unavailable { reason };
        self.province_outcome = Outcome::Unavailable { reason };
        self.failures.fill(Some(reason));
    }
    pub fn required(&self) -> MapCapacity {
        self.required
    }
    pub fn countries(&self) -> &[CountryRow] {
        &self.countries
    }
    pub fn provinces(&self) -> &[ProvinceRow] {
        &self.provinces
    }
    pub fn outcomes(&self) -> [Outcome; 2] {
        [self.country_outcome, self.province_outcome]
    }
    pub fn failures(&self) -> &[Option<&'static str>; 3] {
        &self.failures
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.capacity() * size_of::<CountryRow>()
            + self.provinces.capacity() * size_of::<ProvinceRow>()
            + self.owners.capacity() * size_of::<Option<CountryScalar>>()
            + self.raw.capacity() * size_of::<Option<Field<ccountry::Country>>>()
            + self.states.capacity() * size_of::<StateScalar>()
            + self.state_indices.capacity() * (size_of::<((usize, usize, u32), usize)>() + 1)
            + self.metadata.allocated_bytes()
    }
    pub fn finish_native(&mut self) {
        self.owners.fill(None);
        self.raw.fill(None);
        self.states.clear();
        self.state_indices.clear();
        self.metadata.finish_native();
    }
    /// Explicit reserved-country path. A readable zero link selects the
    /// authenticated reserved object by the same lookup rule; it never
    /// supplies a neutral or null result.
    fn owner<M: Memory>(
        &mut self,
        countries: &ccountry::Reader<'_, M>,
        raw: u32,
        loan: &mut Option<crate::raw_country_observations::MapLoan<'_>>,
    ) -> Field<ccountry::Country> {
        let slot = self
            .raw
            .get_mut(raw as usize)
            .ok_or("selected attack raw owner outside link scratch")?;
        if let Some(observed) = *slot {
            return observed;
        }
        if let Some(loan) = loan.as_mut() {
            loan.admit(raw)?;
        }
        let observed = if raw == 0 {
            countries.reserved()
        } else {
            countries.link(raw).and_then(|slot| match slot {
                None => countries.reserved(),
                Some(slot) => countries
                    .read_at(slot)?
                    .ok_or("selected attack owner link selects null country"),
            })
        };
        if let Some(loan) = loan.as_mut() {
            loan.observe(raw, observed);
        }
        *slot = Some(observed);
        observed
    }
    fn demand<M: Memory>(
        &mut self,
        countries: &ccountry::Reader<'_, M>,
        raw: u32,
        mask: u8,
        loan: &mut Option<crate::raw_country_observations::MapLoan<'_>>,
    ) -> Field<()> {
        let owner = self.owner(countries, raw, loan)?;
        let slot = self
            .owners
            .get_mut(owner.canonical_slot as usize)
            .ok_or("selected attack canonical country outside scratch")?;
        match slot {
            Some(existing) if existing.owner != owner => {
                return Err("selected attack country identity changed");
            }
            Some(existing) => existing.demanded |= mask,
            None => {
                *slot = Some(CountryScalar {
                    owner,
                    demanded: mask,
                    night: Err(UNDEMANDED),
                    bunker: Err(UNDEMANDED),
                })
            }
        }
        Ok(())
    }
    fn state(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        state: usize,
        controller: ccountry::Country,
    ) -> Field<i64> {
        self.metadata.guard?;
        if state == 0 {
            return Ok(0);
        }
        let identity = (state, controller.pointer, controller.id);
        if let Some(&index) = self.state_indices.get(&identity) {
            return self.states[index].value;
        }
        if self.states.len() == self.states.capacity()
            || self.state_indices.len() == self.state_indices.capacity()
        {
            return Err("selected attack state memo arena exhausted");
        }
        // Reuse the actual controller's stored +8 ID authenticated by Reader,
        // never the selecting raw alias. State lookup owns hash and country
        // probes; no custom country override is inferred.
        let value = crate::cstate::movement_modifier_with(
            memory,
            base,
            game_state,
            state,
            controller.id,
            LAND_BUNKER_FACTOR,
            |root, key| self.metadata.value(memory, base, root, key),
        );
        admitted(&mut self.states, StateScalar { value })?;
        self.state_indices.insert(identity, self.states.len() - 1);
        value
    }
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        divisions: Outcome,
        provinces: &[cprovince::Row],
        province_source: Outcome,
        guard: Field<()>,
    ) {
        self.capture_inner(
            memory,
            base,
            game_state,
            hour,
            seeds,
            divisions,
            provinces,
            province_source,
            guard,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn capture_with_observations(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        divisions: Outcome,
        provinces: &[cprovince::Row],
        province_source: Outcome,
        guard: Field<()>,
        journal: &mut crate::raw_country_observations::Journal,
    ) {
        self.capture_inner(
            memory,
            base,
            game_state,
            hour,
            seeds,
            divisions,
            provinces,
            province_source,
            guard,
            Some(journal.map_loan()),
        );
        journal.sort();
        journal.seal();
    }
    #[allow(clippy::too_many_arguments)]
    fn capture_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        divisions: Outcome,
        provinces: &[cprovince::Row],
        province_source: Outcome,
        guard: Field<()>,
        mut loan: Option<crate::raw_country_observations::MapLoan<'_>>,
    ) {
        self.clear();
        self.metadata.set_guard(guard);
        self.required.provinces = province_source.len();
        let result = (|| {
            let countries = match loan.as_ref() {
                Some(loan) => {
                    ccountry::Reader::with_link_observations(memory, base, game_state, loan.links)
                }
                None => ccountry::Reader::new(memory, base, game_state),
            }?;
            self.required.countries = countries.count() as usize;
            self.required.links = countries.arrays()[1].count as usize;
            // Use the original country and province observations; reserve their
            // complete suffix before publication.
            if self.growing {
                self.reserve(self.required)
                    .map_err(|_| "selected attack allocation failed")?;
            }
            if self.required.countries > self.owners.len()
                || self.required.links > self.raw.len()
                || self.required.countries > self.countries.capacity()
            {
                return Err("selected attack country admission arena exhausted");
            }
            // The class0 owner demand has its own completeness outcome. A
            // failed union prevents any country suffix completeness claim.
            let count = match divisions {
                Outcome::Rows { len } => len,
                Outcome::Truncated { .. } => {
                    return Err("selected attack owner division census incomplete");
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for seed in seeds
                .get(..count)
                .ok_or("selected attack owner division census length invalid")?
            {
                authenticate_original(seed, base)?;
                let expeditionary = seed
                    .expeditionary_owner
                    .ok_or("selected attack effective owner expeditionary selector unobserved")?;
                let raw = if (expeditionary as i32) > 0 {
                    expeditionary
                } else {
                    seed.roster_owner
                        .ok_or("selected attack effective owner roster selector unobserved")?
                };
                if let Err(reason) = self.demand(&countries, raw, NIGHT, &mut loan) {
                    self.failures[0].get_or_insert(reason);
                }
            }
            let province_count = match province_source {
                Outcome::Rows { len } => len,
                Outcome::Truncated { .. } => {
                    return Err("selected attack fortified province census incomplete");
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            let provinces = provinces
                .get(..province_count)
                .ok_or("selected attack province census length invalid")?;
            if province_count > self.provinces.capacity() || province_count > self.states.capacity()
            {
                return Err("selected attack province suffix arena exhausted");
            }
            if provinces.windows(2).any(|pair| pair[0].id >= pair[1].id) {
                return Err("selected attack province census identities invalid");
            }
            for province in provinces {
                // Fort modifiers are reached only after the signed positive target-
                // controller gate. Positive aliases may still resolve to the genuine
                // reserved country object.
                if (province.controller as i32) > 0
                    && province.attack_fort_sum.is_some_and(|sum| sum > 0)
                    && let Err(reason) =
                        self.demand(&countries, province.controller, BUNKER, &mut loan)
                {
                    self.failures[1].get_or_insert(reason);
                }
            }
            for entry in self.owners.iter_mut().flatten() {
                let root = at(entry.owner.pointer, ccountry::MODIFIER_ACCUMULATOR);
                if entry.demanded & NIGHT != 0 {
                    entry.night = root.and_then(|root| {
                        self.metadata.value(memory, base, root, LAND_NIGHT_ATTACK)
                    });
                }
                if entry.demanded & BUNKER != 0 {
                    entry.bunker = root.and_then(|root| {
                        self.metadata.value(memory, base, root, LAND_BUNKER_FACTOR)
                    });
                }
                self.countries.push(CountryRow {
                    country: entry.owner.id,
                    canonical_slot: entry.owner.canonical_slot,
                    sample_hour: hour,
                    land_night_attack: entry.night,
                    land_bunker_effectiveness_factor: entry.bunker,
                });
            }
            self.countries.sort_unstable_by_key(|row| row.country);
            for province in provinces {
                let fort_sum = province
                    .attack_fort_sum
                    .ok_or("selected attack same-read healthy fort sum unavailable");
                let state = if (province.controller as i32) <= 0
                    || fort_sum.is_ok_and(|sum| sum <= 0)
                {
                    Err(UNDEMANDED)
                } else {
                    fort_sum.and_then(|_| self.owner(&countries, province.controller, &mut loan))
                        .and_then(|controller| province.attack_state.ok_or("selected attack province state selecting observation unavailable")
                            .and_then(|state| self.state(memory, base, game_state, state, controller)))
                };
                if let Err(reason) = state
                    && reason != UNDEMANDED
                {
                    self.failures[2].get_or_insert(reason);
                }
                self.provinces.push(ProvinceRow {
                    province: province.id,
                    sample_hour: hour,
                    fort_sum,
                    land_bunker_effectiveness_factor: state,
                });
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.country_outcome = Outcome::Rows {
                    len: self.countries.len(),
                };
                self.province_outcome = Outcome::Rows {
                    len: self.provinces.len(),
                };
            }
            Err(reason) => {
                self.countries.clear();
                self.provinces.clear();
                self.country_outcome = Outcome::Unavailable { reason };
                self.province_outcome = Outcome::Unavailable { reason };
            }
        }
        self.finish_native();
    }
}
