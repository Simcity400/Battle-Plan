
use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportRegion {
    pub id: u32,
    pub pointer: usize,
}

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
        || value::<u64>(memory, pointer, 0)? != key(base, cprovince::VTABLE_RVA)? as u64
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

fn memo<T: Copy>(
    slot: &mut Option<Result<T, &'static str>>,
    read: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    *slot.get_or_insert_with(read)
}

#[derive(Clone, Copy)]
enum Enemy {
    Skipped,
    Counted {
        weight: i64,
        regions: Array,
    },
}

#[derive(Clone, Copy)]
struct Penalty {
    definition: usize,
}

struct Shared<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    owner: engine_native_memory::country_identity::Country,
    countries: &'a ccountry::Reader<'a, M>,
    speed: Option<Result<i64, &'static str>>,
    penalty: Option<Result<Option<Penalty>, &'static str>>,
    enemies: Option<Result<Vec<Result<Enemy, &'static str>>, &'static str>>,
    maximum: Option<Result<i64, &'static str>>,
    reduction: Option<Result<i64, &'static str>>,
    minimum: Option<Result<i64, &'static str>>,
    upper: Option<Result<i64, &'static str>>,
    scale: Option<Result<i64, &'static str>>,
    raw: Option<Result<engine_native_memory::modifier_cache::Value, &'static str>>,
    category: Option<Result<i64, &'static str>>,
    operands: Option<Result<modifier_definition::Operands, &'static str>>,
    cache: &'a mut SpeedCache,
}

#[derive(Default)]
pub struct SpeedCache {
    regions: Vec<RegionTerms>,
    census: Option<Result<Array, &'static str>>,
    weather_regions: Option<Result<Array, &'static str>>,
    definitions: modifier_definition::DefinitionMemo,
    navies: Vec<(usize, Option<Vec<u8>>)>,
}

#[derive(Clone, Copy, Default)]
struct RegionTerms {
    identity: Option<Result<u32, &'static str>>,
    weather: Option<Result<i64, &'static str>>,
}

impl SpeedCache {
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

    pub fn finish(&mut self) {
        self.regions.clear();
        self.census = None;
        self.weather_regions = None;
        self.navies.clear();
        self.definitions.clear();
    }

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
                    engine_native_memory::read_slice(memory, regions.data, &mut bytes)
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
    fn weather(&mut self, region: u32) -> Result<i64, &'static str> {
        let (memory, base, state) = (self.memory, self.base, self.state);
        let array = memo(&mut self.cache.weather_regions, || {
            let manager = pointer(memory, state, cgame_state::WEATHER_MANAGER)?;
            if value::<u64>(memory, manager, 0)? != key(base, cweather_manager::VTABLE_RVA)? as u64 {
                return Err("transport weather manager identity mismatch");
            }
            let root = at(manager, cweather_manager::REGIONS)?;
            Array::read(memory, root, sweather_per_region::STRIDE)
                .ok_or("transport weather array invalid")
        })?;
        let Some(row) = array.element(region, sweather_per_region::STRIDE) else {
            return Ok(0);
        };
        if value::<u64>(memory, row, 0)? != key(base, sweather_per_region::VTABLE_RVA)? as u64
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
        let (memory, base, state, owner) = (self.memory, self.base, self.state, self.owner);
        memo(&mut self.penalty, || {
            let flags = global::<u32>(memory, base, modifier_definition::TRADE_FEATURE_FLAGS_RVA)?;
            if flags & (1 << 4) == 0 {
                return Ok(None);
            }
            let definitions = global_pointer(memory, base, MODIFIER_SET_RVA)?;
            let definition = value::<u64>(memory, definitions, TRANSPORT_MODIFIER)?;
            if definition == 0 {
                return Ok(None);
            }
            let definition =
                usize::try_from(definition).map_err(|_| "transport modifier pointer invalid")?;
            if value::<u64>(memory, definition, 0)?
                != key(base, cstatic_modifier::VTABLE_RVA)? as u64
            {
                return Err("transport modifier identity mismatch");
            }
            cstrategic_navy_manager::navy(memory, base, state, owner)?;
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
            let root = at(relations, engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES)?;
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
                    let navy = cstrategic_navy_manager::navy(memory, base, state, country)?;
                    let regions = cstrategic_navy::regional_array(memory, navy)?;
                    Ok(Enemy::Counted { weight, regions })
                })();
                let refused = enemy.is_err();
                enemies.push(enemy);
                if refused {
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

    fn composed(&mut self, region: u32) -> Result<ComposedInputs, &'static str> {
        let (memory, base, owner) = (self.memory, self.base, self.owner);
        let Some(Penalty { definition }) = self.penalty()? else {
            return Ok(ComposedInputs::Zero);
        };
        let (total, mean) = self.totals(region)?;
        let maximum = memo(&mut self.maximum, || {
            global::<i64>(memory, base, MAXIMUM_TOTAL_RVA)
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
                    at(owner.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
                    REDUCTION_TOKEN,
                )?
                .effective)
            })?
        } else {
            0
        };
        let (minimum, upper, scale) = if first_fraction != 0 {
            let minimum = memo(&mut self.minimum, || {
                global::<i64>(memory, base, MINIMUM_INTEL_RVA)
            })?;
            if mean.wrapping_sub(minimum) > 0 {
                (
                    minimum,
                    memo(&mut self.upper, || {
                        global::<i64>(memory, base, MAXIMUM_INTEL_RVA)
                    })?,
                    memo(&mut self.scale, || {
                        global::<i64>(memory, base, INTEL_REDUCTION_RVA)
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
            let metadata = global_pointer(memory, base, modifier_definition::TABLE_RVA)?;
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

#[expect(
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

#[expect(
    clippy::too_many_arguments,
    reason = "the owner, regions and wanted list select the speeds, and the cache and emit callback are caller-owned"
)]
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
                global::<i64>(memory, base, BASE_SPEED_RVA)
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
    let vtable = key(base, cstrategic_region::VTABLE_RVA)? as u64;
    if pointer == 0
        || region_id > i32::MAX as u32
        || read::<u64>(memory, entry) != Some(pointer as u64)
        || read::<u64>(memory, pointer) != Some(vtable)
    {
        return Err("transport region identity mismatch");
    }
    Ok(region_id)
}
