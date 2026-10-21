//! Ordinary land-model choice over the shared sub-unit requirement resolver.
//! Authored content and recorded local clones live below the viewer; scoring,
//! equipment exclusions and model-country selection remain presentation rules.
use dashboard_text::subunit_requirements as shared;
pub use dashboard_text::subunit_requirements::{
    Definition, EquipmentModel, Group, Sources, SubUnit, Template, text,
};
use installed_content_core::ResolvedLayers;
use replay_data::rows::{DivisionEquipmentRow, VariantDefinitionRow};
use std::{cmp::Reverse, collections::BTreeMap, io, ops::Deref};
use viewer_foundation::table_access::TokenNames;

pub(crate) mod facts;
pub(crate) mod profile;
use profile::{Stage, Stages};

#[derive(Clone, Debug, Default)]
pub struct Content(shared::Content, facts::Retained);
impl Deref for Content {
    type Target = shared::Content;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Content {
    /// An empty, lazy fact view. No variant or live answer is evaluated here.
    pub(crate) fn classification_facts(&self, frame: &replay_frame::frame::DecodedFrame) -> Option<facts::Hour> {
        self.1.hour(frame)
    }

    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        shared::Content::load(layers).map(|content| Self(content, facts::Retained::default()))
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_test_documents(documents: &[&[u8]]) -> Self {
        Self(
            shared::Content::from_test_documents(documents),
            facts::Retained::default(),
        )
    }
    /// Supply only this division's equipment rows, in any order;
    /// the recorded ordinal recovers native order before the stable quantity sort.
    /// Missing authored/recorded identities are errors, never silent re-ranking.
    pub fn select_pair<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[&DivisionEquipmentRow],
        variants: &BTreeMap<u64, &'a VariantDefinitionRow>,
        tokens: &TokenNames<'_>,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        self.select_pair_profiled(
            template,
            model_category,
            excluded_variants,
            equipment,
            variants,
            tokens,
            &Stages::new(false),
        )
    }

    /// Coarse stage timing only: both stable sorts and the input/error order
    /// are identical to the ordinary entry; no clock is read per equipment row.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn select_pair_profiled<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[&DivisionEquipmentRow],
        variants: &BTreeMap<u64, &'a VariantDefinitionRow>,
        tokens: &TokenNames<'_>,
        stages: &Stages,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        self.select_pair_classified(
            template, model_category, excluded_variants, equipment, variants, tokens, stages, None,
        )
    }

    /// Only successful static row classification is shared; all candidate
    /// admission, both stable sorts and live ranking below remain ordinary.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn select_pair_classified<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[&DivisionEquipmentRow],
        variants: &BTreeMap<u64, &'a VariantDefinitionRow>,
        tokens: &TokenNames<'_>,
        stages: &Stages,
        facts: Option<&facts::Hour>,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        let Some(representative) = &template.representative else {
            return Ok(None);
        };
        let mut candidates = stages.measure(Stage::PairClassify, || -> io::Result<_> {
            // The first strictly largest positive requirement.
            let mut largest = None;
            let mut largest_amount = 0;
            for (archetype, amount) in representative.requirements.iter() {
                if *amount > largest_amount {
                    largest = Some(archetype.as_str());
                    largest_amount = *amount;
                }
            }
            let mut candidates = Vec::new();
            for row in equipment {
                let variant = *variants.get(&row.variant_handle).ok_or_else(|| {
                    invalid(format!("missing equipment variant {}", row.variant_handle))
                })?;
                let (key, archetype, cached_category) = match facts {
                    Some(facts) => {
                        let (key, archetype, category) = facts.classify(self, variant, tokens)?;
                        (key, archetype, Some(category))
                    }
                    None => {
                        let key = self.equipment_key(tokens, variant).map_err(invalid)?;
                        (key, self.archetype(key)?, None)
                    }
                };
                let matches = match model_category {
                    Some(category) => {
                        cached_category.unwrap_or_else(|| {
                            self.equipment_content().category(
                                self.equipment_content()
                                    .named(key)
                                    .expect("resolved equipment"),
                            )
                        }) == category
                    }
                    None => largest == Some(archetype),
                };
                if matches {
                    candidates.push((*row, variant, key, archetype));
                }
            }
            Ok(candidates)
        })?;
        stages.measure(Stage::PairSorts, || {
            candidates.sort_by_key(|(row, _, _, _)| row.ordinal);
            candidates.sort_by_key(|(row, _, _, _)| Reverse(row.amount_raw));
        });
        stages.measure(Stage::PairRank, || {
            for (_, variant, key, archetype) in candidates {
                if model_category.is_none() {
                    return Ok(Some(SelectedPair {
                        variant,
                        subunit: representative.subunit,
                        identity: representative.identity,
                    }));
                }
                if excluded_variants.contains(&variant.id) {
                    continue;
                }
                let equipment_priority = self
                    .equipment_content()
                    .named(key)
                    .expect("resolved equipment")
                    .model_priority;
                let mut selected = None;
                let mut best = -1;
                for group in &template.groups {
                    if !group
                        .requirements
                        .iter()
                        .any(|(key, amount)| key == archetype && *amount > 0)
                    {
                        continue;
                    }
                    // Native signed dword arithmetic, including its wrap behavior.
                    let score = group.subunit.model_priority.unwrap_or_else(|| {
                        equipment_priority
                            .wrapping_add(10)
                            .wrapping_add(group.count.wrapping_mul(100))
                            .wrapping_mul(10)
                    });
                    if score > best {
                        best = score;
                        selected = Some(group);
                    }
                }
                if let Some(group) = selected {
                    return Ok(Some(SelectedPair {
                        variant,
                        subunit: group.subunit,
                        identity: group.identity,
                    }));
                }
            }
            Ok(None)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SelectedPair<'a> {
    pub variant: &'a VariantDefinitionRow,
    pub subunit: &'a SubUnit,
    pub identity: Definition,
}
/// Model selection prefers a signed-positive variant origin over its creator. A missing
/// origin branch does not remap the ordinary roster country to its original tag.
pub fn model_country(pair: SelectedPair<'_>, roster_owner: u32) -> u32 {
    variant_country(pair.variant, pair.subunit, roster_owner)
}

/// The same rule for any (variant, sub-unit) pair and fallback country.
pub fn variant_country(variant: &VariantDefinitionRow, subunit: &SubUnit, fallback: u32) -> u32 {
    let category = variant.classifier_flags;
    let source = if (variant.origin_link as i32) > 0 {
        variant.origin_link
    } else {
        variant.creator_link
    };
    if category & 0x20 == 0 && category == subunit.category && (source as i32) > 0 {
        source
    } else {
        fallback
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
#[cfg(test)]
mod tests;
