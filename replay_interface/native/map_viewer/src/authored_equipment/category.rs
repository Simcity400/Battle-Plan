use super::*;

impl AuthoredEquipmentDefinition {
    /// Native module+88; each reached field resets the destination before reading its mask.
    pub(crate) fn forbidden_type_mask(&self) -> EquipmentResult<u64> {
        let mut mask = 0;
        for entry in self.expanded.fields().iter().filter(|entry| entry.key_matches("forbid_equipment_type")) {
            mask = self.category_value(&entry.value, "forbid_equipment_type", true)?;
        }
        Ok(mask)
    }
}

impl AuthoredEquipmentCatalog {
    /// None proves that 140c86220 finds no designated slot. A matching name still
    /// needs its runtime token binding to distinguish the sentinel and join an assignment.
    pub(crate) fn category_override_slot_name(&self, archetype: &str) -> EquipmentResult<Option<String>> {
        let definition = self.definition(archetype)?;
        let Some(name) = definition.scalar("type_override")? else { return Ok(None) };
        // 140c8c260 interns slot names through the same 1424a6500 path used by
        // 1424aba00 for type_override; reuse the catalog's resolved slot list.
        Ok(self.designer_module_slots(archetype)?.into_iter()
            .find(|slot| slot.eq_ignore_ascii_case(name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::catalog;

    #[test]
    fn module_exclusions_use_the_shared_mask_reader_without_parent_inheritance() {
        let c = catalog(r#"equipment_modules = {
            parent = { add_equipment_type = armor forbid_equipment_type = fighter }
            absent = { parent = parent }
            repeated = { forbid_equipment_type = armor forbid_equipment_type = { fighter 9223372036854775808 } }
            cleared = { forbid_equipment_type = fighter forbid_equipment_type = {} }
            signed = { forbid_equipment_type = -1 }
            malformed = { forbid_equipment_type = { fighter = yes } }
            unrelated = { forbid_equipment_type = { support } }
        }"#);
        assert_eq!(c.module("parent").unwrap().forbidden_type_mask().unwrap(), 0x400);
        let absent = c.module("absent").unwrap();
        assert_eq!(absent.additional_type_mask().unwrap(), 0);
        assert_eq!(absent.forbidden_type_mask().unwrap(), 0);
        assert_eq!(c.module("repeated").unwrap().forbidden_type_mask().unwrap(), (1u64 << 63) | 0x400);
        assert_eq!(c.module("cleared").unwrap().forbidden_type_mask().unwrap(), 0);
        assert_eq!(c.module("signed").unwrap().forbidden_type_mask().unwrap(), u64::MAX);
        assert!(matches!(c.module("malformed").unwrap().forbidden_type_mask(),
            Err(EquipmentDefinitionError::MalformedField { field, .. }) if field == "forbid_equipment_type"));
        assert!(matches!(c.module("unrelated").unwrap().forbidden_type_mask(),
            Err(EquipmentDefinitionError::UnknownCategory { .. })));
    }

    #[test]
    fn category_override_resolves_only_names_in_the_current_copied_slot_list() {
        let c = catalog(r#"equipments = {
            root = { is_archetype = yes module_slots = { main = {} other = {} }
                type_override = missing type_override = MAIN }
            absent = { is_archetype = yes module_slots = { main = {} } }
            missing = { is_archetype = yes type_override = unknown module_slots = { main = {} } }
            no_slots = { is_archetype = yes type_override = main }
            inherited = { parent = root module_slots = parent type_override = main }
        }
        duplicate_archetypes = { copied = { archetype = root } cleared = { archetype = root module_slots = none } }
        equipments = { root = { is_archetype = yes module_slots = { replacement = {} } type_override = main } }
        "#).resolve_generated().unwrap();
        for token in ["root", "absent", "missing", "no_slots", "cleared"] {
            assert_eq!(c.category_override_slot_name(token).unwrap(), None, "{token}");
        }
        for token in ["inherited", "copied"] {
            assert_eq!(c.category_override_slot_name(token).unwrap().as_deref(), Some("main"), "{token}");
        }
    }
}
