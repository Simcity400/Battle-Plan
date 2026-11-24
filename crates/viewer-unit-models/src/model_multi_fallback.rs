
use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet};
use dashboard_text::table_access::TokenNames;

use replay_data::{rows::DivisionTemplateIconRow, rows::VariantDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{
    equipment_graphics::{self, bridge, legacy},
    model_labels::{self, Label},
    model_selection::{self, Group, Template},
};
use content_reader_equipment_graphics::equipment_graphics::{Candidate, context, countries::ModelCountry};

pub const LABEL_ORDERING: &str = "equipment model option labels unavailable";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub entity: String,
    pub subunit_key: String,
    pub graphical: bool,
}

pub struct Inputs<'a, 'b> {
    pub template: &'a DivisionTemplateIconRow,
    pub composition: &'a Template<'b>,
    pub content: &'b model_selection::Content,
    pub graphics: &'a content_reader_equipment_graphics::equipment_graphics::Content,
    pub geography: &'a context::Prepared,
    pub conventional: &'a bridge::Prepared<'b>,
    pub countries: &'a BTreeMap<u32, ModelCountry>,
    pub links: &'a BTreeMap<u32, u32>,
    pub definitions: model_selection::Variants<'b>,
    pub tokens: &'a TokenNames<'b>,
    pub viewing_country_ref: u32,
    pub limits: &'a equipment_graphics::Limits<'a>,
    pub registered: &'a HashSet<String>,
    pub labels: &'a model_labels::Labels<'a>,
}

pub fn has_qualifying_requirements(
    template: &DivisionTemplateIconRow,
    composition: &Template<'_>,
) -> bool {
    let selected = template.model_category.unwrap_or_else(|| {
        composition
            .representative
            .as_ref()
            .map_or(0, |group| group.subunit.category)
    });
    let strict = template.model_category.is_some();
    composition.groups.iter().any(|group| {
        category_reaches(selected, group.subunit.category, strict)
            && group.requirements.iter().any(|(_, amount)| *amount > 0)
    })
}

#[derive(Clone)]
struct Row {
    resolved: Resolved,
    score: i64,
    label: Label,
}

pub fn resolve(inputs: &Inputs<'_, '_>) -> Result<Option<Resolved>, String> {
    let selected_mask = inputs.template.model_category.unwrap_or_else(|| {
        inputs
            .composition
            .representative
            .as_ref()
            .map_or(0, |group| group.subunit.category)
    });
    if inputs.composition.representative.is_none() && inputs.template.model_category.is_none() {
        return Err("template model fallback has no visual representative".into());
    }
    if selected_mask as u32 == 0 || inputs.composition.groups.is_empty() {
        return Ok(None);
    }
    let strict = inputs.template.model_category.is_some();
    let mut required = BTreeSet::new();
    for group in &inputs.composition.groups {
        if !category_reaches(selected_mask, group.subunit.category, strict) {
            continue;
        }
        for (archetype, amount) in group.requirements.iter() {
            if *amount > 0 {
                required.insert(archetype.as_str());
            }
        }
    }
    if required.is_empty() {
        return Ok(None);
    }
    let mut roster = inputs
        .conventional
        .no_pair_variants(inputs.viewing_country_ref, &required)?;
    if roster.is_empty() {
        return Ok(None);
    }
    let viewing = legacy::Country {
        tag: "",
        cosmetic: "",
        graphical_culture: "",
        reference: inputs.viewing_country_ref as i32,
        canonical_slot: inputs
            .conventional
            .canonical_slot_for(inputs.viewing_country_ref)? as i32,
    };
    roster.sort_by(|left, right| legacy::compare(viewing, left, right));

    let excluded: BTreeSet<u64> = inputs.template.excluded_variants.iter().copied().collect();
    let has_match = inputs.composition.groups.iter().any(|group| {
        category_reaches(selected_mask, group.subunit.category, strict)
            && roster.iter().any(|variant| {
                !excluded.contains(&variant.handle)
                    && positive_match(group.requirements.as_ref(), variant.equipment.archetype)
            })
    });
    if !has_match {
        return Ok(None);
    }
    let (initial, suffix_country) = template_countries(inputs.template)?;
    evaluate(
        inputs,
        &roster,
        strict,
        selected_mask,
        initial,
        suffix_country,
        &excluded,
    )
}

fn template_countries(template: &DivisionTemplateIconRow) -> Result<(u32, u32), String> {
    let initial = template.country;
    if initial <= 0 {
        return Err(format!(
            "template {} model country is nonpositive ({initial})",
            template.template
        ));
    }
    let initial = initial as u32;
    let suffix_country = if template.foreign_country > 0 {
        template.foreign_country as u32
    } else {
        initial
    };
    Ok((initial, suffix_country))
}

fn evaluate<'a>(
    inputs: &Inputs<'_, 'a>,
    roster: &[legacy::Variant<'a>],
    strict: bool,
    selected_mask: u64,
    initial: u32,
    suffix_country: u32,
    excluded: &BTreeSet<u64>,
) -> Result<Option<Resolved>, String> {
    let mut variant_bins: [Vec<Row>; 3] = std::array::from_fn(|_| Vec::new());
    let mut suffix_bins: [Vec<Row>; 3] = std::array::from_fn(|_| Vec::new());
    let mut variant_seen = BTreeSet::new();
    for group in &inputs.composition.groups {
        if !category_reaches(selected_mask, group.subunit.category, strict) {
            continue;
        }
        for variant in roster {
            if excluded.contains(&variant.handle)
                || !positive_match(group.requirements.as_ref(), variant.equipment.archetype)
            {
                continue;
            }
            replay_frame::frame::kept::note_row(
                replay_data::rows::TableId::VariantDefinitions,
                u128::from(variant.handle),
                u64::MAX,
            );
            let definition = inputs.definitions.get(variant.handle).ok_or_else(|| {
                format!(
                    "model roster variant {} lacks its definition",
                    variant.handle
                )
            })?;
            let graphic_country = model_country(group.subunit, definition, initial);
            let candidates = pair_candidates(inputs, group, variant, definition, graphic_country)?;
            let visual_priority = group.subunit.model_priority;
            let count100 = group.count.wrapping_mul(100);
            let normal = visual_priority.unwrap_or_else(|| {
                count100
                    .wrapping_add(10)
                    .wrapping_add(variant.equipment.model_priority)
                    .wrapping_mul(10)
            });
            let explicit =
                visual_priority.unwrap_or_else(|| count100.wrapping_add(110).wrapping_mul(10));
            let explicit_name = definition.explicit_entity.as_slice();
            for outer in low_bits(selected_mask) {
                admit_variant_rows(
                    &mut variant_bins,
                    &mut variant_seen,
                    outer,
                    group.subunit.category,
                    strict,
                    visual_priority.is_some(),
                    normal,
                    explicit,
                    explicit_name,
                    &candidates,
                );
                let suffix_bin = if group.subunit.category == u64::from(outer) {
                    Some(0)
                } else if strict {
                    None
                } else if visual_priority.is_some() {
                    Some(1)
                } else {
                    Some(2)
                };
                if let Some(bin) = suffix_bin {
                    append_suffix(inputs, group, suffix_country, normal, &mut suffix_bins[bin])?;
                }
            }
        }
    }
    second_row(&inputs.labels.default_label()?, variant_bins, suffix_bins)
}

#[expect(clippy::too_many_arguments, reason = "the bins and seen names it fills, and each recorded fact one variant row is admitted on")]
fn admit_variant_rows(
    bins: &mut [Vec<Row>; 3],
    seen: &mut BTreeSet<String>,
    outer: u32,
    subunit_mask: u64,
    strict: bool,
    visual_present: bool,
    normal: i32,
    explicit: i32,
    explicit_name: &[u8],
    rows: &[Row],
) {
    for inner in low_bits(subunit_mask) {
        let bin = if inner == outer {
            Some(0)
        } else if strict {
            None
        } else if visual_present {
            Some(1)
        } else {
            Some(2)
        };
        let Some(bin) = bin else { continue };
        for row in rows.iter().rev() {
            if !seen.insert(row.resolved.entity.clone()) {
                continue;
            }
            let priority = if row.resolved.entity.as_bytes() == explicit_name {
                explicit
            } else {
                normal
            };
            let score = i64::from(priority) * 100_000;
            let at = bins[bin].partition_point(|old| old.score > score);
            bins[bin].insert(
                at,
                Row {
                    resolved: row.resolved.clone(),
                    score,
                    label: row.label.clone(),
                },
            );
        }
    }
}

fn second_row(
    default: &[u8],
    variants: [Vec<Row>; 3],
    suffix: [Vec<Row>; 3],
) -> Result<Option<Resolved>, String> {
    let rows = drain_rows(variants, suffix);
    if rows.iter().all(|row| {
        rows.first()
            .is_some_and(|first| first.resolved.entity == row.resolved.entity)
    }) {
        return Ok(rows.into_iter().next().map(|row| row.resolved));
    }
    let labels: Vec<&Label> = rows.iter().map(|row| &row.label).collect();
    let at = model_labels::second_row(default, &labels)
        .map_err(|reason| format!("{LABEL_ORDERING}: {reason}"))?;
    Ok(at.map(|at| rows[at].resolved.clone()))
}

fn drain_rows(variants: [Vec<Row>; 3], suffix: [Vec<Row>; 3]) -> Vec<Row> {
    let mut seen = BTreeSet::from([String::new()]);
    variants
        .into_iter()
        .flatten()
        .chain(suffix.into_iter().flatten())
        .filter(|row| seen.insert(row.resolved.entity.clone()))
        .collect()
}

fn low_bits(mask: u64) -> impl Iterator<Item = u32> {
    let mut remaining = mask as u32;
    std::iter::from_fn(move || {
        if remaining == 0 {
            return None;
        }
        let bit = remaining & remaining.wrapping_neg();
        remaining &= remaining - 1;
        Some(bit)
    })
}

fn category_reaches(selected: u64, subunit: u64, strict: bool) -> bool {
    selected as u32 != 0 && (!strict || (selected as u32 & subunit as u32) != 0)
}

fn positive_match(requirements: &[(String, i64)], archetype: &str) -> bool {
    requirements
        .iter()
        .any(|(key, amount)| key == archetype && *amount > 0)
}

fn model_country(
    subunit: &model_selection::SubUnit,
    variant: &VariantDefinitionRow,
    initial: u32,
) -> u32 {
    model_selection::variant_country(variant, subunit, initial)
}

pub fn creator_operands<'a>(
    definition: &VariantDefinitionRow,
    links: &BTreeMap<u32, u32>,
    countries: &'a BTreeMap<u32, ModelCountry>,
) -> Result<(Option<&'a str>, Option<&'a str>), String> {
    let creator = definition.creator_link;
    if (creator as i32) <= 0 {
        return Ok((None, None));
    }
    let country = links
        .get(&creator)
        .and_then(|slot| countries.get(slot))
        .ok_or_else(|| {
            format!(
                "variant {} creator {creator} has no model country",
                definition.id
            )
        })?;
    let culture = country
        .graphical_culture
        .as_deref()
        .ok_or_else(|| format!("variant {} creator culture is unavailable", definition.id))?;
    let ideology = country.ruling_ideology.as_ref().ok_or_else(|| {
        format!(
            "variant {} creator ruling party is unavailable",
            definition.id
        )
    })?;
    Ok((
        ideology.as_deref().filter(|token| !token.is_empty()),
        Some(culture),
    ))
}

pub fn view_country_ref(frame: &DecodedFrame, slot: u32) -> Result<u32, String> {
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|error| error.to_string())?;
    let mut matches = countries
        .iter()
        .filter(|country| country.canonical_slot == slot);
    let country = matches
        .next()
        .ok_or_else(|| format!("viewing country slot {slot} has no Countries row"))?;
    if matches.next().is_some() {
        return Err(format!(
            "viewing country slot {slot} has several Countries rows"
        ));
    }
    if (country.id as i32) <= 0 {
        return Err(format!(
            "viewing country slot {slot} has nonpositive native reference {}",
            country.id
        ));
    }
    Ok(country.id)
}

fn country<'a>(inputs: &'a Inputs<'_, '_>, reference: u32) -> Result<legacy::Country<'a>, String> {
    let slot = inputs.conventional.canonical_slot_for(reference)?;
    let country = inputs
        .countries
        .get(&slot)
        .ok_or_else(|| format!("model country {reference} naming is unavailable"))?;
    let culture = country
        .graphical_culture
        .as_deref()
        .ok_or_else(|| format!("model country {reference} culture is unavailable"))?;
    Ok(legacy::Country {
        tag: &country.tag,
        cosmetic: &country.cosmetic,
        graphical_culture: culture,
        reference: reference as i32,
        canonical_slot: slot as i32,
    })
}

fn pair_candidates<'a>(
    inputs: &Inputs<'_, 'a>,
    group: &Group<'a>,
    variant: &legacy::Variant<'a>,
    definition: &VariantDefinitionRow,
    country_ref: u32,
) -> Result<Vec<Row>, String> {
    let equipment_key = inputs.content.equipment_key(inputs.tokens, definition)?;
    let equipment = inputs
        .content
        .equipment_model(equipment_key)
        .map_err(|error| error.to_string())?;
    let family = Some((
        equipment.family,
        Some(
            inputs
                .content
                .equipment_model(equipment.family)
                .map_err(|error| error.to_string())?
                .archetype,
        ),
    ));
    let (ideology, culture) = creator_operands(definition, inputs.links, inputs.countries)?;
    let geography = inputs.geography.context(country_ref)?;
    let capital_continent = match geography {
        Some(country) => country
            .capital_continent
            .as_ref()
            .map(Option::as_deref)
            .map_err(String::as_str),
        None => Ok(None),
    };
    let graphical = inputs.graphics.lookup_with_limits(
        &content_reader_equipment_graphics::equipment_graphics::Context {
            equipment: equipment_key,
            archetype: Some(equipment.archetype),
            family,
            subunit: Some(&group.subunit.key),
            ideology,
            culture,
            country: geography.map(|country| country.country_tag.as_str()),
            faction_tags: geography.map_or(&[], |country| country.faction_tags.as_slice()),
            capital_continent,
        },
        inputs.limits,
        |name| inputs.registered.contains(name),
    )?;
    let graphical_names: BTreeSet<_> = graphical.iter().map(|row| row.entity.clone()).collect();
    let archetype_label: Label = inputs.labels.archetype(&equipment);
    let first = group.requirements.first().map(|(key, _)| key.as_str());
    let available = inputs.conventional.primary_variants(country_ref)?;
    let best = inputs.conventional.best_candidates(country_ref)?;
    let mut candidates = graphical;
    let owner = country(inputs, country_ref)?;
    let sources: BTreeMap<String, legacy::Source> = legacy::append_candidates_with_sources(
        &legacy::Context {
            country: owner,
            subunit: legacy::Subunit {
                key: &group.subunit.key,
                sprite: &group.subunit.sprite,
                categories: group.subunit.category,
                first_archetype: first,
            },
            selected: Some(variant),
            available: &available,
            best_candidates: &best,
        },
        &mut candidates,
        0,
        |name| inputs.registered.contains(name),
    )?
    .into_iter()
    .collect();
    let own = inputs.labels.display(definition, equipment_key);
    let mut labels: Vec<Label> = vec![inputs.labels.default_label()];
    labels.extend(candidates.iter().map(|candidate| {
        if graphical_names.contains(&candidate.entity) {
            return archetype_label.clone();
        }
        match sources.get(&candidate.entity) {
            Some(legacy::Source::Variant(handle) | legacy::Source::Best(handle))
                if *handle == variant.handle =>
            {
                own.clone()
            }
            Some(source) => source_label(inputs, group, *source),
            None => Err(format!(
                "candidate {} has no recorded producer",
                candidate.entity
            )),
        }
    }));
    model_labels::number_repeats(&mut labels);
    Ok(candidates
        .into_iter()
        .zip(labels.into_iter().skip(1))
        .map(|(candidate, label)| Row {
            resolved: Resolved {
                graphical: graphical_names.contains(candidate.entity.as_str()),
                entity: candidate.entity,
                subunit_key: group.subunit.key.clone(),
            },
            score: candidate.weight,
            label,
        })
        .collect())
}

fn source_label(inputs: &Inputs<'_, '_>, group: &Group<'_>, source: legacy::Source) -> Label {
    let handle = match source {
        legacy::Source::Subunit => {
            return inputs
                .labels
                .subunit(inputs.viewing_country_ref, &group.subunit.key);
        }
        legacy::Source::Variant(handle) | legacy::Source::Best(handle) => handle,
    };
    replay_frame::frame::kept::note_row(
        replay_data::rows::TableId::VariantDefinitions,
        u128::from(handle),
        u64::MAX,
    );
    let definition = inputs
        .definitions
        .get(handle)
        .ok_or_else(|| format!("variant {handle} lacks its definition"))?;
    let key = inputs.content.equipment_key(inputs.tokens, definition)?;
    let equipment = inputs
        .content
        .equipment_model(key)
        .map_err(|error| error.to_string())?;
    inputs.labels.variant(definition, key, &equipment)
}

fn append_suffix<'a>(
    inputs: &Inputs<'_, 'a>,
    group: &Group<'a>,
    country_ref: u32,
    priority: i32,
    bin: &mut Vec<Row>,
) -> Result<(), String> {
    let first = group.requirements.first().map(|(key, _)| key.as_str());
    if first.is_none() {
        return Ok(());
    }
    let owner = country(inputs, country_ref)?;
    let available = inputs.conventional.primary_variants(country_ref)?;
    let best = inputs.conventional.best_candidates(country_ref)?;
    let mut candidates: Vec<Candidate> = bin
        .iter()
        .map(|row| Candidate {
            entity: row.resolved.entity.clone(),
            weight: row.score,
        })
        .collect();
    let sources: BTreeMap<String, legacy::Source> = legacy::append_candidates_with_sources(
        &legacy::Context {
            country: owner,
            subunit: legacy::Subunit {
                key: &group.subunit.key,
                sprite: &group.subunit.sprite,
                categories: group.subunit.category,
                first_archetype: first,
            },
            selected: None,
            available: &available,
            best_candidates: &best,
        },
        &mut candidates,
        priority,
        |name| inputs.registered.contains(name),
    )?
    .into_iter()
    .collect();
    let old: BTreeMap<_, _> = std::mem::take(bin)
        .into_iter()
        .map(|row| (row.resolved.entity.clone(), row))
        .collect();
    *bin = candidates
        .into_iter()
        .map(|candidate| {
            old.get(&candidate.entity).cloned().unwrap_or_else(|| Row {
                label: sources.get(&candidate.entity).map_or_else(
                    || {
                        Err(format!(
                            "candidate {} has no recorded producer",
                            candidate.entity
                        ))
                    },
                    |source| source_label(inputs, group, *source),
                ),
                resolved: Resolved {
                    entity: candidate.entity,
                    subunit_key: group.subunit.key.clone(),
                    graphical: false,
                },
                score: candidate.weight,
            })
        })
        .collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(entity: &str, score: i64) -> Row {
        labelled(entity, score, entity)
    }

    fn labelled(entity: &str, score: i64, label: &str) -> Row {
        Row {
            resolved: Resolved {
                entity: entity.into(),
                subunit_key: "infantry".into(),
                graphical: false,
            },
            score,
            label: Ok(label.as_bytes().to_vec()),
        }
    }

    #[test]
    fn six_queue_order_precedes_scores_and_final_dedup_keeps_first_bucket() {
        let variants = [
            vec![row("", 0), row("exact", -100)],
            vec![],
            vec![row("shared", 20)],
        ];
        let suffix = [
            vec![row("suffix", 1_000_000), row("shared", 2_000_000)],
            vec![],
            vec![],
        ];
        assert_eq!(
            drain_rows(variants, suffix)
                .iter()
                .map(|row| row.resolved.entity.as_str())
                .collect::<Vec<_>>(),
            ["exact", "shared", "suffix"]
        );
    }

    #[test]
    fn reverse_input_restores_own_ties_but_later_variant_precedes_equal_score() {
        let mut bins: [Vec<Row>; 3] = std::array::from_fn(|_| Vec::new());
        let mut seen = BTreeSet::new();
        admit_variant_rows(
            &mut bins,
            &mut seen,
            1,
            1,
            true,
            false,
            2,
            2,
            b"",
            &[row("A", 0), row("B", 0)],
        );
        assert_eq!(
            bins[0]
                .iter()
                .map(|row| row.resolved.entity.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
        admit_variant_rows(
            &mut bins,
            &mut seen,
            1,
            1,
            true,
            false,
            2,
            2,
            b"",
            &[row("C", 0)],
        );
        assert_eq!(
            bins[0]
                .iter()
                .map(|row| row.resolved.entity.as_str())
                .collect::<Vec<_>>(),
            ["C", "A", "B"]
        );
        assert_eq!(bins[0][0].score, 200_000);

        let mut bins: [Vec<Row>; 3] = std::array::from_fn(|_| Vec::new());
        let mut seen = BTreeSet::new();
        admit_variant_rows(
            &mut bins,
            &mut seen,
            2,
            3,
            false,
            false,
            1,
            1,
            b"",
            &[row("first mismatch owns token", 0)],
        );
        assert!(bins[0].is_empty());
        assert_eq!(bins[2][0].resolved.entity, "first mismatch owns token");
    }

    #[test]
    fn the_second_option_row_decides_between_different_entities() {
        let empty = || std::array::from_fn(|_| Vec::new());
        let figure = |variants, suffix| {
            second_row(b"Use default model", variants, suffix)
                .map(|resolved| resolved.map(|resolved| resolved.entity))
        };
        assert_eq!(
            figure(
                [vec![row("A", 0)], vec![], vec![]],
                [vec![row("A", 1)], vec![], vec![]]
            ),
            Ok(Some("A".into()))
        );
        assert_eq!(
            figure([vec![row("A", 0), row("B", 0)], vec![], vec![]], empty()),
            Ok(Some("A".into()))
        );
        assert_eq!(
            figure(
                [vec![row("A", 0)], vec![], vec![]],
                [vec![labelled("B", 0, "Use default model")], vec![], vec![]]
            ),
            Ok(Some("B".into()))
        );
        let mut unknown = row("B", 0);
        unknown.label = Err("country 9 has no Countries row".into());
        let refused = figure([vec![row("A", 0), unknown], vec![], vec![]], empty()).unwrap_err();
        assert!(refused.starts_with(LABEL_ORDERING), "{refused}");
    }

    #[test]
    fn low_word_category_traversal_and_positive_requirements() {
        assert_eq!(
            low_bits((1_u64 << 63) | 0x8000_0003).collect::<Vec<_>>(),
            vec![1, 2, 0x8000_0000]
        );
        assert!(!positive_match(&[("rifles".into(), 0)], "rifles"));
        assert!(positive_match(&[("rifles".into(), 1)], "rifles"));
        assert!(category_reaches(2, 0, false));
        assert!(category_reaches(2, 3, true));
        assert!(!category_reaches(2, 1, true));
        assert!(!category_reaches(1_u64 << 32, 1, false));
    }

    #[test]
    fn country_override_requires_exact_category_and_positive_origin() {
        let mut subunit = model_selection::SubUnit::default();
        subunit.category = 4;
        let mut variant = VariantDefinitionRow {
            classifier_flags: 4,
            creator_link: 7,
            ..Default::default()
        };
        assert_eq!(model_country(&subunit, &variant, 3), 7);
        variant.origin_link = 8;
        assert_eq!(model_country(&subunit, &variant, 3), 8);
        variant.classifier_flags = 0x24;
        assert_eq!(model_country(&subunit, &variant, 3), 3);
    }

    #[test]
    fn graphical_country_uses_the_template_country_and_suffix_the_positive_foreign_one() {
        let mut template = DivisionTemplateIconRow {
            country: 7,
            ..Default::default()
        };
        assert_eq!(template_countries(&template).unwrap(), (7, 7));
        template.foreign_country = 8;
        assert_eq!(template_countries(&template).unwrap(), (7, 8));
        template.country = 0;
        assert!(template_countries(&template).is_err());
    }

    #[test]
    fn a_creator_must_resolve_and_a_nonpositive_one_has_no_operands() {
        let links = BTreeMap::from([(7, 1)]);
        let countries = BTreeMap::from([(
            1,
            ModelCountry {
                graphical_culture: Some("western_european_gfx".into()),
                ruling_ideology: Some(Some("fascism".into())),
                ..Default::default()
            },
        )]);
        let mut variant = VariantDefinitionRow::default();
        assert_eq!(
            creator_operands(&variant, &links, &countries),
            Ok((None, None))
        );
        variant.creator_link = 7;
        assert_eq!(
            creator_operands(&variant, &links, &countries),
            Ok((Some("fascism"), Some("western_european_gfx")))
        );
        variant.creator_link = 8;
        assert!(creator_operands(&variant, &links, &countries).is_err());
    }
}
