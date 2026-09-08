mod pools;
mod culture;
mod alternate;
mod label;
mod template_queue;
pub(super) mod frame;

use super::*;
use crate::authored_equipment::AuthoredEquipmentCatalog;
use snapshot::design_catalog::{DivisionComposition, NativeDesignerInputs, NativeDesignerVariantInputs};
use std::collections::BTreeMap;

pub(super) struct AllocatedModel<'a> {
    pub variant: &'a NativeDesignerVariantInputs,
    pub subunit_name: &'a str,
    pub subunit: &'a SubunitModel,
}

fn variant_id(variant: &NativeDesignerVariantInputs) -> Result<u32> {
    let (id, kind) = variant.design_id.split_once(':')
        .ok_or("XMAP13 A27 variant identity is malformed")?;
    let id = id.parse::<u32>()?;
    if kind != "70" || id == 0 || id > i32::MAX as u32 {
        return Err("XMAP13 A27 variant identity is not type 70".into());
    }
    Ok(id)
}

pub(super) fn allocated_model<'a>(
    input: &DivisionModelInputs,
    composition: &DivisionComposition,
    subunits: &'a BTreeMap<String, SubunitModel>,
    equipment: &AuthoredEquipmentCatalog,
    native: &'a NativeDesignerInputs,
) -> Result<Option<AllocatedModel<'a>>> {
    let (primary_name, primary) = primary_subunit(composition, subunits)?;
    let preference = input.template_preference.measured()
        .ok_or("XMAP13 selected-template preference is unavailable")?;
    let need = primary.need_pool(equipment)?;
    let primary_archetype = greatest_positive_need(&need).map(|row| row.archetype.as_str());
    let allocations = input.equipment.measured().ok_or("XMAP13 D26 allocation census is unavailable")?;
    let mut by_id = BTreeMap::new();
    for variant in &native.variants {
        if by_id.insert(variant_id(variant)?, variant).is_some() {
            return Err("XMAP13 A27 repeats a type-70 identity".into());
        }
    }
    let mut candidates = Vec::new();
    candidates.try_reserve(allocations.len())?;
    for allocation in allocations {
        let variant = *by_id.get(&allocation.equip_id)
            .ok_or_else(|| format!("XMAP13 D26 variant {} is absent from the same-hour A27 census", allocation.equip_id))?;
        let matches = match preference.category {
            Some(category) => equipment.designer_category_mask(&variant.equipment_definition_token)? == category,
            None => equipment.archetype_identity(&variant.equipment_definition_token)?
                .as_ref().map(|row| row.token.as_str()) == primary_archetype,
        };
        if matches { candidates.push((allocation.amount_raw, allocation.equip_id, variant)); }
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, id, variant) in candidates {
        if preference.category.is_none() {
            return Ok(Some(AllocatedModel { variant, subunit_name: primary_name, subunit: primary }));
        }
        let excluded = preference.excluded_variants.measured()
            .ok_or("XMAP13 template variant exclusions are unavailable")?;
        if excluded.iter().flatten().any(|excluded| *excluded == id) { continue; }
        let archetype = equipment.archetype_identity(&variant.equipment_definition_token)?;
        let mut selected = None;
        let mut maximum = -1;
        for (name, subunit, count) in counted_subunits(composition, subunits)?.0 {
            let needs = subunit.need_pool(equipment)?;
            if !needs.iter().any(|need| Some(need.archetype.as_str()) == archetype.as_ref().map(|row| row.token.as_str())
                && need.amount_raw > 0) { continue; }
            let priority = match subunit.visual_priority {
                Some(priority) => priority,
                None => equipment.effective_model_priority(&variant.equipment_definition_token)?
                    .wrapping_add(10).wrapping_add(count.wrapping_mul(100)).wrapping_mul(10),
            };
            if priority > maximum {
                maximum = priority;
                selected = Some((name, subunit));
            }
        }
        if let Some((subunit_name, subunit)) = selected {
            return Ok(Some(AllocatedModel { variant, subunit_name, subunit }));
        }
    }
    Ok(None)
}

mod category;
use category::variant_category;

fn creator_override_allowed(subunit_mask: u64, variant_mask: impl FnOnce() -> Result<u64>) -> Result<bool> {
    // 14191a770 requires both variant bit 0x20 clear and exact mask equality.
    // A subunit containing that bit disproves their conjunction without a module-slot lookup.
    if subunit_mask & 0x20 != 0 { return Ok(false); }
    let variant_mask = variant_mask()?;
    Ok(variant_mask & 0x20 == 0 && variant_mask == subunit_mask)
}

fn allocated_entity<T, R>(
    selected: &AllocatedModel<'_>, resolve: &mut R,
    ordinary: impl FnOnce(&mut R) -> Result<Option<T>>,
) -> Result<Option<T>>
where R: FnMut(&str) -> Result<Option<T>> {
    use snapshot::design_catalog::DesignTextObservation;
    match &selected.variant.model.explicit_entity {
        DesignTextObservation::Measured { value, .. } if !value.is_empty() => resolve(value),
        DesignTextObservation::Measured { .. } => ordinary(resolve),
        DesignTextObservation::Unavailable { reason } =>
            Err(format!("XMAP13 variant explicit entity: {reason:?}").into()),
    }
}

fn allocated_model_list<T>(
    selected: &AllocatedModel<'_>, context: &SuffixContext<'_>,
    graphic_pools: impl FnOnce() -> Result<Vec<crate::authored_graphic_pool::AuthoredGraphicPool<crate::authored_graphic_pool::ItemPoolLimit>>>,
    alternate_selection: impl FnOnce() -> Result<bool>,
    resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
) -> Result<Option<T>> {
    let pools = graphic_pools()?;
    let mut borrowed = Vec::new();
    borrowed.try_reserve(pools.len())?;
    borrowed.extend(pools.iter());
    let mut models = ModelList::default();
    for item in crate::authored_graphic_pool::model_items(&borrowed)? {
        models.graphic(item, resolve)?;
    }
    models.suffix_families(selected.subunit_name, selected.subunit, Some(selected.variant), 0, context, resolve)?;
    // 141918420 prepends the default row after ordering real rows; 141918f90 reads index one.
    if selected.subunit.type_mask()? & 0x1f0037fc00 != 0 && alternate_selection()? {
        return alternate::entity(selected, context, resolve);
    }
    Ok(models.rows.into_iter().next().map(|row| row.entity))
}

fn complete_model_list<T>(
    selected: &AllocatedModel<'_>, context: &SuffixContext<'_>,
    pools: &[crate::authored_graphic_pool::AuthoredGraphicPool<crate::authored_graphic_pool::ItemPoolLimit>],
    default_label: String, override_label: String,
    labeler: &dyn Fn(ModelLabelSource<'_>) -> Result<String>,
    resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
) -> Result<Vec<template_queue::ModelRow<T>>> {
    let mut borrowed = Vec::new();
    borrowed.try_reserve(pools.len())?;
    borrowed.extend(pools.iter());
    let mut models = ModelList { labeler: Some(labeler), selected_label_override: Some(override_label),
        ..ModelList::default() };
    if !pools.is_empty() {
        let archetype = context.equipment.archetype_identity(&selected.variant.equipment_definition_token)?
            .ok_or("XMAP13 graphic model label has no equipment archetype")?;
        models.set_label(ModelLabelSource::GraphicArchetype(&archetype.token))?;
    }
    for item in crate::authored_graphic_pool::model_items(&borrowed)? { models.graphic(item, resolve)?; }
    models.suffix_families(selected.subunit_name, selected.subunit, Some(selected.variant), 0, context, resolve)?;
    let mut rows = Vec::new();
    rows.try_reserve(models.rows.len().checked_add(1).ok_or("XMAP13 complete model row count overflow")?)?;
    rows.push(template_queue::ModelRow { label: default_label, token: String::new(), payload_raw: -100_000, entity: None });
    for row in models.rows {
        rows.push(template_queue::ModelRow { label: row.label, token: row.token,
            payload_raw: row.weight_raw, entity: Some(row.entity) });
    }
    label::decorate_model_labels(&mut rows, |row| &mut row.label)?;
    // 141919100 discards 141918420's separate optional entity result. The complete
    // vector above is its input; feature26 does not replace this vector.
    Ok(rows)
}

fn for_each_suffix_family(
    subunit_name: &str,
    subunit: &SubunitModel,
    selected: Option<&NativeDesignerVariantInputs>,
    equipment: &AuthoredEquipmentCatalog,
    priority: i32,
    mut visit: impl FnMut(&str, i32) -> Result<()>,
) -> Result<()> {
    if let Some(selected) = selected {
        if let Some(model) = equipment.definition(&selected.equipment_definition_token)?.scalar("model")? {
            if !model.is_empty() { visit(model, priority.wrapping_add(1000))?; }
        }
    }
    visit(&subunit.sprite, priority)?;
    if subunit_name != subunit.sprite { visit(subunit_name, priority)?; }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
struct WeightedModel<T> {
    label: String,
    token: String,
    weight_raw: i64,
    entity: T,
}

enum ModelLabelSource<'a> {
    GraphicArchetype(&'a str),
    Variant { row: &'a NativeDesignerVariantInputs, override_label: Option<&'a str> },
    GenericSubunit(&'a str),
}

struct ModelList<'a, T> {
    rows: Vec<WeightedModel<T>>,
    labeler: Option<&'a dyn Fn(ModelLabelSource<'_>) -> Result<String>>,
    current_label: String,
    selected_label_override: Option<String>,
}

impl<T> Default for ModelList<'_, T> {
    fn default() -> Self {
        Self { rows: Vec::new(), labeler: None, current_label: String::new(), selected_label_override: None }
    }
}

struct SuffixContext<'a> {
    native: &'a NativeDesignerInputs,
    world: &'a replay_core::WorldFrame,
    equipment: &'a AuthoredEquipmentCatalog,
    country: &'a str,
    prefixes: [&'a replay_core::country_name_semantics::NamingInput<String>; 3],
}

impl<T> ModelList<'_, T> {
    fn set_label(&mut self, source: ModelLabelSource<'_>) -> Result<()> {
        if let Some(labeler) = self.labeler { self.current_label = labeler(source)?; }
        Ok(())
    }

    fn set_variant_label(&mut self, row: &NativeDesignerVariantInputs, selected: bool) -> Result<()> {
        if let Some(labeler) = self.labeler {
            self.current_label = labeler(ModelLabelSource::Variant {
                row, override_label: if selected { self.selected_label_override.as_deref() } else { None },
            })?;
        }
        Ok(())
    }

    fn suffix_families(
        &mut self, subunit_name: &str, subunit: &SubunitModel,
        selected: Option<&NativeDesignerVariantInputs>, priority: i32,
        context: &SuffixContext<'_>, resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<()> {
        for_each_suffix_family(subunit_name, subunit, selected, context.equipment, priority,
            |base, priority| self.variant_suffix_family(base, priority, subunit_name, subunit, selected, context, resolve))
    }

    fn alternatives(
        &mut self, base: &str, priority: i32, context: &SuffixContext<'_>,
        resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<i32> {
        let mut successes = 0i32;
        for ordinal in 0..4 {
            if self.country_suffix(&format!("{base}_alt_{ordinal}{}", MODEL_ENTITY_SUFFIX),
                priority.wrapping_sub(1).wrapping_sub(successes), context.prefixes, resolve)? {
                successes += 1;
            }
        }
        Ok(successes)
    }

    fn variant_suffix_family(
        &mut self, base: &str, priority: i32, subunit_name: &str, subunit: &SubunitModel,
        selected: Option<&NativeDesignerVariantInputs>, context: &SuffixContext<'_>,
        resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<()> {
        let needs = subunit.need_pool(context.equipment)?;
        // 140b88e70 uses the first pool key, independently of the greatest-positive allocation need.
        let Some(first) = needs.first() else { return Ok(()) };
        let (owned, preferred) = owned_and_preferred(context.native, context.country, &first.archetype,
            selected, context.world, context.equipment)?;
        let level = |variant: &NativeDesignerVariantInputs| -> Result<i32> {
            let token = &variant.equipment_definition_token;
            if subunit.type_mask()? & 0x80004003c1 != 0 {
                Ok(context.equipment.effective_model_priority(token)?)
            } else {
                Ok(context.equipment.definition(token)?.signed_integer("visual_level")?.unwrap_or(0))
            }
        };
        let is_selected = |variant: &NativeDesignerVariantInputs| selected.is_some_and(|row| row.design_id == variant.design_id);
        let mut preferred_versioned = false;
        for variant in owned {
            self.set_variant_label(variant, is_selected(variant))?;
            let level = level(variant)?;
            let preferred_row = preferred.is_some_and(|row| row.design_id == variant.design_id);
            let weight = priority.wrapping_add(level)
                .wrapping_add(if is_selected(variant) { 30 } else { 0 })
                .wrapping_add(if preferred_row { 20 } else { 0 }).wrapping_add(5);
            let versioned = format!("{base}_{level}");
            let inserted = self.country_suffix(&format!("{versioned}{}", MODEL_ENTITY_SUFFIX), weight,
                context.prefixes, resolve)?;
            let alternatives = self.alternatives(&versioned, weight, context, resolve)?;
            preferred_versioned |= preferred_row && (inserted || alternatives > 0);
        }
        if !preferred_versioned {
            if let Some(preferred) = preferred {
                self.set_variant_label(preferred, is_selected(preferred))?;
                let weight = priority.wrapping_add(level(preferred)?)
                    .wrapping_add(if is_selected(preferred) { 30 } else { 0 }).wrapping_add(28);
                self.country_suffix(&format!("{base}{}", MODEL_ENTITY_SUFFIX), weight, context.prefixes, resolve)?;
                // This native arm prefixes the helper tag before visiting every country suffix again.
                self.alternatives(&format!("{}_{base}", context.country), weight, context, resolve)?;
            }
        }
        self.set_label(ModelLabelSource::GenericSubunit(subunit_name))?;
        self.country_suffix(&format!("{base}{}", MODEL_ENTITY_SUFFIX), priority, context.prefixes, resolve)?;
        self.alternatives(base, priority, context, resolve)?;
        Ok(())
    }

    fn graphic(
        &mut self,
        item: crate::authored_graphic_pool::GraphicModelItem<'_>,
        resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<()> {
        let Some(entity) = resolve(item.token)? else { return Ok(()); };
        if self.rows.iter().any(|row| row.token == item.token) { return Ok(()); }
        self.rows.try_reserve(1)?;
        // 14191a910 inserts a graphic row after existing equal weights.
        let index = self.rows.partition_point(|row| row.weight_raw >= item.weight_raw);
        self.rows.insert(index, WeightedModel {
            label: self.current_label.clone(), token: item.token.into(), weight_raw: item.weight_raw, entity,
        });
        Ok(())
    }

    fn suffix(
        &mut self, base: &str, priority: i32,
        resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<bool> {
        let mut sequence = 1i32;
        let mut inserted = false;
        loop {
            let token = if sequence == 1 { base.to_owned() } else { format!("{base}_{sequence}") };
            let Some(entity) = resolve(&token)? else { return Ok(inserted); };
            if !self.rows.iter().any(|row| row.token == token) {
                let weight_raw = i64::from(priority.wrapping_mul(10).wrapping_sub(sequence)) * 100_000;
                self.rows.try_reserve(1)?;
                // 140b884f0 inserts a suffix row before existing equal weights.
                let index = self.rows.partition_point(|row| row.weight_raw > weight_raw);
                self.rows.insert(index, WeightedModel { label: self.current_label.clone(), token, weight_raw, entity });
                inserted = true;
            }
            sequence = sequence.wrapping_add(1);
        }
    }

    fn country_suffix(
        &mut self, suffix: &str, priority: i32,
        prefixes: [&replay_core::country_name_semantics::NamingInput<String>; 3],
        resolve: &mut impl FnMut(&str) -> Result<Option<T>>,
    ) -> Result<bool> {
        use replay_core::country_name_semantics::NamingInput;
        let mut inserted = false;
        for ((name, value), bonus) in [
            ("cosmetic_name_base", prefixes[0]), ("tag", prefixes[1]), ("graphical_culture", prefixes[2]),
        ].into_iter().zip([4, 3, 2]) {
            let prefix = match value {
                NamingInput::Measured(value) => value.as_str(),
                NamingInput::Absent => "",
                NamingInput::Unavailable(reason) => return Err(format!("XMAP13 model suffix {name}: {reason:?}").into()),
            };
            if bonus != 3 && prefix.is_empty() { continue; }
            inserted |= self.suffix(&format!("{prefix}_{suffix}"), priority.wrapping_add(bonus), resolve)?;
        }
        inserted |= self.suffix(suffix, priority.wrapping_add(1), resolve)?;
        Ok(inserted)
    }
}

fn upgrade_version(record: &snapshot::design_catalog::DesignRecord) -> Result<i32> {
    use snapshot::design_catalog::FieldCompleteness;
    if record.upgrade_completeness != FieldCompleteness::Complete {
        return Err("XMAP13 variant upgrade census is unavailable".into());
    }
    record.upgrades.iter().try_fold(0i32, |sum, row| {
        let level = u8::try_from(row.level)
            .map_err(|_| "XMAP13 variant upgrade level is not its captured byte")?;
        Ok(sum.wrapping_add(i32::from(level)))
    })
}

#[derive(Clone, Copy)]
enum VariantSourceList { Owned, Combined, Developed }

fn country_variants<'a>(
    native: &'a NativeDesignerInputs,
    country: &str,
    list: VariantSourceList,
) -> Result<Vec<&'a NativeDesignerVariantInputs>> {
    let mut countries = native.countries.iter().filter(|row| row.tag == country);
    let owner = countries.next().ok_or_else(|| format!("XMAP13 A27 has no current country {country}"))?;
    if countries.next().is_some() { return Err(format!("XMAP13 A27 repeats country {country}").into()); }
    let mut output = Vec::new();
    let lists: &[&str] = match list {
        VariantSourceList::Owned => &["A"],
        VariantSourceList::Combined => &["A", "B"],
        VariantSourceList::Developed => &["developed"],
    };
    for source_list in lists {
        let mut ordered = Vec::new();
        for variant in &native.variants {
            let occurrences = if matches!(list, VariantSourceList::Developed) {
                &variant.model.developed
            } else {
                &variant.source_occurrences
            };
            for occurrence in occurrences {
                if occurrence.source_country_registry_index != owner.registry_index { continue; }
                if occurrence.source_country_tag != country {
                    return Err("XMAP13 A27 country occurrence identity disagrees".into());
                }
                if occurrence.source_list == *source_list {
                    ordered.try_reserve(1)?;
                    ordered.push((occurrence.source_list_ordinal, variant));
                }
            }
        }
        ordered.sort_by_key(|(ordinal, _)| *ordinal);
        output.try_reserve(ordered.len())?;
        for (expected, (ordinal, variant)) in ordered.into_iter().enumerate() {
            if ordinal != u64::try_from(expected)? {
                return Err(format!("XMAP13 A27 {country} {source_list} source order is incomplete or contended").into());
            }
            output.push(variant);
        }
    }
    Ok(output)
}

fn owned_and_preferred<'a>(
    native: &'a NativeDesignerInputs,
    country: &str,
    archetype: &str,
    selected: Option<&'a NativeDesignerVariantInputs>,
    world: &replay_core::WorldFrame,
    equipment: &AuthoredEquipmentCatalog,
) -> Result<(Vec<&'a NativeDesignerVariantInputs>, Option<&'a NativeDesignerVariantInputs>)> {
    let helper = world.country_identity_for_tag(country)?;
    let matches_archetype = |variant: &NativeDesignerVariantInputs| -> Result<bool> {
        Ok(equipment.archetype_identity(&variant.equipment_definition_token)?
            .as_ref().is_some_and(|definition| definition.token == archetype))
    };
    let mut owned = Vec::new();
    for variant in country_variants(native, country, VariantSourceList::Owned)? {
        if matches_archetype(variant)? { owned.try_reserve(1)?; owned.push(variant); }
    }
    sort_variants(&mut owned, helper, world, equipment, &native.runtime_dlc_mask)?;
    let mut preferred = None;
    for variant in country_variants(native, country, VariantSourceList::Developed)? {
        if !matches_archetype(variant)? { continue; }
        if match preferred {
            None => true,
            Some(previous) => variant_order(variant, previous, helper, world, equipment, &native.runtime_dlc_mask)?.is_lt(),
        } { preferred = Some(variant); }
    }
    if let Some(selected) = selected {
        if !owned.iter().any(|row| row.design_id == selected.design_id) {
            owned.try_reserve(1)?;
            owned.push(selected);
        }
        if preferred.is_none() { preferred = Some(selected); }
    }
    Ok((owned, preferred))
}

fn modular_category_mask(runtime: &snapshot::design_catalog::DesignU32Observation) -> Result<u64> {
    let snapshot::design_catalog::DesignU32Observation::Measured { raw_u32 } = runtime else {
        return Err("XMAP13 modular equipment classification requires A27 runtime_dlc_mask".into());
    };
    let mut mask = 0;
    if raw_u32 & (1 << 4) != 0 { mask |= 0x80004001c0; }
    if raw_u32 & (1 << 7) != 0 { mask |= 4; }
    if raw_u32 & (1 << 8) != 0 { mask |= 0x1d0026fc00; }
    Ok(mask)
}

fn variant_version(
    variant: &NativeDesignerVariantInputs,
    equipment: &AuthoredEquipmentCatalog,
    runtime: &snapshot::design_catalog::DesignU32Observation,
) -> Result<i32> {
    let token = &variant.equipment_definition_token;
    if !equipment.designer_module_slots(token)?.is_empty() {
        let modular = modular_category_mask(runtime)?;
        if equipment.designer_category_mask(token)? & modular != 0 { return Ok(0); }
    }
    upgrade_version(&variant.record)
}

fn variant_order(
    left: &NativeDesignerVariantInputs,
    right: &NativeDesignerVariantInputs,
    helper_country: u32,
    world: &replay_core::WorldFrame,
    equipment: &AuthoredEquipmentCatalog,
    runtime: &snapshot::design_catalog::DesignU32Observation,
) -> Result<std::cmp::Ordering> {
    let archetype_ordinal = |variant: &NativeDesignerVariantInputs| -> Result<i32> {
        let archetype = equipment.archetype_identity(&variant.equipment_definition_token)?
            .ok_or("XMAP13 null equipment archetype registration ordinal is unbound")?;
        Ok(equipment.definition_ordinal(&archetype.token)?)
    };
    let order = archetype_ordinal(left)?.cmp(&archetype_ordinal(right)?);
    if !order.is_eq() { return Ok(order); }
    let depth = |variant: &NativeDesignerVariantInputs| -> Result<i32> {
        Ok(equipment.parent_depth(&variant.equipment_definition_token)?)
    };
    let order = depth(right)?.cmp(&depth(left)?);
    if !order.is_eq() { return Ok(order); }
    let left_version = variant_version(left, equipment, runtime)?;
    let right_version = variant_version(right, equipment, runtime)?;
    let order = right_version.cmp(&left_version);
    if !order.is_eq() { return Ok(order); }
    let left_creator = world.country_identity_for_tag(&left.model.creator_tag)?;
    let right_creator = world.country_identity_for_tag(&right.model.creator_tag)?;
    // 140bc2f10 reads all three canonical cells before either equality short-circuit.
    let helper_link = world.country_handle_link_value(helper_country)?;
    let left_link = world.country_handle_link_value(left_creator)?;
    let right_link = world.country_handle_link_value(right_creator)?;
    let equivalent = |a: u32, av: u32, b: u32, bv: u32| a == b || (a != 0 && b != 0 && av == bv);
    let left_owned = equivalent(left_creator, left_link, helper_country, helper_link);
    let right_owned = equivalent(right_creator, right_link, helper_country, helper_link);
    let order = right_owned.cmp(&left_owned);
    if !order.is_eq() { return Ok(order); }
    if equivalent(left_creator, left_link, right_creator, right_link) {
        Ok(variant_id(right)?.cmp(&variant_id(left)?))
    } else {
        Ok((left_link as i32).cmp(&(right_link as i32)))
    }
}

fn sort_variants(
    variants: &mut [&NativeDesignerVariantInputs],
    helper_country: u32,
    world: &replay_core::WorldFrame,
    equipment: &AuthoredEquipmentCatalog,
    runtime: &snapshot::design_catalog::DesignU32Observation,
) -> Result<()> {
    for index in 1..variants.len() {
        let mut current = index;
        while current > 0 && variant_order(variants[current], variants[current - 1],
            helper_country, world, equipment, runtime)?.is_lt() {
            variants.swap(current, current - 1);
            current -= 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authored_graphic_pool::GraphicModelItem;
    use replay_core::country_name_semantics::NamingInput;

    pub(super) fn variant(id: u32, definition: &str, creator: &str, level: i64) -> NativeDesignerVariantInputs {
        use snapshot::design_catalog::*;
        let unavailable = || DesignFieldRefusal { kind: "unreadable".into(), field: "unconsumed comparator fixture input".into() };
        NativeDesignerVariantInputs {
            slot_identities: DesignSlotIdentityObservation::Unavailable { reason: unavailable() },
            append_variant_name: DesignByteObservation::Unavailable { reason: unavailable() },
            display_name: DesignTextObservation::Unavailable { reason: unavailable() },
            variant_name: DesignTextObservation::Unavailable { reason: unavailable() },
            design_id: format!("{id}:70"), catalog_scan_ordinal: 0,
            equipment_definition_token: definition.into(), equipment_token_raw: 0,
            source_occurrences: Vec::new(), item_icon_override: DesignItemIconObservation::Unavailable { reason: unavailable() },
            serialized_variant_flag_2b9d: DesignByteObservation::Unavailable { reason: unavailable() },
            cached_stats: Vec::new(), mission_stat_operands: Vec::new(),
            record: DesignRecord {
                design_id: format!("{id}:70"), source_record_id: "test-79".into(), kind: DesignKind::EquipmentVariant,
                owner: None, name: None, archetype: None, lifecycle_state: None,
                modules: Vec::new(), stats: Vec::new(),
                upgrades: vec![DesignUpgrade { upgrade_id: "handle:5".into(), level, source_record_id: "test-79".into() }],
                definition_completeness: FieldCompleteness::Complete, owner_completeness: FieldCompleteness::Unknown,
                lifecycle_completeness: FieldCompleteness::Unknown, module_completeness: FieldCompleteness::Unknown,
                stat_completeness: FieldCompleteness::Unknown, upgrade_completeness: FieldCompleteness::Complete,
            },
            model: VariantModelInputs { creator_tag: creator.into(), origin_override_tag: None,
                explicit_entity: DesignTextObservation::Unavailable { reason: unavailable() }, owned: Vec::new(), developed: Vec::new() },
        }
    }

    #[test]
    fn helper_country_override_requires_both_native_category_predicates() {
        assert!(!creator_override_allowed(0x20, || panic!("bit-0x20 conjunction is impossible")).unwrap());
        assert!(!creator_override_allowed(u64::MAX, || panic!("high bits do not remove bit 0x20")).unwrap());
        for (subunit, variant, expected) in [(0,0,true),(0,0x20,false),(4,4,true),
            (4,0x24,false),(4,0x100000004,false),(0x100000004,0x100000004,true)] {
            assert_eq!(creator_override_allowed(subunit, || Ok(variant)).unwrap(), expected);
        }
        assert!(creator_override_allowed(4, || Err("designated module slot identity unavailable".into())).is_err());
    }

    #[test]
    fn country_source_lists_preserve_occurrence_order_duplicates_and_independent_developed_membership() {
        use snapshot::design_catalog::{DesignSourceOccurrence, NativeDesignerCountryInputs, DesignU32Observation, DesignFieldRefusal};
        let occurrence = |list: &str, ordinal| DesignSourceOccurrence {
            source_country_tag: "GER".into(), source_country_registry_index: 9,
            source_list: list.into(), source_list_ordinal: ordinal,
        };
        let mut first = variant(11, "equipment", "GER", 0);
        first.source_occurrences = vec![occurrence("A", 1), occurrence("A", 2)];
        first.model.developed = vec![occurrence("developed", 1)];
        let mut second = variant(12, "equipment", "GER", 0);
        second.source_occurrences = vec![occurrence("B", 1), occurrence("A", 0)];
        let mut third = variant(13, "equipment", "GER", 0);
        third.source_occurrences = vec![occurrence("B", 0)];
        third.model.developed = vec![occurrence("developed", 0)];
        let mut native = NativeDesignerInputs {
            templates: Vec::new(),
            runtime_dlc_mask: DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
                kind: "unreadable".into(), field: "unconsumed occurrence fixture input".into(),
            } }, source_record_id: "native-order-79".into(), hour: 79,
            countries: vec![
                NativeDesignerCountryInputs { registry_index: 9, tag: "GER".into(), designer_pointer_order_ordinal: None },
                NativeDesignerCountryInputs { registry_index: 12, tag: "FRA".into(), designer_pointer_order_ordinal: None },
            ], variants: vec![third, first, second],
        };
        let ids = |native: &NativeDesignerInputs, list| country_variants(native, "GER", list).unwrap()
            .iter().map(|row| variant_id(row).unwrap()).collect::<Vec<_>>();
        assert_eq!(ids(&native, VariantSourceList::Owned), [12, 11, 11]);
        assert_eq!(ids(&native, VariantSourceList::Combined), [12, 11, 11, 13, 12]);
        assert_eq!(ids(&native, VariantSourceList::Developed), [13, 11]);
        assert!(country_variants(&native, "FRA", VariantSourceList::Combined).unwrap().is_empty());
        assert!(country_variants(&native, "ENG", VariantSourceList::Combined).unwrap_err().to_string().contains("no current country"));
        native.variants[1].source_occurrences[1].source_list_ordinal = 4;
        assert!(country_variants(&native, "GER", VariantSourceList::Owned).unwrap_err().to_string().contains("incomplete or contended"));
        assert_eq!(ids(&native, VariantSourceList::Developed), [13, 11]);
        native.variants[1].source_occurrences[1].source_list_ordinal = 1;
        assert!(country_variants(&native, "GER", VariantSourceList::Owned).is_err());
        native.variants[1].source_occurrences[1].source_list_ordinal = 2;
        native.variants[0].source_occurrences[0].source_country_tag = "FRA".into();
        assert!(country_variants(&native, "GER", VariantSourceList::Combined).unwrap_err().to_string().contains("identity disagrees"));
        native.variants[0].source_occurrences[0].source_country_tag = "GER".into();
        native.countries.push(native.countries[0].clone());
        assert!(country_variants(&native, "GER", VariantSourceList::Owned).unwrap_err().to_string().contains("repeats country"));
    }

    #[test]
    fn admitted_variant_order_joins_definition_depth_upgrades_and_signed_current_country_links() {
        use crate::test_support::{admitted_fixture_read_set, temp_test_dir, write_modvfs_fixture};
        use snapshot::design_catalog::{DesignU32Observation, DesignFieldRefusal};
        use serde_json::json;
        let root = temp_test_dir("xmap13-variant-order");
        write_modvfs_fixture(&root, "common/units/equipment/order.txt", br#"@model_base = "authored_model"
        equipments = {
            zeta = { is_archetype = yes type = infantry model = replaced model = @model_base visual_level = 3 priority = malformed }
            zeta_1 = { archetype = zeta parent = zeta visual_level = 5 }
            alpha = { is_archetype = yes type = infantry }
            modular = { is_archetype = yes type = armor module_slots = { main = {} } }
            malformed_model = { parent = zeta model = {} }
        }"#);
        let raw_resolver = crate::GameFileResolver::vanilla(&root).unwrap();
        let scopes = [portable_content_manifest::ContentNamespaceScope {
            directory: "common/units/equipment".into(), extension: Some("txt".into()), recursive: true,
        }];
        let (resolver, admission) = admitted_fixture_read_set(&raw_resolver, &Default::default(), &scopes, "xmap13-variant-order");
        let equipment = AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
        let subunits = subunits_from_source("sub_units = { infantry = { sprite = infantry_sprite } }").unwrap();
        let subunit = &subunits["infantry"];
        let mut families = Vec::new();
        for_each_suffix_family("infantry", subunit, Some(&variant(1,"zeta","GER",0)), &equipment, i32::MAX,
            |base, priority| { families.push((base.to_owned(), priority)); Ok(()) }).unwrap();
        assert_eq!(families, [("authored_model".into(), i32::MAX.wrapping_add(1000)),
            ("infantry_sprite".into(), i32::MAX), ("infantry".into(), i32::MAX)]);
        families.clear();
        for_each_suffix_family("infantry_sprite", subunit, None, &equipment, 4,
            |base, priority| { families.push((base.to_owned(), priority)); Ok(()) }).unwrap();
        assert_eq!(families, [("infantry_sprite".into(),4)]);
        families.clear();
        let error = for_each_suffix_family("infantry", subunit, Some(&variant(1,"malformed_model","GER",0)), &equipment, 0,
            |base, priority| { families.push((base.to_owned(),priority)); Ok(()) }).unwrap_err();
        assert!(error.to_string().contains("model"));
        assert!(families.is_empty(), "an unavailable earlier family cannot be replaced by a later sprite");
        let raw = json!({"tag_db_id_index":{"country_composition_version":2,
            "country_link_indices":{"status":"measured","indices":[99,9,9,2147483648u32,7]}},
            "countries":{"GER":{"_r2_tag_db_id":1},"ALT":{"_r2_tag_db_id":2},
                "ENG":{"_r2_tag_db_id":3},"FRA":{"_r2_tag_db_id":4}}});
        let make_world = |raw: serde_json::Value| replay_core::WorldFrame::from_raw_scheduled(replay_core::RawFrame {
            hour_index:79, date:None, save_version:None, schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        },None,None,None,replay_core::BuildSchedule::Serial);
        let world = make_world(raw.clone());
        assert_eq!(world.country_identity_for_tag("GER").unwrap(), 1);
        let unknown_runtime = DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "runtime_dlc_mask".into(),
        }};
        let order = |left: &NativeDesignerVariantInputs, right: &NativeDesignerVariantInputs| {
            variant_order(left, right, 1, &world, &equipment, &unknown_runtime).unwrap()
        };
        assert!(order(&variant(1,"zeta","GER",0), &variant(2,"alpha","GER",255)).is_lt());
        assert!(order(&variant(1,"zeta_1","GER",0), &variant(2,"zeta","GER",255)).is_lt());
        assert!(order(&variant(1,"zeta","FRA",2), &variant(2,"zeta","GER",1)).is_lt());
        assert!(order(&variant(1,"zeta","ALT",0), &variant(2,"zeta","ENG",0)).is_lt());
        assert!(order(&variant(9,"zeta","ALT",0), &variant(2,"zeta","GER",0)).is_lt());
        assert!(order(&variant(1,"zeta","ENG",0), &variant(2,"zeta","FRA",0)).is_lt());
        let variants = [variant(2,"zeta","FRA",0),variant(1,"zeta","ENG",0),variant(9,"zeta","ALT",0)];
        let mut sorted: Vec<_> = variants.iter().collect();
        sort_variants(&mut sorted,1,&world,&equipment,&unknown_runtime).unwrap();
        assert_eq!(sorted.iter().map(|row| row.design_id.as_str()).collect::<Vec<_>>(), ["9:70","1:70","2:70"]);
        let low = variant(2,"modular","GER",0);
        let high = variant(1,"modular","GER",9);
        assert!(variant_order(&low,&high,1,&world,&equipment,&DesignU32Observation::Measured { raw_u32:1<<7 }).unwrap().is_lt());
        assert!(variant_order(&low,&high,1,&world,&equipment,&DesignU32Observation::Measured { raw_u32:0 }).unwrap().is_gt());
        assert!(variant_order(&low,&high,1,&world,&equipment,&unknown_runtime).is_err());
        assert_eq!(equipment.effective_model_priority("zeta_1").unwrap(),2);
        assert!(equipment.hierarchy_profile("zeta_1").is_ok());
        assert!(equipment.hierarchy_profile("zeta").is_err(), "unread generic priority is malformed");
        let suffix_subunits = subunits_from_source("sub_units = { infantry = { sprite = base type = infantry need = { zeta = 0 alpha = 10 } } empty = { sprite = base type = unproved } }").unwrap();
        let suffix_subunit = &suffix_subunits["infantry"];
        let needs = suffix_subunit.need_pool(&equipment).unwrap();
        assert_eq!(greatest_positive_need(&needs).unwrap().archetype,"alpha");
        let occurrence = |list: &str| snapshot::design_catalog::DesignSourceOccurrence {
            source_country_tag:"GER".into(), source_country_registry_index:9,
            source_list:list.into(), source_list_ordinal:0,
        };
        let mut selected = variant(11,"zeta","GER",0);
        selected.model.developed.push(occurrence("developed"));
        let mut owned = variant(10,"zeta_1","GER",0);
        owned.source_occurrences.push(occurrence("A"));
        let native = NativeDesignerInputs {
            templates: Vec::new(),
            runtime_dlc_mask:unknown_runtime.clone(), source_record_id:"test-79".into(), hour:79,
            countries:vec![snapshot::design_catalog::NativeDesignerCountryInputs {
                registry_index:9, tag:"GER".into(), designer_pointer_order_ordinal:None,
            }], variants:vec![selected,owned],
        };
        let prefixes = [NamingInput::Absent,NamingInput::Measured("GER".into()),NamingInput::Absent];
        let context = SuffixContext { native:&native,world:&world,equipment:&equipment,country:"GER",
            prefixes:[&prefixes[0],&prefixes[1],&prefixes[2]] };
        let selected = Some(&native.variants[0]);
        let mut calls = Vec::new();
        let mut models = ModelList::default();
        models.suffix_families("infantry",suffix_subunit,selected,0,&context,&mut |token| {
            calls.push(token.to_owned());
            Ok(["GER_base_3_entity","GER_base_3_alt_1_entity","GER_base_3_alt_2_entity",
                "GER_base_5_entity","GER_base_entity","GER_base_alt_2_entity"]
                .contains(&token).then(|| token.to_owned()))
        }).unwrap();
        assert_eq!(models.rows.iter().map(|row|(row.token.as_str(),row.weight_raw)).collect::<Vec<_>>(), [
            ("GER_base_3_entity",60_900_000),("GER_base_3_alt_1_entity",59_900_000),
            ("GER_base_3_alt_2_entity",58_900_000),("GER_base_5_entity",12_900_000),
            ("GER_base_entity",2_900_000),("GER_base_alt_2_entity",1_900_000),
        ]);
        assert!(calls.iter().any(|token|token=="GER_authored_model_5_entity"));
        assert!(calls.iter().any(|token|token=="GER_base_3_alt_3_entity"));
        assert!(!calls.iter().any(|token|token=="GER_base_3_alt_4_entity"));
        assert!(!calls.iter().any(|token|token.starts_with("GER_GER_base_alt_")), "a newly inserted preferred version suppresses the special boost");
        let mut fallback = ModelList::default();
        fallback.variant_suffix_family("base",0,"infantry",suffix_subunit,selected,&context,&mut |token|
            Ok(["GER_base_entity","GER_GER_base_alt_1_entity"].contains(&token).then(||token.to_owned()))).unwrap();
        assert_eq!(fallback.rows.iter().map(|row|(row.token.as_str(),row.weight_raw)).collect::<Vec<_>>(),
            [("GER_base_entity",63_900_000),("GER_GER_base_alt_1_entity",62_900_000)]);
        let mut repeated = ModelList::default();
        repeated.graphic(GraphicModelItem {token:"GER_base_3_entity",weight_raw:1},&mut |token|Ok(Some(token.to_owned()))).unwrap();
        repeated.variant_suffix_family("base",0,"infantry",suffix_subunit,selected,&context,&mut |token|
            Ok(["GER_base_3_entity","GER_base_entity"].contains(&token).then(||token.to_owned()))).unwrap();
        assert_eq!(repeated.rows[0].token,"GER_base_entity");
        assert_eq!(repeated.rows[0].weight_raw,63_900_000,"an existing preferred entity is not a new suffix insertion");
        let mut empty:ModelList<()> = ModelList::default();
        empty.suffix_families("empty",&suffix_subunits["empty"],None,0,&context,&mut |_|panic!("empty need pool reads no entity")).unwrap();
        assert!(empty.rows.is_empty());
        let mut selected = native.variants[0].clone();
        selected.model.explicit_entity = snapshot::design_catalog::DesignTextObservation::Measured {
            value:String::new(), utf8_hex:String::new(),
        };
        let allocation = AllocatedModel { variant:&selected, subunit_name:"infantry", subunit:suffix_subunit };
        let pool = || -> Result<_> {
            let root = portable_content_manifest::paradox_text::parse_str_strict(
                "pool = { models_weight = 2000 models = { graphic_lower graphic_top } }").unwrap();
            Ok(vec![crate::authored_graphic_pool::AuthoredGraphicPool::parse(root.first_value_ci("pool").unwrap(),
                crate::authored_graphic_pool::ItemPoolLimit::parse).unwrap()])
        };
        let chosen = allocated_entity(&allocation,
            &mut |token| Ok(["graphic_lower","graphic_top","GER_base_3_entity"].contains(&token).then(||token.to_owned())),
            |resolve| allocated_model_list(&allocation,&context,pool,
                || panic!("infantry does not reach the alternate-selection feature query"),resolve)).unwrap();
        assert_eq!(chosen.as_deref(),Some("graphic_top"));
        let mut explicit = selected.clone();
        explicit.model.explicit_entity = snapshot::design_catalog::DesignTextObservation::Measured {
            value:"explicit".into(), utf8_hex:"6578706c69636974".into(),
        };
        let allocation = AllocatedModel { variant:&explicit, subunit_name:"infantry", subunit:suffix_subunit };
        assert_eq!(allocated_entity(&allocation,&mut |token|Ok(Some(token.to_owned())),
            |_|panic!("explicit entity reads no ordinary context or database")).unwrap().as_deref(),Some("explicit"));
        assert!(allocated_entity(&allocation,&mut |_|Ok::<Option<String>,_>(None),
            |_|panic!("missing explicit entity enters outer fallback, not ordinary list")).unwrap().is_none());
        let mut unavailable=explicit.clone();
        unavailable.model.explicit_entity=snapshot::design_catalog::DesignTextObservation::Unavailable {
            reason:snapshot::design_catalog::DesignFieldRefusal { kind:"changed".into(),field:"explicit_entity".into() },
        };
        let allocation=AllocatedModel { variant:&unavailable,subunit_name:"infantry",subunit:suffix_subunit };
        let result=allocated_entity(&allocation,&mut |_|Ok::<Option<String>,_>(None),
            |_|panic!("unavailable explicit entity does not imply empty"));
        assert!(result.unwrap_err().to_string().contains("changed"));

        let mut changed = raw;
        changed["tag_db_id_index"]["country_link_indices"] = json!({"status":"unavailable","reason":"changed"});
        assert!(variant_order(&variants[0],&variants[1],1,&make_world(changed),&equipment,&unknown_runtime)
            .unwrap_err().to_string().contains("changed"));
        drop((sorted, equipment, resolver, admission, raw_resolver));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn graphic_and_suffix_equal_weights_keep_their_distinct_native_insertion_rules() {
        let mut models = ModelList::default();
        let mut resolve = |token: &str| Ok(Some(token.to_owned()));
        for token in ["graphic_a", "graphic_b"] {
            models.graphic(GraphicModelItem { token, weight_raw: 900_000 }, &mut resolve).unwrap();
        }
        let mut resolve_suffix = |token: &str| Ok((token == "suffix" || token == "other").then(|| token.to_owned()));
        assert!(models.suffix("suffix", 1, &mut resolve_suffix).unwrap());
        assert!(models.suffix("other", 1, &mut resolve_suffix).unwrap());
        assert_eq!(models.rows.iter().map(|row| row.token.as_str()).collect::<Vec<_>>(),
            ["other", "suffix", "graphic_a", "graphic_b"]);
        assert!(!models.suffix("suffix", 100, &mut resolve_suffix).unwrap());
        assert_eq!(models.rows[1].weight_raw, 900_000, "an existing entity is not reprioritized");
    }

    #[test]
    fn suffix_series_stops_at_first_missing_and_country_pools_all_participate() {
        let prefixes = [NamingInput::Measured("COS".into()), NamingInput::Measured("GER".into()),
            NamingInput::Measured("culture".into())];
        let available = ["COS_tank_entity", "COS_tank_entity_2", "COS_tank_entity_4",
            "GER_tank_entity", "culture_tank_entity", "tank_entity"];
        let mut calls = Vec::new();
        let mut models = ModelList::default();
        assert!(models.country_suffix("tank_entity", 0, [&prefixes[0], &prefixes[1], &prefixes[2]],
            &mut |token: &str| {
                calls.push(token.to_owned());
                Ok(available.contains(&token).then(|| token.to_owned()))
            }).unwrap());
        assert_eq!(models.rows.iter().map(|row| row.token.as_str()).collect::<Vec<_>>(),
            ["COS_tank_entity", "COS_tank_entity_2", "GER_tank_entity", "culture_tank_entity", "tank_entity"]);
        assert!(!calls.iter().any(|token| token == "COS_tank_entity_4"));
        assert_eq!(models.rows[0].weight_raw, 3_900_000);
        assert_eq!(models.rows[1].weight_raw, 3_800_000);
        let mut models = ModelList::default();
        let prefix = NamingInput::Measured(String::new());
        models.country_suffix("tank_entity", i32::MAX, [&prefix, &prefix, &prefix],
            &mut |token: &str| Ok((token == "tank_entity").then(|| token.to_owned()))).unwrap();
        assert_eq!(models.rows[0].weight_raw,
            i64::from(i32::MAX.wrapping_add(1).wrapping_mul(10).wrapping_sub(1)) * 100_000);
    }
}
