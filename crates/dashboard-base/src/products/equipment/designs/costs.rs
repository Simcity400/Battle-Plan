//! Save XP for one selected source and target. No pair values are recorded.
use super::{queries::*, *};
use crate::products::industrial_organisations::{local_modifier, owner_roster};
use dashboard_text::triggers::{Evaluator, OrganisationCondition};
use engine_executable::modifier_keys::MILITARY_INDUSTRIAL_ORGANIZATION_DESIGN_TEAM_CHANGE_COST as ORGANISATION_CHANGE;
use installed_content_core::equipment::{ModuleXpRules, UpgradeXpRules};

#[derive(Default)]
pub struct OrganisationContent {
    pub traits: installed_content_core::mio_traits::Catalog,
    pub definitions: installed_content_core::industrial_organisations::Database,
    pub groups: installed_content_core::equipment::Groups,
}
impl OrganisationContent {
    pub fn load(layers: &installed_content_core::ResolvedLayers) -> std::io::Result<Self> {
        let names = installed_content_core::key_names::KeyNames::read(layers)?;
        Ok(Self {
            traits: installed_content_core::mio_traits::read(layers)?,
            definitions: installed_content_core::industrial_organisations::read(layers, &names)?,
            groups: installed_content_core::equipment::Groups::load(layers)?,
        })
    }
}

/// Current production bonus for the requested organisation and target type.
/// Frozen design-team bonuses belong to the sheet, not this production query.
pub fn organisation_conversion_bonus(
    frame: &DecodedFrame,
    reference: u64,
    target: u64,
    inputs: &OrganisationContent,
    equipment: &Database,
    world: &dashboard_text::triggers::World<'_>,
) -> Result<i64, String> {
    if reference == 0 || !author_live(frame, reference)? {
        return Ok(0);
    }
    let details = frame
        .complete::<tables::IndustrialOrganisationDetails>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.organisation == reference)
        .ok_or_else(|| {
            format!("organisation {reference} completed traits unavailable (count: 1)")
        })?;
    let organisation = frame
        .complete::<tables::IndustrialOrganisations>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.organisation == reference)
        .ok_or_else(|| format!("organisation {reference} owner unavailable (count: 1)"))?;
    let name =
        std::str::from_utf8(spelling(frame, organisation.token)?).map_err(|e| e.to_string())?;
    let tree = inputs
        .traits
        .tree(name)?
        .ok_or_else(|| format!("organisation {name} content unavailable (count: 1)"))?;
    let kind = equipment
        .named(spelling(frame, definition(frame, target)?.equipment_token)?)
        .ok_or("organisation production target type unavailable (count: 1)")?;
    let index = equipment
        .equipment
        .iter()
        .position(|row| std::ptr::eq(row, kind))
        .unwrap();
    let admits = |names: &[Vec<u8>]| -> Result<bool, String> {
        // Past a group reading the port cannot follow, any name may be a later group.
        if let Some(lost) = &inputs.groups.lost {
            return Err(format!(
                "organisation equipment groups are unread past {lost} (count: 1)"
            ));
        }
        let members = inputs.groups.expand(names)?;
        for member in members {
            let mask = installed_content_core::equipment::category_mask(&member).unwrap_or(0);
            if mask != 0 {
                if equipment.category(kind) & mask == mask {
                    return Ok(true);
                }
            } else if equipment.group_admits(&[member], index) {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let bonus = |terms: &[installed_content_core::mio_traits::Bonus]| {
        terms
            .iter()
            .find(|term| term.name == "production_conversion_speed_factor")
            .map_or(0, |term| term.value)
    };
    let mut sum = 0_i64;
    // As the organisation's modifiers: `visible`, then `available`, in its organisation/owner scope pair.
    let ordinary = if tree.visible.is_empty() && tree.available.is_empty() {
        true
    } else {
        Evaluator::new(world, None).organisation_conditions(
            reference,
            &[
                (OrganisationCondition::Visible, tree.visible),
                (OrganisationCondition::Available, tree.available),
            ],
        )?
    };
    if ordinary {
        for token in &details.unlocked {
            let token = std::str::from_utf8(spelling(frame, *token)?).map_err(|e| e.to_string())?;
            if let Some(trait_) = inputs.traits.trait_definition(name, token)?
                && (trait_.equipment_types.is_empty() || admits(&trait_.equipment_types)?)
            {
                sum = sum.wrapping_add(bonus(&trait_.bonuses));
            }
        }
    }
    if let Some(initial) = tree.initial
        && (initial.equipment_types.is_empty() || admits(&initial.equipment_types)?)
    {
        sum = sum.wrapping_add(bonus(&initial.bonuses));
    }
    // The raw owner can be an alias; the roster is keyed by its country's id.
    let roster = owner_roster(frame, organisation).map_err(|why| format!("{why} (count: 1)"))?;
    if let Some(selected) = roster
        .policies
        .iter()
        .find(|row| row.token == details.state.policy)
    {
        for (group, terms) in &inputs
            .definitions
            .policies
            .policy(spelling(frame, selected.token)?)?
            .groups
        {
            if admits(std::slice::from_ref(group))?
                || (group.as_slice() == b"same_as_mio" && admits(tree.equipment_types)?)
            {
                sum = sum.wrapping_add(bonus(terms));
            }
        }
    }
    Ok(sum)
}

/// Existing organisation rows supply mutable inputs; authored modifiers and
/// reached organisation gates resolve through the recorded content stack.
pub fn organisation_change_xp(
    frame: &DecodedFrame,
    reference: u64,
    traits: &installed_content_core::mio_traits::Catalog,
    policies: &installed_content_core::industrial_organisations::policies::Catalog,
    world: &dashboard_text::triggers::World<'_>,
) -> Result<i64, String> {
    if reference == 0 || !author_live(frame, reference)? {
        return Ok(0);
    }
    let details = frame
        .complete::<tables::IndustrialOrganisationDetails>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.organisation == reference)
        .ok_or_else(|| format!("organisation {reference} mutable design costs are not recorded"))?;
    let organisation = frame
        .complete::<tables::IndustrialOrganisations>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.organisation == reference)
        .ok_or_else(|| format!("organisation {reference} identity is not recorded"))?;
    let local = local_modifier(
        frame,
        organisation,
        details,
        ORGANISATION_CHANGE,
        traits,
        policies,
        Ok(world),
    )?;
    let factor = local
        .wrapping_add(country_modifier(
            frame,
            organisation.owner,
            ORGANISATION_CHANGE,
        )?)
        .wrapping_add(100_000);
    let raw = i64::from(details.state.design_team_change_cost)
        .wrapping_mul(100_000)
        .wrapping_mul(factor)
        / 100_000;
    let rounded = raw.wrapping_add(if raw < 0 { -50_000 } else { 50_000 }) / 100_000;
    Ok(i64::from(rounded as i32))
}

fn equipment<'a>(
    frame: &DecodedFrame,
    id: u64,
    content: &'a Database,
) -> Result<&'a installed_content_core::equipment::Equipment, String> {
    if let Some(reason) = &content.lost {
        return Err(reason.clone());
    }
    let name = spelling(frame, definition(frame, id)?.equipment_token)?;
    content.named(name).ok_or_else(|| {
        format!(
            "equipment {} is absent from recorded content",
            String::from_utf8_lossy(name)
        )
    })
}

/// Generic upgrade cost and organisation change form the base save cost. The
/// template discount here belongs to the source family with its designer DLC
/// disabled; the module designer's final discount is a separate stage.
pub fn base_save_xp(
    frame: &DecodedFrame,
    target: u64,
    source: u64,
    content: &Database,
    defines: &installed_content_core::defines::Defines,
    organisation_change: impl FnOnce(u64) -> Result<i64, String>,
) -> Result<i64, String> {
    let target_row = definition(frame, target)?;
    let source_row = definition(frame, source)?;
    if target == source {
        return Ok(0);
    }
    let target_state = target_row
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let source_state = source_row
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let mut cost = generic_upgrade_xp(
        frame,
        target,
        source,
        content,
        &UpgradeXpRules::load(defines).map_err(|e| e.to_string())?,
    )?;
    if (target_state.design_team != source_state.design_team
        || target_state.design_team_bonus != source_state.design_team_bonus)
        && defines
            .int32("NIndustrialOrganisation", "DESIGN_TEAM_CHANGE_XP_COST")
            .map_err(|e| e.to_string())?
            != 0
    {
        let selected = if target_state.design_team != 0 {
            target_state.design_team
        } else {
            source_state.design_team
        };
        if selected == 0 {
            return Err(
                "changed frozen organisation bonuses have no organisation reference".into(),
            );
        }
        // The organisation getter rounds to an integer before returning it to
        // the fixed-point design cost accumulator.
        cost = cost.wrapping_add(organisation_change(selected)?.wrapping_mul(100_000));
    }
    if cost == 0 {
        return Ok(0);
    }
    let families = content.designer_families(equipment(frame, source, content)?)?;
    if families.iter().any(|family| *family) {
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|e| e.to_string())?
        else {
            return Err("designer DLC singleton is unavailable".into());
        };
        if families
            .into_iter()
            .zip([0x80, 0x100, 0x10])
            .any(|(family, bit)| family && features.dlc_mask & bit == 0)
        {
            cost = cost
                .wrapping_mul(100_000 - template_discount(frame, target_row.creator_link, source)?)
                / 100_000;
        }
    }
    Ok(cost)
}

/// Complete module designer stages. Slot lookup uses only explicit saved
/// assignments in the target type's declared slots, with no default fallback.
pub fn module_save_xp(
    frame: &DecodedFrame,
    target: u64,
    source: u64,
    content: &Database,
    defines: &installed_content_core::defines::Defines,
    organisation_change: impl FnOnce(u64) -> Result<i64, String>,
) -> Result<i64, String> {
    let row = definition(frame, target)?;
    if target == source {
        return Ok(0);
    }
    let equipment = equipment(frame, target, content)?;
    let base = base_save_xp(frame, target, source, content, defines, organisation_change)?;
    let rules = ModuleXpRules::load(defines).map_err(|e| e.to_string())?;
    let modules = frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?;
    let mut slots = 0_i64;
    for slot in &equipment.module_slots {
        let mut next = None;
        let mut previous = None;
        for module in modules
            .iter()
            .filter(|module| module.variant == target || module.variant == source)
        {
            if !spelling(frame, module.slot)?.eq_ignore_ascii_case(slot) {
                continue;
            }
            if module.variant == target {
                next = Some(module.definition_token);
            }
            if module.variant == source {
                previous = Some((module.definition_token, module.definition_active != 0));
            }
        }
        if next == previous.map(|(token, _)| token) {
            continue;
        }
        slots = slots.wrapping_add(module_change_xp(
            frame,
            row.creator_link,
            next,
            previous,
            content,
            &rules,
        )?);
    }
    let licensed_factor = UpgradeXpRules::load(defines)
        .map_err(|e| e.to_string())?
        .licensed_factor;
    let licensed = row.origin_link as i32 > 0;
    let applied_slots = if licensed {
        slots.wrapping_mul(licensed_factor) / 100_000
    } else {
        slots
    }
    .max(0);
    if base.wrapping_add(applied_slots) == 0 {
        return Ok(0);
    }
    let [tank, air, ship] = content.designer_families(equipment)?;
    let family = if ship {
        0x1d8
    } else if tank {
        0x1d9
    } else if air {
        0x1da
    } else {
        return Err("module designer equipment has no designer family".into());
    };
    Ok(installed_content_core::equipment::module_save_xp(
        base,
        slots,
        licensed,
        licensed_factor,
        country_modifier(frame, row.creator_link, 0x1d7)?,
        country_modifier(frame, row.creator_link, family)?,
        content_modifier(frame, row.creator_link, row.archetype_token)?,
        template_discount(frame, row.creator_link, target)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::industrial_organisations::organisation_modifier_value;
    use replay_data::rows::{Recorded, TableId, TableRows};
    use replay_frame::frame::{Metrics, TableView};
    use std::cell::Cell;

    fn inputs() -> (DecodedFrame, Database, installed_content_core::defines::Defines) {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let content = crate::test_fixtures::equipment_types(&[br#"
            equipment_modules = { new_module = { xp_cost = 3 } unused_module = { xp_cost = 999 } }
            upgrades = { generic = { cost = land } }
            equipments = {
                old_type = { is_archetype = yes type = armor }
                new_type = { is_archetype = yes type = armor module_slots = { main = { } } }
            }
        "#])
        .unwrap();
        let defines = installed_content_core::defines::Defines::new().unwrap();
        defines
            .load(
                "synthetic.lua",
                br#"NDefines={
            NMilitary={LAND_EQUIPMENT_BASE_COST=10, LAND_EQUIPMENT_RAMP_COST=0,
                NAVAL_EQUIPMENT_BASE_COST=20, NAVAL_EQUIPMENT_RAMP_COST=0,
                AIR_EQUIPMENT_BASE_COST=30, AIR_EQUIPMENT_RAMP_COST=0},
            NProduction={LICENSE_EQUIPMENT_UPGRADE_XP_FACTOR=1.5,
                EQUIPMENT_MODULE_ADD_XP_COST=5, EQUIPMENT_MODULE_REPLACE_XP_COST=6,
                EQUIPMENT_MODULE_CONVERT_XP_COST=7, EQUIPMENT_MODULE_REMOVE_XP_COST=8},
            NIndustrialOrganisation={DESIGN_TEAM_CHANGE_XP_COST=5}}
        "#,
            )
            .unwrap();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "missing fixture input".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(
                [(10, 1), (20, 2)]
                    .into_iter()
                    .map(|(id, equipment_token)| rows::VariantDefinitionRow {
                        id,
                        creator_link: 5_000,
                        origin_link: 1,
                        equipment_token,
                        archetype_token: 9,
                        design: Recorded::Value(rows::VariantDesignState::default()),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                [
                    (1, "old_type"),
                    (2, "new_type"),
                    (3, "main"),
                    (4, "new_module"),
                    (5, "generic"),
                    (6, "extra"),
                    (7, "unused_module"),
                    (9, "root"),
                ]
                .into_iter()
                .map(|(token, name)| rows::LexicalTokenRow {
                    token,
                    literal: name.as_bytes().to_vec(),
                })
                .collect(),
            ));
        *frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(
                [(10, 0), (20, 1)]
                    .into_iter()
                    .map(|(variant, level)| rows::VariantUpgradeRow {
                        variant,
                        definition_token: 5,
                        level,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![
                rows::VariantModuleRow {
                    variant: 10,
                    slot: 6,
                    definition_token: 7,
                    definition_active: 1,
                    ..Default::default()
                },
                rows::VariantModuleRow {
                    variant: 20,
                    slot: 3,
                    definition_token: 4,
                    definition_active: 1,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(
                [(777, 9), (778, 4)]
                    .into_iter()
                    .map(|(index, token)| rows::ModifierDefinitionRow {
                        index,
                        token,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DesignCountryModifiers.index()] =
            TableView::Complete(TableRows::DesignCountryModifiers(
                [(0x1d7, 10_000), (0x1d9, 20_000), (777, 30_000), (778, 0)]
                    .into_iter()
                    .map(|(modifier, value)| rows::DesignCountryModifierRow {
                        country: 5_000,
                        modifier,
                        value: Some(value),
                        failure: 0,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DesignCountryTemplates.index()] =
            TableView::Complete(TableRows::DesignCountryTemplates(vec![
                rows::DesignCountryTemplatesRow {
                    country: 5_000,
                    uses: vec![0, 1],
                    factors: vec![25_000, 50_000],
                    equipment: vec![vec![1], vec![2]],
                    failure: 0,
                },
            ]));
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![
                rows::SimulationFeatureRow::default(),
            ]));
        // The organisation owner 5_000 resolves through CountryLinks to the country keying its roster.
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![rows::CountryRow {
                id: 5_000,
                canonical_slot: 50,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![rows::CountryLinkRow {
                id: 5_000,
                canonical_slot: 50,
            }]));
        (frame, content, defines)
    }

    #[test]
    fn organisation_modifier_getter_reads_only_own_present_key_and_applies_feature_and_clamp_per_source()
     {
        let (mut frame, _, _) = inputs();
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
                token: 10,
                literal: b"synthetic".to_vec(),
            }]));
        let parse = |text: &str| {
            installed_content_core::modifiers::values(
                &installed_content_core::parser::parse(text.as_bytes())
                    .unwrap()
                    .items,
            )
        };
        let nested = parse("added_modifier={synthetic=10}");
        assert_eq!(
            organisation_modifier_value(&frame, &nested, "synthetic", 9).unwrap(),
            0
        );
        let source = parse("synthetic=1.4 added_modifier={synthetic=10}");
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                rows::ModifierDefinitionRow {
                    index: 9,
                    token: 10,
                    flags: engine_native::modifier_definition::CLAMP_MAXIMUM,
                    ..Default::default()
                },
            ]));
        assert_eq!(
            organisation_modifier_value(&frame, &source, "synthetic", 9).unwrap(),
            100_000
        );
        assert_eq!(
            organisation_modifier_value(&frame, &parse("synthetic=-1"), "synthetic", 9).unwrap(),
            0
        );
        let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            unreachable!()
        };
        rows[0].feature_selector = 2;
        assert_eq!(
            organisation_modifier_value(&frame, &source, "synthetic", 9).unwrap(),
            0,
            "null callback does not reach feature state"
        );
        let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            unreachable!()
        };
        rows[0].eligibility = 1;
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![
                rows::SimulationFeatureRow {
                    dlc_mask: 2,
                    ..Default::default()
                },
            ]));
        assert_eq!(
            organisation_modifier_value(&frame, &source, "synthetic", 9).unwrap(),
            100_000
        );
    }

    #[test]
    fn selected_pair_save_xp_orders_licensing_source_discount_slot_family_and_target_discount() {
        let (mut frame, content, defines) = inputs();
        let unused =
            |_| -> Result<i64, String> { panic!("unchanged organisation must not be evaluated") };
        let absent_defines = installed_content_core::defines::Defines::new().unwrap();
        assert_eq!(
            base_save_xp(
                &frame,
                20,
                20,
                &Database::default(),
                &absent_defines,
                unused
            )
            .unwrap(),
            0
        );
        assert_eq!(
            module_save_xp(
                &frame,
                20,
                20,
                &Database::default(),
                &absent_defines,
                unused
            )
            .unwrap(),
            0
        );
        assert_eq!(
            base_save_xp(&frame, 20, 10, &content, &defines, unused).unwrap(),
            1_800_000
        );
        // The source-only extra slot has a large removal cost but is not in
        // the target's declared slots and must never be visited.
        assert_eq!(
            module_save_xp(&frame, 20, 10, &content, &defines, unused).unwrap(),
            1_800_000
        );
        let TableView::Complete(TableRows::SimulationFeatures(features)) =
            &mut *frame.tables[TableId::SimulationFeatures.index()]
        else {
            unreachable!()
        };
        features[0].dlc_mask = 0x80;
        assert_eq!(
            base_save_xp(&frame, 20, 10, &content, &defines, unused).unwrap(),
            2_400_000
        );
        assert_eq!(
            module_save_xp(&frame, 20, 10, &content, &defines, unused).unwrap(),
            2_280_000
        );
        *frame.tables[TableId::DesignCountryTemplates.index()] = TableView::Unavailable {
            reason: "missing demanded discount owner".into(),
        };
        assert!(
            module_save_xp(&frame, 20, 10, &content, &defines, unused)
                .unwrap_err()
                .contains("missing demanded discount owner")
        );
    }

    #[test]
    fn save_xp_compares_frozen_bonus_arrays_and_uses_source_organisation_when_target_is_null() {
        let (mut frame, content, defines) = inputs();
        let TableView::Complete(TableRows::VariantDefinitions(designs)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        else {
            unreachable!()
        };
        let Recorded::Value(old) = &mut designs[0].design else {
            unreachable!()
        };
        old.design_team = u64::MAX;
        let called = Cell::new(None);
        assert_eq!(
            base_save_xp(&frame, 20, 10, &content, &defines, |reference| {
                called.set(Some(reference));
                Ok(3)
            })
            .unwrap(),
            2_025_000
        );
        assert_eq!(called.get(), Some(u64::MAX));
        assert!(
            base_save_xp(&frame, 20, 10, &content, &defines, |_| Err(
                "organisation owner factor missing".into()
            ))
            .unwrap_err()
            .contains("owner factor missing")
        );
        let TableView::Complete(TableRows::VariantDefinitions(designs)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        else {
            unreachable!()
        };
        let Recorded::Value(next) = &mut designs[1].design else {
            unreachable!()
        };
        next.design_team = u64::MAX;
        next.design_team_bonus
            .push(rows::VariantStatEntry { index: 1, value: 1 });
        called.set(None);
        base_save_xp(&frame, 20, 10, &content, &defines, |reference| {
            called.set(Some(reference));
            Ok(0)
        })
        .unwrap();
        assert_eq!(
            called.get(),
            Some(u64::MAX),
            "equal authors with changed frozen bonuses still incur organisation XP"
        );
    }
    #[test]
    fn organisation_save_cost_joins_organisation_gates_unlocked_initial_policy_country_and_integer_rounding()
     {
        use dashboard_text::triggers::{ContentFacts, World};
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let modifier = engine_executable::modifier_keys::name(0x269).unwrap();
        for (gate, expected) in [("no", 8), ("yes", 10)] {
            let root =
                std::env::temp_dir().join(format!("design_mio_cost_{}", uuid::Uuid::new_v4()));
            let write = |path: &str, text: &str| {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            };
            write(
                "common/military_industrial_organization/organizations/cost.txt",
                &format!(
                    "synthetic_org={{equipment_type={{armor}} visible={{always=yes}} available={{always={gate}}} initial_trait={{token=initial available={{always=no}} organization_modifier={{{modifier}=0.1}} production_bonus={{production_conversion_speed_factor=0.1}}}} trait={{token=unlocked limit_to_equipment_type={{armor}} organization_modifier={{{modifier}=0.4}} production_bonus={{production_conversion_speed_factor=0.2}}}}}}"
                ),
            );
            write(
                "common/military_industrial_organization/policies/cost.txt",
                &format!(
                    "synthetic_policy={{organization_modifier={{{modifier}=0.2}} production_bonus={{armor={{production_conversion_speed_factor=0.3}}}}}}"
                ),
            );
            let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
            let content = OrganisationContent::load(&layers).unwrap();
            let (mut frame, equipment, _) = inputs();
            let TableView::Complete(TableRows::LexicalTokens(tokens)) =
                &mut *frame.tables[TableId::LexicalTokens.index()]
            else {
                unreachable!()
            };
            for (token, name) in [
                (100, "synthetic_org"),
                (200, "unlocked"),
                (300, "synthetic_policy"),
                (400, modifier),
            ] {
                tokens.push(rows::LexicalTokenRow {
                    token,
                    literal: name.as_bytes().to_vec(),
                });
            }
            *frame.tables[TableId::ModifierDefinitions.index()] =
                TableView::Complete(TableRows::ModifierDefinitions(vec![
                    rows::ModifierDefinitionRow {
                        index: 0x269,
                        token: 400,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::DesignOrganisationLiveness.index()] =
                TableView::Complete(TableRows::DesignOrganisationLiveness(vec![
                    rows::DesignOrganisationLivenessRow {
                        organisation: u64::MAX,
                        live: true,
                    },
                ]));
            *frame.tables[TableId::IndustrialOrganisations.index()] =
                TableView::Complete(TableRows::IndustrialOrganisations(vec![
                    rows::IndustrialOrganisationRow {
                        organisation: u64::MAX,
                        country: 4_999,
                        owner: 5_000,
                        token: 100,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::IndustrialOrganisationDetails.index()] =
                TableView::Complete(TableRows::IndustrialOrganisationDetails(vec![
                    rows::IndustrialOrganisationDetailsRow {
                        organisation: u64::MAX,
                        country: 4_999,
                        unlocked: vec![200],
                        state: rows::IndustrialOrganisationState {
                            design_team_change_cost: 5,
                            policy: 300,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::IndustrialOrganisationPolicies.index()] =
                TableView::Complete(TableRows::IndustrialOrganisationPolicies(vec![
                    rows::IndustrialOrganisationPoliciesRow {
                        country: 5_000,
                        policies: vec![rows::IndustrialOrganisationPolicy {
                            token: 300,
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::DesignCountryModifiers.index()] =
                TableView::Complete(TableRows::DesignCountryModifiers(vec![
                    rows::DesignCountryModifierRow {
                        country: 5_000,
                        modifier: 0x269,
                        value: Some(30_000),
                        failure: 0,
                    },
                ]));
            let dlcs = Default::default();
            let triggers = Default::default();
            let aliases = Default::default();
            let world = World::read(
                &frame,
                ContentFacts::content_only(&dlcs, &triggers, &aliases),
            );
            assert_eq!(
                organisation_change_xp(
                    &frame,
                    u64::MAX,
                    &content.traits,
                    &content.definitions.policies,
                    &world
                )
                .unwrap(),
                expected
            );
            assert_eq!(
                organisation_conversion_bonus(&frame, u64::MAX, 20, &content, &equipment, &world)
                    .unwrap(),
                if gate == "yes" { 60_000 } else { 40_000 }
            );
            write(
                "common/military_industrial_organization/policies/cost.txt",
                "synthetic_policy={production_bonus={same_as_mio={production_conversion_speed_factor=0.3}}}",
            );
            let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
            let inherited = OrganisationContent::load(&layers).unwrap();
            assert_eq!(
                organisation_conversion_bonus(&frame, u64::MAX, 20, &inherited, &equipment, &world)
                    .unwrap(),
                if gate == "yes" { 60_000 } else { 40_000 }
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    /// Owner-country roster, owner-scoped composed conditions and partly read groups.
    #[test]
    fn organisation_conversion_bonus_reads_the_owner_country_scope_and_known_groups() {
        use dashboard_text::triggers::{ContentFacts, World};
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root =
            std::env::temp_dir().join(format!("design_mio_conversion_{}", uuid::Uuid::new_v4()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "common/military_industrial_organization/policies/cost.txt",
            "synthetic_policy={production_bonus={armor={production_conversion_speed_factor=0.3}}}",
        );
        let (mut frame, equipment, _) = inputs();
        let TableView::Complete(TableRows::LexicalTokens(tokens)) =
            &mut *frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        for (token, name) in [
            (100, "synthetic_org"),
            (200, "unlocked"),
            (300, "synthetic_policy"),
            (500, "synthetic_flag"),
        ] {
            tokens.push(rows::LexicalTokenRow {
                token,
                literal: name.as_bytes().to_vec(),
            });
        }
        // Raw owner 5_000 aliases country 6_000; a roster keyed by the alias is not the owner's.
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![rows::CountryRow {
                id: 6_000,
                canonical_slot: 50,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            [5_000, 6_000]
                .into_iter()
                .map(|id| rows::CountryLinkRow {
                    id,
                    canonical_slot: 50,
                })
                .collect(),
        ));
        *frame.tables[TableId::IndustrialOrganisationPolicies.index()] =
            TableView::Complete(TableRows::IndustrialOrganisationPolicies(vec![
                rows::IndustrialOrganisationPoliciesRow {
                    country: 5_000,
                    ..Default::default()
                },
                rows::IndustrialOrganisationPoliciesRow {
                    country: 6_000,
                    policies: vec![rows::IndustrialOrganisationPolicy {
                        token: 300,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::DesignOrganisationLiveness.index()] =
            TableView::Complete(TableRows::DesignOrganisationLiveness(vec![
                rows::DesignOrganisationLivenessRow {
                    organisation: 7,
                    live: true,
                },
            ]));
        *frame.tables[TableId::IndustrialOrganisations.index()] =
            TableView::Complete(TableRows::IndustrialOrganisations(vec![
                rows::IndustrialOrganisationRow {
                    organisation: 7,
                    country: 4_999,
                    owner: 5_000,
                    token: 100,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::IndustrialOrganisationDetails.index()] =
            TableView::Complete(TableRows::IndustrialOrganisationDetails(vec![
                rows::IndustrialOrganisationDetailsRow {
                    organisation: 7,
                    country: 4_999,
                    unlocked: vec![200],
                    state: rows::IndustrialOrganisationState {
                        policy: 300,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ]));
        let dlcs = Default::default();
        let triggers = Default::default();
        let aliases = Default::default();
        let world = World::read(
            &frame,
            ContentFacts::content_only(&dlcs, &triggers, &aliases),
        );
        let bonus = |visible: &str, available: &str, lost: Option<&str>| {
            write(
                "common/military_industrial_organization/organizations/cost.txt",
                &format!(
                    "synthetic_org={{equipment_type={{armor}} visible={{{visible}}} available={{always={available}}} initial_trait={{token=initial production_bonus={{production_conversion_speed_factor=0.1}}}} trait={{token=unlocked limit_to_equipment_type={{armor}} production_bonus={{production_conversion_speed_factor=0.2}}}}}}"
                ),
            );
            let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
            let mut content = OrganisationContent::load(&layers).unwrap();
            content.groups.lost = lost.map(str::to_owned);
            organisation_conversion_bonus(&frame, 7, 20, &content, &equipment, &world)
        };
        // FROM is the owner; initial, completed trait and the owner's attached policy.
        assert_eq!(bonus("FROM={always=yes}", "yes", None), Ok(60_000));
        // Unrecorded owner flags leave `visible` undecided; only a false `available` decides it.
        let undecided = "FROM={has_country_flag=synthetic_flag}";
        assert_eq!(bonus(undecided, "no", None), Ok(40_000));
        assert!(bonus(undecided, "yes", None).is_err());
        // Past an unfollowed group reading, membership is unknown, never a mismatch.
        let refused = bonus("always=yes", "yes", Some("groups.txt line 1: unsettled")).unwrap_err();
        assert!(refused.contains("unread past groups.txt"), "{refused}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Fixed results of the organisation change cost across clamp, feature and
    /// rounding edges: the shared organisation modifier sum must not move them.
    #[test]
    fn organisation_change_cost_keeps_its_clamped_ineligible_and_negative_results() {
        use dashboard_text::triggers::{ContentFacts, World};
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let modifier = engine_executable::modifier_keys::name(0x269).unwrap();
        let root = std::env::temp_dir().join(format!("design_mio_edges_{}", uuid::Uuid::new_v4()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        // The completed trait counts only while the organisation is visible; a
        // visible body that reads FROM reads the owner, as the game's context links it.
        for visible in ["always=yes", "FROM={always=yes}"] {
        write(
            "common/military_industrial_organization/organizations/cost.txt",
            &format!(
                "synthetic_org={{visible={{{visible}}} available={{always=yes}} initial_trait={{token=initial organization_modifier={{{modifier}=0.3}}}} trait={{token=unlocked organization_modifier={{{modifier}=-0.9}}}}}}"
            ),
        );
        write(
            "common/military_industrial_organization/policies/cost.txt",
            &format!("synthetic_policy={{organization_modifier={{{modifier}=0.25}}}}"),
        );
        let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let content = OrganisationContent::load(&layers).unwrap();
        // (definition flags, feature selector, country value, base cost, cost)
        for (flags, selector, country, base, expected) in [
            (0, 0, -200_000, 3, -4),
            (
                engine_native::modifier_definition::CLAMP_MAXIMUM,
                0,
                -200_000,
                3,
                -1,
            ),
            (0, 4, -200_000, 3, -3),
            (0, 0, -85_000, 7, -1),
            (
                engine_native::modifier_definition::CLAMP_MAXIMUM,
                0,
                10_000,
                9,
                15,
            ),
        ] {
            let (mut frame, _, _) = inputs();
            let TableView::Complete(TableRows::LexicalTokens(tokens)) =
                &mut *frame.tables[TableId::LexicalTokens.index()]
            else {
                unreachable!()
            };
            for (token, name) in [
                (100, "synthetic_org"),
                (200, "unlocked"),
                (300, "synthetic_policy"),
                (400, modifier),
            ] {
                tokens.push(rows::LexicalTokenRow {
                    token,
                    literal: name.as_bytes().to_vec(),
                });
            }
            *frame.tables[TableId::ModifierDefinitions.index()] =
                TableView::Complete(TableRows::ModifierDefinitions(vec![
                    rows::ModifierDefinitionRow {
                        index: 0x269,
                        token: 400,
                        flags,
                        feature_selector: selector,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::DesignOrganisationLiveness.index()] =
                TableView::Complete(TableRows::DesignOrganisationLiveness(vec![
                    rows::DesignOrganisationLivenessRow {
                        organisation: 7,
                        live: true,
                    },
                ]));
            *frame.tables[TableId::IndustrialOrganisations.index()] =
                TableView::Complete(TableRows::IndustrialOrganisations(vec![
                    rows::IndustrialOrganisationRow {
                        organisation: 7,
                        country: 4_999,
                        owner: 5_000,
                        token: 100,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::IndustrialOrganisationDetails.index()] =
                TableView::Complete(TableRows::IndustrialOrganisationDetails(vec![
                    rows::IndustrialOrganisationDetailsRow {
                        organisation: 7,
                        country: 4_999,
                        unlocked: vec![200],
                        state: rows::IndustrialOrganisationState {
                            design_team_change_cost: base,
                            policy: 300,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::IndustrialOrganisationPolicies.index()] =
                TableView::Complete(TableRows::IndustrialOrganisationPolicies(vec![
                    rows::IndustrialOrganisationPoliciesRow {
                        country: 5_000,
                        policies: vec![rows::IndustrialOrganisationPolicy {
                            token: 300,
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::DesignCountryModifiers.index()] =
                TableView::Complete(TableRows::DesignCountryModifiers(vec![
                    rows::DesignCountryModifierRow {
                        country: 5_000,
                        modifier: 0x269,
                        value: Some(country),
                        failure: 0,
                    },
                ]));
            let dlcs = Default::default();
            let triggers = Default::default();
            let aliases = Default::default();
            let world = World::read(
                &frame,
                ContentFacts::content_only(&dlcs, &triggers, &aliases),
            );
            assert_eq!(
                organisation_change_xp(
                    &frame,
                    7,
                    &content.traits,
                    &content.definitions.policies,
                    &world
                ),
                Ok(expected),
                "{visible} {flags} {selector} {country} {base}"
            );
        }
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
