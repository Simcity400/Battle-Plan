use crate::{
    ccountry_resources, cresource_exchange, cstrategic_resource, cstrategic_resource_database,
    cstrategic_resource_pool,
};
use engine_native::{
    Memory,
    ccountry,
    cgame_state,
    cpdx_array::Array,
    cprovince,
    crelation,
    crelation_status,
    cstate,
    csubject_relation,
    memory::Raw,
    modifier_accumulator,
    read,
};
use engine_native_politics::{cdiplomacy_status, cpolitical_party, cpolitical_status};
use engine_sites::Rva;

const ONE: i64 = 100_000;
const BASE: Rva = Rva(0x33df638);
const DISTANCE_FACTOR: Rva = Rva(0x33e5d40);
const SUBJECT: Rva = Rva(0x33df388);
const SUBJECT_OTHER: Rva = Rva(0x33df410);
const IDEOLOGY_FACTOR: Rva = Rva(0x33df750);
const OPINION_FACTOR: Rva = Rva(0x33e5dc8);
const BORDER_FACTOR: Rva = Rva(0x33df7cc);
const NAVY_FACTOR: Rva = Rva(0x33df868);
const MONOPOLY_THRESHOLD: Rva = Rva(0x33df7e0);
const MONOPOLY_VALUE: Rva = Rva(0x33df878);
const OPINION_MIN: Rva = Rva(0x33e2e54);
const OPINION_MAX: Rva = Rva(0x33e2db8);
const OPINION_DATABASE: Rva = Rva(0x33dd408);
const MAP: Rva = Rva(0x33e7700);
const DLC_MASK: Rva = Rva(0x33dd698);
const COUNTRY_POLITICS: usize = 0xf90;
const COUNTRY_BORDERS: usize = 0x548;
const BORDER_COUNTRIES: usize = 0x48;
const BORDER_COUNTRY_STRIDE: usize = 0x40;
const BORDER_OWN_PROVINCES: usize = 0x8;
const BORDER_OTHER_PROVINCES: usize = 0x20;
const PROVINCE_UNITS: usize = 0x110;
const UNIT_OWNER: usize = 0x1e0;
const FLEET_FORCES: usize = 0xb8;
const FORCE_OWNER: usize = 0x1d8;
const FORCE_LOCATION: usize = 0x1f0;
const FORCE_COUNTRY_STATUS: usize = 0x278;
const FORCE_STATUS_STRIDE: usize = 24;
const FORCE_STATUS_COUNTRY: usize = 0;
const FORCE_STATUS_VALUE: usize = 16;
const FORCE_SHIPS: usize = 0x348;
const DEFINITION_FLAGS: usize = 0xd2;
const DEFINITION_POSITION: usize = 0xb0;
const DEFINITION_PROVINCES: usize = 0x70;
const NAVAL_AREA_STRIDE: usize = 0x30;
const NAVAL_AREA_PROVINCE: usize = 8;
const MAP_DEFINITIONS: usize = 0x268;
const MAP_WIDTH: usize = 0x40;
const OPINION_MODIFIERS: usize = 0x100;
const MODIFIER_DEFINITION: usize = 0x28;
const MODIFIER_VALUE: usize = 0x30;
const DEFINITION_TRADE: usize = 0x79;
const DEFINITION_OPINION: usize = 0x70;
const OPINION_FORWARD_DEFINITIONS: usize = 0x28;
const OPINION_REVERSE_DEFINITIONS: usize = 0x40;
const OPINION_DEFINITION_TYPE: usize = 8;
const TRADE_TO_TARGET_FACTOR: u32 = 0x178;
const TRADE_TO_OVERLORD_FACTOR: u32 = 0x112;
const OVERLORD_TRADE_COST_FACTOR: u32 = 0x113;
const TRADE_COST_TO_TARGET_FACTOR: u32 = 0x179;
const RESOURCE_TRADE_COST_BONUS_PER_FACTORY: u32 = 0x17a;
const FACTION_TRADE_OPINION_FACTOR: u32 = 0xfa;
const TRADE_OPINION_FACTOR: u32 = 0xe1;

fn field<T: Raw>(m: &impl Memory, p: usize, offset: usize) -> Result<T, &'static str> {
    p.checked_add(offset)
        .and_then(|p| read(m, p))
        .ok_or("trade influence operand unreadable")
}

fn array(m: &impl Memory, p: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
    p.checked_add(offset)
        .and_then(|p| Array::read(m, p, stride))
        .ok_or("trade influence array invalid")
}

fn element<T: Raw>(
    m: &impl Memory,
    a: Array,
    index: u32,
    stride: usize,
) -> Result<T, &'static str> {
    a.element(index, stride)
        .and_then(|p| read(m, p))
        .ok_or("trade influence element unreadable")
}

fn ptr(m: &impl Memory, p: usize, offset: usize) -> Result<usize, &'static str> {
    Ok(field::<u64>(m, p, offset)? as usize)
}

fn global<T: Raw>(m: &impl Memory, base: usize, key: Rva) -> Result<T, &'static str> {
    engine_sites::at(base, key)
        .and_then(|p| read(m, p))
        .ok_or("trade influence operand unreadable")
}

fn global_ptr(m: &impl Memory, base: usize, key: Rva) -> Result<usize, &'static str> {
    Ok(global::<u64>(m, base, key)? as usize)
}

fn times(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}
fn whole(a: i64) -> i64 {
    i64::from((a / ONE) as i32) * ONE
}
fn quotient(a: i64, b: i64) -> i64 {
    if b == 0 || !(-i64::MAX + 1..i64::MAX).contains(&a) {
        return i64::from(u32::MAX);
    }
    a.wrapping_mul(ONE)
        .checked_div(b)
        .unwrap_or(i64::from(u32::MAX))
}

const BATCH: usize = 64;
const UNIT_OWNER_FIELDS: [engine_native::memory::RecordField; 1] =
    engine_native::memory::packed_fields([(UNIT_OWNER, 4)]);
const UNIT_OWNER_LAYOUT: engine_native::memory::RecordLayout<'static> =
    match engine_native::memory::RecordLayout::new(&UNIT_OWNER_FIELDS, 4) {
        Some(layout) => layout,
        None => panic!("valid trade influence unit owner layout"),
    };

struct Context<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    game: usize,
    countries: &'a ccountry::Reader<'a, M>,
    canonical: std::cell::RefCell<Vec<(u32, u32)>>,
    friendly: std::cell::RefCell<Vec<((u32, u32), bool)>>,
}

impl<M: Memory> Context<'_, M> {
    fn canonical(&self, raw: u32) -> Result<u32, &'static str> {
        if let Some(&(_, slot)) = self
            .canonical
            .borrow()
            .iter()
            .find(|(held, _)| *held == raw)
        {
            return Ok(slot);
        }
        let slot = self.countries.resolve(raw)?.canonical_slot;
        self.canonical.borrow_mut().push((raw, slot));
        Ok(slot)
    }

    fn equal(&self, a: u32, b: u32) -> Result<bool, &'static str> {
        Ok(a == b || (a != 0 && b != 0 && self.canonical(a)? == self.canonical(b)?))
    }

    fn diplomacy(&self, p: usize) -> Result<usize, &'static str> {
        ptr(self.memory, p, ccountry::DIPLOMACY_STATUS)
    }

    fn relation(&self, diplomacy: usize, other: ccountry::Country) -> Result<usize, &'static str> {
        element::<u64>(
            self.memory,
            array(self.memory, diplomacy, cdiplomacy_status::RELATIONS, 8)?,
            other.canonical_slot,
            8,
        )
        .map(|p| p as usize)
    }

    fn modifier(&self, root: usize, key: u32) -> Result<i64, &'static str> {
        modifier_accumulator::lookup_effective_at(self.memory, self.base, root, key)
            .map(|v| v.effective)
    }

    fn country_modifier(&self, p: usize, key: u32) -> Result<i64, &'static str> {
        self.modifier(
            p.checked_add(ccountry::MODIFIER_ACCUMULATOR)
                .ok_or("trade influence accumulator overflow")?,
            key,
        )
    }

    fn relation_modifier(&self, p: usize, key: u32) -> Result<i64, &'static str> {
        self.modifier(
            p.checked_add(crelation_status::MODIFIER_ACCUMULATOR)
                .ok_or("trade influence relation overflow")?,
            key,
        )
    }

    fn subject_endpoint(&self, relation: usize, endpoint: usize) -> Result<bool, &'static str> {
        let subject = ptr(self.memory, relation, crelation_status::SUBJECT_RELATION)?;
        Ok(subject != 0
            && field::<u32>(self.memory, subject, csubject_relation::TYPE_ID)?
                == csubject_relation::TYPE_VALUE
            && self.equal(
                field(self.memory, subject, endpoint)?,
                field(self.memory, relation, crelation_status::TARGET_COUNTRY)?,
            )?)
    }

    fn friendly(
        &self,
        owner: ccountry::Country,
        other: ccountry::Country,
    ) -> Result<bool, &'static str> {
        let key = (owner.canonical_slot, other.canonical_slot);
        if let Some(&(_, verdict)) = self.friendly.borrow().iter().find(|(held, _)| *held == key) {
            return Ok(verdict);
        }
        let verdict = self.friendly_uncached(owner, other)?;
        self.friendly.borrow_mut().push((key, verdict));
        Ok(verdict)
    }

    fn friendly_uncached(
        &self,
        owner: ccountry::Country,
        other: ccountry::Country,
    ) -> Result<bool, &'static str> {
        if owner.canonical_slot == other.canonical_slot {
            return Ok(true);
        }
        let diplomacy = self.diplomacy(owner.pointer)?;
        let relation = self.relation(diplomacy, other)?;
        let war = ptr(self.memory, relation, crelation_status::WAR_RELATION)?;
        if war != 0 && field::<u8>(self.memory, war, crelation::RETIRED)? == 0 {
            return Ok(false);
        }
        let faction = ptr(self.memory, diplomacy, cdiplomacy_status::FACTION)?;
        if faction != 0
            && faction
                == ptr(
                    self.memory,
                    self.diplomacy(other.pointer)?,
                    cdiplomacy_status::FACTION,
                )?
        {
            return Ok(true);
        }
        Ok(
            self.subject_endpoint(relation, csubject_relation::OVERLORD)?
                || self.subject_endpoint(relation, csubject_relation::SUBJECT)?,
        )
    }

    fn capital_province(&self, country: usize) -> Result<Option<u32>, &'static str> {
        let id = field::<i32>(self.memory, country, ccountry::CAPITAL)?;
        let states = array(self.memory, self.game, cgame_state::STATES, 8)?;
        if id < 1 || id as u32 >= states.count {
            return Ok(None);
        }
        let state = element::<u64>(self.memory, states, id as u32, 8)? as usize;
        if state == 0 {
            return Ok(None);
        }
        let provinces = array(self.memory, state, cstate::PROVINCES, 8)?;
        let province = element::<u64>(self.memory, provinces, 0, 8)? as usize;
        field(self.memory, province, cprovince::ID).map(Some)
    }

    fn distance(&self, source: usize, other: usize) -> Result<i64, &'static str> {
        let distance = if field::<i32>(self.memory, source, ccountry::CAPITAL)? < 1 {
            99_999_900_000
        } else if let (Some(a), Some(b)) = (
            self.capital_province(source)?,
            self.capital_province(other)?,
        ) {
            if a == b {
                0
            } else {
                let map = global_ptr(self.memory, self.base, MAP)?;
                let definitions = array(self.memory, map, MAP_DEFINITIONS, 8)?;
                let position = |id| -> Result<[i32; 2], &'static str> {
                    let p = element::<u64>(
                        self.memory,
                        definitions,
                        if id > 0 && id < definitions.count {
                            id
                        } else {
                            0
                        },
                        8,
                    )? as usize;
                    let bytes = field::<[u8; 8]>(self.memory, p, DEFINITION_POSITION)?;
                    Ok([
                        i32::from_le_bytes(bytes[..4].try_into().unwrap()),
                        i32::from_le_bytes(bytes[4..].try_into().unwrap()),
                    ])
                };
                let [ax, ay] = position(a)?;
                let [bx, by] = position(b)?;
                let width = field::<i32>(self.memory, map, MAP_WIDTH)?;
                let dx = ax.wrapping_sub(bx);
                let dy = ay.wrapping_sub(by);
                let dx = if dx.wrapping_abs() <= width / 2 {
                    dx
                } else {
                    width.wrapping_sub(dx.wrapping_abs())
                };
                sqrt_distance(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32)
            }
        } else {
            9_999_900_000
        };
        Ok(i64::from(
            (distance.wrapping_mul(global::<i64>(self.memory, self.base, DISTANCE_FACTOR)?)
                / 10_000_000_000) as i32,
        ) * ONE)
    }

    fn ideology(&self, source: usize, other: usize) -> Result<i64, &'static str> {
        let source = ptr(self.memory, source, COUNTRY_POLITICS)?;
        let party = ptr(self.memory, source, cpolitical_status::RULING_PARTY)?;
        if party == 0 {
            return Ok(0);
        }
        let ideology = ptr(self.memory, party, cpolitical_party::IDEOLOGY_GROUP)?;
        let parties = array(
            self.memory,
            ptr(self.memory, other, COUNTRY_POLITICS)?,
            cpolitical_status::PARTIES,
            8,
        )?;
        for index in 0..parties.count {
            let party = element::<u64>(self.memory, parties, index, 8)? as usize;
            if ptr(self.memory, party, cpolitical_party::IDEOLOGY_GROUP)? == ideology {
                let popularity = field::<i64>(self.memory, party, cpolitical_party::POPULARITY)?;
                return Ok(whole(
                    times(popularity, global(self.memory, self.base, IDEOLOGY_FACTOR)?)
                        .wrapping_mul(1000)
                        / ONE,
                ));
            }
        }
        Ok(0)
    }

    fn border_units(&self, provinces: Array, country: u32) -> Result<i32, &'static str> {
        let mut count = 0i32;
        let mut handles = [0_u8; BATCH * 8];
        let mut units = [0_usize; BATCH];
        let mut owners = [0_u8; BATCH * 4];
        for index in 0..provinces.count {
            let p = element::<u64>(self.memory, provinces, index, 8)? as usize;
            let list = array(self.memory, p, PROVINCE_UNITS, 8)?;
            let mut first = 0;
            while first < list.count {
                let len = ((list.count - first) as usize).min(BATCH);
                let at = list
                    .element(first, 8)
                    .ok_or("trade influence element unreadable")?;
                if !engine_native::read_slice(self.memory, at, &mut handles[..len * 8]) {
                    return Err("trade influence element unreadable");
                }
                for (unit, handle) in units.iter_mut().zip(handles[..len * 8].chunks_exact(8)) {
                    *unit = u64::from_le_bytes(handle.try_into().unwrap()) as usize;
                }
                if self.memory.copy_records(
                    engine_native::memory::RecordSources::Pointers(&units[..len]),
                    &UNIT_OWNER_LAYOUT,
                    &mut owners[..len * 4],
                ) != len
                {
                    return Err("trade influence operand unreadable");
                }
                for owner in owners[..len * 4].chunks_exact(4) {
                    if self.equal(u32::from_le_bytes(owner.try_into().unwrap()), country)? {
                        count = count.wrapping_add(1);
                    }
                }
                first += len as u32;
            }
        }
        Ok(count)
    }

    fn borders(
        &self,
        source: ccountry::Country,
        other: ccountry::Country,
    ) -> Result<i64, &'static str> {
        let holders = array(self.memory, source.pointer, COUNTRY_BORDERS, 8)?;
        let mut counts = [0i32; 2];
        for index in 0..holders.count {
            let holder = element::<u64>(self.memory, holders, index, 8)? as usize;
            let countries = array(self.memory, holder, BORDER_COUNTRIES, BORDER_COUNTRY_STRIDE)?;
            for index in 0..countries.count {
                let p = countries
                    .element(index, BORDER_COUNTRY_STRIDE)
                    .ok_or("trade influence border extent invalid")?;
                if self.equal(field(self.memory, p, 0)?, other.id)? {
                    counts[0] = counts[0].wrapping_add(self.border_units(
                        array(self.memory, p, BORDER_OWN_PROVINCES, 8)?,
                        source.id,
                    )?);
                    counts[1] = counts[1].wrapping_add(self.border_units(
                        array(self.memory, p, BORDER_OTHER_PROVINCES, 8)?,
                        other.id,
                    )?);
                    break;
                }
            }
        }
        let difference = counts[1].wrapping_sub(counts[0]);
        Ok(if difference > 0 {
            i64::from(difference.wrapping_mul(global(self.memory, self.base, BORDER_FACTOR)?)) * ONE
        } else {
            0
        })
    }

    fn navies(
        &self,
        source: ccountry::Country,
        other: ccountry::Country,
    ) -> Result<i64, &'static str> {
        let fleets = array(self.memory, other.pointer, ccountry::FLEETS, 8)?;
        let provinces = array(self.memory, self.game, cgame_state::PROVINCES, 8)?;
        let mut count = 0i32;
        for index in 0..fleets.count {
            let fleet = element::<u64>(self.memory, fleets, index, 8)? as usize;
            let forces = array(self.memory, fleet, FLEET_FORCES, 8)?;
            for index in 0..forces.count {
                let force = element::<u64>(self.memory, forces, index, 8)? as usize;
                let owner = self
                    .countries
                    .resolve(field(self.memory, force, FORCE_OWNER)?)?;
                let mut status = if self.friendly(owner, source)? { 3 } else { 0 };
                if status == 0 {
                    let statuses = array(
                        self.memory,
                        force,
                        FORCE_COUNTRY_STATUS,
                        FORCE_STATUS_STRIDE,
                    )?;
                    for index in 0..statuses.count {
                        let p = statuses
                            .element(index, FORCE_STATUS_STRIDE)
                            .ok_or("trade influence naval status extent invalid")?;
                        if field::<u32>(self.memory, p, FORCE_STATUS_COUNTRY)?
                            == source.canonical_slot
                        {
                            status =
                                i32::from(field::<u8>(self.memory, p, FORCE_STATUS_VALUE)? as i8);
                            break;
                        }
                    }
                }
                if status <= 0 {
                    continue;
                }
                let location = ptr(self.memory, force, FORCE_LOCATION)?;
                if location == 0 {
                    return Err("trade influence task force location unavailable");
                }
                let definition = ptr(self.memory, location, cprovince::DEFINITION)?;
                let area = array(
                    self.memory,
                    definition,
                    DEFINITION_PROVINCES,
                    NAVAL_AREA_STRIDE,
                )?;
                for index in 0..area.count {
                    let p = area
                        .element(index, NAVAL_AREA_STRIDE)
                        .ok_or("trade influence naval area extent invalid")?;
                    let id = field::<u32>(self.memory, p, NAVAL_AREA_PROVINCE)?;
                    let province = element::<u64>(self.memory, provinces, id, 8)? as usize;
                    let definition = ptr(self.memory, province, cprovince::DEFINITION)?;
                    if field::<u8>(self.memory, definition, DEFINITION_FLAGS)? & 1 == 0 {
                        continue;
                    }
                    let state = ptr(self.memory, province, cprovince::STATE)?;
                    let owner = if state == 0 {
                        0
                    } else {
                        field(self.memory, state, cstate::OWNER)?
                    };
                    if self.equal(owner, source.id)? {
                        count = count
                            .wrapping_add(array(self.memory, force, FORCE_SHIPS, 8)?.count as i32);
                        break;
                    }
                }
            }
        }
        Ok(if count > 0 {
            i64::from(count.wrapping_mul(global(self.memory, self.base, NAVY_FACTOR)?)) * ONE
        } else {
            0
        })
    }

    fn autonomy(
        &self,
        country: ccountry::Country,
        other: ccountry::Country,
    ) -> Result<bool, &'static str> {
        let diplomacy = self.diplomacy(country.pointer)?;
        Ok(self.equal(
            field(self.memory, diplomacy, cdiplomacy_status::OVERLORD)?,
            other.id,
        )? && ptr(
            self.memory,
            diplomacy,
            cdiplomacy_status::CURRENT_AUTONOMY_STATUS,
        )? != 0
            && global::<u32>(self.memory, self.base, DLC_MASK)? & 0xe != 0)
    }

    fn monopoly(
        &self,
        source: ccountry::Country,
        other: ccountry::Country,
    ) -> Result<i64, &'static str> {
        let resources =
            ccountry_resources::resources(self.memory, self.base, self.countries, other)?;
        let relation = self.relation(self.diplomacy(other.pointer)?, source)?;
        let mut allowance = self.relation_modifier(relation, TRADE_TO_TARGET_FACTOR)?;
        if self.autonomy(other, source)? {
            allowance = allowance
                .wrapping_add(self.country_modifier(other.pointer, TRADE_TO_OVERLORD_FACTOR)?);
        }
        let pool = |offset, index| -> Result<i64, &'static str> {
            let data = ptr(self.memory, resources, offset)?;
            field(
                self.memory,
                data,
                (index as usize)
                    .checked_mul(cstrategic_resource_pool::VALUE_STRIDE)
                    .ok_or("trade influence resource extent overflow")?,
            )
        };
        let definitions = cstrategic_resource_database::Reader::new(self.memory, self.base)?;
        let buckets = array(
            self.memory,
            resources,
            ccountry_resources::EXPORT_BUCKETS,
            ccountry_resources::EXPORT_BUCKET_STRIDE,
        )?;
        let mut total = 0i64;
        let mut count = 0i64;
        for index in 1..definitions.count() {
            let (descriptor, _) = definitions.at(index)?;
            let mut available = pool(
                ccountry_resources::BASE_EXPORT_POOL + cstrategic_resource_pool::VALUES,
                index,
            )?;
            if allowance > 0 {
                let remainder = pool(
                    ccountry_resources::ADJUSTMENT_POOL + cstrategic_resource_pool::VALUES,
                    index,
                )?
                .wrapping_add(pool(
                    ccountry_resources::PRODUCED_POOL + cstrategic_resource_pool::VALUES,
                    index,
                )?)
                .wrapping_sub(available);
                if remainder >= 0 {
                    available = whole(times(remainder, allowance.min(ONE)).wrapping_add(available));
                }
            }
            if available <= 0 {
                continue;
            }
            count = count.wrapping_add(ONE);
            let bucket = buckets
                .element(index, ccountry_resources::EXPORT_BUCKET_STRIDE)
                .ok_or("trade influence resource bucket absent")?;
            let exchanges = array(self.memory, bucket, 0, 8)?;
            for index in 0..exchanges.count {
                let exchange = element::<u64>(self.memory, exchanges, index, 8)? as usize;
                if !self.equal(
                    field(self.memory, exchange, cresource_exchange::IMPORTER)?,
                    source.id,
                )? {
                    continue;
                }
                let factories = field::<i32>(
                    self.memory,
                    exchange,
                    cresource_exchange::REQUIRED_FACTORIES,
                )?;
                let cost =
                    field::<i64>(self.memory, descriptor, cstrategic_resource::FACTORY_COST)?;
                let exporter_resources = ptr(
                    self.memory,
                    exchange,
                    cresource_exchange::EXPORTER_RESOURCES,
                )?;
                let exporter = self.countries.resolve(field(
                    self.memory,
                    exporter_resources,
                    ccountry_resources::OWNER,
                )?)?;
                let relation = self.relation(self.diplomacy(exporter.pointer)?, source)?;
                let mut factor = ONE;
                if self.autonomy(exporter, source)? {
                    factor = factor.wrapping_add(
                        self.country_modifier(exporter.pointer, OVERLORD_TRADE_COST_FACTOR)?,
                    );
                }
                factor = times(
                    ONE.wrapping_add(
                        self.relation_modifier(relation, TRADE_COST_TO_TARGET_FACTOR)?,
                    ),
                    factor,
                );
                let amount = if cost > 0 && factor > 0 && factories > 0 {
                    let factories = i64::from(factories);
                    let basic = quotient(factories.wrapping_mul(10_000_000_000) / cost, factor);
                    basic.wrapping_add(times(
                        factories.wrapping_mul(ONE),
                        self.country_modifier(
                            source.pointer,
                            RESOURCE_TRADE_COST_BONUS_PER_FACTORY,
                        )?,
                    ))
                } else {
                    0
                };
                total = total.wrapping_add(quotient(amount, available));
                break;
            }
        }
        Ok(
            if count != 0
                && quotient(total, count) > global(self.memory, self.base, MONOPOLY_THRESHOLD)?
            {
                whole(global(self.memory, self.base, MONOPOLY_VALUE)?)
            } else {
                0
            },
        )
    }

    fn scripted(&self, source: ccountry::Country, relation: usize) -> Result<i64, &'static str> {
        let database = global_ptr(self.memory, self.base, OPINION_DATABASE)?;
        let limits = [
            global::<i32>(self.memory, self.base, OPINION_MIN)?,
            global::<i32>(self.memory, self.base, OPINION_MAX)?,
        ];
        let relations = array(self.memory, relation, crelation_status::SUBRELATIONS, 8)?;
        let mut total = 0i64;
        for index in 0..relations.count {
            let p = element::<u64>(self.memory, relations, index, 8)? as usize;
            let reverse =
                self.equal(source.id, field(self.memory, p, crelation::SECOND_COUNTRY)?)?;
            let definitions = array(
                self.memory,
                database,
                if reverse {
                    OPINION_REVERSE_DEFINITIONS
                } else {
                    OPINION_FORWARD_DEFINITIONS
                },
                8,
            )?;
            let key = field::<u32>(self.memory, p, crelation::TYPE_ID)?;
            let mut found = None;
            for index in 1..definitions.count {
                let p = element::<u64>(self.memory, definitions, index, 8)? as usize;
                if field::<u32>(self.memory, p, OPINION_DEFINITION_TYPE)? == key {
                    found = Some(p);
                    break;
                }
            }
            let definition = match found {
                Some(p) => p,
                None => element::<u64>(self.memory, definitions, 0, 8)? as usize,
            };
            if field::<u8>(self.memory, definition, DEFINITION_TRADE)? != 0 {
                let value = i32::from(field::<i16>(self.memory, definition, DEFINITION_OPINION)?)
                    .max(limits[0])
                    .min(limits[1]);
                total = total.wrapping_add(i64::from(value) * ONE);
            }
        }
        let modifiers = array(self.memory, relation, OPINION_MODIFIERS, 8)?;
        for index in 0..modifiers.count {
            let modifier = element::<u64>(self.memory, modifiers, index, 8)? as usize;
            let definition = ptr(self.memory, modifier, MODIFIER_DEFINITION)?;
            if field::<u8>(self.memory, definition, DEFINITION_TRADE)? != 0 {
                total = total.wrapping_add(
                    i64::from((field::<i64>(self.memory, modifier, MODIFIER_VALUE)? / ONE) as i32)
                        * ONE,
                );
            }
        }
        Ok(total)
    }
}

fn sqrt_fixed(mut value: i64) -> i64 {
    let small = value < 10_001;
    if small {
        value = value.wrapping_mul(100);
    }
    let mut scale = 1i64;
    while value > 2_999_999_999 {
        value = times(value, 1000);
        scale = scale.wrapping_mul(10);
    }
    let mut root = if value < 2 {
        0
    } else {
        value.wrapping_mul(ONE) / 200_000
    };
    if value >= 2 {
        let tolerance = times(value, 1000);
        loop {
            let error = times(root, root).wrapping_sub(value);
            let change = quotient(error, root.wrapping_mul(2));
            root = root.wrapping_sub(change);
            if (error <= tolerance && error >= -tolerance) || change == 0 {
                break;
            }
        }
    }
    if small {
        times(root, 10_000)
    } else {
        root.wrapping_mul(scale)
    }
}

fn sqrt_distance(value: u32) -> i64 {
    if value as i32 <= 0 {
        return 0;
    }
    if value & 0xffe0_0000 != 0 {
        sqrt_fixed(i64::from(value >> 10) * ONE) << 5
    } else {
        sqrt_fixed(i64::from(value) * ONE)
    }
}

pub fn read_current<M: Memory>(
    memory: &M,
    base: usize,
    game: usize,
    countries: &ccountry::Reader<'_, M>,
    source: u32,
    other: u32,
    operative_influence: i64,
) -> Result<i64, &'static str> {
    let source = countries.resolve(source)?;
    let other = countries.resolve(other)?;
    if field::<i32>(memory, other.pointer, ccountry::OWNED_STATE_COUNT)? < 1
        || source.canonical_slot == other.canonical_slot
    {
        return Ok(0);
    }
    let c = Context {
        memory,
        base,
        game,
        countries,
        canonical: Default::default(),
        friendly: Default::default(),
    };
    let diplomacy = c.diplomacy(source.pointer)?;
    let relation = c.relation(diplomacy, other)?;
    let subject = c.subject_endpoint(relation, csubject_relation::SUBJECT)?;
    let mut total = whole(global::<i64>(memory, base, BASE)?);
    total = total.wrapping_add(c.distance(source.pointer, other.pointer)?);
    total = total.wrapping_add(if subject {
        whole(global(memory, base, SUBJECT)?)
    } else if field::<i32>(memory, diplomacy, cdiplomacy_status::OVERLORD)? > 0
        || field::<i32>(
            memory,
            c.diplomacy(other.pointer)?,
            cdiplomacy_status::OVERLORD,
        )? > 0
    {
        whole(global(memory, base, SUBJECT_OTHER)?)
    } else {
        0
    });
    total = total.wrapping_add(c.ideology(source.pointer, other.pointer)?);
    let mut opinion = i64::from(field::<i32>(memory, relation, crelation_status::OPINION)?)
        .wrapping_mul(global::<i64>(memory, base, OPINION_FACTOR)?);
    let faction = ptr(memory, diplomacy, cdiplomacy_status::FACTION)?;
    if faction != 0 {
        let members = array(memory, faction, engine_native::cfaction::MEMBERS, 8)?;
        for index in 0..members.count {
            let member = element::<u64>(memory, members, index, 8)? as usize;
            if c.equal(field(memory, member, ccountry::ID)?, other.id)? {
                opinion = times(
                    opinion,
                    ONE.wrapping_add(
                        c.country_modifier(source.pointer, FACTION_TRADE_OPINION_FACTOR)?,
                    ),
                );
                break;
            }
        }
    }
    total = total.wrapping_add(whole(opinion));
    total = total.wrapping_add(c.borders(source, other)?);
    total = total.wrapping_add(c.navies(source, other)?);
    total = total.wrapping_add(c.monopoly(source, other)?);
    total = total.wrapping_add(c.scripted(source, relation)?);
    total = total.wrapping_add(operative_influence);
    if total > 0 {
        total = times(
            total,
            ONE.wrapping_add(c.country_modifier(other.pointer, TRADE_OPINION_FACTOR)?),
        );
    }
    Ok(total)
}
