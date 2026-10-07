//! Source inventories consumed by CountryDoctrineState's tactic lookup (1413B3560).

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
}

#[derive(Clone, Debug, Default)]
pub struct RewardSource {
    pub tactics: Vec<Vec<u8>>,
    /// Zero selects NDoctrines.DEFAULT_REWARD_MASTERY (1409D7F50).
    pub mastery: i64,
}

#[derive(Clone, Debug, Default)]
pub struct TacticSources {
    /// The loaders append the reused pointer even for duplicate tokens
    /// (14012F900/1401310E0). Each occurrence observes the final object.
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
    // 141517770 copies the value slot's C string, without ReadStringInto.
    fields
        .iter()
        .filter(|field| field.keyword_is("enable_tactic"))
        .filter_map(|field| field.value.scalar())
        .map(|value| parser::c_string(value).to_vec())
        .collect()
}

pub(super) fn append(sources: &mut TacticSources, item: &Item, kind: u8) {
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
    for field in fields {
        if field.keyword_is("available") {
            // CDoctrineBaseTemplate+208 owns this conjunction; repeated
            // ReadObjectFromParser calls append children (1409BF8C0).
            definition
                .available
                .extend(field.value.items().unwrap_or_default().iter().cloned());
        } else if kind == 0 && field.keyword_is("tracks") {
            // Uniform statement reader 1409BFE50 keeps unassigned scalar
            // entries; its native dispatch skipped assigned/nested forms.
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
            // 1409D89D0 appends only resolved non-null track pointers.
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
            // 1409C06E0 appends fresh records. GetMilestoneForTrackIndex
            // 1409C0BD0 pairs this array by the resolved track index.
            definition.milestones.extend(
                field
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .map(|milestone| tactics(milestone.value.items().unwrap_or_default())),
            );
        } else if kind == 1 && field.keyword_is("rewards") {
            // 1409D9920 appends every reward, including repeated keys.
            for reward in field.value.items().unwrap_or_default() {
                let fields = reward.value.items().unwrap_or_default();
                let mut source = RewardSource {
                    tactics: tactics(fields),
                    mastery: 0,
                };
                for field in fields.iter().filter(|field| field.keyword_is("mastery")) {
                    if let Some(value) = field.value.scalar().and_then(crate::fixed5::script_fixed5)
                    {
                        source.mastery = value; // ReadFixed5 retains the old value on failure.
                    }
                }
                definition.rewards.push(source);
            }
        }
    }
}
