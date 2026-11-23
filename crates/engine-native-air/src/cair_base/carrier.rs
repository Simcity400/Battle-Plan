
use engine_native_memory::{Memory, cpdx_array::{self, Array}, read, walk::Buffer};
use engine_native_parts::{
    cair_wing_pool,
    creference_registry,
    csub_unit_definition as definition,
};
use engine_native::{ccountry, cname_group_member, cship};
use crate::{ccountry_air_container, cstrategic_air};

const TYPE_INDEX: usize = 0x60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedContainer {
    pub base: u64,
    pub pointer: usize,
    pub country: u32,
    pub slot: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountryOccurrence {
    pub country: u32,
    pub slot: u32,
}

pub(crate) struct DenseCountries<'a> {
    pub rows: Buffer<'a, CountryOccurrence>,
    pub len: usize,
    pub required: usize,
}

pub(super) fn dense_countries<M: Memory>(
    memory: &M,
    air_base: usize,
    base_handle: u64,
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut super::Resolver<'_>,
    indexed: &[Option<IndexedContainer>],
    output: &mut DenseCountries<'_>,
) -> Result<(), &'static str> {
    output.len = 0;
    output.required = 0;
    let dense = dense_header(memory, air_base)?;
    output.required = dense.count as usize;
    output.rows.ensure(output.required)?;
    cpdx_array::pointers(
        memory,
        dense,
        "carrier dense country pointer unreadable or overflow",
        |_, pointer| {
            if pointer == 0 {
                return Err("carrier dense country occurrence is null");
            }
            let country = field(memory, pointer, ccountry_air_container::COUNTRY)?;
            let slot = country_slot(countries, resolved, country)?;
            let certificate = indexed
                .get(slot as usize)
                .and_then(|value| *value)
                .ok_or("carrier dense country has no authenticated indexed registration")?;
            if certificate.base != base_handle
                || certificate.pointer != pointer
                || certificate.country != country
                || certificate.slot != slot
            {
                return Err("carrier dense country differs from indexed registration");
            }
            if let Some(row) = output.rows.get_mut(output.len) {
                *row = CountryOccurrence { country, slot };
                output.len += 1;
            }
            Ok(())
        },
    )?;
    if output.len != output.required {
        return Err("carrier dense country arena too short");
    }
    Ok(())
}

pub(super) fn dense_demand(memory: &impl Memory, air_base: usize) -> Result<usize, &'static str> {
    dense_header(memory, air_base).map(|dense| dense.count as usize)
}

fn dense_header(memory: &impl Memory, air_base: usize) -> Result<Array, &'static str> {
    field::<[u8; 16]>(memory, air_base, super::REGISTERED_CONTAINERS).and_then(|bytes| {
        Array::from_header(bytes, 8).ok_or("carrier dense country header invalid")
    })
}

pub(super) fn country_slot<M: Memory>(
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut super::Resolver<'_>,
    raw: u32,
) -> Result<u32, &'static str> {
    if raw == 0 || countries.link(raw)?.is_none() {
        Ok(0)
    } else {
        Ok(resolved.resolve(countries, raw)?.canonical_slot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerFuel {
    pub demand: Result<i64, &'static str>,
    pub allocated: Result<i64, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DefinitionIdentity {
    pub pointer: usize,
    pub type_index: i32,
    pub subunit_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolMetadata {
    pub pool: usize,
    pub owner: usize,
    pub base_reference: u64,
    pub definition: DefinitionIdentity,
    pub members: Array,
    pub wings_start: usize,
    pub wings_len: u32,
}

pub(crate) struct PoolObservation {
    pub pointer: usize,
    pub registration: IndexedContainer,
    pub members: Array,
    pub wings_start: usize,
}

pub(super) fn pool_metadata<M: Memory>(
    memory: &M,
    image: usize,
    observed: PoolObservation,
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut super::Resolver<'_>,
    database: &mut Option<Result<definition::Database, &'static str>>,
) -> Result<PoolMetadata, &'static str> {
    let PoolObservation {
        pointer: pool,
        registration,
        members,
        wings_start,
    } = observed;
    if field::<u64>(memory, pool, cair_wing_pool::AIR_BASE_REFERENCE)? != registration.base {
        return Err("carrier registered pool belongs to another base");
    }
    let owner = pointer(memory, pool, cair_wing_pool::OWNER)?;
    if field::<u64>(memory, owner, 0)?
        != engine_sites::at(image, cstrategic_air::VTABLE_RVA)
            .ok_or("carrier module address overflow")? as u64
    {
        return Err("carrier registered pool owner identity mismatch");
    }
    let owner_country = field(memory, owner, cstrategic_air::COUNTRY)?;
    if country_slot(countries, resolved, owner_country)? != registration.slot {
        return Err("carrier registered pool country differs from its container");
    }
    let definition = pool_definition(memory, image, pool, database)?;
    wings_start
        .checked_add(members.count as usize)
        .ok_or("carrier pool wing boundary overflow")?;
    Ok(PoolMetadata {
        pool,
        owner,
        base_reference: registration.base,
        definition,
        members,
        wings_start,
        wings_len: members.count,
    })
}

fn pool_definition(
    memory: &impl Memory,
    image: usize,
    pool: usize,
    database: &mut Option<Result<definition::Database, &'static str>>,
) -> Result<DefinitionIdentity, &'static str> {
    let pointer = pointer(memory, pool, cair_wing_pool::DEFINITION)?;
    let (subunit_token, real) = definition_header(memory, image, pointer)?;
    let database = *database.get_or_insert_with(|| definition::Database::read(memory, image));
    let database = database?;
    let subunit_index = if real {
        let index = field(memory, pointer, definition::DATABASE_INDEX)?;
        let global = database.entry(memory, index)?;
        let (token, global_real) = definition_header(memory, image, global)?;
        if !global_real || token != subunit_token {
            return Err("carrier pool definition differs from its content database entry");
        }
        index as u32
    } else {
        let singleton = engine_sites::at(image, definition::NULL_OBJECT_RVA)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("carrier null definition singleton unreadable")?;
        let null = database
            .vector()
            .element(0, 8)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("carrier null definition content entry unreadable")?;
        if pointer != singleton || pointer != null {
            return Err("carrier null definition differs from the native content singleton");
        }
        0
    };
    Ok(DefinitionIdentity {
        pointer,
        type_index: field(memory, pointer, TYPE_INDEX)?,
        subunit_index,
    })
}

fn definition_header(
    memory: &impl Memory,
    image: usize,
    pointer: usize,
) -> Result<(u32, bool), &'static str> {
    let bytes = field::<[u8; 17]>(memory, pointer, 0)?;
    let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let token = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let expected = |rva| {
        engine_sites::at(image, rva)
            .map(|value| value as u64)
            .ok_or("carrier module address overflow")
    };
    match bytes[16] {
        1 if vtable == expected(definition::VTABLE_RVA)? => Ok((token, true)),
        0 if vtable == expected(definition::NULL_VTABLE_RVA)?
            && token == definition::NULL_TOKEN =>
        {
            Ok((token, false))
        }
        _ => Err("carrier pool definition identity mismatch"),
    }
}


pub(crate) fn host_name(
    memory: &impl Memory,
    image: usize,
    source: &super::Row,
    resolver: &mut creference_registry::ScopedResolver,
    names: &mut cname_group_member::Names<'_>,
) -> Result<Option<cname_group_member::Span>, &'static str> {
    if source.state.is_some() {
        return Err("carrier host-name query reached a state base");
    }
    if names.len > names.bytes.len() {
        return Err("carrier host-name arena cursor invalid");
    }
    let Some(ship) = resolver.resolve(memory, source.host)? else {
        return Ok(Some((names.len, 0)));
    };
    cship::cached_name(memory, image, ship, source.host, names)
}

pub(crate) fn host_name_demand(
    memory: &impl Memory,
    image: usize,
    source: &super::Row,
    resolver: &mut creference_registry::ScopedResolver,
) -> Result<usize, &'static str> {
    if source.state.is_some() {
        return Err("carrier host-name query reached a state base");
    }
    let Some(ship) = resolver.resolve(memory, source.host)? else {
        return Ok(0);
    };
    cship::cached_name_demand(memory, image, ship, source.host)
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|address| read(memory, address))
        .ok_or("carrier inventory field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("carrier inventory pointer invalid or null")
}

#[cfg(test)]
mod tests;
