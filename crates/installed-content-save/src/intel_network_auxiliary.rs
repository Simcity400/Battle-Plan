//! Independent saved network lists and subnetwork operands. Ordered
//! occurrences, duplicates, empty lists and missing scalar fields stay distinct.
use super::super::{Decimal, block, decimal, field, integer, invalid, required_bytes, scalar};
use installed_content_core::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Network {
    pub vertices: Option<u32>,
    pub edges: Vec<(u64, u64)>,
    pub depth: Vec<i32>,
    pub subnetwork_count: Option<i32>,
    pub previous_max_depth: Option<i32>,
    pub previous_sources: Vec<i32>,
    pub operatives: Vec<(u64, i32)>,
    pub records: Vec<Record>,
    pub subnetworks: Vec<Subnetwork>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subnetwork {
    pub coverage: Option<[i32; 3]>,
    pub coverable: Option<[i32; 3]>,
    pub aggregates: [Option<Decimal>; 3],
    pub detection: [Option<Decimal>; 2],
    pub records: Vec<Record>,
    pub operatives: Vec<u64>,
    pub states: Vec<(i32, Decimal)>,
    pub core_states: Vec<i32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub country: Vec<u8>,
    pub fields: BTreeMap<Vec<u8>, Decimal>,
}
fn list<'a>(items: &'a [Item], key: &[u8]) -> io::Result<&'a [Item]> {
    field(items, key)?
        .map(block)
        .transpose()
        .map(|v| v.unwrap_or_default())
}
fn integers<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Vec<T>>
where
    T::Err: std::fmt::Display,
{
    list(items, key)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("network auxiliary integer list must be anonymous"));
            }
            integer(scalar(&item.value)?)
        })
        .collect()
}
fn count(items: &[Item], key: &[u8]) -> io::Result<Option<i32>> {
    field(items, key)?.map(|v| integer(scalar(v)?)).transpose()
}
fn coverage(items: &[Item], key: &[u8]) -> io::Result<Option<[i32; 3]>> {
    field(items, key)?
        .map(|value| {
            let fields = block(value)?;
            Ok([
                integer(&required_bytes(fields, b"core_states")?)?,
                integer(&required_bytes(fields, b"controlled_states")?)?,
                integer(&required_bytes(fields, b"owned_worth")?)?,
            ])
        })
        .transpose()
}
fn records(items: &[Item]) -> io::Result<Vec<Record>> {
    list(items, b"records")?
        .iter()
        .map(|item| {
            let fields = if item.key.is_some() {
                std::slice::from_ref(item)
            } else {
                block(&item.value)?
            };
            if fields.len() != 1 {
                return Err(invalid(
                    "network country record must contain one country entry",
                ));
            }
            let record = &fields[0];
            let country = record
                .key
                .clone()
                .ok_or_else(|| invalid("network country record lacks country key"))?;
            let mut values = BTreeMap::new();
            for item in block(&record.value)? {
                let key = item
                    .key
                    .clone()
                    .ok_or_else(|| invalid("network record operand lacks key"))?;
                if values
                    .insert(key, Decimal::parse(scalar(&item.value)?)?)
                    .is_some()
                {
                    return Err(invalid("duplicate network record operand"));
                }
            }
            Ok(Record {
                country,
                fields: values,
            })
        })
        .collect()
}
pub(super) fn parse(items: &[Item], local: &[Item]) -> io::Result<Network> {
    let edges = integers::<u64>(local, b"edges")?;
    if !edges.len().is_multiple_of(2) {
        return Err(invalid("network edges have an unmatched endpoint"));
    }
    let mut result = Network {
        vertices: field(local, b"vertices")?
            .map(|v| integer(scalar(v)?))
            .transpose()?,
        edges: edges.chunks_exact(2).map(|v| (v[0], v[1])).collect(),
        depth: integers(local, b"depth")?,
        subnetwork_count: count(local, b"sub_network_count")?,
        previous_max_depth: count(local, b"prev_max_depth")?,
        previous_sources: integers(local, b"prev_gain_sources")?,
        records: records(items)?,
        ..Network::default()
    };
    for item in list(items, b"operatives")? {
        if item.key.is_some() {
            return Err(invalid("network operative occurrence must be anonymous"));
        }
        let pair = block(&item.value)?;
        if pair.len() != 2 || pair.iter().any(|v| v.key.is_some()) {
            return Err(invalid(
                "network operative occurrence must contain reference and state",
            ));
        }
        result.operatives.push((
            super::super::null_permitted_handle(&pair[0].value)?,
            integer(scalar(&pair[1].value)?)?,
        ));
    }
    for item in list(items, b"sub_intel_networks")? {
        let fields = block(&item.value)?;
        let modifiers = field(fields, b"modifiers")?
            .map(block)
            .transpose()?
            .unwrap_or_default();
        let mut sub = Subnetwork {
            coverage: coverage(fields, b"coverage_stats")?,
            coverable: coverage(fields, b"total_coverable")?,
            aggregates: [
                decimal(fields, b"strength_sum")?,
                decimal(fields, b"strength_sum_over_cores")?,
                decimal(fields, b"national_coverage")?,
            ],
            detection: [
                decimal(modifiers, b"own_operative_detection_chance")?,
                decimal(modifiers, b"own_operative_detection_chance_factor")?,
            ],
            records: records(fields)?,
            core_states: integers(fields, b"core_states")?,
            ..Subnetwork::default()
        };
        for item in list(fields, b"operatives")? {
            if item.key.is_some() {
                return Err(invalid("subnetwork operative occurrence must be anonymous"));
            }
            sub.operatives
                .push(super::super::null_permitted_handle(&item.value)?);
        }
        for item in list(fields, b"states")? {
            let pair = block(&item.value)?;
            if pair.len() != 2 || pair.iter().any(|v| v.key.is_some()) {
                return Err(invalid(
                    "subnetwork state occurrence must contain state and strength",
                ));
            }
            sub.states.push((
                integer(scalar(&pair[0].value)?)?,
                Decimal::parse(scalar(&pair[1].value)?)?,
            ));
        }
        result.subnetworks.push(sub);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn network_auxiliary_parser_preserves_duplicate_occurrences_and_exact_signed_payloads() {
        let parsed=installed_content_core::parser::parse(b"vertices=2 edges={0 1 0 1} depth={-1 7} sub_network_count=3 prev_max_depth=-4 prev_gain_sources={0 0}").unwrap();
        let items=installed_content_core::parser::parse(b"operatives={{ {id=4294967295 type=4713} -7} {{id=4294967295 type=4713} -7}} records={{XYZ={owned_worth=-3}}} sub_intel_networks={{modifiers={own_operative_detection_chance=-0.00001 own_operative_detection_chance_factor=2} coverage_stats={core_states=-1 controlled_states=2 owned_worth=3} total_coverable={core_states=4 controlled_states=5 owned_worth=6} strength_sum=7 strength_sum_over_cores=8 national_coverage=9 operatives={{id=0 type=0}} states={{0 -1.00001}{0 0}} core_states={0 0}}}").unwrap();
        let result = parse(&items.items, &parsed.items).unwrap();
        assert_eq!(result.edges, vec![(0, 1), (0, 1)]);
        assert_eq!(result.previous_sources, vec![0, 0]);
        assert_eq!(result.operatives.len(), 2);
        assert_eq!(result.operatives[0].1, -7);
        assert_eq!(result.subnetworks[0].states.len(), 2);
        assert_eq!(result.subnetworks[0].coverage, Some([-1, 2, 3]));
        assert_eq!(result.records[0].country, b"XYZ");
    }
}
