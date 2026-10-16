//! Native pool values shared by the observer and progressive selected-table reads.
use super::{PoolOperand, resource_values};
use crate::prepared::query::progressive::Samples;
use replay::{
    frame::DecodedFrame,
    rows::{TableId, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub(crate) const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
];
pub(crate) const FIELDS: [&str; 12] = [
    "supply",
    "demand",
    "produced",
    "subjectImports",
    "imports",
    "specialProjects",
    "production",
    "subjectExports",
    "exportReserve",
    "rounding",
    "net",
    "exported",
];
pub(crate) fn field(resource: u32, index: usize) -> String {
    format!("{}:{resource}", FIELDS[index])
}
pub(crate) fn parse(field: &str) -> io::Result<(usize, u32)> {
    let invalid = || crate::invalid("Invalid resource sample field");
    let (name, resource) = field.split_once(':').ok_or_else(invalid)?;
    let index = FIELDS
        .iter()
        .position(|field| *field == name)
        .ok_or_else(invalid)?;
    Ok((index, resource.parse().map_err(|_| invalid())?))
}

/// Empty field selectors expand to all native pool fields for that country.
pub(crate) fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<Samples> {
    let reads = frame
        .complete::<tables::Countries>()
        .ok()
        .map(|countries| crate::products::territory::CountryReads::new(frame, countries));
    sample_rows(frame, requested, reads.as_ref())
}
pub(super) fn sample_rows(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
    reads: Option<&crate::products::territory::CountryReads>,
) -> io::Result<Samples> {
    let mut wanted = BTreeSet::new();
    let mut countries_wanted = BTreeSet::new();
    let mut wanted_tags = BTreeSet::new();
    if let Some(requested) = requested {
        for (tag, field) in requested {
            wanted_tags.insert(tag.as_str());
            if field.is_empty() {
                countries_wanted.insert(tag.as_str());
            } else {
                parse(field)?;
                wanted.insert((tag.as_str(), field.as_str()));
            }
        }
    }
    let mut result = BTreeMap::new();
    if let (Some(reads), Ok(_links), Ok(definitions), Ok(lexical), Ok(pools)) = (
        reads,
        frame.complete::<tables::CountryLinks>(),
        frame.complete::<tables::ResourceDefinitions>(),
        frame.complete::<tables::LexicalTokens>(),
        frame.complete::<tables::CountryResources>(),
    ) {
        let slots = reads.slots();
        let mut rows = BTreeMap::<u32, Vec<_>>::new();
        for row in pools.iter().map(PoolOperand::from) {
            if let Some(&slot) = slots.get(&row.country) {
                rows.entry(slot).or_default().push(row);
            }
        }
        for (tag, identity) in reads.holders() {
            let tag = tag.as_str();
            let Some((slot, country_id)) = *identity else {
                continue;
            };
            if requested.is_some() && !wanted_tags.contains(tag) {
                continue;
            }
            for row in rows.get(&slot).into_iter().flatten() {
                if definitions
                    .binary_search_by_key(&row.resource, |definition| definition.id)
                    .ok()
                    .and_then(|index| {
                        lexical
                            .binary_search_by_key(&definitions[index].token, |token| token.token)
                            .ok()
                    })
                    .and_then(|index| std::str::from_utf8(&lexical[index].literal).ok())
                    .is_none_or(|token| token.is_empty())
                {
                    continue;
                }
                let values = resource_values(&row.run(frame.hour))?;
                for (index, value) in values
                    .into_iter()
                    .chain([i128::from(row.values[4])])
                    .enumerate()
                {
                    let field = field(row.resource, index);
                    if requested.is_none()
                        || countries_wanted.contains(tag)
                        || wanted.contains(&(tag, field.as_str()))
                    {
                        result.insert(
                            (tag.to_owned(), field),
                            (Some((slot, country_id)), Some(value)),
                        );
                    }
                }
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested.iter().filter(|(_, field)| !field.is_empty()) {
            result.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(result)
}
