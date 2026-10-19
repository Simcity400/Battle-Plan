//! On-demand design statistics over recorded state and the verified content stack.
use installed_content_core::equipment::{Database, STAT_COUNT, StatComponents, StatGroup, StatValue};
use replay_data::rows::{self, tables};
use replay_frame::frame::DecodedFrame;
pub mod costs;
pub mod detail;
pub mod industrial;
pub mod queries;
pub mod unit_effects;
pub mod unlocks;

/// Every source the calculation retains, before averages and factors combine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub scale: i64,
    pub terms: StatGroup,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub values: [i64; STAT_COUNT],
    pub components: StatComponents,
    pub sources: Vec<Source>,
    pub upgrade_level_sum: i32,
    pub mission_stats: Vec<rows::VariantMissionStats>,
    pub build_cost_raw: i64,
    pub build_resources: Vec<rows::VariantStatEntry>,
    pub manpower: i32,
    pub terrain_attrition: Vec<i64>,
    pub niches: Vec<u32>,
    /// A computed cached result cannot reconstruct each named runtime source.
    pub breakdown_refusal: Option<String>,
}

/// One effective designer modifier, acquired for a country/key actually
/// demanded by saved designs. A missing or failed scope is never a zero.
/// `country` is the raw creator reference, as retained by the design row.
pub fn country_modifier(frame: &DecodedFrame, country: u32, modifier: u32) -> Result<i64, String> {
    let rows = frame
        .complete::<tables::DesignCountryModifiers>()
        .map_err(|e| e.to_string())?;
    let index = rows
        .binary_search_by_key(&(country, modifier), |row| (row.country, row.modifier))
        .map_err(|_| format!("country {country} design modifier {modifier} is not recorded"))?;
    rows[index]
        .value
        .filter(|_| rows[index].failure == 0)
        .ok_or_else(|| {
            format!(
                "country {country} design modifier {modifier} is unavailable (scope {})",
                rows[index].failure
            )
        })
}

/// A type/module's registered design modifier uses that content token in the
/// shared modifier metadata. No vanilla spelling or registration order is used.
pub fn content_modifier(frame: &DecodedFrame, country: u32, token: u32) -> Result<i64, String> {
    let definitions = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?;
    let mut matching = definitions.iter().filter(|row| row.token == token);
    let modifier = matching
        .next()
        .ok_or_else(|| format!("design content token {token} has no modifier registration"))?;
    if matching.next().is_some() {
        return Err(format!(
            "design content token {token} has ambiguous modifier registrations"
        ));
    }
    country_modifier(frame, country, modifier.index)
}

/// The XP contribution of one requested saved module change. Whole-design
/// licensing, organisation and template discounts belong to the final query.
pub fn module_change_xp(
    frame: &DecodedFrame,
    country: u32,
    target: Option<u32>,
    source: Option<(u32, bool)>,
    content: &Database,
    rules: &installed_content_core::equipment::ModuleXpRules,
) -> Result<i64, String> {
    if let Some(reason) = &content.lost {
        return Err(reason.clone());
    }
    let Some(modifier_token) = target.or(source.map(|(token, _)| token)) else {
        return Ok(0);
    };
    let modifier = content_modifier(frame, country, modifier_token)?;
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    let module = |token| {
        let name = &tokens[tokens
            .binary_search_by_key(&token, |row| row.token)
            .map_err(|_| format!("module token {token} has no recorded spelling"))?]
        .literal;
        let design = content.module_design(name).ok_or_else(|| {
            format!(
                "module {} is absent from recorded content",
                String::from_utf8_lossy(name)
            )
        })?;
        Ok::<_, String>((name.as_slice(), design))
    };
    let target = target.map(module).transpose()?;
    let original = source.map(|(token, _)| module(token)).transpose()?;
    Ok(rules.slot(
        target,
        original,
        source.is_some_and(|(_, active)| active),
        modifier,
    ))
}

/// Derive only the requested saved design. There is no country/design matrix
/// and no native game read during playback. Unknown reached input is an error.
pub fn sheet(frame: &DecodedFrame, variant: u64, content: &Database) -> Result<Sheet, String> {
    if let Some(reason) = &content.lost {
        return Err(reason.clone());
    }
    let definitions = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|e| e.to_string())?;
    let definition = &definitions[definitions
        .binary_search_by_key(&variant, |row| row.id)
        .map_err(|_| format!("design {variant} is not recorded"))?];
    let state = definition
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    let spelling = |token| -> Result<&[u8], String> {
        let row = &tokens[tokens
            .binary_search_by_key(&token, |row| row.token)
            .map_err(|_| format!("design token {token} has no recorded spelling"))?];
        Ok(row.literal.as_slice())
    };
    let equipment_name = spelling(definition.equipment_token)?;
    let kind = content.named(equipment_name).ok_or_else(|| {
        format!(
            "equipment {} is not in the recorded content",
            String::from_utf8_lossy(equipment_name)
        )
    })?;
    let mut components = StatComponents::base(&kind.stats);
    let mut sources = vec![Source {
        name: String::from_utf8_lossy(&kind.name).into_owned(),
        scale: 100_000,
        terms: StatGroup {
            add: kind
                .stats
                .iter()
                .enumerate()
                .filter(|(_, value)| **value != 0)
                .map(|(index, value)| StatValue {
                    index,
                    value: *value,
                })
                .collect(),
            ..Default::default()
        },
    }];
    let snapshots = if state.bonus_sources.is_empty() && state.named_sources.is_empty() {
        &[][..]
    } else {
        frame
            .complete::<tables::VariantBonusSnapshots>()
            .map_err(|e| e.to_string())?
    };
    for (ordinal, id) in state
        .bonus_sources
        .iter()
        .chain(&state.named_sources)
        .enumerate()
    {
        let source = &snapshots[snapshots
            .binary_search_by_key(id, |row| row.id)
            .map_err(|_| format!("design bonus snapshot {id} is not recorded"))?]
        .source;
        let name = source
            .named
            .as_ref()
            .map(|named| String::from_utf8_lossy(&named.name).into_owned())
            .or_else(|| {
                source
                    .idea
                    .as_ref()
                    .map(|idea| String::from_utf8_lossy(idea).into_owned())
            })
            .unwrap_or_else(|| format!("applied equipment bonus {ordinal}"));
        let mut multiply = Vec::new();
        for entry in &source.stats {
            components
                .applied(false, entry.index as usize, entry.value)
                .map_err(str::to_owned)?;
            multiply.push(StatValue {
                index: entry.index as usize,
                value: entry.value,
            });
        }
        sources.push(Source {
            name,
            scale: 100_000,
            terms: StatGroup {
                multiply,
                ..Default::default()
            },
        });
    }
    let mut organisation = StatGroup::default();
    for entry in &state.design_team_bonus {
        let name = spelling(entry.index)?;
        let index =
            installed_content_core::equipment::stat_index(&name.to_ascii_lowercase()).ok_or_else(|| {
                format!(
                    "organisation bonus {} is not an equipment stat",
                    String::from_utf8_lossy(name)
                )
            })?;
        let target = if [0x25, 0x26, 0x27].contains(&index) {
            &mut organisation.add
        } else {
            &mut organisation.multiply
        };
        target.push(StatValue {
            index,
            value: entry.value,
        });
        components
            .applied([0x25, 0x26, 0x27].contains(&index), index, entry.value)
            .map_err(str::to_owned)?;
    }
    if !state.design_team_bonus.is_empty() {
        sources.push(Source {
            name: format!("industrial organisation {}", state.design_team),
            terms: organisation,
            scale: 100_000,
        });
    }
    let mut apply = |name: String, terms: StatGroup, scale: i64| -> Result<(), String> {
        components.apply(&terms, scale).map_err(str::to_owned)?;
        sources.push(Source { name, scale, terms });
        Ok(())
    };
    let mut upgrade_level_sum = 0_i32;
    for upgrade in frame
        .complete::<tables::VariantUpgrades>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == variant)
    {
        upgrade_level_sum = upgrade_level_sum.wrapping_add(i32::from(upgrade.level));
        let name = spelling(upgrade.definition_token as u32)?;
        let group = content.upgrade_stats(name).ok_or_else(|| {
            format!(
                "upgrade {} has no content stat group",
                String::from_utf8_lossy(name)
            )
        })?;
        apply(
            String::from_utf8_lossy(name).into_owned(),
            group.clone(),
            i64::from(upgrade.level) * 100_000,
        )?;
    }
    for module in frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == variant)
    {
        let name = spelling(module.definition_token)?;
        let group = content.module_stats(name).ok_or_else(|| {
            format!(
                "module {} has no content stat group",
                String::from_utf8_lossy(name)
            )
        })?;
        apply(
            String::from_utf8_lossy(name).into_owned(),
            group.clone(),
            100_000,
        )?;
    }
    let mut values = components.finish();
    if definition.classifier_flags & 0x1f0037fc00 != 0 {
        values[0x2f] = values[0x2f].max(100_000);
        values[0x31] = values[0x31].max(100_000);
    }
    values[0x3e] = values[0x3e].max(0);
    let caches = frame
        .complete::<tables::VariantStats>()
        .map_err(|e| e.to_string())?;
    let cached = caches
        .iter()
        .find(|row| row.variant == variant)
        .ok_or_else(|| format!("design {variant} lacks its native stat residuals"))?;
    // These retained cache slots include the rebuild-mode reliability clamp
    // and the aircraft thrust/weight post-adjustment. The designer does not
    // retain its last rebuild-mode argument in the design's source inputs.
    for (index, value) in [0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d]
        .into_iter()
        .zip(cached.values)
    {
        values[index] = value;
    }
    let breakdown_refusal = if state.named_bonuses.is_empty() {
        None
    } else {
        if state.unresolved_stats.len() != STAT_COUNT - 10 {
            return Err("named runtime bonus stat residuals are incomplete".into());
        }
        for entry in &state.unresolved_stats {
            let target = values
                .get_mut(entry.index as usize)
                .ok_or("native stat residual index outside metadata")?;
            *target = entry.value;
        }
        let computed = components.finish();
        state
            .unresolved_stats
            .iter()
            .any(|entry| computed[entry.index as usize] != entry.value)
            .then(|| "named source values differ from the design's last cached rebuild".to_owned())
    };
    Ok(Sheet {
        values,
        components,
        sources,
        upgrade_level_sum,
        mission_stats: state.mission_stats.clone(),
        build_cost_raw: state.build_cost,
        build_resources: state.build_resources.clone(),
        manpower: state.manpower,
        terrain_attrition: state.terrain_attrition.clone(),
        niches: state.niches.clone(),
        breakdown_refusal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{Recorded, TableId, TableRows};
    use replay_frame::frame::{Metrics, TableView};

    #[test]
    fn design_module_xp_joins_country_context_by_recorded_registration_and_refuses_missing_scope() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let content = crate::test_fixtures::equipment_types(&[br#"
            equipment_modules = {
                synthetic_next = { can_convert_from = { module = synthetic_old } }
                synthetic_old = { category = synthetic }
            }
        "#])
        .unwrap();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                rows::LexicalTokenRow {
                    token: 2,
                    literal: b"synthetic_next".to_vec(),
                },
                rows::LexicalTokenRow {
                    token: 3,
                    literal: b"synthetic_old".to_vec(),
                },
            ]));
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                rows::ModifierDefinitionRow {
                    index: 987,
                    token: 2,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::DesignCountryModifiers.index()] =
            TableView::Complete(TableRows::DesignCountryModifiers(vec![
                rows::DesignCountryModifierRow {
                    country: 5_000,
                    modifier: 987,
                    value: Some(-12_345),
                    failure: 0,
                },
            ]));
        let rules = installed_content_core::equipment::ModuleXpRules {
            add: 300_001,
            replace: 700_001,
            convert: 200_001,
            remove: 100_001,
        };
        assert_eq!(
            module_change_xp(&frame, 5_000, Some(2), Some((3, true)), &content, &rules).unwrap(),
            175_310
        );
        assert_eq!(
            module_change_xp(&frame, 5_000, Some(2), Some((3, false)), &content, &rules).unwrap(),
            262_965
        );
        assert_eq!(
            module_change_xp(&frame, 5_000, Some(2), Some((2, true)), &content, &rules).unwrap(),
            0
        );
        assert!(
            module_change_xp(&frame, 2, Some(2), None, &content, &rules)
                .unwrap_err()
                .contains("not recorded")
        );
        assert!(
            module_change_xp(&frame, 5_000, None, Some((3, true)), &content, &rules)
                .unwrap_err()
                .contains("no modifier registration")
        );
        *frame.tables[TableId::DesignCountryModifiers.index()] =
            TableView::Complete(TableRows::DesignCountryModifiers(vec![
                rows::DesignCountryModifierRow {
                    country: 5_000,
                    modifier: 987,
                    value: None,
                    failure: 2,
                },
            ]));
        assert!(
            module_change_xp(&frame, 5_000, Some(2), None, &content, &rules)
                .unwrap_err()
                .contains("unavailable")
        );
    }

    #[test]
    fn design_sheet_uses_recorded_sources_modules_upgrades_and_native_residuals() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let content = crate::test_fixtures::equipment_types(&[br#"
            equipment_modules = {
                synthetic_zero = { add_stats = { soft_attack = 2 } add_average_stats = { soft_attack = 0 }
                    mission_type_stats = { limit = { interception } add_stats = { soft_attack = 1.25 } } }
                synthetic_second = { add_average_stats = { soft_attack = 4 } multiply_stats = { soft_attack = 0.1 } }
            }
            upgrades = { synthetic_upgrade = { soft_attack = 0.25 } }
            equipments = { synthetic_type = { soft_attack = 10 } }
        "#]).unwrap();
        let index = installed_content_core::equipment::stat_index(b"soft_attack").unwrap();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                [
                    b"synthetic_type".as_slice(),
                    b"synthetic_zero",
                    b"synthetic_second",
                    b"synthetic_upgrade",
                    b"soft_attack",
                ]
                .into_iter()
                .enumerate()
                .map(|(token, name)| rows::LexicalTokenRow {
                    token: token as u32 + 1,
                    literal: name.to_vec(),
                })
                .collect(),
            ));
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![
                rows::VariantModuleRow {
                    variant: 70,
                    source_ordinal: 0,
                    definition_token: 2,
                    ..Default::default()
                },
                rows::VariantModuleRow {
                    variant: 70,
                    source_ordinal: 1,
                    definition_token: 3,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(vec![rows::VariantUpgradeRow {
                variant: 70,
                definition_token: 4,
                level: 2,
                ..Default::default()
            }]));
        *frame.tables[TableId::VariantStats.index()] =
            TableView::Complete(TableRows::VariantStats(vec![rows::VariantStatsRow {
                variant: 70,
                values: [7; 10],
                ..Default::default()
            }]));
        let source = rows::VariantBonusSource {
            idea: Some(b"synthetic_company".to_vec()),
            stats: vec![rows::VariantStatEntry {
                index: index as u32,
                value: 20_000,
            }],
            ..Default::default()
        };
        let source_id = rows::variant_bonus_snapshot_id(&source);
        *frame.tables[TableId::VariantBonusSnapshots.index()] =
            TableView::Complete(TableRows::VariantBonusSnapshots(vec![
                rows::VariantBonusSnapshotRow {
                    id: source_id,
                    source,
                },
            ]));
        let design = rows::VariantDesignState {
            design_team: 9,
            bonus_sources: vec![source_id],
            design_team_bonus: vec![rows::VariantStatEntry {
                index: 5,
                value: 10_000,
            }],
            ..Default::default()
        };
        for flags in [
            0,
            engine_native::cequipment_type::SHIP_MASK,
            engine_native::cequipment_type::PLANE_MASK,
            1 << 10,
        ] {
            *frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(vec![
                    rows::VariantDefinitionRow {
                        id: 70,
                        equipment_token: 1,
                        classifier_flags: flags,
                        design: Recorded::Value(design.clone()),
                        ..Default::default()
                    },
                ]));
            let sheet = sheet(&frame, 70, &content).unwrap();
            assert_eq!(sheet.values[index], 2_660_000);
            assert_eq!(sheet.components.values[index].average_count, 2);
            assert_eq!(sheet.values[0x41], 7);
            assert_eq!(sheet.sources.len(), 6);
            assert!(sheet.breakdown_refusal.is_none());
            let sections = detail::inspect(
                &frame,
                70,
                None,
                None,
                None,
                &content,
                None,
                Err("defines fixture unavailable"),
            );
            assert!(
                sections
                    .iter()
                    .any(|section| section.title == "Stat sheet"
                        && section.fields.len() == STAT_COUNT)
            );
            assert!(sections.iter().any(|section| {
                section.title == "Stat contributors"
                    && section
                        .fields
                        .iter()
                        .any(|field| field.label.contains("synthetic_company"))
            }));
            assert!(
                sections
                    .iter()
                    .find(|section| section.title == "Validity and roles")
                    .unwrap()
                    .fields
                    .iter()
                    .any(|field| field.refusal.is_some())
            );
        }
        let mission = installed_content_core::equipment::mission_mask(b"interception");
        assert_ne!(mission, 0);
        let TableView::Complete(TableRows::VariantDefinitions(definitions)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        else {
            unreachable!()
        };
        let Recorded::Value(state) = &mut definitions[0].design else {
            unreachable!()
        };
        state.mission_stats.push(rows::VariantMissionStats {
            mask: mission,
            stats: vec![rows::VariantStatEntry {
                index: index as u32,
                value: 2_897_500,
            }],
        });
        let mission_sheet = queries::mission_sheet(&frame, 70, mission, &content).unwrap();
        assert_eq!(mission_sheet.values[index], 2_897_500);
        assert!(mission_sheet.reconciliation.is_ok());
        assert_eq!(mission_sheet.contributors.len(), 7);
        let TableView::Complete(TableRows::VariantDefinitions(definitions)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        else {
            unreachable!()
        };
        let Recorded::Value(state) = &mut definitions[0].design else {
            unreachable!()
        };
        state
            .module_mission_exclusions
            .push(rows::VariantModuleMissionExclusion {
                module_token: 2,
                mask: mission,
            });
        let mission_sheet = queries::mission_sheet(&frame, 70, mission, &content).unwrap();
        assert_eq!(
            mission_sheet.values[index], 2_897_500,
            "held native result survives contributor refusal"
        );
        assert!(mission_sheet.reconciliation.is_err());
        assert_eq!(mission_sheet.contributors.len(), 6);
        *frame.tables[TableId::VariantUpgrades.index()] = TableView::Unavailable {
            reason: "lost upgrade assignment".into(),
        };
        assert!(
            sheet(&frame, 70, &content)
                .unwrap_err()
                .contains("lost upgrade assignment")
        );
    }
}
