use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EquipmentMissionMasks {
    pub allowed: u32,
    pub forbidden: u32,
}

impl AuthoredEquipmentCatalog {
    pub fn designer_mission_masks(&self, token: &str) -> EquipmentResult<EquipmentMissionMasks> {
        self.require_resolved_generation(token)?;
        let mut masks = EquipmentMissionMasks::default();
        let mut visiting = Vec::new();
        let mut current = Some(token.to_owned());
        while let Some(token) = current {
            if visiting.contains(&token) {
                visiting.push(token);
                return Err(EquipmentDefinitionError::RelationshipCycle(visiting));
            }
            visiting.push(token.clone());
            let definition = self.definition(&token)?;
            masks.allowed |= definition.mission_field("allow_mission_type")?;
            masks.forbidden |= definition.mission_field("forbid_mission_type")?;
            current = if let Some(parent) = definition.scalar("parent")?.filter(|parent| *parent != token) {
                Some(parent.to_owned())
            } else {
                self.resolve_archetype(&token, &mut Vec::new())?.filter(|row| row.token != token).map(|row| row.token)
            };
        }
        Ok(masks)
    }

    pub fn module_mission_mask(&self, token: &str) -> EquipmentResult<u32> {
        self.require_resolved_generation(token)?;
        self.module(token)?.mission_field("allow_mission_type")
    }
}

impl AuthoredEquipmentDefinition {
    fn mission_field(&self, field: &str) -> EquipmentResult<u32> {
        let read = |token: &str| mission_mask(token).ok_or_else(|| EquipmentDefinitionError::UnsupportedAuthored {
            token: self.token.clone(), field: format!("{field}.unknown_mission.{token}"),
        });
        let mut mask = 0;
        for value in self.expanded.values_ci(field) {
            if let Some(token) = value.as_scalar() {
                mask |= read(token)?;
            } else {
                for entry in value.as_object().ok_or_else(|| self.malformed(field))? {
                    if entry.key.is_some() { return Err(self.malformed(field)); }
                    mask |= read(entry.value.as_scalar().ok_or_else(|| self.malformed(field))?)?;
                }
            }
        }
        Ok(mask)
    }
}

fn mission_mask(token: &str) -> Option<u32> {
    Some(match token {
        "air_superiority" => 0x1,
        "cas" => 0x2,
        "interception" => 0x4,
        "strategic_bomber" => 0x8,
        "naval_bomber" => 0x10,
        "drop_nuke" => 0x20,
        "paradrop" => 0x40,
        "naval_kamikaze" => 0x80,
        "port_strike" => 0x100,
        "attack_logistics" => 0x200,
        "air_supply" => 0x400,
        "training" => 0x800,
        "naval_mines_planting" => 0x1000,
        "naval_mines_sweeping" => 0x2000,
        "recon" => 0x4000,
        "naval_patrol" => 0x8000,
        "barrage_mission" => 0x10000,
        "sam_mission" => 0x20000,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::catalog;

    #[test]
    fn mission_fields_or_repeated_scalars_lists_and_parent_then_archetype_chain() {
        let c = catalog(r#"equipment_modules = { engine = { allow_mission_type = training allow_mission_type = { recon cas } } }
            equipments = {
                base = { is_archetype = yes allow_mission_type = air_superiority forbid_mission_type = naval_bomber }
                parent = { archetype = base allow_mission_type = { cas training } }
                child = { parent = parent archetype = base allow_mission_type = naval_bomber allow_mission_type = {} forbid_mission_type = cas }
                zero = { is_archetype = yes }
            }"#).resolve_generated().unwrap();
        assert_eq!(c.designer_mission_masks("child").unwrap(), EquipmentMissionMasks { allowed:0x813, forbidden:0x12 });
        assert_eq!(c.module_mission_mask("engine").unwrap(),0x4802);
        assert_eq!(c.designer_mission_masks("zero").unwrap(),EquipmentMissionMasks { allowed:0, forbidden:0 });
    }

    #[test]
    fn generated_missions_copy_original_fields_then_apply_remapped_final_hierarchy() {
        let c = catalog(r#"equipments = {
            base = { is_archetype = yes allow_mission_type = cas }
            base_1 = { archetype = base parent = base allow_mission_type = training }
        }
        duplicate_archetypes = { derived = { archetype = base forbid_mission_type = cas } }
        equipments = { base = { is_archetype = yes allow_mission_type = recon } }
        "#).resolve_generated().unwrap();
        assert_eq!(c.designer_mission_masks("derived_1").unwrap(),EquipmentMissionMasks { allowed:0x802, forbidden:2 });
        assert_eq!(c.designer_mission_masks("base_1").unwrap(),EquipmentMissionMasks { allowed:0x4800, forbidden:0 });
    }

    #[test]
    fn unknown_missions_missing_parents_cycles_and_unresolved_generation_are_named() {
        let c = catalog("equipments = { base = { is_archetype = yes allow_mission_type = invented } }").resolve_generated().unwrap();
        assert!(matches!(c.designer_mission_masks("base"),Err(EquipmentDefinitionError::UnsupportedAuthored {field,..}) if field=="allow_mission_type.unknown_mission.invented"));
        let c = catalog("equipments = { base = { parent = missing } }");
        assert!(matches!(c.designer_mission_masks("base"),Err(EquipmentDefinitionError::MissingDefinition(_))));
        let c = catalog("equipments = { a = { parent = b } b = { parent = a } }");
        assert!(matches!(c.designer_mission_masks("a"),Err(EquipmentDefinitionError::RelationshipCycle(_))));
        let c = catalog("equipments = { base = { is_archetype = yes } } duplicate_archetypes = { new = { archetype = base } }");
        assert!(matches!(c.designer_mission_masks("base"),Err(EquipmentDefinitionError::UnresolvedGeneration {..})));
    }
}
