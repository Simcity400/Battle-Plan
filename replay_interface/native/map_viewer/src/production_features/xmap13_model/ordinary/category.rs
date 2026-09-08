use super::*;

pub(super) fn variant_category(equipment: &AuthoredEquipmentCatalog, variant: &NativeDesignerVariantInputs) -> Result<u64> {
    if variant.record.module_completeness != snapshot::design_catalog::FieldCompleteness::Complete {
        return Err("XMAP13 variant model category requires the complete actual module census".into());
    }
    let definition = equipment.definition(&variant.equipment_definition_token)?;
    let archetype = equipment.archetype_identity(&variant.equipment_definition_token)?
        .ok_or("XMAP13 variant model category requires its actual archetype")?;
    let archetype = equipment.definition(&archetype.token)?;
    let own = definition.type_mask()?;
    let mut included = if own == 0 { archetype.type_mask()? } else { own };
    let allowed = archetype.allowed_type_mask()?;
    let mut excluded = if allowed == 0 { 0 } else { !allowed };
    // Empty assignments contribute neither arm, even when an override slot is designated.
    if !variant.record.modules.is_empty() {
        if equipment.category_override_slot_name(&archetype.token)?.is_some() {
            return Err("XMAP13 variant model category requires the designated module-slot identity join".into());
        }
        // 140bc4530's sentinel arm visits every actual assignment. Authored defaults are not inputs.
        for assignment in &variant.record.modules {
            let module = equipment.module(&assignment.module_token)?;
            included |= module.additional_type_mask()?;
            excluded |= module.forbidden_type_mask()?;
        }
    }
    Ok(crate::equipment_model_law::normalize_equipment_category(included & !excluded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::variant;

    #[test]
    fn actual_nonempty_modules_combine_inclusions_and_exclusions_before_normalization() {
        use snapshot::design_catalog::{FieldCompleteness, DesignModule};
        use crate::test_support::{temp_test_dir, write_modvfs_fixture, admitted_fixture_read_set};
        let root = temp_test_dir("xmap13-nonempty-module-category");
        write_modvfs_fixture(&root, "common/units/equipment/test.txt", br#"
            equipments = {
                base = { is_archetype = yes type = { convoy screen_ship }
                    default_modules = { ignored = missing }
                    allowed_types = { convoy screen_ship capital_ship carrier } }
                variant = { archetype = base type_override = unproved_child_slot }
                missing_override = { is_archetype = yes type = { convoy screen_ship }
                    type_override = absent module_slots = { other = {} } }
                designated = { is_archetype = yes type = convoy
                    type_override = main module_slots = { main = {} } }
            }
            equipment_modules = {
                capital = { add_equipment_type = capital_ship forbid_equipment_type = capital_ship
                    forbid_equipment_type = {} }
                excluded = { add_equipment_type = { carrier fighter }
                    forbid_equipment_type = convoy forbid_equipment_type = capital_ship }
                parent = { add_equipment_type = armor forbid_equipment_type = -1 }
                child = { parent = parent }
                malformed = { forbid_equipment_type = { fighter = yes } }
            }
        "#);
        let raw = crate::GameFileResolver::vanilla(&root).unwrap();
        let scope = portable_content_manifest::ContentNamespaceScope {
            directory: "common/units/equipment".into(), extension: Some("txt".into()), recursive: true,
        };
        let (resolver, admission) = admitted_fixture_read_set(&raw, &Default::default(), &[scope], "xmap13-nonempty-module-category");
        let equipment = AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
        let mut input = variant(1, "variant", "GER", 0);
        input.record.module_completeness = FieldCompleteness::Complete;
        let assignment = |id, token: &str| DesignModule {
            slot_id: format!("handle:{id}"), module_token: token.into(), source_record_id: "test-79".into(),
        };
        input.record.modules = vec![assignment(91, "capital"), assignment(92, "excluded"), assignment(93, "child")];
        assert_eq!(variant_category(&equipment, &input).unwrap(), 0x400100);
        input.record.modules.reverse();
        assert_eq!(variant_category(&equipment, &input).unwrap(), 0x400100);
        input.equipment_definition_token = "missing_override".into();
        assert_eq!(variant_category(&equipment, &input).unwrap(), 0x400500);
        input.record.modules = vec![assignment(91, "malformed")];
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("forbid_equipment_type"));
        input.record.modules = vec![assignment(91, "missing")];
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("MissingDefinition"));
        input.equipment_definition_token = "designated".into();
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("designated module-slot identity"));
        drop((equipment, resolver, admission, raw));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn actual_empty_module_census_uses_archetype_allowed_types_without_default_modules() {
        use snapshot::design_catalog::{FieldCompleteness, DesignModule};
        use crate::test_support::{temp_test_dir, write_modvfs_fixture, admitted_fixture_read_set};
        let root = temp_test_dir("xmap13-empty-module-category");
        write_modvfs_fixture(&root, "common/units/equipment/test.txt", br#"
            equipments = {
                base = { is_archetype = yes type = { convoy capital_ship carrier }
                    allowed_types = { convoy } allowed_types = { capital_ship carrier } }
                variant = { archetype = base type_override = missing_slot
                    allowed_types = {} default_modules = { pretend = unavailable_module } }
                cleared = { is_archetype = yes type = armor
                    allowed_types = artillery allowed_types = {} }
                own = { archetype = base type = armor }
                high = { is_archetype = yes type = armor allowed_types = 9223372036854775812
                    module_slots = { main = {} } type_override = main }
            }
        "#);
        let raw = crate::GameFileResolver::vanilla(&root).unwrap();
        let scope = portable_content_manifest::ContentNamespaceScope {
            directory: "common/units/equipment".into(), extension: Some("txt".into()), recursive: true,
        };
        let (resolver, admission) = admitted_fixture_read_set(&raw, &Default::default(), &[scope], "xmap13-empty-module-category");
        let equipment = AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
        let mut input = variant(1, "variant", "GER", 0);
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("complete actual module census"));
        input.record.module_completeness = FieldCompleteness::Complete;
        assert_eq!(variant_category(&equipment, &input).unwrap(), 0x400040);
        input.equipment_definition_token = "cleared".into();
        assert_eq!(variant_category(&equipment, &input).unwrap(), 4);
        input.equipment_definition_token = "own".into();
        assert_eq!(variant_category(&equipment, &input).unwrap(), 0);
        input.equipment_definition_token = "high".into();
        assert_eq!(variant_category(&equipment, &input).unwrap(), 4);
        input.record.modules.push(DesignModule {
            slot_id: "handle:123".into(), module_token: "unavailable_module".into(), source_record_id: "test-79".into(),
        });
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("designated module-slot identity"));
        drop((equipment, resolver, admission, raw));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }
}
