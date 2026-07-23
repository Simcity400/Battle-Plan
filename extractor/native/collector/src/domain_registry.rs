//! Collector-side domain registry.
//!
//! Descriptor types, the assigned-domain table, and validation live in the
//! shared `compact_contract` crate; this module only keeps the
//! collector-specific runtime registry and lane-module trait.

pub use compact_contract::{
    assigned_descriptor_by_id, assigned_descriptor_by_name, descriptor_names, domain,
    validate_descriptor, CompactDomainId, CompactDomainStatus, DomainRegistryError,
    ASSIGNED_COMPACT_DOMAINS,
};

pub type CollectorDomainDescriptor = compact_contract::CompactDomainDescriptor;

pub const ASSIGNED_COLLECTOR_DOMAINS: &[CollectorDomainDescriptor] = ASSIGNED_COMPACT_DOMAINS;

pub trait CollectorDomainModule {
    fn descriptor(&self) -> CollectorDomainDescriptor;

    fn register(&self, registry: &mut CollectorDomainRegistry) -> Result<(), DomainRegistryError> {
        registry.register(self.descriptor())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CollectorDomainRegistry {
    domains: Vec<CollectorDomainDescriptor>,
}

impl CollectorDomainRegistry {
    pub fn new() -> Self {
        Self {
            domains: Vec::new(),
        }
    }

    pub fn with_builtin_modules() -> Result<Self, DomainRegistryError> {
        let mut registry = Self::new();
        for descriptor in ASSIGNED_COLLECTOR_DOMAINS
            .iter()
            .copied()
            .filter(|descriptor| descriptor.schedulable)
        {
            registry.register(descriptor)?;
        }
        Ok(registry)
    }

    pub fn register(
        &mut self,
        descriptor: CollectorDomainDescriptor,
    ) -> Result<(), DomainRegistryError> {
        validate_descriptor(descriptor)?;
        if let Some(existing) = self.domains.iter().find(|entry| entry.id == descriptor.id) {
            return Err(DomainRegistryError::DuplicateId {
                id: descriptor.id,
                existing_name: existing.canonical_name,
                new_name: descriptor.canonical_name,
            });
        }
        for name in descriptor_names(descriptor) {
            if let Some(existing) = self
                .domains
                .iter()
                .find(|entry| entry.matches_any_name(name))
            {
                return Err(DomainRegistryError::DuplicateName {
                    name,
                    existing_id: existing.id,
                    new_id: descriptor.id,
                });
            }
        }
        self.domains.push(descriptor);
        Ok(())
    }

    pub fn descriptors(&self) -> &[CollectorDomainDescriptor] {
        &self.domains
    }

    pub fn by_id(&self, id: CompactDomainId) -> Option<CollectorDomainDescriptor> {
        self.domains.iter().copied().find(|entry| entry.id == id)
    }

    pub fn id_for_scheduled_name(&self, name: &str) -> Option<CompactDomainId> {
        self.domains
            .iter()
            .find(|entry| entry.matches_scheduled_name(name))
            .map(|entry| entry.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestLaneModule;

    impl CollectorDomainModule for TestLaneModule {
        fn descriptor(&self) -> CollectorDomainDescriptor {
            // Deliberately outside the active production id sequence so this exercises
            // registering a brand-new lane on top of the builtins.
            domain(
                999,
                "test_lane_live",
                "test_lane_live",
                CompactDomainStatus::ActiveIntegration,
                true,
                true,
                &[],
            )
        }
    }

    #[test]
    fn builtins_keep_assigned_ids_and_aliases() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();

        assert_eq!(registry.id_for_scheduled_name("fuel_live"), Some(10));
        assert_eq!(registry.id_for_scheduled_name("state_control"), Some(22));
        assert_eq!(
            registry.id_for_scheduled_name("air_variant_archetypes"),
            Some(46)
        );
        assert_eq!(
            registry.id_for_scheduled_name("land_combat_detail_events_live"),
            Some(55)
        );
        assert_eq!(
            registry.id_for_scheduled_name("naval_missions_status_live"),
            Some(56)
        );
        assert_eq!(
            registry.id_for_scheduled_name("session_multiplayer_metadata_live"),
            Some(57)
        );
        assert_eq!(
            registry.id_for_scheduled_name("army_battleplans_depth_live"),
            Some(54)
        );
        assert_eq!(
            registry.id_for_scheduled_name("construction_buildings_state_damage_live"),
            Some(48)
        );
        assert_eq!(
            registry.id_for_scheduled_name("diplomacy_intel_operatives_live"),
            Some(53)
        );
        assert_eq!(
            registry.by_id(53).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
        assert_eq!(
            registry.id_for_scheduled_name("characters_leaders_cabinet_live"),
            Some(47)
        );
        assert_eq!(
            registry.id_for_scheduled_name("air_variant_archetypes_live"),
            None
        );
        assert_eq!(
            registry.by_id(47).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
        assert_eq!(
            registry.by_id(57).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedPartialSlice)
        );
        assert_eq!(
            registry.id_for_scheduled_name("hub_motorization_live"),
            Some(62)
        );
        assert_eq!(
            registry.by_id(62).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
        assert_eq!(
            assigned_descriptor_by_name("air_variant_archetypes_live").map(|d| d.id),
            Some(46)
        );
    }

    #[test]
    fn reserved_lane_ids_are_assigned_but_not_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        for id in [58, 59] {
            assert!(
                assigned_descriptor_by_id(id).is_some(),
                "domain {id} must be assigned in compact_contract"
            );
            assert!(
                registry.by_id(id).is_none(),
                "domain {id} must not schedule"
            );
        }
    }

    #[test]
    fn country_core_scalars_lane_schedules() {
        // W1: new per-country core-scalars lane (domain 63), distinct from the
        // reserved economy-counter badge lane (domain 58).
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("country_core_scalars_live"),
            Some(63)
        );
        assert_eq!(
            registry.by_id(63).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
    }

    #[test]
    fn a17_nuke_stockpile_transport_is_production_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            assigned_descriptor_by_name("nuke_stockpile_live").map(|d| d.id),
            Some(83)
        );
        assert_eq!(
            registry.id_for_scheduled_name("nuke_stockpile_live"),
            Some(83)
        );
        assert_eq!(
            registry.by_id(83).map(|descriptor| descriptor.status),
            Some(CompactDomainStatus::ProductionComplete)
        );
    }

    #[test]
    fn exact_fail_closed_runtime_producers_are_production_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        for (id, name) in [
            (70, "sunk_convoy_history_live"),
            (72, "resolved_tooltip_instances_live"),
            (75, "fired_event_popups_live"),
            (82, "character_presentation_live"),
            (85, "selected_designer_identity_live"),
            (86, "sunk_ship_history_live"),
            (88, "active_command_abilities_live"),
            (90, "convoy_transport_attack_live"),
            (84, "scripted_gui_runtime_live"),
        ] {
            assert_eq!(
                registry.id_for_scheduled_name(name),
                Some(id),
                "{name} must be admitted by the production registry"
            );
            assert_eq!(
                registry.by_id(id).map(|descriptor| descriptor.status),
                Some(if id == 84 {
                    CompactDomainStatus::ValidatedSlice
                } else {
                    CompactDomainStatus::ProductionComplete
                }),
                "{name} must retain its exact fail-closed production status"
            );
        }
    }

    #[test]
    fn a33_transport_is_scheduled_while_a34_remains_reserved() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            assigned_descriptor_by_name("sunk_ship_history_live").map(|d| d.id),
            Some(86)
        );
        assert_eq!(
            assigned_descriptor_by_name("decision_panel_live").map(|d| d.id),
            Some(87)
        );
        assert_eq!(registry.id_for_scheduled_name("sunk_ship_history_live"), Some(86));
        assert_eq!(
            registry.by_id(86).map(|descriptor| descriptor.status),
            Some(CompactDomainStatus::ProductionComplete)
        );
        assert!(registry.by_id(87).is_none());
    }

    #[test]
    fn a31_focus_presentation_transport_is_production_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            assigned_descriptor_by_name("national_focus_presentation_live").map(|d| d.id),
            Some(89)
        );
        assert_eq!(registry.id_for_scheduled_name("national_focus_presentation_live"), Some(89));
        assert_eq!(
            registry.by_id(89).map(|descriptor| descriptor.status),
            Some(CompactDomainStatus::ProductionComplete)
        );
    }

    #[test]
    fn a36_core_ability_transport_is_production_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("active_command_abilities_live"),
            Some(88)
        );
        assert_eq!(
            registry.by_id(88).map(|descriptor| descriptor.status),
            Some(CompactDomainStatus::ProductionComplete)
        );
    }

    #[test]
    fn d90_convoy_transport_is_production_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            assigned_descriptor_by_name("convoy_transport_attack_live").map(|d| d.id),
            Some(90)
        );
        assert_eq!(
            registry.id_for_scheduled_name("convoy_transport_attack_live"),
            Some(90)
        );
        assert_eq!(
            registry.by_id(90).map(|descriptor| descriptor.status),
            Some(CompactDomainStatus::ProductionComplete)
        );
    }

    #[test]
    fn b7_raid_instances_has_a_fail_closed_production_reader() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            assigned_descriptor_by_name("raid_instances_live").map(|d| d.id),
            Some(91)
        );
        assert_eq!(registry.id_for_scheduled_name("raid_instances_live"), Some(91));
        assert_eq!(
            registry.by_id(91).map(|descriptor| descriptor.status),
            Some(CompactDomainStatus::ProductionComplete)
        );
    }

    #[test]
    fn w3_delayed_events_and_program_lanes_schedule() {
        // W3: rooted per-country delayed events (domain 64) and owner-joined
        // program facilities (domain 65). Both validated_slice_open_gaps.
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("delayed_events_live"),
            Some(64)
        );
        assert_eq!(
            registry.id_for_scheduled_name("special_project_programs_live"),
            Some(65)
        );
        assert_eq!(
            registry.by_id(64).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
        assert_eq!(
            registry.by_id(65).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
    }

    #[test]
    fn w4_occupation_state_extras_lane_schedules() {
        // W4: rooted per-occupied-state occupation extras (domain 66):
        // per-state occupation law FNV token + garrison manpower source tag.
        // validated_slice_open_gaps; keeps the forbidden d52 lane byte-identical.
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("occupation_state_extras_live"),
            Some(66)
        );
        assert_eq!(
            registry.by_id(66).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
    }

    #[test]
    fn w5_country_dynamic_modifiers_lane_schedules() {
        // W5: rooted per-country dynamic-modifier id set (domain 67), the
        // country's OWN active dynamic-modifier definition ids off
        // CCountry+0xE58. Variable-length per-country string set;
        // validated_slice_open_gaps.
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("country_dynamic_modifiers_live"),
            Some(67)
        );
        assert_eq!(
            registry.by_id(67).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
    }

    #[test]
    fn w6_session_header_and_tech_sharing_groups_lanes_schedule() {
        // W6: rooted top-level session header (domain 68) and tech-sharing
        // groups (domain 69). Both validated_slice_open_gaps; distinct from the
        // existing d57 session_multiplayer_metadata_live lane.
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("session_header_live"),
            Some(68)
        );
        assert_eq!(
            registry.id_for_scheduled_name("tech_sharing_groups_live"),
            Some(69)
        );
        assert_eq!(
            registry.by_id(68).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
        assert_eq!(
            registry.by_id(69).map(|d| d.status),
            Some(CompactDomainStatus::ValidatedSliceOpenGaps)
        );
        // d57 stays a distinct lane (not clobbered by the new session header).
        assert_eq!(
            registry.id_for_scheduled_name("session_multiplayer_metadata_live"),
            Some(57)
        );
    }

    #[test]
    fn naval_combat_lane_ids_schedule_after_shared_integration() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        assert_eq!(
            registry.id_for_scheduled_name("naval_combat_lifecycle_live"),
            Some(49)
        );
        assert_eq!(
            registry.id_for_scheduled_name("naval_combat_members_live"),
            Some(50)
        );
        for id in [49, 50] {
            assert_eq!(
                registry.by_id(id).map(|d| d.status),
                Some(CompactDomainStatus::ValidatedSliceOpenGaps),
                "domain {id} must schedule with the validated-slice-open-gaps status"
            );
        }
    }

    #[test]
    fn lane_module_registration_isolated_from_builtins() {
        let mut registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        TestLaneModule.register(&mut registry).unwrap();

        assert_eq!(registry.id_for_scheduled_name("test_lane_live"), Some(999));
        assert_eq!(
            registry.by_id(999).map(|d| d.status),
            Some(CompactDomainStatus::ActiveIntegration)
        );
        // The real domain 60 builtin stays intact alongside the test lane.
        assert_eq!(
            registry.id_for_scheduled_name("capitulation_war_state_live"),
            Some(60)
        );
    }

    #[test]
    fn duplicate_id_and_name_are_rejected() {
        let mut registry = CollectorDomainRegistry::new();
        registry
            .register(domain(
                60,
                "test_lane_live",
                "test_lane_live",
                CompactDomainStatus::ActiveIntegration,
                true,
                true,
                &[],
            ))
            .unwrap();

        assert!(matches!(
            registry.register(domain(
                60,
                "other_lane_live",
                "other_lane_live",
                CompactDomainStatus::ActiveIntegration,
                true,
                true,
                &[],
            )),
            Err(DomainRegistryError::DuplicateId { id: 60, .. })
        ));
        assert!(matches!(
            registry.register(domain(
                61,
                "test_lane_live",
                "test_lane_live",
                CompactDomainStatus::ActiveIntegration,
                true,
                true,
                &[],
            )),
            Err(DomainRegistryError::DuplicateName {
                name: "test_lane_live",
                ..
            })
        ));
    }
}
