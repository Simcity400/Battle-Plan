use super::*;

pub(crate) fn read<'a>(
    frame: &'a DecodedFrame,
    content: &'a Content,
) -> Result<Prepared<'a>, String> {
    let variants = complete::<tables::VariantDefinitions>(frame)?;
    let upgrades = complete::<tables::VariantUpgrades>(frame)?;
    let available = complete::<tables::AvailableEquipmentVariants>(frame)?;
    let links = complete::<tables::CountryLinks>(frame)?;
    let features = complete::<tables::SimulationFeatures>(frame)?;
    let [features] = features else {
        return Err("SimulationFeatures must have one complete row".into());
    };
    let module_categories = content_reader_key_names::equipment::category_families()?
        .modular_categories(features.dlc_mask);
    let tokens = TokenNames::new(frame).map_err(|error| error.to_string())?;
    from_rows(
        content,
        &tokens,
        variants,
        upgrades,
        available,
        links,
        module_categories,
    )
}

fn from_rows<'a>(
    content: &'a Content,
    tokens: &TokenNames<'_>,
    definitions: &'a [VariantDefinitionRow],
    upgrades: &[VariantUpgradeRow],
    available: &[AvailableEquipmentVariantRow],
    links: &[CountryLinkRow],
    module_categories: u64,
) -> Result<Prepared<'a>, String> {
    let mut by_handle = BTreeMap::new();
    for (index, row) in definitions.iter().enumerate() {
        if by_handle.insert(row.id, index).is_some() {
            return Err(format!("duplicate equipment variant handle {}", row.id));
        }
    }
    let links: BTreeMap<_, _> = links
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    for row in definitions {
        if (row.creator_link as i32) > 0 && !links.contains_key(&row.creator_link) {
            return Err(format!(
                "variant {} creator {} has no CountryLinks row",
                row.id, row.creator_link
            ));
        }
    }
    let upgrade_sums = sum_upgrades(upgrades, &by_handle)?;
    let mut variants = Vec::with_capacity(definitions.len());
    let mut states = Vec::with_capacity(definitions.len());
    let mut invalid_by_archetype = BTreeMap::new();
    let mut invalid_unknown = None;
    for row in definitions {
        match variant(
            content,
            tokens,
            row,
            &links,
            &upgrade_sums,
            module_categories,
        ) {
            Ok(variant) => {
                states.push(VariantState::Ready(variants.len()));
                variants.push(variant);
            }
            Err(error) => {
                let archetype = content
                    .equipment_key(tokens, row)
                    .ok()
                    .and_then(|key| content.archetype(key).ok());
                let index = states.len();
                states.push(match archetype {
                    Some(archetype) => {
                        invalid_by_archetype
                            .entry(archetype.to_owned())
                            .or_insert(index);
                        VariantState::InvalidKnown { error }
                    }
                    None => {
                        invalid_unknown.get_or_insert(index);
                        VariantState::InvalidUnknown { error }
                    }
                });
            }
        }
    }
    let available = join_available(available, &by_handle, &links)?;
    Ok(Prepared {
        variants: Variants::Borrowed(variants),
        data: Arc::new(Census {
            states,
            invalid_by_archetype,
            invalid_unknown,
            by_handle,
            available,
            links,
        }),
    })
}

fn variant<'a>(
    content: &'a Content,
    tokens: &TokenNames<'_>,
    row: &'a VariantDefinitionRow,
    links: &BTreeMap<u32, u32>,
    upgrade_sums: &BTreeMap<u64, i32>,
    module_categories: u64,
) -> Result<legacy::Variant<'a>, String> {
    let key = content.equipment_key(tokens, row)?;
    let model = content
        .equipment_model(key)
        .map_err(|error| format!("variant {}: {error}", row.id))?;
    let creator_reference = row.creator_link as i32;
    let creator_slot = if creator_reference > 0 {
        *links.get(&row.creator_link).ok_or_else(|| {
            format!(
                "variant {} creator {} has no CountryLinks row",
                row.id, row.creator_link
            )
        })? as i32
    } else {
        creator_reference
    };
    let has_enabled_module_slots = content
        .has_enabled_module_slots(key, module_categories)
        .map_err(|error| format!("variant {}: {error}", row.id))?;
    Ok(legacy::Variant {
        handle: row.id,
        id: (row.id >> 32) as u32,
        creator_reference,
        creator_slot,
        upgrade_sum: upgrade_sums.get(&row.id).copied().unwrap_or_default(),
        equipment: legacy::Equipment {
            archetype: model.archetype,
            archetype_ordinal: model.archetype_ordinal,
            sprite: model.model,
            visual_level: model.visual_level,
            model_priority: model.model_priority,
            parent_depth: model.depth,
            has_enabled_module_slots,
        },
    })
}
