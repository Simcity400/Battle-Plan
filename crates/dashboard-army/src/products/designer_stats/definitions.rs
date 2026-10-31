//! A country's effective sub-unit definitions: the authored definition, then each researched
//! technology that holds a bonus for it, then the country's persistent unit bonuses in their
//! stored order, each with a multiplier of one.
//!
//! Within one bonus, the first block for the definition itself applies, then for each of the
//! definition's categories in order the first block for that category. A block adds to the
//! definition's words: each stat word adds the bonus times the multiplier (fuel capacity and
//! fuel consumption add the bonus once), combat width adds the scaled bonus, and a category
//! grant merges into the definition's grant of the same category and kind, or is appended.
//! After each block the definition's supply consumption and reliability are floored at zero and
//! its build cost at one tenth.
//!
//! Requirements come from the recorded clone (a bonus's `need` is already in them); terrain and
//! special adjusters and the definition's metadata (sprite, priorities, transport, own fuel
//! multiplier) are not calculator words. Any other authored bonus form is refused by name, and
//! the definition with it.

use dashboard_text::subunit_requirements::{Content, SubUnit};
use installed_content_core::{equipment::STAT_COUNT, parser::Item};

use super::kernel::{Definition, Grant, ONE, Slots, mul};

/// One block of a bonus: the words it adds to the sub-unit or category it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// The sub-unit or category key it names.
    pub target: String,
    /// It names a sub-unit category rather than a sub-unit.
    pub category: bool,
    /// Stat slots and fixed5 words in order.
    pub stats: Vec<(usize, i64)>,
    pub combat_width: i64,
    pub grants: Vec<Grant>,
}

/// One unit bonus: its blocks, its multiplier and what gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bonus {
    pub blocks: Vec<Block>,
    /// Fixed5 multiplier of its words: one for a technology and a persistent bonus.
    pub multiplier: i64,
    /// What gave it, for refusals and explanations.
    pub source: String,
}

/// Bonus keys that are not calculator words.
const NOT_WORDS: [&[u8]; 7] = [
    b"sprite",
    b"priority",
    b"ai_priority",
    b"own_equipment_fuel_consumption_mult",
    b"transport",
    b"need",
    b"need_equipment",
];

/// The special adjusters a bonus can carry besides the terrains.
const SPECIAL_ADJUSTERS: [&[u8]; 5] = [b"night", b"fort", b"river", b"amphibious", b"snow"];

/// The authored definition as the calculator reads it, before any bonus, with `requirements`
/// its effective (archetype, fixed5 amount) needs.
pub fn authored(
    content: &Content,
    subunit: &SubUnit,
    requirements: &[(String, i64)],
) -> Result<Definition, String> {
    let sheet = &subunit.sheet;
    if let Some(refusal) = sheet.refused.first() {
        return Err(refusal.why.clone());
    }
    let mut stats = [0_i64; STAT_COUNT];
    // A definition that does not say otherwise affects its division's speed.
    let mut affects_speed = true;
    for (key, text) in &sheet.words {
        if let Some(slot) = installed_content_core::equipment::stat_index(key.as_bytes()) {
            stats[slot] = fixed5(&subunit.key, key, text)?;
        } else if key.eq_ignore_ascii_case(AFFECTS_SPEED) {
            affects_speed = match text.to_ascii_lowercase().as_str() {
                "yes" => true,
                "no" => false,
                _ => {
                    return Err(format!(
                        "sub-unit {}: {key} = {text} is neither yes nor no",
                        subunit.key
                    ));
                }
            };
        }
    }
    let archetype = |name: &str| -> Result<usize, String> {
        let archetype = content
            .archetype(name)
            .map_err(|why| format!("sub-unit {}: {why}", subunit.key))?;
        content.equipment.get(archetype).ok_or_else(|| {
            format!(
                "sub-unit {}: archetype {archetype} is not equipment",
                subunit.key
            )
        })
    };
    let requirements = requirements
        .iter()
        .map(|(name, amount)| Ok((archetype(name)?, *amount)))
        .collect::<Result<Vec<_>, String>>()?;
    let support = installed_content_core::equipment::category_mask("support")
        .ok_or("the executable's equipment categories have no support")?;
    Ok(Definition {
        stats,
        combat_width: sheet.combat_width,
        manpower: sheet.manpower,
        training_time: sheet.training_time,
        support_type: subunit.category & support != 0,
        support_group: sheet
            .group
            .as_deref()
            .is_some_and(|group| group.eq_ignore_ascii_case("support")),
        categories: sheet.categories.clone(),
        grants: sheet
            .grants
            .iter()
            .map(|grant| {
                let mut stats = Vec::with_capacity(grant.words.len());
                for (key, text) in &grant.words {
                    match installed_content_core::equipment::stat_index(key.as_bytes()) {
                        Some(slot) => stats.push((slot, fixed5(&subunit.key, key, text)?)),
                        None if key == "display_as_percentage" => {}
                        None => {
                            return Err(format!(
                                "sub-unit {}: battalion_mult field {key} is not reproduced",
                                subunit.key
                            ));
                        }
                    }
                }
                Ok(Grant {
                    category: grant.category.clone(),
                    add: grant.add,
                    stats,
                })
            })
            .collect::<Result<_, String>>()?,
        requirements,
        critical: sheet
            .essential
            .iter()
            .map(|name| archetype(name))
            .collect::<Result<_, String>>()?,
        transport: sheet.transport.as_deref().map(archetype).transpose()?,
        own_fuel_consumption: sheet.own_fuel_consumption.unwrap_or(ONE),
        affects_speed,
    })
}

/// The definition word that takes its equipment speed out of the division's speed selection.
const AFFECTS_SPEED: &str = "affects_speed";

/// An authored stat word as fixed5, or why it is no number.
fn fixed5(subunit: &str, key: &str, text: &str) -> Result<i64, String> {
    dashboard_text::subunit_requirements::authored_fixed5(text.as_bytes())
        .map_err(|_| format!("sub-unit {subunit}: {key} = {text} is not a number"))
}

/// A technology's authored unit bonus: one block per sub-unit or category it names, in authored
/// order; a key names a sub-unit when `content` has one by it, else a category. `terrains` are
/// the content's terrain names, whose adjusters are no calculator words.
pub fn authored_bonus(
    content: &Content,
    items: &[Item],
    terrains: &[Vec<u8>],
    multiplier: i64,
    source: String,
) -> Result<Bonus, String> {
    let refuse = |what: String| Err(format!("{source}: {what} is not reproduced"));
    let mut blocks = Vec::with_capacity(items.len());
    for selector in items {
        let Some(target) = selector.keyword() else {
            return refuse("an unkeyed unit bonus".into());
        };
        let target = String::from_utf8_lossy(&target.folded()).into_owned();
        let mut block = Block {
            category: content.subunit(&target).is_none(),
            target,
            ..Block::default()
        };
        for item in selector.value.items().unwrap_or_default() {
            let Some(key) = item.keyword() else {
                return refuse("an unkeyed bonus entry".into());
            };
            let key = key.folded();
            let name = String::from_utf8_lossy(&key).into_owned();
            if let Some(slot) = installed_content_core::equipment::stat_index(&key) {
                let Some(value) = installed_content_core::modifiers::read_fixed5(item) else {
                    return refuse(format!("bonus {name}"));
                };
                block.stats.push((slot, value));
            } else if key == b"combat_width" {
                let Some(value) = installed_content_core::modifiers::read_fixed5(item) else {
                    return refuse("bonus combat_width".into());
                };
                block.combat_width = block.combat_width.wrapping_add(value);
            } else if key == b"battalion_mult" {
                block
                    .grants
                    .push(authored_grant(item).map_err(|what| format!("{source}: {what}"))?);
            } else if !(NOT_WORDS.contains(&key.as_slice())
                || SPECIAL_ADJUSTERS.contains(&key.as_slice())
                || terrains
                    .iter()
                    .any(|terrain| terrain.eq_ignore_ascii_case(&key)))
            {
                return refuse(format!("bonus {name}"));
            }
        }
        blocks.push(block);
    }
    Ok(Bonus {
        blocks,
        multiplier,
        source,
    })
}

fn authored_grant(item: &Item) -> Result<Grant, String> {
    let mut category = None;
    let mut add = false;
    let mut stats = Vec::new();
    for field in item.value.items().unwrap_or_default() {
        if field.keyword_is("category") {
            category = field
                .value
                .read_string()
                .and_then(|name| std::str::from_utf8(name).ok())
                .map(str::to_owned);
        } else if field.keyword_is("add") {
            add = field.value.read_string() == Some(b"yes".as_slice());
        } else if field.keyword_is("display_as_percentage") {
        } else if let Some(slot) = field
            .keyword()
            .and_then(|key| installed_content_core::equipment::stat_index(&key.folded()))
            && let Some(value) = installed_content_core::modifiers::read_fixed5(field)
        {
            stats.push((slot, value));
        } else {
            return Err("a battalion_mult entry is not reproduced".into());
        }
    }
    Ok(Grant {
        category: category.ok_or("a battalion_mult names no category")?,
        add,
        stats,
    })
}

/// The blocks of `bonus` that reach `subunit`, in the order they apply: the first for the
/// definition itself, then for each of its categories the first for that category.
pub fn reaching<'a>(subunit: &SubUnit, bonus: &'a Bonus) -> Vec<&'a Block> {
    let first = |name: &str, category: bool| {
        bonus
            .blocks
            .iter()
            .find(|block| block.category == category && block.target.eq_ignore_ascii_case(name))
    };
    first(&subunit.key, false)
        .into_iter()
        .chain(
            subunit
                .sheet
                .categories
                .iter()
                .filter_map(|name| first(name, true)),
        )
        .collect()
}

/// Applies one block of `bonus` to `definition`.
pub fn apply(definition: &mut Definition, slots: &Slots, block: &Block, multiplier: i64) {
    for &(slot, value) in &block.stats {
        let add = if slot == slots.fuel_capacity || slot == slots.fuel_consumption {
            value
        } else {
            mul(value, multiplier)
        };
        definition.stats[slot] = definition.stats[slot].wrapping_add(add);
    }
    definition.combat_width = definition
        .combat_width
        .wrapping_add(mul(block.combat_width, multiplier));
    for grant in &block.grants {
        let scaled = grant
            .stats
            .iter()
            .map(|&(slot, value)| (slot, mul(value, multiplier)));
        match definition.grants.iter_mut().find(|held| {
            held.add == grant.add && held.category.eq_ignore_ascii_case(&grant.category)
        }) {
            Some(held) => {
                for (slot, value) in scaled {
                    match held
                        .stats
                        .iter_mut()
                        .find(|(existing, _)| *existing == slot)
                    {
                        Some((_, existing)) => *existing = existing.wrapping_add(value),
                        None => held.stats.push((slot, value)),
                    }
                }
            }
            None => definition.grants.push(Grant {
                category: grant.category.clone(),
                add: grant.add,
                stats: scaled.collect(),
            }),
        }
    }
    definition.stats[slots.supply_consumption] = definition.stats[slots.supply_consumption].max(0);
    definition.stats[slots.reliability] = definition.stats[slots.reliability].max(0);
    definition.stats[slots.build_cost_ic] = definition.stats[slots.build_cost_ic].max(ONE / 10);
}

/// The effective definition: the authored one with every bonus in order.
pub fn effective(
    content: &Content,
    slots: &Slots,
    subunit: &SubUnit,
    requirements: &[(String, i64)],
    bonuses: &[&Bonus],
) -> Result<Definition, String> {
    let mut definition = authored(content, subunit, requirements)?;
    for bonus in bonuses {
        for block in reaching(subunit, bonus) {
            apply(&mut definition, slots, block, bonus.multiplier);
        }
    }
    Ok(definition)
}
