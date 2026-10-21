//! A country's persistent unit-bonus list, saved as `deployment.unit_modifiers`:
//! anonymous entries in stored order, written only when the list is nonempty.
//! An entry's targets are blocks keyed directly on it by token name, exact
//! targets before category targets; the save does not mark the arm. Its source
//! follows as `type`, `number`, `id` (omitted for the `none` token) and
//! `localization_key` (omitted when empty).

use std::io;

use installed_content_core::parser::{Item, Value};

use crate::{Decimal, block, boolean, bytes, decimal, field, integer, invalid, scalar};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitBonus {
    pub targets: Vec<UnitBonusTarget>,
    /// `type`: the source kind's name.
    pub kind: Option<Vec<u8>>,
    pub number: Option<i32>,
    pub id: Option<Vec<u8>>,
    pub label: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitBonusTarget {
    /// The block's key: the target sub-unit's or category's name.
    pub token: Vec<u8>,
    /// Omitted when zero.
    pub combat_width: Option<Decimal>,
    /// The nonzero stats, by stat name in saved order.
    pub stats: Vec<(Vec<u8>, Decimal)>,
    /// Repeated `battalion_mult` blocks in saved order.
    pub grants: Vec<UnitBonusGrant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitBonusGrant {
    pub category: Option<Vec<u8>>,
    pub add: Option<bool>,
    pub display_as_percentage: Option<bool>,
    /// The nonzero stats, by stat name in saved order.
    pub stats: Vec<(Vec<u8>, Decimal)>,
}

/// Saved target scalars the recording does not hold: the priorities and the
/// sprite. Every target block other than `battalion_mult` (requirements and the
/// special and terrain adjusters) is skipped as well.
const UNRECORDED_SCALARS: [&[u8]; 3] = [b"priority", b"ai_priority", b"sprite"];

fn member(item: &Item) -> io::Result<&[u8]> {
    item.key
        .as_deref()
        .ok_or_else(|| invalid("unit modifier has an anonymous member"))
}

/// The list in a country's `deployment` block; an omitted key is empty.
pub(super) fn parse(deployment: &[Item]) -> io::Result<Vec<UnitBonus>> {
    let Some(list) = field(deployment, b"unit_modifiers")? else {
        return Ok(Vec::new());
    };
    block(list)?
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("unit modifier entry is not anonymous"));
            }
            parse_entry(block(&entry.value)?)
        })
        .collect()
}

fn parse_entry(items: &[Item]) -> io::Result<UnitBonus> {
    let mut targets = Vec::new();
    for item in items {
        let key = member(item)?;
        match &item.value {
            Value::Block(fields) => targets.push(parse_target(key, fields)?),
            _ if matches!(key, b"type" | b"number" | b"id" | b"localization_key") => {}
            _ => {
                return Err(invalid(format!(
                    "unit modifier entry has an unknown field {}",
                    String::from_utf8_lossy(key)
                )));
            }
        }
    }
    Ok(UnitBonus {
        targets,
        kind: bytes(items, b"type")?,
        number: bytes(items, b"number")?
            .map(|value| integer(&value))
            .transpose()?,
        id: bytes(items, b"id")?,
        label: bytes(items, b"localization_key")?,
    })
}

fn parse_target(token: &[u8], items: &[Item]) -> io::Result<UnitBonusTarget> {
    let mut stats = Vec::new();
    let mut grants = Vec::new();
    for item in items {
        let key = member(item)?;
        match &item.value {
            Value::Block(fields) if key == b"battalion_mult" => grants.push(parse_grant(fields)?),
            Value::Block(_) => {}
            _ if key == b"combat_width" || UNRECORDED_SCALARS.contains(&key) => {}
            value => stats.push((key.to_vec(), Decimal::parse(scalar(value)?)?)),
        }
    }
    Ok(UnitBonusTarget {
        token: token.to_vec(),
        combat_width: decimal(items, b"combat_width")?,
        stats,
        grants,
    })
}

fn parse_grant(items: &[Item]) -> io::Result<UnitBonusGrant> {
    let mut stats = Vec::new();
    for item in items {
        let key = member(item)?;
        if !matches!(key, b"category" | b"add" | b"display_as_percentage") {
            stats.push((key.to_vec(), Decimal::parse(scalar(&item.value)?)?));
        }
    }
    Ok(UnitBonusGrant {
        category: bytes(items, b"category")?,
        add: field(items, b"add")?.map(boolean).transpose()?,
        display_as_percentage: field(items, b"display_as_percentage")?
            .map(boolean)
            .transpose()?,
        stats,
    })
}
