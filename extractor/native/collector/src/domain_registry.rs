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
            domain(
                60,
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
            assigned_descriptor_by_name("air_variant_archetypes_live").map(|d| d.id),
            Some(46)
        );
    }

    #[test]
    fn reserved_lane_ids_are_assigned_but_not_scheduled() {
        let registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        for id in [49, 50, 58, 59] {
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
    fn lane_module_registration_isolated_from_builtins() {
        let mut registry = CollectorDomainRegistry::with_builtin_modules().unwrap();
        TestLaneModule.register(&mut registry).unwrap();

        assert_eq!(registry.id_for_scheduled_name("test_lane_live"), Some(60));
        assert_eq!(
            registry.by_id(60).map(|d| d.status),
            Some(CompactDomainStatus::ActiveIntegration)
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
