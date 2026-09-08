use super::*;

impl AuthoredEquipmentCatalog {
    pub fn graphics_source<'a>(&'a self, definition: &'a AuthoredEquipmentDefinition)
        -> EquipmentResult<&'a AuthoredEquipmentDefinition> {
        self.require_resolution_profile()?;
        if !self.definitions.values().flatten().any(|row| std::ptr::eq(row, definition)) {
            return Err(EquipmentDefinitionError::UnsupportedAuthored {
                token: definition.token.clone(), field: "graphics_source.detached_definition".into(),
            });
        }
        let token = definition.resolved_graphics_source.as_ref().ok_or_else(||
            EquipmentDefinitionError::UnsupportedAuthored {
                token: definition.token.clone(), field: "graphics_source.ordered_resolution".into(),
            })?.as_ref().map_err(Clone::clone)?;
        self.definition(token)
    }

    pub(super) fn model_sprite_before_insert(&self, definition: &AuthoredEquipmentDefinition)
        -> EquipmentResult<String> {
        let mut sprite = String::new();
        for field in definition.expanded.fields() {
            if field.key_matches("sprite") {
                sprite = field.value.as_scalar().ok_or_else(|| definition.malformed("sprite"))?.into();
            } else if field.key_matches("archetype") && sprite.is_empty() {
                let token = field.value.as_scalar().ok_or_else(|| definition.malformed("archetype"))?;
                if token == definition.token { continue; }
                match self.admitted_definition(&self.definitions, token) {
                    Ok(target) if target.boolean("is_archetype")? == Some(true) => {
                        sprite = target.model_sprite()?.into();
                    },
                    Ok(_) | Err(EquipmentDefinitionError::MissingDefinition(_)
                        | EquipmentDefinitionError::DisabledByContentCondition(_)) => {},
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(sprite)
    }

    pub(super) fn graphics_source_before_insert(&self, definition: &AuthoredEquipmentDefinition)
        -> EquipmentResult<String> {
        // 140c84830 starts +4e8 at self; 140c89cc0 retains it on an invalid family lookup.
        let mut target = definition.token.clone();
        for field in definition.expanded.fields().iter().filter(|field| field.key_matches("family")) {
            let token = field.value.as_scalar().ok_or_else(|| definition.malformed("family"))?;
            if token == definition.token {
                target = token.into();
            } else {
                match self.admitted_definition(&self.definitions, token) {
                    Ok(row) => target = row.token.clone(),
                    Err(EquipmentDefinitionError::MissingDefinition(_)
                        | EquipmentDefinitionError::DisabledByContentCondition(_)) => {},
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::catalog;

    #[test]
    fn sprite_inheritance_occurs_at_each_archetype_field_only_while_empty() {
        let catalog = catalog(r#"equipments = {
            root = { is_archetype = yes sprite = inherited }
            future = { is_archetype = yes sprite = future_sprite }
            before = { sprite = own archetype = root }
            copied = { sprite = "" archetype = root }
            cleared = { archetype = root sprite = "" }
            repeated = { archetype = root sprite = "" archetype = future }
            parent_only = { parent = root }
            root = { is_archetype = yes sprite = replaced }
        } duplicate_archetypes = { generated = { archetype = root sprite = generated_sprite } }"#)
            .resolve_generated().unwrap();
        for (token, expected) in [("before","own"),("copied","inherited"),
            ("cleared",""),("repeated","future_sprite"),("parent_only",""),("root","replaced"),
            ("generated","generated_sprite")] {
            assert_eq!(catalog.definition(token).unwrap().model_sprite().unwrap(), expected, "{token}");
        }
        // The sprite getter retains empty on a missing earlier archetype. Full
        // catalog admission independently requires the inherited stat inputs.
        let pending = super::super::tests::catalog("equipments = { late = { archetype = future } }");
        let prior = super::super::tests::catalog("equipments = {}").resolve_generated().unwrap();
        assert_eq!(prior.model_sprite_before_insert(&pending.definitions["late"][0]).unwrap(), "");
        assert!(matches!(pending.resolve_generated(),
            Err(EquipmentDefinitionError::MissingDefinition(token)) if token == "future"));
        let malformed = super::super::tests::catalog("equipments = { root = { sprite = {} } }")
            .resolve_generated().unwrap();
        assert!(matches!(malformed.definition("root").unwrap().model_sprite(),
            Err(EquipmentDefinitionError::MalformedField {field,..}) if field == "sprite"));
    }

    #[test]
    fn family_is_ordered_direct_object_selection_and_invalid_lookup_retains_previous() {
        let catalog = catalog("equipments = { a = {} b = { family = a } c = { family = b family = missing } late = { family = future } future = {} }")
            .resolve_generated().unwrap();
        let a = catalog.definition("a").unwrap();
        let b = catalog.definition("b").unwrap();
        let c = catalog.definition("c").unwrap();
        assert!(std::ptr::eq(catalog.graphics_source(a).unwrap(), a));
        assert!(std::ptr::eq(catalog.graphics_source(b).unwrap(), a));
        assert!(std::ptr::eq(catalog.graphics_source(c).unwrap(), b));
        let late = catalog.definition("late").unwrap();
        assert!(std::ptr::eq(catalog.graphics_source(late).unwrap(), late));
    }

    #[test]
    fn redefinition_resets_self_but_existing_family_reference_retains_target_object() {
        let catalog = catalog("equipments = { a = {} b = { family = a } a = { visual_level = 7 } b = {} }")
            .resolve_generated().unwrap();
        let b = catalog.definition("b").unwrap();
        assert!(std::ptr::eq(catalog.graphics_source(b).unwrap(), b));
        let catalog = super::super::tests::catalog("equipments = { a = {} b = { family = a } a = { visual_level = 7 } }")
            .resolve_generated().unwrap();
        let selected = catalog.graphics_source(catalog.definition("b").unwrap()).unwrap();
        assert!(std::ptr::eq(selected, catalog.definition("a").unwrap()));
        assert_eq!(selected.signed_integer("visual_level").unwrap(), Some(7));
    }

    #[test]
    fn generated_copy_points_to_original_object_not_original_family() {
        let catalog = catalog("equipments = { graphics = {} base = { is_archetype = yes type = armor family = graphics } } duplicate_archetypes = { copy = { archetype = base type = fighter } }")
            .resolve_generated().unwrap();
        let base = catalog.definition("base").unwrap();
        let copy = catalog.definition("copy").unwrap();
        assert!(std::ptr::eq(catalog.graphics_source(base).unwrap(), catalog.definition("graphics").unwrap()));
        assert!(std::ptr::eq(catalog.graphics_source(copy).unwrap(), base));
        assert_eq!(copy.scalar("family").unwrap(), Some("graphics"));
        assert!(!std::ptr::eq(catalog.graphics_source(copy).unwrap(), catalog.graphics_source(base).unwrap()));
    }

    #[test]
    fn unknown_resolution_and_detached_copy_are_refused_without_token_rejoin() {
        let unresolved = catalog("equipments = { a = {} }");
        assert!(unresolved.graphics_source(unresolved.definition("a").unwrap()).is_err());
        let resolved = unresolved.resolve_generated().unwrap();
        let copy = resolved.definition("a").unwrap().clone();
        assert!(matches!(resolved.graphics_source(&copy), Err(EquipmentDefinitionError::UnsupportedAuthored {field,..})
            if field == "graphics_source.detached_definition"));
    }
}
