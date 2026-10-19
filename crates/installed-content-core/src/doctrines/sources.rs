//! Source inventories consumed by the country-doctrine tactic lookup.

use super::*;
use crate::{gui::native::BoundLocalization, parser::Item};

#[derive(Clone, Debug, Default)]
pub struct DefinitionSources {
    pub available: Vec<Item>,
    pub tactics: Vec<Vec<u8>>,
    pub tracks: Vec<Token>,
    /// Retains a lookup dependency even when a partial track registry
    /// could not resolve its authored operand to a native pointer.
    pub tracks_requested: bool,
    pub milestones: Vec<Vec<Vec<u8>>>,
    pub rewards: Vec<RewardSource>,
    pub benefits: Benefits,
    pub milestone_benefits: Vec<(i64, Benefits)>,
    pub xp_type: Vec<u8>,
    pub xp_cost: i64,
}

#[derive(Clone, Debug, Default)]
pub struct RewardSource {
    pub tactics: Vec<Vec<u8>>,
    /// Zero selects NDoctrines.DEFAULT_REWARD_MASTERY.
    pub mastery: i64,
    pub token: Vec<u8>,
    pub benefits: Benefits,
}

/// Authored doctrine bonuses share unit-stat and modifier readers with technologies.
#[derive(Clone, Debug, Default)]
pub struct Benefits {
    pub prose: Option<BoundLocalization>,
    pub units: Vec<Item>,
    pub modifiers: crate::modifiers::Modifiers,
    pub effects: Vec<Item>,
    pub tactics: Vec<Vec<u8>>,
}
fn benefits(target: &mut Benefits, fields: &[Item], names: &KeyNames) {
    for field in fields {
        let unit = field.keyword().is_some_and(|key| {
            names.sub_units.as_ref().is_some_and(|units| {
                units
                    .registered()
                    .any(|unit| unit.eq_ignore_ascii_case(&key.folded()))
                    || units.categories().contains(&key.folded())
            })
        });
        if field.keyword_is("description") {
            target.prose = Some(BoundLocalization::from_value(&field.value));
        } else if unit {
            target.units.push(field.clone());
        } else if field.keyword_is("modifier") {
            target
                .modifiers
                .read_all(field.value.items().unwrap_or_default());
        } else if field.keyword_is("effect") {
            target
                .effects
                .extend(field.value.items().unwrap_or_default().iter().cloned());
        } else if field.keyword_is("enable_tactic") {
            if let Some(value) = field.value.scalar() {
                target.tactics.push(parser::c_string(value).to_vec());
            }
        } else if field.keyword_is("custom_modifier_tooltip")
            || field.keyword_is("hidden_modifier")
            || field.keyword_is("added_modifier")
            || field
                .key
                .as_deref()
                .is_some_and(|key| crate::modifiers::registered(key, Some(names)) == Some(true))
        {
            target.modifiers.read(field);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TacticSources {
    /// The loaders append the reused pointer even for duplicate tokens
    /// Each occurrence observes the final object.
    pub grand_order: Vec<Token>,
    pub sub_order: Vec<Token>,
    pub grand: BTreeMap<Token, DefinitionSources>,
    pub sub: BTreeMap<Token, DefinitionSources>,
    pub tracks: BTreeMap<Token, BoundLocalization>,
    /// Supplied by the caller's already cached define reader.
    pub default_reward_mastery: Option<i64>,
    pub grand_unread: Option<String>,
    pub sub_unread: Option<String>,
    pub tracks_unread: Option<String>,
}

fn tactics(fields: &[Item]) -> Vec<Vec<u8>> {
    // Copy first-NUL value text directly without a string-field read.
    fields
        .iter()
        .filter(|field| field.keyword_is("enable_tactic"))
        .filter_map(|field| field.value.scalar())
        .map(|value| parser::c_string(value).to_vec())
        .collect()
}

pub(super) fn append(sources: &mut TacticSources, item: &Item, kind: u8, names: &KeyNames) {
    let token = Token::c_str(item.key.as_deref().unwrap_or_default());
    let fields = item.value.items().unwrap_or_default();
    if kind == 2 {
        let name = sources.tracks.entry(token).or_default();
        for field in fields.iter().filter(|field| field.keyword_is("name")) {
            *name = BoundLocalization::from_value(&field.value);
        }
        return;
    }
    let definition = if kind == 0 {
        sources.grand_order.push(token.clone());
        sources.grand.entry(token).or_default()
    } else {
        sources.sub_order.push(token.clone());
        sources.sub.entry(token).or_default()
    };
    definition.tactics.extend(tactics(fields));
    benefits(&mut definition.benefits, fields, names);
    for field in fields {
        if field.keyword_is("xp_type") {
            definition.xp_type = field.value.read_string().unwrap_or_default().to_vec();
        }
        if field.keyword_is("xp_cost")
            && let Some(value) = field
                .value
                .scalar()
                .and_then(|value| crate::crt::scan_i32(value).stored())
        {
            definition.xp_cost = i64::from(value) * 100_000;
        }
    }
    for field in fields {
        if field.keyword_is("available") {
            // Repeated unlock conjunctions append children to the same object.
            definition
                .available
                .extend(field.value.items().unwrap_or_default().iter().cloned());
        } else if kind == 0 && field.keyword_is("tracks") {
            // Statement reads retain unassigned scalars; assigned and nested forms skip.
            let requested = field
                .value
                .items()
                .unwrap_or_default()
                .iter()
                .filter(|entry| entry.key.is_none())
                .filter_map(|entry| entry.value.scalar())
                .map(Token::c_str)
                .collect::<Vec<_>>();
            definition.tracks_requested |= !requested.is_empty();
            definition.tracks.extend(
                requested
                    .into_iter()
                    .filter(|token| sources.tracks.contains_key(token)),
            );
        } else if kind == 1 && field.keyword_is("track") {
            // Append only resolved, non-null track references.
            let values: Vec<_> = if let Some(items) = field.value.items() {
                items
                    .iter()
                    .filter_map(|item| item.value.scalar())
                    .collect()
            } else {
                field.value.scalar().into_iter().collect()
            };
            definition.tracks_requested |= !values.is_empty();
            definition.tracks.extend(
                values
                    .into_iter()
                    .map(Token::c_str)
                    .filter(|token| sources.tracks.contains_key(token)),
            );
        } else if kind == 0 && field.keyword_is("milestones") {
            // Append fresh milestone records, paired by resolved track index.
            for milestone in field.value.items().unwrap_or_default() {
                let fields = milestone.value.items().unwrap_or_default();
                let mut detail = Benefits::default();
                benefits(&mut detail, fields, names);
                let required = fields
                    .iter()
                    .filter(|item| item.keyword_is("required_progress"))
                    .filter_map(crate::modifiers::read_fixed5)
                    .next_back()
                    .unwrap_or(0);
                definition.milestone_benefits.push((required, detail));
            }
            definition.milestones.extend(
                field
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .map(|milestone| tactics(milestone.value.items().unwrap_or_default())),
            );
        } else if kind == 1 && field.keyword_is("rewards") {
            // Append every reward, including repeated keys.
            for reward in field.value.items().unwrap_or_default() {
                let fields = reward.value.items().unwrap_or_default();
                let mut source = RewardSource {
                    tactics: tactics(fields),
                    mastery: 0,
                    token: parser::c_string(reward.key.as_deref().unwrap_or_default()).to_vec(),
                    benefits: Benefits::default(),
                };
                for field in fields.iter().filter(|field| field.keyword_is("mastery")) {
                    if let Some(value) = field.value.scalar().and_then(crate::fixed5::script_fixed5)
                    {
                        source.mastery = value; // Failed fixed5 scans retain the old value.
                    }
                }
                benefits(&mut source.benefits, fields, names);
                definition.rewards.push(source);
            }
        }
    }
}
