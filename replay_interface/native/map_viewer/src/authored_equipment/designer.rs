use super::*;

impl AuthoredEquipmentCatalog {
    pub fn designer_window_tokens(&self, token: &str) -> EquipmentResult<Vec<String>> {
        self.require_resolved_generation(token)?;
        let definition = self.definition(token)?;
        let mut tokens = vec![definition.token.clone()];
        if let Some(archetype) = self.resolve_archetype(token, &mut Vec::new())? {
            tokens.push(archetype.token);
        } else {
            return Err(EquipmentDefinitionError::MissingField { token: token.into(), field: "archetype".into() });
        }
        Ok(tokens)
    }

    pub fn designer_module_slots(&self, token: &str) -> EquipmentResult<Vec<String>> {
        self.require_resolved_generation(token)?;
        self.module_slots(token, &mut Vec::new())
    }

    pub(super) fn module_slots(&self, token: &str, visiting: &mut Vec<String>) -> EquipmentResult<Vec<String>> {
        if let Some(slots) = &self.definition(token)?.resolved_module_slots { return slots.clone() }
        if visiting.iter().any(|entry| entry == token) {
            let mut cycle = visiting.clone(); cycle.push(token.into());
            return Err(EquipmentDefinitionError::RelationshipCycle(cycle));
        }
        visiting.push(token.into());
        let definition = self.definition(token)?;
        let mut parent: Option<String> = None;
        let mut archetype: Option<String> = None;
        let mut slots = Vec::<String>::new();
        for entry in definition.expanded.fields() {
            if entry.key_matches("parent") {
                let target = entry.value.as_scalar().ok_or_else(|| definition.malformed("parent"))?;
                self.definition(target)?;
                parent = Some(target.into());
                if archetype.is_none() && target != token {
                    archetype = self.resolve_archetype(target, &mut Vec::new())?.map(|row| row.token);
                }
            } else if entry.key_matches("archetype") {
                let target = entry.value.as_scalar().ok_or_else(|| definition.malformed("archetype"))?;
                let target_definition = self.definition(target)?;
                if target_definition.boolean("is_archetype")? != Some(true) {
                    return Err(EquipmentDefinitionError::NotArchetype(target.into()));
                }
                archetype = Some(target.into());
            } else if entry.key_matches("is_archetype") {
                match entry.value.as_scalar() {
                    Some("yes") => archetype = Some(token.into()),
                    Some("no") => {},
                    _ => return Err(definition.malformed("is_archetype")),
                }
            } else if entry.key_matches("module_slots") {
                let inherited = parent.as_deref().filter(|name| *name != token)
                    .or_else(|| archetype.as_deref().filter(|name| *name != token));
                if let Some(value) = entry.value.as_scalar() {
                    if value == "parent" {
                        if let Some(inherited) = inherited { slots = self.module_slots(inherited, visiting)? }
                    }
                    // The scalar parser ignores every token other than parent.
                    continue;
                }
                for slot in entry.value.fields() {
                    let name = slot.key.as_deref().filter(|name| !name.is_empty())
                        .ok_or_else(|| definition.malformed("module_slots.slot_name"))?;
                    if slots.iter().any(|current| current.eq_ignore_ascii_case(name)) { continue }
                    let append = if slot.value.as_object().is_some() {
                        true
                    } else if slot.value.as_scalar() == Some("parent") {
                        match inherited {
                            Some(inherited) => self.module_slots(inherited, visiting)?.iter()
                                .any(|current| current.eq_ignore_ascii_case(name)),
                            None => false,
                        }
                    } else {
                        let source = slot.value.as_scalar().ok_or_else(|| definition.malformed("module_slots.slot"))?;
                        // 140c8c260 logs an unresolved source without appending a slot.
                        slots.iter().any(|current| current.eq_ignore_ascii_case(source))
                    };
                    if append { slots.push(name.into()) }
                }
            }
        }
        visiting.pop();
        Ok(slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::catalog;

    #[test]
    fn ordered_slot_inheritance_uses_already_assigned_parent_then_archetype() {
        let c = catalog(r#"equipments = {
            root = { is_archetype = yes type = armor module_slots = { main = {} secondary = {} } }
            empty = { is_archetype = yes type = armor }
            early = { module_slots = parent parent = root }
            inherited = { archetype = root module_slots = parent module_slots = { main = parent third = {} } }
            named = { parent = root module_slots = { main = parent missing = parent local = {} } }
            replaced = { parent = root module_slots = parent parent = empty module_slots = parent }
        }"#);
        assert_eq!(c.designer_module_slots("early").unwrap(), Vec::<String>::new());
        assert_eq!(c.designer_module_slots("inherited").unwrap(), ["main", "secondary", "third"]);
        assert_eq!(c.designer_module_slots("named").unwrap(), ["main", "local"]);
        assert!(c.designer_module_slots("replaced").unwrap().is_empty());
    }

    #[test]
    fn duplicate_slots_and_scalar_copy_preserve_order_and_skip_unresolved_sources() {
        let c = catalog(r#"equipments = {
            slots = { module_slots = { first = {} FIRST = absent second = first third = second } }
            unknown = { module_slots = { first = {} second = unregistered third = FIRST fourth = {} } }
            no_slots = { module_slots = { first = unregistered } }
        }"#);
        assert_eq!(c.designer_module_slots("slots").unwrap(), ["first", "second", "third"]);
        assert_eq!(c.designer_module_slots("unknown").unwrap(), ["first", "third", "fourth"]);
        assert!(c.designer_module_slots("no_slots").unwrap().is_empty());
    }

    #[test]
    fn copied_slot_lists_survive_later_parent_and_original_redefinitions() {
        let c = catalog(r#"equipments = {
            root = { is_archetype = yes type = armor module_slots = { original = {} } }
            root_child = { parent = root module_slots = parent }
        }
        duplicate_archetypes = { copied = { archetype = root type = armor } }
        equipments = { root = { is_archetype = yes type = armor module_slots = { replacement = {} } } }
        "#).resolve_generated().unwrap();
        assert_eq!(c.designer_module_slots("root").unwrap(), ["replacement"]);
        for token in ["root_child", "copied", "copied_child"] {
            assert_eq!(c.designer_module_slots(token).unwrap(), ["original"], "{token}");
        }
    }

    #[test]
    fn generated_slots_copy_resolved_original_before_parent_remapping_and_clear_explicitly() {
        let c = catalog(r#"equipments = {
            root = { is_archetype = yes type = armor module_slots = { main = {} } }
            root_child = { parent = root module_slots = parent }
        }
        duplicate_archetypes = { copied = { archetype = root type = armor } cleared = { archetype = root type = armor module_slots = none } }"#).resolve_generated().unwrap();
        assert_eq!(c.designer_module_slots("copied_child").unwrap(), ["main"]);
        assert!(c.designer_module_slots("cleared_child").unwrap().is_empty());
        assert!(matches!(catalog("equipments = { a = { parent = b module_slots = parent } b = { parent = a module_slots = parent } }").designer_module_slots("a"), Err(EquipmentDefinitionError::RelationshipCycle(_))));
    }
}
