//! Selected-design joins. Missing context is a named refusal, never an assumed
//! zero, and unit-specific context does not modify the global design sheet.
use super::*;
use installed_content_core::equipment::{UpgradeCurrency, UpgradeXpRules};

pub fn definition(frame: &DecodedFrame, id: u64) -> Result<&rows::VariantDefinitionRow, String> {
    let definitions = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|e| e.to_string())?;
    definitions
        .binary_search_by_key(&id, |row| row.id)
        .map(|i| &definitions[i])
        .map_err(|_| format!("design {id} is not recorded"))
}
pub fn spelling(frame: &DecodedFrame, token: u32) -> Result<&[u8], String> {
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    tokens
        .binary_search_by_key(&token, |row| row.token)
        .map(|i| tokens[i].literal.as_slice())
        .map_err(|_| format!("design token {token} has no recorded spelling"))
}

pub fn author_live(frame: &DecodedFrame, reference: u64) -> Result<bool, String> {
    if reference == 0 {
        return Ok(false);
    }
    frame
        .complete::<tables::DesignOrganisationLiveness>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.organisation == reference)
        .map(|row| row.live)
        .ok_or_else(|| {
            format!("design organisation {reference} liveness is not recorded (count: 1)")
        })
}

pub fn resource_name(frame: &DecodedFrame, token: u32) -> Result<&[u8], String> {
    let row = frame
        .complete::<tables::ResourceDefinitions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.token == token)
        .ok_or_else(|| format!("resource token {token} definition is not recorded"))?;
    spelling(frame, row.token)
}

pub fn design_templates(
    frame: &DecodedFrame,
    country: u32,
) -> Result<&rows::DesignCountryTemplatesRow, String> {
    let row = frame
        .complete::<tables::DesignCountryTemplates>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.country == country)
        .ok_or_else(|| format!("country {country} design discounts are not recorded"))?;
    if row.failure != 0 {
        return Err(format!(
            "country {country} design discount acquisition refused ({})",
            row.failure
        ));
    }
    Ok(row)
}

/// The source equipment type and its archetype select one owner record. The
/// getter retains exhausted entries and first-match order for equal factors.
pub fn template_discount(frame: &DecodedFrame, country: u32, source: u64) -> Result<i64, String> {
    let source = definition(frame, source)?;
    let mut factor = None;
    for (_, value, equipment) in design_templates(frame, country)?.entries() {
        if (equipment.contains(&source.equipment_token)
            || equipment.contains(&source.archetype_token))
            && factor.is_none_or(|previous| value > previous)
        {
            factor = Some(value);
        }
    }
    Ok(factor.unwrap_or(0).clamp(0, 100_000))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionSheet {
    pub values: [i64; STAT_COUNT],
    pub contributors: Vec<Source>,
    pub reconciliation: Result<(), String>,
}

/// One mission bit over the selected design. Native held overrides supply the
/// result; authored terms explain it only when they reconcile with that cache.
pub fn mission_sheet(
    frame: &DecodedFrame,
    variant: u64,
    mission: u32,
    content: &Database,
) -> Result<MissionSheet, String> {
    if !mission.is_power_of_two() {
        return Err("mission statistics require one mission bit".into());
    }
    let definition = definition(frame, variant)?;
    let state = definition
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let sheet = sheet(frame, variant, content)?;
    let mut components = sheet.components.clone();
    let mut contributors = sheet.sources;
    let mut affected = std::collections::BTreeSet::new();
    for row in frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == variant)
    {
        if state
            .module_mission_exclusions
            .iter()
            .any(|entry| entry.module_token == row.definition_token && entry.mask & mission != 0)
        {
            continue;
        }
        let name = spelling(frame, row.definition_token)?;
        let module = content.module_design(name).ok_or_else(|| {
            format!(
                "module {} mission terms are absent",
                String::from_utf8_lossy(name)
            )
        })?;
        for (limits, terms) in &module.mission_terms {
            let mask = limits.iter().fold(0, |mask, name| {
                mask | installed_content_core::equipment::mission_mask(name)
            });
            if mask & mission != 0 {
                affected.extend(
                    terms
                        .add
                        .iter()
                        .chain(&terms.average)
                        .chain(&terms.multiply)
                        .map(|term| term.index),
                );
                components.apply(terms, 100_000).map_err(str::to_owned)?;
                contributors.push(Source {
                    name: format!("{} mission", String::from_utf8_lossy(name)),
                    scale: 100_000,
                    terms: terms.clone(),
                });
            }
        }
    }
    let computed = components.finish();
    let mut values = sheet.values;
    let mut mismatch = sheet.breakdown_refusal;
    if let Some(held) = state
        .mission_stats
        .iter()
        .find(|entry| entry.mask == mission)
    {
        for entry in &held.stats {
            let index = entry.index as usize;
            let target = values
                .get_mut(index)
                .ok_or("mission stat index is outside equipment metadata")?;
            *target = entry.value;
            if computed[index] != entry.value {
                mismatch =
                    Some("mission contributors differ from the design's cached rebuild".into());
            }
        }
    }
    if affected
        .into_iter()
        .any(|index| computed[index] != values[index])
    {
        mismatch = Some("mission contributors differ from the design's cached rebuild".into());
    }
    Ok(MissionSheet {
        values,
        contributors,
        reconciliation: mismatch.map_or(Ok(()), Err),
    })
}

/// Native detail visibility for one requested observer and saved design. The
/// caller supplies an observed fog switch or a named refusal, never a default.
pub fn recorded_fog(frame: &DecodedFrame) -> Result<bool, &'static str> {
    let rows = frame
        .complete::<tables::DesignObserverContext>()
        .map_err(|_| "game fog writer certificate or singleton read unavailable")?;
    let [row] = rows else {
        return Err("recorded game fog singleton is unavailable");
    };
    Ok(row.fog != 0)
}

pub fn reveal(
    frame: &DecodedFrame,
    variant: u64,
    observer: u32,
    target_country: u32,
    fog: Result<bool, &'static str>,
    bypass: bool,
    defines: &installed_content_core::defines::Defines,
) -> Result<bool, String> {
    use replay_derive::visibility::IntelBranch;
    let design = definition(frame, variant)?;
    let (branch, lane, threshold) =
        if design.classifier_flags & engine_native::cequipment_type::SHIP_MASK != 0 {
            (
                IntelBranch::Navy,
                2,
                "NAVY_MIN_INTEL_TO_SHOW_EQUIPMENT_DESIGN_DETAILS",
            )
        } else if design.classifier_flags & engine_native::cequipment_type::PLANE_MASK != 0 {
            (
                IntelBranch::Air,
                3,
                "AIR_MIN_INTEL_TO_SHOW_EQUIPMENT_DESIGN_DETAILS",
            )
        } else {
            (
                IntelBranch::Army,
                1,
                "ARMY_MIN_INTEL_TO_SHOW_EQUIPMENT_DESIGN_DETAILS",
            )
        };
    let maxima = defines
        .fixed5_array("NIntel", "COUNTRY_LEVEL_INTEL_MAXIMUMS")
        .map_err(|e| e.to_string())?;
    let maximum = *maxima
        .get(lane)
        .ok_or("COUNTRY_LEVEL_INTEL_MAXIMUMS lacks the designer's branch")?;
    let threshold = defines
        .fixed5("NIntel", threshold)
        .map_err(|e| e.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    replay_derive::visibility::design_details(
        frame,
        &links,
        observer,
        target_country,
        fog,
        bypass,
        branch,
        maximum,
        threshold,
    )
    .map_err(str::to_owned)
}

/// Generic-upgrade source/target query. Native merge traversal stops when
/// either sorted upgrade roster ends. Partitions belong to the target roster;
/// source-only definitions outside those partitions contribute zero.
pub fn generic_upgrade_xp(
    frame: &DecodedFrame,
    target: u64,
    source: u64,
    content: &Database,
    rules: &UpgradeXpRules,
) -> Result<i64, String> {
    let target_definition = definition(frame, target)?;
    definition(frame, source)?;
    if let Some(reason) = &content.lost {
        return Err(reason.clone());
    }
    let rows = frame
        .complete::<tables::VariantUpgrades>()
        .map_err(|e| e.to_string())?;
    let roster = |id| {
        rows.iter()
            .filter(move |row| row.variant == id)
            .collect::<Vec<_>>()
    };
    let target = roster(target);
    let source = roster(source);
    if target
        .windows(2)
        .any(|pair| pair[0].definition_token >= pair[1].definition_token)
        || source
            .windows(2)
            .any(|pair| pair[0].definition_token >= pair[1].definition_token)
    {
        return Err("saved upgrade roster is not in unique native token order".into());
    }
    let licensed = target_definition.origin_link as i32 > 0;
    let mut raw = 0_i64;
    let (mut t, mut s) = (0, 0);
    while t < target.len() && s < source.len() {
        let a = target[t];
        let b = source[s];
        let (token, from, to) = match a.definition_token.cmp(&b.definition_token) {
            std::cmp::Ordering::Less => {
                t += 1;
                (a.definition_token, 0, a.level)
            }
            std::cmp::Ordering::Equal => {
                t += 1;
                s += 1;
                (a.definition_token, b.level, a.level)
            }
            std::cmp::Ordering::Greater => {
                s += 1;
                (b.definition_token, b.level, 0)
            }
        };
        if !target.iter().any(|row| row.definition_token == token) {
            continue;
        }
        let upgrade = content
            .upgrade_design(spelling(frame, token as u32)?)
            .ok_or_else(|| format!("upgrade token {token} has no authored cost operands"))?;
        raw = raw.wrapping_add(rules.change(upgrade, from, to, licensed));
    }
    let currency = target
        .first()
        .map(|row| -> Result<_, String> {
            Ok(content
                .upgrade_design(spelling(frame, row.definition_token as u32)?)
                .ok_or("first upgrade cost currency is not in content")?
                .currency)
        })
        .transpose()?
        .unwrap_or(UpgradeCurrency::Land);
    // A zero sum remains zero under every final factor. Do not require an
    // unrelated missing country operand for an unchanged upgrade roster.
    if raw == 0 {
        return Ok(0);
    }
    let family = match currency {
        UpgradeCurrency::Land => 0x1d9,
        UpgradeCurrency::Naval => 0x1d8,
        UpgradeCurrency::Air => 0x1da,
    };
    let country = target_definition.creator_link;
    Ok(rules.finish(
        raw,
        country_modifier(frame, country, 0x1d7)?,
        country_modifier(frame, country, family)?,
        content_modifier(frame, country, target_definition.archetype_token)?,
        licensed,
    ))
}

/// Authored admission gates in the designer's fresh country scope. The shared
/// evaluator retains reached missing operands and unsupported trigger names.
pub fn module_admission(
    world: &dashboard_text::triggers::World<'_>,
    country: u32,
    module: &[u8],
    content: &Database,
) -> Result<bool, String> {
    use dashboard_text::triggers::{Scope, Scopes};
    let conditions = content.module_conditions(module).ok_or_else(|| {
        format!(
            "module {} admission projection is absent",
            String::from_utf8_lossy(module)
        )
    })?;
    let slot = if country == 0 {
        0
    } else {
        world
            .slot(country)
            .ok_or("module admission country reference is not recorded")?
    };
    let scope = Scopes::of(Scope::Country(slot));
    if let Some(limit) = &conditions.limit
        && !world.evaluate(limit, scope).decided()?
    {
        return Ok(false);
    }
    world.evaluate(&conditions.allowed, scope).decided()
}

pub fn structural_validity(
    frame: &DecodedFrame,
    variant: u64,
    content: &Database,
) -> Result<installed_content_core::equipment::ModuleValidity, String> {
    let row = definition(frame, variant)?;
    let kind = content
        .named(spelling(frame, row.equipment_token)?)
        .ok_or("equipment validation content is absent (count: 1)")?;
    let assignments = frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == variant)
        .map(|row| {
            Ok(installed_content_core::equipment::Assignment {
                slot: spelling(frame, row.slot)?,
                module: if row.definition_token == 0 {
                    b""
                } else {
                    spelling(frame, row.definition_token)?
                },
                active: row.definition_active != 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    content.validate_modules(kind, &assignments)
}

pub fn permitted_roles(
    frame: &DecodedFrame,
    variant: u64,
    content: &Database,
) -> Result<Vec<String>, String> {
    let row = definition(frame, variant)?;
    let kind = content
        .named(spelling(frame, row.equipment_token)?)
        .ok_or("equipment role content is absent (count: 1)")?;
    let assignments = frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == variant)
        .map(|row| {
            Ok(installed_content_core::equipment::Assignment {
                slot: spelling(frame, row.slot)?,
                module: if row.definition_token == 0 {
                    b""
                } else {
                    spelling(frame, row.definition_token)?
                },
                active: row.definition_active != 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut roles = Vec::new();
    for (name, mask) in engine_executable::executable_tables::auxiliary_part(|tables| {
        &tables.category_names
    })
    .map_err(|withheld| withheld.to_string())?
    {
        if *mask != 0 && content.role_permitted(kind, &assignments, *mask)? {
            roles.push((*name).to_owned());
        }
    }
    Ok(roles)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WingContext<'a> {
    pub wing: &'a rows::AirWingRow,
    pub experience: i64,
    pub ace: Option<&'a rows::AirAceRow>,
}
pub fn wing_context(
    frame: &DecodedFrame,
    variant: u64,
    wing: u64,
) -> Result<WingContext<'_>, String> {
    let wings = frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?;
    let row = wings
        .iter()
        .find(|row| row.id == wing)
        .ok_or_else(|| format!("wing {wing} is not recorded"))?;
    if !frame
        .complete::<tables::WingEquipment>()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|row| row.wing == wing && row.variant == variant)
    {
        return Err(format!(
            "wing {wing} does not hold selected design {variant}"
        ));
    }
    let experience = frame
        .complete::<tables::WingCumulative>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == wing)
        .ok_or_else(|| format!("wing {wing} experience is not recorded"))?
        .experience;
    let ace = if row.ace == 0 {
        None
    } else {
        Some(
            frame
                .complete::<tables::AirAces>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|ace| ace.id == row.ace)
                .ok_or_else(|| format!("wing {wing} ace {} is not recorded", row.ace))?,
        )
    };
    Ok(WingContext {
        wing: row,
        experience,
        ace,
    })
}
pub fn ship_context(
    frame: &DecodedFrame,
    variant: u64,
    ship: u64,
) -> Result<&rows::ShipRow, String> {
    let model = frame
        .complete::<tables::ShipModels>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.ship == ship)
        .ok_or_else(|| format!("ship {ship} model is not recorded"))?;
    if model.variant != variant {
        return Err(format!(
            "ship {ship} does not hold selected design {variant}"
        ));
    }
    frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == ship)
        .ok_or_else(|| format!("ship {ship} experience is not recorded"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{Metrics, TableView};
    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture operand unavailable".into(),
                }
                .into()
            }),
        }
    }
    #[test]
    fn design_resource_pool_keys_join_resource_tokens_independently_of_definition_indices() {
        let mut f = frame();
        *f.tables[TableId::ResourceDefinitions.index()] =
            TableView::Complete(TableRows::ResourceDefinitions(vec![
                rows::ResourceDefinitionRow {
                    id: 7,
                    token: 5_000,
                    ..Default::default()
                },
            ]));
        *f.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
                token: 5_000,
                literal: b"synthetic_resource".to_vec(),
            }]));
        assert_eq!(resource_name(&f, 5_000).unwrap(), b"synthetic_resource");
        assert!(
            resource_name(&f, 7)
                .unwrap_err()
                .contains("resource token 7")
        );
    }
    #[test]
    fn recorded_game_fog_preserves_nonzero_and_refuses_missing_or_multiple_singletons() {
        let mut f = frame();
        assert!(recorded_fog(&f).is_err());
        for (byte, expected) in [(255, true), (0, false)] {
            *f.tables[TableId::DesignObserverContext.index()] =
                TableView::Complete(TableRows::DesignObserverContext(vec![
                    rows::DesignObserverContextRow { fog: byte },
                ]));
            assert_eq!(recorded_fog(&f), Ok(expected));
        }
        *f.tables[TableId::DesignObserverContext.index()] = TableView::Complete(
            TableRows::DesignObserverContext(vec![rows::DesignObserverContextRow::default(); 2]),
        );
        assert!(recorded_fog(&f).is_err());
    }
    #[test]
    fn design_discount_joins_source_type_or_archetype_without_filtering_uses() {
        let mut f = frame();
        *f.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![
                rows::VariantDefinitionRow {
                    id: 70,
                    equipment_token: 9,
                    archetype_token: 8,
                    ..Default::default()
                },
            ]));
        let row = rows::DesignCountryTemplatesRow {
            country: 5_000,
            uses: vec![0, -4, 7],
            factors: vec![120_000, 120_000, 500_000],
            equipment: vec![vec![8], vec![9, 9], vec![7]],
            failure: 0,
        };
        *f.tables[TableId::DesignCountryTemplates.index()] =
            TableView::Complete(TableRows::DesignCountryTemplates(vec![row.clone()]));
        assert_eq!(template_discount(&f, 5_000, 70).unwrap(), 100_000);
        assert!(
            template_discount(&f, 4_999, 70)
                .unwrap_err()
                .contains("not recorded")
        );
        let mut failed = row;
        failed.failure = 2;
        failed.uses.clear();
        failed.factors.clear();
        failed.equipment.clear();
        *f.tables[TableId::DesignCountryTemplates.index()] =
            TableView::Complete(TableRows::DesignCountryTemplates(vec![failed.clone()]));
        assert!(
            template_discount(&f, 5_000, 70)
                .unwrap_err()
                .contains("refused")
        );
        failed.failure = 0;
        *f.tables[TableId::DesignCountryTemplates.index()] =
            TableView::Complete(TableRows::DesignCountryTemplates(vec![failed]));
        assert_eq!(template_discount(&f, 5_000, 70).unwrap(), 0);
    }
    #[test]
    fn design_generic_xp_joins_mixed_authored_domains_and_preserves_generic_zero_clamp() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let content = crate::test_fixtures::equipment_types(&[br#"upgrades = {
            synthetic_land = { cost = land }
            synthetic_air = { cost = air }
            synthetic_linear = { cost = naval linear_cost = { cost_by_level = 1.23457 cost_by_level_for_licensed_equipment = 2.34567 } }
        }"#]).unwrap();
        let mut f = frame();
        *f.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![
                rows::VariantDefinitionRow {
                    id: 60,
                    ..Default::default()
                },
                rows::VariantDefinitionRow {
                    id: 70,
                    creator_link: 5_000,
                    archetype_token: 99,
                    ..Default::default()
                },
            ]));
        *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
            [
                b"synthetic_land".as_slice(),
                b"synthetic_air",
                b"synthetic_linear",
            ]
            .iter()
            .enumerate()
            .map(|(index, name)| rows::LexicalTokenRow {
                token: index as u32 + 1,
                literal: name.to_vec(),
            })
            .collect(),
        ));
        let mut upgrades = Vec::new();
        for variant in [60, 70] {
            for token in 1..=3 {
                upgrades.push(rows::VariantUpgradeRow {
                    variant,
                    source_ordinal: token - 1,
                    definition_token: token as i32,
                    level: if variant == 70 { token as u8 } else { 0 },
                });
            }
        }
        *f.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(upgrades));
        *f.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                rows::ModifierDefinitionRow {
                    index: 777,
                    token: 99,
                    ..Default::default()
                },
            ]));
        *f.tables[TableId::DesignCountryModifiers.index()] =
            TableView::Complete(TableRows::DesignCountryModifiers(
                [0x1d7, 0x1d9, 777]
                    .into_iter()
                    .map(|modifier| rows::DesignCountryModifierRow {
                        country: 5_000,
                        modifier,
                        value: Some(0),
                        failure: 0,
                    })
                    .collect(),
            ));
        let rules = UpgradeXpRules {
            base: [10, 20, 30],
            ramp: [1, 2, 3],
            licensed_factor: 123_457,
        };
        assert_eq!(
            generic_upgrade_xp(&f, 70, 60, &content, &rules).unwrap(),
            7_670_371
        );
        let TableView::Complete(TableRows::DesignCountryModifiers(values)) =
            &mut *f.tables[TableId::DesignCountryModifiers.index()]
        else {
            unreachable!()
        };
        values[1].value = Some(-100_000);
        assert_eq!(generic_upgrade_xp(&f, 70, 60, &content, &rules).unwrap(), 0);
        *f.tables[TableId::DesignCountryModifiers.index()] = TableView::Unavailable {
            reason: "lost effective country factor".into(),
        };
        assert!(
            generic_upgrade_xp(&f, 70, 60, &content, &rules)
                .unwrap_err()
                .contains("lost effective country factor")
        );
        assert_eq!(
            generic_upgrade_xp(&f, 70, 70, &content, &rules).unwrap(),
            0,
            "an unchanged roster does not need any effective factor"
        );
    }
    #[test]
    fn design_unit_context_joins_only_selected_live_unit_and_requires_ace_inputs() {
        let mut f = frame();
        *f.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![rows::AirWingRow {
                id: 5_000,
                ace: 9,
                ..Default::default()
            }]));
        *f.tables[TableId::WingEquipment.index()] =
            TableView::Complete(TableRows::WingEquipment(vec![rows::WingEquipmentRow {
                wing: 5_000,
                variant: 70,
                ..Default::default()
            }]));
        *f.tables[TableId::WingCumulative.index()] =
            TableView::Complete(TableRows::WingCumulative(vec![rows::WingCumulativeRow {
                id: 5_000,
                experience: 123_457,
                ..Default::default()
            }]));
        assert!(
            wing_context(&f, 70, 5_000)
                .unwrap_err()
                .contains("fixture operand")
        );
        *f.tables[TableId::AirAces.index()] =
            TableView::Complete(TableRows::AirAces(vec![rows::AirAceRow {
                id: 9,
                assigned_wing: 5_000,
                name: b"Synthetic".to_vec(),
                ..Default::default()
            }]));
        let context = wing_context(&f, 70, 5_000).unwrap();
        assert_eq!(context.experience, 123_457);
        assert_eq!(context.ace.unwrap().name, b"Synthetic");
        assert!(
            wing_context(&f, 71, 5_000)
                .unwrap_err()
                .contains("does not hold")
        );
        *f.tables[TableId::ShipModels.index()] =
            TableView::Complete(TableRows::ShipModels(vec![rows::ShipModelRow {
                ship: 8,
                variant: 70,
                ..Default::default()
            }]));
        *f.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![rows::ShipRow {
                id: 8,
                experience: 765_432,
                ..Default::default()
            }]));
        assert_eq!(ship_context(&f, 70, 8).unwrap().experience, 765_432);
        assert!(
            ship_context(&f, 71, 8)
                .unwrap_err()
                .contains("does not hold")
        );
    }
}
