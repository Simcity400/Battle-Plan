use crate::{Decimal, block, field, integer, invalid, required_bytes, scalar};
use content_reader_parser::parser::{Item, Value};
use std::{collections::BTreeMap, io};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub influence_count: u32,
    pub influence: BTreeMap<u32, Decimal>,
    pub initiative: Decimal,
    pub contribution: Decimal,
    pub gain: Vec<Decimal>,
    pub previous_score: Decimal,
    pub upgrades: Vec<Vec<u8>>,
    pub completed: Vec<Vec<u8>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accounts {
    pub extracted: Option<crate::economy::ResourcePool>,
    pub count: u32,
    pub entries: BTreeMap<u32, Account>,
}
fn sparse<T>(
    value: &Value,
    mut read: impl FnMut(&Value) -> io::Result<T>,
) -> io::Result<(u32, BTreeMap<u32, T>)> {
    let items = block(value)?;
    let (size, entries) = items
        .split_first()
        .ok_or_else(|| invalid("faction sparse array has no size"))?;
    if size.key.is_some() {
        return Err(invalid("faction sparse array size is keyed"));
    }
    let count: u32 = integer(scalar(&size.value)?)?;
    let mut result = BTreeMap::new();
    for entry in entries {
        if entry.key.is_some() {
            return Err(invalid("faction sparse array entry is keyed"));
        }
        let fields = block(&entry.value)?;
        let index: u32 = integer(&required_bytes(fields, b"index")?)?;
        if index >= count {
            return Err(invalid("faction sparse array index outside size"));
        }
        let value = read(
            field(fields, b"data")?.ok_or_else(|| invalid("faction sparse entry lacks data"))?,
        )?;
        if result.insert(index, value).is_some() {
            return Err(invalid("duplicate faction sparse array index"));
        }
    }
    Ok((count, result))
}
fn number(items: &[Item], key: &[u8]) -> io::Result<Decimal> {
    field(items, key)?
        .map(|value| Decimal::parse(scalar(value)?))
        .unwrap_or_else(|| Decimal::parse(b"0"))
}
fn tokens(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("faction account token occurrence is keyed"));
            }
            Ok(scalar(&item.value)?.to_vec())
        })
        .collect()
}
pub(crate) fn parse(items: &[Item]) -> io::Result<Option<Accounts>> {
    let Some(system) = field(items, b"faction_system")? else {
        return Ok(None);
    };
    let system = block(system)?;
    let extracted = field(system, b"extracted")?
        .map(|value| crate::faction_companions::parse_pool(block(value)?))
        .transpose()?;
    let Some(countries) = field(system, b"countries")? else {
        return Ok(None);
    };
    let (count, entries) = sparse(countries, |value| {
        let fields = block(value)?;
        let mut influence = BTreeMap::new();
        let mut influence_count = 0;
        if let Some(value) = field(fields, b"influence_status")? {
            let (count, values) = sparse(value, |value| Decimal::parse(scalar(value)?))?;
            influence_count = count;
            influence = values;
        }
        let gain = field(fields, b"contribution_gain")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|item| Decimal::parse(scalar(&item.value)?))
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Account {
            influence_count,
            influence,
            initiative: number(fields, b"initiative")?,
            contribution: number(fields, b"contribution")?,
            gain,
            previous_score: number(fields, b"war_score_breakdown")?,
            upgrades: tokens(fields, b"member_upgrades")?,
            completed: tokens(fields, b"completed_faction_goals")?,
        })
    })?;
    Ok(Some(Accounts {
        extracted,
        count,
        entries,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_accounts_keep_retained_occurrences_sizes_negative_values_and_dynamic_indices() {
        let doc=content_reader_parser::parser::parse(br#"faction_system={countries={5000 {index=4999 data={influence_status={6 {index=5 data=-2.5}} initiative=3 contribution_gain={1 -2} war_score_breakdown=7 member_upgrades={mod_b mod_a} completed_faction_goals={mod_a mod_a}}}}}"#).unwrap();
        let result = parse(&doc.items).unwrap().unwrap();
        assert_eq!(result.count, 5000);
        let row = &result.entries[&4999];
        assert_eq!(row.influence_count, 6);
        assert_eq!(row.influence[&5].raw, -250000);
        assert_eq!(row.gain[1].raw, -200000);
        assert_eq!(row.completed, [b"mod_a".to_vec(), b"mod_a".to_vec()]);
        let invalid =
            content_reader_parser::parser::parse(b"faction_system={countries={1 {index=1 data={}}}}")
                .unwrap();
        assert!(parse(&invalid.items).is_err());
    }
}
