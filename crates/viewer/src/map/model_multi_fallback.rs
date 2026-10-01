//! No-selected-pair land model selection from the six native template queues.
//!
//! `141919100` reads the viewing country's two production rosters and the
//! template's stored grouped entries (composition `+0xA8`); each candidate
//! country's stored best-variant list comes from the recorded third roster.
//! The final native label sort can move an arbitrary custom variant label
//! beside `USE_DEFAULT_MODEL`; without that raw name operand, only an
//! invariant entity is displayed, and a stack whose rows disagree is refused
//! with [`LABEL_ORDERING`], which the model report counts.
//!
//! The viewing country is `CGameState+0x520` when it holds a positive country
//! reference, else `+0x524`; a replay settles both in `Vision`, whose country
//! is the unique `Countries` row for that canonical slot.

use std::collections::{BTreeMap, BTreeSet};

use replay::{
    frame::DecodedFrame,
    rows::{DivisionTemplateIconRow, VariantDefinitionRow, tables},
};

use super::{
    equipment_graphics::{self, Candidate, bridge, context, legacy},
    model_countries::ModelCountry,
    model_selection::{self, Group, Template},
    perspective::Vision,
};

/// Why a stack whose drained rows name more than one entity has no figure.
pub const LABEL_ORDERING: &str =
    "equipment model label ordering needs unavailable raw variant display names";

/// One resolvable entity and the producer used for the model report.
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
    pub graphics: &'a equipment_graphics::Content,
    pub geography: &'a context::Prepared,
    pub conventional: &'a bridge::Prepared<'b>,
    pub countries: &'a BTreeMap<u32, ModelCountry>,
    pub links: &'a BTreeMap<u32, u32>,
    pub definitions: &'a BTreeMap<u64, &'b VariantDefinitionRow>,
    /// Spells the definitions' equipment type tokens.
    pub tokens: &'a crate::table_access::TokenNames<'b>,
    pub viewing_country_ref: u32,
    pub limits: &'a equipment_graphics::Limits<'a>,
    pub registered: &'a std::collections::HashSet<String>,
}

/// A zero category traversal or no positive matching need cannot produce a
/// nondefault queue row, so it needs no production or graphics operands.
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
}

/// The native default row is always present. A zero production match leaves
/// only that row, and the caller must take the ordinary numbered fallback.
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
    // 140BC2F10 reads only reference identity. Defer naming/culture until a
    // real candidate reaches 141918420; an empty queue needs neither.
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
    // 140B88010 points to composition +0x1D4, the template's country.
    // 140B90DF0 supplies +0x1DC, the preferred suffix country, when positive.
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

#[allow(clippy::too_many_arguments)]
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
            let definition = inputs.definitions.get(&variant.handle).ok_or_else(|| {
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
    // 141919100 drains q0..q5, then 14191AB30 sorts by first-seen localized
    // label. The label of a custom variant may equal USE_DEFAULT_MODEL, and
    // the rank byte can wrap. Without names, only one distinct entity across
    // all drained rows proves physical output row one.
    invariant_entity(variant_bins, suffix_bins)
}

#[allow(clippy::too_many_arguments)]
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
            // The first admission owns a token across q0..q2, even if a
            // later category bit would have placed it in a stronger bin.
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
                },
            );
        }
    }
}

fn invariant_entity(
    variants: [Vec<Row>; 3],
    suffix: [Vec<Row>; 3],
) -> Result<Option<Resolved>, String> {
    let mut unique = BTreeMap::new();
    for row in drain_rows(variants, suffix) {
        unique
            .entry(row.resolved.entity.clone())
            .or_insert(row.resolved);
    }
    match unique.len() {
        0 => Ok(None),
        1 => Ok(unique.into_values().next()),
        _ => Err(LABEL_ORDERING.into()),
    }
}

fn drain_rows(variants: [Vec<Row>; 3], suffix: [Vec<Row>; 3]) -> Vec<Row> {
    // 141919100 pre-seeds the outer output with an empty entity token.
    // Each 141918420 input has its own empty sentinel; the final drain drops it.
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

/// `14191A090`: the variant creator's ruling ideology group and culture, or
/// neither for a nonpositive creator. A positive creator must resolve.
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

/// The viewing country's raw reference, for the roster the fallback reads.
pub fn view_country_ref(frame: &DecodedFrame, vision: &Vision) -> Result<u32, String> {
    let slot = vision.ours();
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
    // Native positive-reference checks are signed.
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
    // `CEquipmentType+0x4E8`, the type itself unless another is named.
    let family = Some((
        equipment.family,
        inputs
            .content
            .equipment_model(equipment.family)
            .map_err(|error| error.to_string())?
            .archetype,
    ));
    let (ideology, culture) = creator_operands(definition, inputs.links, inputs.countries)?;
    let geography = inputs.geography.context(country_ref)?;
    let graphical = inputs.graphics.lookup(
        &equipment_graphics::Context {
            equipment: equipment_key,
            archetype: equipment.archetype,
            family,
            subunit: Some(&group.subunit.key),
            ideology,
            culture,
            country: geography.map(|country| country.country_tag.as_str()),
            faction_tags: geography.map_or(&[], |country| country.faction_tags.as_slice()),
            capital_continent: geography.and_then(|country| country.capital_continent.as_deref()),
        },
        inputs.limits,
        |name| inputs.registered.contains(name),
    )?;
    let graphical_names: BTreeSet<_> = graphical.iter().map(|row| row.entity.clone()).collect();
    let first = group.requirements.first().map(|(key, _)| key.as_str());
    let available = inputs.conventional.primary_variants(country_ref)?;
    let best = inputs.conventional.best_candidates(country_ref)?;
    let mut candidates = graphical;
    let owner = country(inputs, country_ref)?;
    legacy::append_candidates(
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
        |name| inputs.registered.contains(name),
    )?;
    Ok(candidates
        .into_iter()
        .map(|candidate| Row {
            resolved: Resolved {
                graphical: graphical_names.contains(candidate.entity.as_str()),
                entity: candidate.entity,
                subunit_key: group.subunit.key.clone(),
            },
            score: candidate.weight,
        })
        .collect())
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
    legacy::append_candidates_with_priority(
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
    )?;
    // The helper's native ranked list is persistent. Existing rows retain
    // their first producer while new rows carry this grouped subunit.
    let old: BTreeMap<_, _> = std::mem::take(bin)
        .into_iter()
        .map(|row| (row.resolved.entity.clone(), row))
        .collect();
    *bin = candidates
        .into_iter()
        .map(|candidate| {
            old.get(&candidate.entity).cloned().unwrap_or_else(|| Row {
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
        Row {
            resolved: Resolved {
                entity: entity.into(),
                subunit_key: "infantry".into(),
                graphical: false,
            },
            score,
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
    fn a_label_collision_is_explicit() {
        assert_eq!(
            invariant_entity(
                [vec![row("A", 0)], vec![], vec![]],
                [vec![row("A", 1)], vec![], vec![]]
            )
            .unwrap()
            .unwrap()
            .entity,
            "A"
        );
        assert_eq!(
            invariant_entity(
                [vec![row("A", 0), row("B", 0)], vec![], vec![]],
                std::array::from_fn(|_| Vec::new())
            )
            .unwrap_err(),
            LABEL_ORDERING
        );
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
