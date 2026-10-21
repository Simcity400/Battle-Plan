//! Transport speeds for one owner in chosen strategic regions.
//!
//! [`movement_speeds`] answers [`movement_speed`] for each chosen region. The
//! owner's country, the base speed, the transport penalty gates, the owner's
//! and every enemy's region-independent operands and the conditional defines
//! are read once per owner; the region census, the weather manager's region
//! array, the speed key's metadata, each region's identity and weather and
//! each navy's regional operands once for every owner ([`SpeedCache`]). Within
//! one quiescent acquisition every reused read returns what a fresh read
//! would, and each region takes its reads in [`movement_speed`]'s order, so a
//! region's speed or first refusal is exactly the single query's.

use super::*;

/// A strategic region chosen for a query.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportRegion {
    pub id: u32,
    /// Borrowed acquisition input; it never leaves the acquisition.
    pub pointer: usize,
}

/// The strategic region holding `province`, from the game state's province
/// array: an authenticated province with that id and its region pointer.
pub fn province_region(
    memory: &impl Memory,
    base: usize,
    state: usize,
    province: u32,
) -> Result<TransportRegion, &'static str> {
    let root = at(state, cgame_state::PROVINCES)?;
    let provinces = Array::read(memory, root, 8).ok_or("transport province array invalid")?;
    let entry = provinces
        .element(province, 8)
        .ok_or("transport province outside the province array")?;
    let pointer =
        usize::try_from(read::<u64>(memory, entry).ok_or("transport province entry unreadable")?)
            .map_err(|_| "transport province pointer invalid")?;
    if pointer == 0
        || value::<u64>(memory, pointer, 0)? != at(base, cprovince::VTABLE_RVA)? as u64
        || value::<u32>(memory, pointer, cprovince::ID)? != province
    {
        return Err("transport province identity mismatch");
    }
    let region = usize::try_from(value::<u64>(memory, pointer, cprovince::STRATEGIC_REGION)?)
        .map_err(|_| "transport region pointer invalid")?;
    let id = cstrategic_region::id(memory, base, region)?
        .ok_or("transport province has no strategic region")?;
    Ok(TransportRegion {
        id,
        pointer: region,
    })
}

/// A shared read, taken the first time a region reaches it.
fn memo<T: Copy>(
    slot: &mut Option<Result<T, &'static str>>,
    read: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    *slot.get_or_insert_with(read)
}

/// One enemy of the owner, as the regional totals visit it.
#[derive(Clone, Copy)]
enum Enemy {
    /// No fog network, or capitulated: the totals skip it.
    Skipped,
    Counted {
        weight: i64,
        regions: Array,
    },
}

/// The penalty's region-independent prefix: `None` when a gate leaves it out.
#[derive(Clone, Copy)]
struct Penalty {
    definition: usize,
}

struct Shared<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    owner: ccountry::Country,
    countries: &'a ccountry::Reader<'a, M>,
    speed: Option<Result<i64, &'static str>>,
    penalty: Option<Result<Option<Penalty>, &'static str>>,
    /// The enemy roster up to and including its first refusal.
    enemies: Option<Result<Vec<Result<Enemy, &'static str>>, &'static str>>,
    maximum: Option<Result<i64, &'static str>>,
    reduction: Option<Result<i64, &'static str>>,
    minimum: Option<Result<i64, &'static str>>,
    upper: Option<Result<i64, &'static str>>,
    scale: Option<Result<i64, &'static str>>,
    raw: Option<Result<modifier_accumulator::Value, &'static str>>,
    category: Option<Result<i64, &'static str>>,
    operands: Option<Result<modifier_definition::Operands, &'static str>>,
    cache: &'a mut SpeedCache,
}

/// What every owner's query of one hour shares: each region's identity and
/// weather (neither depends on the owner), the region census, the weather
/// region array, the speed key's metadata and each navy's regional operands,
/// copied whole once. Valid for one acquisition and one region slice;
/// [`SpeedCache::begin`] forgets it.
#[derive(Default)]
pub struct SpeedCache {
    regions: Vec<RegionTerms>,
    census: Option<Result<Array, &'static str>>,
    weather_regions: Option<Result<Array, &'static str>>,
    /// Successful speed-key metadata reads; a failure is read again.
    definitions: modifier_definition::DefinitionMemo,
    /// Each navy's regional operand array by data address, copied whole, or
    /// `None` where the whole copy failed and each operand is read alone.
    navies: Vec<(usize, Option<Vec<u8>>)>,
}

#[derive(Clone, Copy, Default)]
struct RegionTerms {
    identity: Option<Result<u32, &'static str>>,
    weather: Option<Result<i64, &'static str>>,
}

impl SpeedCache {
    /// Forget the previous acquisition and size for `regions` regions, the
    /// slice every owner's [`movement_speeds`] call of this acquisition takes.
    pub fn begin(&mut self, regions: usize) -> Result<(), &'static str> {
        self.regions.clear();
        self.regions
            .try_reserve(regions)
            .map_err(|_| "transport speed cache allocation failed")?;
        self.regions.resize(regions, RegionTerms::default());
        self.census = None;
        self.weather_regions = None;
        self.definitions.clear();
        self.navies.clear();
        Ok(())
    }

    pub fn allocated_bytes(&self) -> usize {
        self.regions.capacity() * size_of::<RegionTerms>()
            + self.definitions.allocated_bytes()
            + self
                .navies
                .iter()
                .map(|(_, copy)| copy.as_ref().map_or(0, Vec::capacity))
                .sum::<usize>()
            + self.navies.capacity() * size_of::<(usize, Option<Vec<u8>>)>()
    }

    /// No later query may use this acquisition. Release owned navy copies on
    /// the consumer's thread rather than at the next parked begin().
    pub fn finish(&mut self) {
        self.regions.clear();
        self.census = None;
        self.weather_regions = None;
        self.navies.clear();
        self.definitions.clear();
    }

    /// One navy's operand for `region`, as a single guarded read returns it.
    fn operand(
        &mut self,
        memory: &impl Memory,
        regions: Array,
        region: u32,
    ) -> Result<i64, &'static str> {
        let at = regions
            .element(region, 8)
            .ok_or("navy regional operand outside array")?;
        let index = match self
            .navies
            .iter()
            .position(|(data, _)| *data == regions.data)
        {
            Some(index) => index,
            None => {
                let len = regions.count as usize;
                let mut bytes = Vec::new();
                let copied = bytes.try_reserve(len * 8).is_ok() && {
                    bytes.resize(len * 8, 0_u8);
                    crate::read_slice(memory, regions.data, &mut bytes)
                };
                let copy = copied.then_some(bytes);
                self.navies
                    .try_reserve(1)
                    .map_err(|_| "transport speed cache allocation failed")?;
                self.navies.push((regions.data, copy));
                self.navies.len() - 1
            }
        };
        match &self.navies[index].1 {
            Some(copy) => {
                let at = region as usize * 8;
                Ok(i64::from_le_bytes(copy[at..at + 8].try_into().unwrap()))
            }
            None => read::<i64>(memory, at).ok_or("navy regional operand unreadable"),
        }
    }
}

impl<M: Memory> Shared<'_, M> {
    /// [`weather_modifier`] for one region, its manager and region array read
    /// once for the walk.
    fn weather(&mut self, region: u32) -> Result<i64, &'static str> {
        let (memory, base, state) = (self.memory, self.base, self.state);
        let array = memo(&mut self.cache.weather_regions, || {
            let manager = pointer(memory, state, cgame_state::WEATHER_MANAGER)?;
            if value::<u64>(memory, manager, 0)? != at(base, cweather_manager::VTABLE_RVA)? as u64 {
                return Err("transport weather manager identity mismatch");
            }
            let root = at(manager, cweather_manager::REGIONS)?;
            Array::read(memory, root, sweather_per_region::STRIDE)
                .ok_or("transport weather array invalid")
        })?;
        let Some(row) = array.element(region, sweather_per_region::STRIDE) else {
            // Out-of-range regions yield no weather modifier.
            return Ok(0);
        };
        if value::<u64>(memory, row, 0)? != at(base, sweather_per_region::VTABLE_RVA)? as u64
            || value::<u32>(memory, row, sweather_per_region::ID)? != region
        {
            return Err("transport weather region identity mismatch");
        }
        let raw = modifier_accumulator::lookup_at(
            memory,
            at(row, sweather_per_region::MODIFIERS)?,
            SPEED_TOKEN,
        )?;
        if !raw.present {
            return Ok(0);
        }
        modifier_definition::effective_memo(
            memory,
            base,
            SPEED_TOKEN,
            raw.raw,
            &self.cache.definitions,
        )
    }

    fn penalty(&mut self) -> Result<Option<Penalty>, &'static str> {
        let (memory, base, state, owner, countries) = (
            self.memory,
            self.base,
            self.state,
            self.owner,
            self.countries,
        );
        memo(&mut self.penalty, || {
            let flags = value::<u32>(memory, base, modifier_definition::TRADE_FEATURE_FLAGS_RVA)?;
            if flags & (1 << 4) == 0 {
                return Ok(None);
            }
            let definitions = pointer(memory, base, MODIFIER_SET_RVA)?;
            let definition = value::<u64>(memory, definitions, TRANSPORT_MODIFIER)?;
            if definition == 0 {
                return Ok(None);
            }
            let definition =
                usize::try_from(definition).map_err(|_| "transport modifier pointer invalid")?;
            if value::<u64>(memory, definition, 0)?
                != at(base, cstatic_modifier::VTABLE_RVA)? as u64
            {
                return Err("transport modifier identity mismatch");
            }
            selected_navy(memory, base, state, owner, countries)?;
            Ok(Some(Penalty { definition }))
        })
    }

    fn enemies(&mut self) -> Result<&[Result<Enemy, &'static str>], &'static str> {
        let (memory, base, state, owner, countries) = (
            self.memory,
            self.base,
            self.state,
            self.owner,
            self.countries,
        );
        let enemies = self.enemies.get_or_insert_with(|| {
            let owner_intel = intel(memory, base, owner)?;
            let relations = diplomacy(memory, base, owner)?;
            let root = at(relations, cdiplomacy_status::ENEMY_COUNTRIES)?;
            let array = Array::read(memory, root, 4).ok_or("transport enemy roster invalid")?;
            let mut enemies = Vec::new();
            enemies
                .try_reserve(array.count as usize)
                .map_err(|_| "transport enemy roster allocation failed")?;
            for index in 0..array.count {
                let enemy = (|| {
                    let raw = read::<u32>(
                        memory,
                        array
                            .element(index, 4)
                            .ok_or("transport enemy index overflow")?,
                    )
                    .ok_or("transport enemy reference unreadable")?;
                    let country = countries.resolve(raw)?;
                    if value::<i32>(memory, country.pointer, ccountry::FOG_NETWORK_TABLE_COUNT)?
                        <= 0
                    {
                        return Ok(Enemy::Skipped);
                    }
                    let enemy_relations = diplomacy(memory, base, country)?;
                    if value::<u8>(memory, enemy_relations, cdiplomacy_status::CAPITULATED)? != 0 {
                        return Ok(Enemy::Skipped);
                    }
                    let enemy_intel = intel(memory, base, country)?;
                    let weight = intel_lane(memory, countries, owner_intel, enemy_intel, country)?
                        .wrapping_sub(intel_lane(
                            memory,
                            countries,
                            enemy_intel,
                            owner_intel,
                            owner,
                        )?);
                    let navy = selected_navy(memory, base, state, country, countries)?;
                    let regions = cstrategic_navy::regional_array(memory, navy)?;
                    Ok(Enemy::Counted { weight, regions })
                })();
                let refused = enemy.is_err();
                enemies.push(enemy);
                if refused {
                    // The totals stop at this enemy for every region.
                    break;
                }
            }
            Ok(enemies)
        });
        match enemies {
            Ok(enemies) => Ok(enemies),
            Err(reason) => Err(*reason),
        }
    }

    /// [`totals`] for one region.
    fn totals(&mut self, region: u32) -> Result<(i64, i64), &'static str> {
        let memory = self.memory;
        let mut total = 0_i64;
        let mut weighted = 0_i64;
        let count = self.enemies()?.len();
        for index in 0..count {
            let enemy = match &self.enemies {
                Some(Ok(enemies)) => enemies[index],
                _ => unreachable!("the roster was read above"),
            };
            let (weight, regions) = match enemy {
                Err(reason) => return Err(reason),
                Ok(Enemy::Skipped) => continue,
                Ok(Enemy::Counted { weight, regions }) => (weight, regions),
            };
            let operand = self.cache.operand(memory, regions, region)?;
            total = total.wrapping_add(operand);
            weighted = weighted.wrapping_add(multiply(operand, weight));
        }
        Ok((
            total,
            if total == 0 {
                0
            } else {
                fraction(weighted, total)?
            },
        ))
    }

    /// [`composed_inputs`] for one region.
    fn composed(&mut self, region: u32) -> Result<ComposedInputs, &'static str> {
        let (memory, base, owner) = (self.memory, self.base, self.owner);
        let Some(Penalty { definition }) = self.penalty()? else {
            return Ok(ComposedInputs::Zero);
        };
        let (total, mean) = self.totals(region)?;
        let maximum = memo(&mut self.maximum, || {
            value::<i64>(memory, base, MAXIMUM_TOTAL_RVA)
        })?;
        let bounded = if total < 0 {
            0
        } else if total > maximum {
            maximum
        } else {
            total
        };
        let first_fraction = fraction(bounded, maximum)?;
        let reduction = if first_fraction != 0 {
            memo(&mut self.reduction, || {
                Ok(modifier_accumulator::lookup_effective_at(
                    memory,
                    base,
                    at(owner.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
                    REDUCTION_TOKEN,
                )?
                .effective)
            })?
        } else {
            0
        };
        let (minimum, upper, scale) = if first_fraction != 0 {
            let minimum = memo(&mut self.minimum, || {
                value::<i64>(memory, base, MINIMUM_INTEL_RVA)
            })?;
            if mean.wrapping_sub(minimum) > 0 {
                (
                    minimum,
                    memo(&mut self.upper, || {
                        value::<i64>(memory, base, MAXIMUM_INTEL_RVA)
                    })?,
                    memo(&mut self.scale, || {
                        value::<i64>(memory, base, INTEL_REDUCTION_RVA)
                    })?,
                )
            } else {
                (minimum, minimum, 0)
            }
        } else {
            (0, 0, 0)
        };
        let fraction = reduction_fraction(total, mean, maximum, reduction, minimum, upper, scale)?;
        if fraction <= 0 {
            return Ok(ComposedInputs::Zero);
        }
        let raw = memo(&mut self.raw, || {
            modifier_accumulator::lookup_at(
                memory,
                at(definition, cstatic_modifier::ACCUMULATOR)?,
                SPEED_TOKEN,
            )
        })?;
        if !raw.present {
            return Ok(ComposedInputs::Zero);
        }
        let category = memo(&mut self.category, || {
            let metadata = pointer(memory, base, modifier_definition::TABLE_RVA)?;
            let category_at = at(
                at(metadata, SPEED_TOKEN as usize * modifier_definition::STRIDE)?,
                modifier_definition::CATEGORY_MASK,
            )?;
            read::<u32>(memory, category_at)
                .map(i64::from)
                .ok_or("transport modifier category unreadable")
        })?;
        if category == 0 {
            return Ok(ComposedInputs::Zero);
        }
        let operands = memo(&mut self.operands, || {
            modifier_definition::operands(memory, base, SPEED_TOKEN)
        })?;
        Ok(ComposedInputs::Modifier {
            raw: raw.raw,
            fraction,
            operands,
        })
    }
}

/// [`movement_speed`] for one owner at each of `wanted`, indices into
/// `regions`, in order: `emit` receives each region with its own speed or
/// refusal. Every call sharing `cache` passes the same `regions`.
#[allow(
    clippy::too_many_arguments,
    reason = "One owner's query over the shared region slice, its chosen indices and the hour's cache"
)]
pub fn movement_speeds<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    owner: u32,
    regions: &[TransportRegion],
    wanted: &[usize],
    cache: &mut SpeedCache,
    mut emit: impl FnMut(TransportRegion, Result<i64, &'static str>),
) {
    movement_speeds_inputs(
        memory, base, state, owner, regions, wanted, cache,
        |region, inputs| emit(region, inputs.map(SpeedInputs::evaluate)),
    );
}

/// Native acquisition of movement_speeds, retaining its final arithmetic for
/// an owned-data consumer. All gates and shared-memo decisions stay here.
#[allow(clippy::too_many_arguments)]
pub fn movement_speeds_inputs<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    owner: u32,
    regions: &[TransportRegion],
    wanted: &[usize],
    cache: &mut SpeedCache,
    mut emit: impl FnMut(TransportRegion, Result<SpeedInputs, &'static str>),
) {
    if cache.regions.len() != regions.len()
        && let Err(reason) = cache.begin(regions.len())
    {
        for &index in wanted {
            emit(regions[index], Err(reason));
        }
        return;
    }
    let resolved = ccountry::Reader::new(memory, base, state).and_then(|reader| {
        let countries = ccountry::Reader::from_arrays(memory, base, state, reader.arrays())?;
        let country = countries.resolve(owner)?;
        Ok((countries, country))
    });
    let (countries, country) = match resolved {
        Ok(resolved) => resolved,
        Err(reason) => {
            for &index in wanted {
                emit(regions[index], Err(reason));
            }
            return;
        }
    };
    let mut shared = Shared {
        memory,
        base,
        state,
        owner: country,
        countries: &countries,
        speed: None,
        penalty: None,
        enemies: None,
        maximum: None,
        reduction: None,
        minimum: None,
        upper: None,
        scale: None,
        raw: None,
        category: None,
        operands: None,
        cache,
    };
    for &index in wanted {
        let region = regions[index];
        let result = (|| {
            let identity = shared.cache.regions[index].identity;
            let region_id = match identity {
                Some(identity) => identity,
                None => {
                    let identity = region_identity(memory, base, state, region, shared.cache);
                    shared.cache.regions[index].identity = Some(identity);
                    identity
                }
            }?;
            let speed = memo(&mut shared.speed, || {
                value::<i64>(memory, base, BASE_SPEED_RVA)
            })?;
            let weather = match shared.cache.regions[index].weather {
                Some(weather) => weather,
                None => {
                    let weather = shared.weather(region_id);
                    shared.cache.regions[index].weather = Some(weather);
                    weather
                }
            }?;
            let composed = shared.composed(region_id)?;
            Ok(SpeedInputs { speed, weather, composed })
        })();
        emit(region, result);
    }
}

/// A census region's identity check, as [`movement_speed`] makes it.
fn region_identity(
    memory: &impl Memory,
    base: usize,
    state: usize,
    region: TransportRegion,
    cache: &mut SpeedCache,
) -> Result<u32, &'static str> {
    let pointer = region.pointer;
    let region_id = value::<u32>(memory, pointer, cstrategic_region::ID)?;
    let slot = value::<u32>(memory, pointer, cstrategic_region::RUNTIME_SLOT)?;
    let census = memo(&mut cache.census, || {
        let root = at(state, cgame_state::STRATEGIC_REGIONS)?;
        Array::read(memory, root, 8).ok_or("transport region census invalid")
    })?;
    let entry = census
        .element(slot, 8)
        .ok_or("transport region slot outside census")?;
    let vtable = at(base, cstrategic_region::VTABLE_RVA)? as u64;
    if pointer == 0
        || region_id > i32::MAX as u32
        || read::<u64>(memory, entry) != Some(pointer as u64)
        || read::<u64>(memory, pointer) != Some(vtable)
    {
        return Err("transport region identity mismatch");
    }
    Ok(region_id)
}
