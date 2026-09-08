use super::*;
use snapshot::design_catalog::{DesignSlotIdentityObservation, DesignTextObservation};

fn observed_token<'a>(value: &'a DesignTextObservation, field: &str) -> Result<&'a str> {
    match value {
        DesignTextObservation::Measured { value, .. } => Ok(value),
        DesignTextObservation::Unavailable { reason } =>
            Err(format!("XMAP13 {field} is unavailable: {reason:?}").into()),
    }
}

fn designated_slot_id(equipment: &AuthoredEquipmentCatalog,
    variant: &NativeDesignerVariantInputs, archetype: &str, designated: &str,
) -> Result<u32> {
    let DesignSlotIdentityObservation::Measured { archetype_token_raw: _, archetype_token, slots }
        = &variant.slot_identities else {
        return Err(format!("XMAP13 designated module-slot identity is unavailable (copied slot observation): {:?}", variant.slot_identities).into());
    };
    // CAP owns the native variant -> archetype pointer/read-set association. An equipment
    // definition token ID is not the archetype token ID and must not be substituted here.
    if !observed_token(archetype_token, "slot archetype token")?.eq_ignore_ascii_case(archetype) {
        return Err("XMAP13 copied slot archetype differs from the admitted archetype".into());
    }
    let authored = equipment.designer_module_slots(archetype)?;
    if authored.len() != slots.len() {
        return Err("XMAP13 copied slot count differs from the admitted copied slots".into());
    }
    let mut selected = None;
    for (ordinal, (name, row)) in authored.iter().zip(slots).enumerate() {
        if usize::try_from(row.source_ordinal)? != ordinal
            || !observed_token(&row.token, "copied slot token")?.eq_ignore_ascii_case(name) {
            return Err("XMAP13 copied slot order or name differs from admitted content".into());
        }
        if slots[..ordinal].iter().any(|previous| previous.token_handle_raw == row.token_handle_raw) {
            return Err("XMAP13 distinct copied slot names share a runtime token identity".into());
        }
        if name.eq_ignore_ascii_case(designated) { selected = Some(row.token_handle_raw); }
    }
    selected.ok_or_else(|| "XMAP13 designated slot is absent from the complete copied identity vector".into())
}

fn assignment_handle(row: &snapshot::design_catalog::DesignModule) -> Result<i32> {
    // The existing portable projection spells the producer's raw u32 as handle:<decimal>.
    // No lexical hash, authored ordinal, or sorting can replace that runtime identity.
    let text = row.slot_id.strip_prefix("handle:")
        .ok_or("XMAP13 assignment lacks its native slot handle")?;
    let value = text.parse::<u32>().map_err(|_| "XMAP13 assignment slot handle is not a raw u32")?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0')) {
        return Err("XMAP13 assignment slot handle is not the canonical portable projection".into());
    }
    Ok(value as i32)
}

fn native_selected_assignment(rows: &[snapshot::design_catalog::DesignModule], selected: u32)
    -> Result<Option<usize>> {
    // 140bc45a1..45d5: preserve the native physical sequence, signed CMP/CMOVL,
    // half-count loop, and final <= test, including on an unsorted observed sequence.
    let mut first = 0usize;
    let mut count = rows.len();
    while count != 0 {
        let half = count >> 1;
        if assignment_handle(&rows[first + half])? < selected as i32 {
            first += (count & 1) + half;
        }
        count = half;
    }
    if first != rows.len() && assignment_handle(&rows[first])? <= selected as i32 {
        Ok(Some(first))
    } else { Ok(None) }
}

pub(super) fn variant_category(equipment: &AuthoredEquipmentCatalog,
    variant: &NativeDesignerVariantInputs,
) -> Result<u64> {
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
    if !variant.record.modules.is_empty() {
        let mut slot_id = 0x165;
        let mut selected = None;
        if let Some(name) = equipment.category_override_slot_name(&archetype.token)? {
            slot_id = designated_slot_id(equipment, variant, &archetype.token, &name)?;
            // 140c86220 returns null before the slot walk when the designation is 0x165.
            if slot_id != 0x165 {
                selected = native_selected_assignment(&variant.record.modules, slot_id)?;
                if let Some(index) = selected {
                    let module = equipment.module(&variant.record.modules[index].module_token)?;
                    excluded |= !module.additional_type_mask()?;
                }
            }
        }
        // The native getter's sentinel-null result selects the complete all-row arm.
        if slot_id == 0x165 {
            for assignment in &variant.record.modules {
                let module = equipment.module(&assignment.module_token)?;
                included |= module.additional_type_mask()?;
                excluded |= module.forbidden_type_mask()?;
            }
        } else if let Some(index) = selected {
            let module = equipment.module(&variant.record.modules[index].module_token)?;
            included |= module.additional_type_mask()?;
            excluded |= module.forbidden_type_mask()?;
        }
    }
    Ok(crate::equipment_model_law::normalize_equipment_category(included & !excluded))
}

#[cfg(test)]
mod slot_identity_tests {
    use super::*;
    fn row(id: u32) -> snapshot::design_catalog::DesignModule {
        snapshot::design_catalog::DesignModule {
            slot_id: format!("handle:{id}"), module_token: "unread".into(), source_record_id: "test".into(),
        }
    }
    #[test]
    fn signed_physical_search_keeps_native_duplicate_and_unsorted_results() {
        let rows = [row(u32::MAX), row(3), row(3), row(9)];
        assert_eq!(native_selected_assignment(&rows, u32::MAX).unwrap(), Some(0));
        assert_eq!(native_selected_assignment(&rows, 3).unwrap(), Some(1));
        assert_eq!(native_selected_assignment(&rows, 4).unwrap(), None);
        assert_eq!(native_selected_assignment(&rows, 10).unwrap(), None);
        // No source-order repair: sorting this sequence would return a different row.
        assert_eq!(native_selected_assignment(&[row(9), row(3)], 3).unwrap(), None);
        assert_eq!(native_selected_assignment(&[], 3).unwrap(), None);
    }

    #[test]
    fn designated_slot_uses_copied_runtime_identity_and_preserves_upstream_sentinel_null() {
        use crate::test_support::{temp_test_dir, write_modvfs_fixture, admitted_fixture_read_set};
        use snapshot::design_catalog::{DesignSlotIdentity, DesignFieldRefusal, FieldCompleteness};
        use super::super::tests::variant;
        let root = temp_test_dir("xmap13-designated-slot-identity");
        write_modvfs_fixture(&root, "common/units/equipment/test.txt", br#"
            equipments = {
                base = { is_archetype = yes type = convoy type_override = main
                    module_slots = { main = {} other = {} } }
                child = { archetype = base }
            }
            equipment_modules = {
                armor = { add_equipment_type = armor }
                artillery = { add_equipment_type = artillery }
            }
        "#);
        let raw = crate::GameFileResolver::vanilla(&root).unwrap();
        let scope = portable_content_manifest::ContentNamespaceScope {
            directory: "common/units/equipment".into(), extension: Some("txt".into()), recursive: true,
        };
        let (resolver, admission) = admitted_fixture_read_set(&raw, &Default::default(), &[scope], "xmap13-designated-slot-identity");
        let equipment = AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
        let text = |value: &str| DesignTextObservation::Measured {
            value: value.into(), utf8_hex: value.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect(),
        };
        let identities = |main, other| DesignSlotIdentityObservation::Measured {
            archetype_token_raw: 901, archetype_token: text("base"), slots: vec![
                DesignSlotIdentity { source_ordinal: 0, token_handle_raw: main, token: text("main") },
                DesignSlotIdentity { source_ordinal: 1, token_handle_raw: other, token: text("other") },
            ],
        };
        let assignment = |id, module: &str| {
            let mut value = row(id); value.module_token = module.into(); value
        };
        let mut input = variant(1, "child", "GER", 0);
        input.record.module_completeness = FieldCompleteness::Complete;
        input.slot_identities = identities(91, 92);
        input.record.modules = vec![assignment(91, "armor"), assignment(92, "artillery")];
        let normalized = crate::equipment_model_law::normalize_equipment_category;
        let armor = equipment.module("armor").unwrap().additional_type_mask().unwrap();
        let artillery = equipment.module("artillery").unwrap().additional_type_mask().unwrap();
        let convoy = equipment.definition("base").unwrap().type_mask().unwrap();
        assert_eq!(variant_category(&equipment, &input).unwrap(), normalized(armor));
        input.slot_identities = identities(92, 91);
        assert_eq!(variant_category(&equipment, &input).unwrap(), normalized(artillery));
        // A witnessed unassigned designated slot retains the base without loading other modules.
        input.slot_identities = identities(93, 91);
        input.record.modules = vec![assignment(91, "missing_unselected_module")];
        assert_eq!(variant_category(&equipment, &input).unwrap(), normalized(convoy));
        // The real designation 0x165 makes 140c86220 return null, even when assigned.
        input.slot_identities = identities(0x165, 0x166);
        input.record.modules = vec![assignment(0x165, "armor"), assignment(0x166, "artillery")];
        assert_eq!(variant_category(&equipment, &input).unwrap(), normalized(convoy | armor | artillery));
        input.slot_identities = identities(91, 91);
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("runtime token identity"));
        input.slot_identities = identities(91, 92);
        if let DesignSlotIdentityObservation::Measured { slots, .. } = &mut input.slot_identities {
            slots.swap(0, 1);
        }
        assert!(variant_category(&equipment, &input).unwrap_err().to_string().contains("order or name"));
        input.slot_identities = DesignSlotIdentityObservation::Unavailable {
            reason: DesignFieldRefusal { kind: "unreadable".into(), field: "slot_identities".into() },
        };
        assert!(variant_category(&equipment, &input).is_err());
        input.record.modules.clear();
        assert_eq!(variant_category(&equipment, &input).unwrap(), normalized(convoy));
        drop((equipment, resolver, admission, raw));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }
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
