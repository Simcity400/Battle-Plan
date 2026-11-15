//! Additional air-base inventory inputs for carrier aircraft availability.
//!
//! These helpers run inside the existing registration observation. They do not
//! walk another base, country, pool or wing roster. The registration owner
//! supplies its exact indexed-container certificates and copied pool header;
//! dense countries and retained host bytes fail independently of the existing
//! capacity/membership row. Both fuel scalars come from the registration walk's
//! own container span ([`super::container_span_words`]). All scratch and output
//! storage is caller-owned. Fixed acquisition does not allocate; worker arenas
//! grow from their original native headers.

use crate::{
    Memory, cair_wing_pool, ccountry, ccountry_air_container, cname_group_member,
    cpdx_array::{self, Array},
    creference_registry, cship, cstrategic_air, csub_unit_definition as definition, read,
    walk::Buffer,
};

/// Signed grouping key for pool pointers, separate from the content database index.
const TYPE_INDEX: usize = 0x60;

/// One indexed slot authenticated by the existing registration reader before
/// following its pool vector. Clear the whole used slice for each base.
/// Null slots and slots not yet authenticated have no certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedContainer {
    pub base: u64,
    pub pointer: usize,
    pub country: u32,
    pub slot: u32,
}

/// One dense `REGISTERED_CONTAINERS` occurrence; repeats and native order are retained.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountryOccurrence {
    pub country: u32,
    pub slot: u32,
}

/// Caller-owned dense-country arena for one base. `len` is an accepted
/// prefix, never a complete vector after an error. `required` is the live
/// header's full demand even when the arena is too small.
pub(crate) struct DenseCountries<'a> {
    pub rows: Buffer<'a, CountryOccurrence>,
    pub len: usize,
    pub required: usize,
}

/// Copy dense occurrences in their original order and authenticate each
/// against this base's indexed registration. Read the raw country at that
/// occurrence and reuse the resolver for positive linked countries. Work is
/// linear in indexed slots and dense occurrences, without sorting, deduping,
/// pool rewalks or reconstructed country lists.
///
/// `indexed` covers the complete indexed vector. Install certificates only
/// after checking container vtable, raw country and slot. Clear it for each
/// base and retain it only until this helper returns. The caller supplies
/// the authenticated base pointer and full handle.
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
    // Copy every dense occurrence, including repeats.
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

/// Storage demand from the exact `REGISTERED_CONTAINERS` header only. This does not certify an
/// indexed registration or follow any dense occurrence; no output is admitted when the caller lacks
/// the indexed certificates needed by dense_countries.
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
    // The native indexed getter uses zero for raw zero and readable unlinked
    // references. A failed/out-of-range link stays a refusal, never slot zero.
    if raw == 0 || countries.link(raw)?.is_none() {
        Ok(0)
    } else {
        Ok(resolved.resolve(countries, raw)?.canonical_slot)
    }
}

/// The carrier fuel calculation consumes both scalars even when no pool or
/// active wing exists. The registration walk reads them in its one container
/// span with the country and capacity words ([`super::container_span_words`]);
/// an unobserved container keeps each as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerFuel {
    pub demand: Result<i64, &'static str>,
    pub allocated: Result<i64, &'static str>,
}

/// Authenticated subtype identity. The pointer is transient query context for
/// the same-observation fuel reader and must never be encoded or deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DefinitionIdentity {
    pub pointer: usize,
    pub type_index: i32,
    /// Exact recorded-content entry: a real positive content database index, native null slot0.
    /// Tokens are not unique content identities and cannot replace this index.
    pub subunit_index: u32,
}

/// Inventory of one existing registered pool, including an empty one.
/// These wing boundaries refer to the existing flattened native wing arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolMetadata {
    /// Same-observation addresses for the passive fuel query only.
    pub pool: usize,
    pub owner: usize,
    pub base_reference: u64,
    pub definition: DefinitionIdentity,
    pub members: Array,
    pub wings_start: usize,
    pub wings_len: u32,
}

/// Exact pool observation supplied by the existing registration pass after
/// validating its copied vtable and decoding its copied wing-array header.
pub(crate) struct PoolObservation {
    pub pointer: usize,
    pub registration: IndexedContainer,
    pub members: Array,
    pub wings_start: usize,
}

/// Acquire only fields missing from the registration reader's copied pool
/// identity/header. The caller has already checked this pool's vtable and
/// decoded `members` from that exact observation; no wing is read here.
///
/// Stage this metadata before the existing wing loop, then admit it only when
/// that loop has copied all members into the registration's accepted wing
/// slice. Failure/truncation of that membership must withhold dependent pool
/// metadata, while independently acquired dense countries/names/scalars stay.
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

/// The real/null header rules match the transport pool reader. Country-local
/// clones copy the name and database index together. Return that authenticated
/// index without a first-match token lookup. Capture the grouping key separately.
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
        // The first content-vector entry registers the exact null-definition
        // singleton used by pool construction. Matching vtable and token at another
        // address cannot authenticate that null entry.
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


/// Resolve this base's full host reference and read its retained ship name.
/// Null and known-stale references are known empty text. Resolution or cache
/// failures stay independent errors. Copy raw bytes uncommitted as in
/// cship::cached_name; advance the cursor only when admitting the base packet.
/// `source` is this observation's authenticated base row; `resolver` must use
/// `image` from that observation. Neither can cross callbacks or off-hour hints.
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

/// Sparse off-hour sizing hint with the same selected full host reference.
/// A later capture always performs its own same-observation read again.
/// Its resolver is constructed with `image` for this hint only, with no
/// retained headers reused by the later callback's resolver.
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

fn field<T: crate::memory::Raw>(
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
