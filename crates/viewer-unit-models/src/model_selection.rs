use dashboard_text::subunit_requirements as shared;
pub use dashboard_text::subunit_requirements::{
    Definition, EquipmentModel, Group, SourceIndex, Sources, SubUnit, Template, text,
};
use content_reader_layers::ResolvedLayers;
use replay_data::rows::{DivisionEquipmentRow, VariantDefinitionRow};
use std::{cmp::Reverse, io, ops::Deref};
use dashboard_text::table_access::TokenNames;

pub(crate) mod facts;
pub(crate) mod model_facts;
pub(crate) mod profile;
use profile::{Stage, Stages};

#[derive(Clone, Copy, Debug, Default)]
pub struct Variants<'a>(&'a [VariantDefinitionRow]);

impl<'a> Variants<'a> {
    pub fn new(rows: &'a [VariantDefinitionRow]) -> Self {
        Self(rows)
    }

    pub fn get(self, id: u64) -> Option<&'a VariantDefinitionRow> {
        self.0
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|at| &self.0[at])
    }
}

#[derive(Clone, Debug, Default)]
pub struct Content(shared::Content, facts::Retained, model_facts::Retained);
impl Deref for Content {
    type Target = shared::Content;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Content {
    pub(crate) fn classification_facts(&self, frame: &replay_frame::frame::DecodedFrame) -> Option<facts::Hour> {
        self.1.hour(frame)
    }

    pub(crate) fn model_facts(&self) -> &model_facts::Retained {
        &self.2
    }

    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        shared::Content::load(layers).map(|content| {
            Self(
                content,
                facts::Retained::default(),
                model_facts::Retained::default(),
            )
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_test_documents(documents: &[&[u8]]) -> Self {
        Self(
            shared::Content::from_test_documents(documents),
            facts::Retained::default(),
            model_facts::Retained::default(),
        )
    }
    pub fn select_pair<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[DivisionEquipmentRow],
        variants: Variants<'a>,
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

    #[expect(clippy::too_many_arguments, reason = "the template, its category and exclusions, the equipment and variants, the tokens and stages")]
    pub(crate) fn select_pair_profiled<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[DivisionEquipmentRow],
        variants: Variants<'a>,
        tokens: &TokenNames<'_>,
        stages: &Stages,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        self.select_pair_classified(
            template, model_category, excluded_variants, equipment, variants, tokens, stages, None,
        )
    }

    #[expect(clippy::too_many_arguments, reason = "the template, its category and exclusions, the equipment and variants, the tokens, stages and facts")]
    pub(crate) fn select_pair_classified<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[DivisionEquipmentRow],
        variants: Variants<'a>,
        tokens: &TokenNames<'_>,
        stages: &Stages,
        facts: Option<&facts::Hour>,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        let Some(representative) = &template.representative else {
            return Ok(None);
        };
        let mut candidates = stages.measure(Stage::PairClassify, || -> io::Result<_> {
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
                replay_frame::frame::kept::note_row(
                    replay_data::rows::TableId::VariantDefinitions,
                    u128::from(row.variant_handle),
                    u64::MAX,
                );
                let variant = variants.get(row.variant_handle).ok_or_else(|| {
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
                    candidates.push((row, variant, key, archetype));
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
pub fn model_country(pair: SelectedPair<'_>, roster_owner: u32) -> u32 {
    variant_country(pair.variant, pair.subunit, roster_owner)
}

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
