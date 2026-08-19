pub use compact_contract::{
    assigned_descriptor_by_id, assigned_descriptor_by_name, descriptor_names, domain,
    id_for_scheduled_name, requires_data_record, validate_descriptor, wire_name_for_id,
    CompactDomainDescriptor, CompactDomainId, CompactDomainStatus, DomainRegistryError,
    ASSIGNED_COMPACT_DOMAINS,
};

pub type CompactDataDecoder<S> = fn(offset: usize, payload: &[u8], summary: &mut S);

#[derive(Clone, Copy)]
pub struct CompactDomainModule<S> {
    pub descriptor: CompactDomainDescriptor,
    pub data_decoder: Option<CompactDataDecoder<S>>,
}

#[derive(Clone, Default)]
pub struct CompactDomainRegistry<S> {
    modules: Vec<CompactDomainModule<S>>,
}

impl<S> CompactDomainRegistry<S> {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    pub fn with_builtin_descriptors() -> Result<Self, DomainRegistryError> {
        let mut registry = Self::new();
        for descriptor in ASSIGNED_COMPACT_DOMAINS
            .iter()
            .copied()
            .filter(|descriptor| descriptor.schedulable)
        {
            registry.register_descriptor(descriptor)?;
        }
        Ok(registry)
    }

    pub fn register_descriptor(
        &mut self,
        descriptor: CompactDomainDescriptor,
    ) -> Result<(), DomainRegistryError> {
        self.register(CompactDomainModule {
            descriptor,
            data_decoder: None,
        })
    }

    pub fn register(&mut self, module: CompactDomainModule<S>) -> Result<(), DomainRegistryError> {
        validate_descriptor(module.descriptor)?;
        if let Some(existing) = self
            .modules
            .iter()
            .find(|entry| entry.descriptor.id == module.descriptor.id)
        {
            return Err(DomainRegistryError::DuplicateId {
                id: module.descriptor.id,
                existing_name: existing.descriptor.canonical_name,
                new_name: module.descriptor.canonical_name,
            });
        }
        for name in descriptor_names(module.descriptor) {
            if let Some(existing) = self
                .modules
                .iter()
                .find(|entry| entry.descriptor.matches_any_name(name))
            {
                return Err(DomainRegistryError::DuplicateName {
                    name,
                    existing_id: existing.descriptor.id,
                    new_id: module.descriptor.id,
                });
            }
        }
        self.modules.push(module);
        Ok(())
    }

    pub fn modules(&self) -> &[CompactDomainModule<S>] {
        &self.modules
    }

    pub fn descriptor_by_id(&self, id: CompactDomainId) -> Option<CompactDomainDescriptor> {
        self.modules
            .iter()
            .map(|module| module.descriptor)
            .find(|descriptor| descriptor.id == id)
    }

    pub fn id_for_scheduled_name(&self, name: &str) -> Option<CompactDomainId> {
        self.modules
            .iter()
            .map(|module| module.descriptor)
            .find(|descriptor| descriptor.matches_scheduled_name(name))
            .map(|descriptor| descriptor.id)
    }

    pub fn decoder_for(&self, id: CompactDomainId) -> Option<CompactDataDecoder<S>> {
        self.modules
            .iter()
            .find(|module| module.descriptor.id == id)
            .and_then(|module| module.data_decoder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct DummySummary {
        calls: usize,
    }

    fn dummy_decoder(_offset: usize, _payload: &[u8], summary: &mut DummySummary) {
        summary.calls += 1;
    }

    #[test]
    fn scheduled_lookup_preserves_existing_wire_names_and_aliases() {
        assert_eq!(id_for_scheduled_name("fuel_live"), Some(10));
        assert_eq!(id_for_scheduled_name("state_ownership"), Some(22));
        assert_eq!(id_for_scheduled_name("air_variant_archetypes"), Some(46));
        assert_eq!(
            id_for_scheduled_name("land_combat_detail_events_live"),
            Some(55)
        );
        assert_eq!(
            id_for_scheduled_name("session_multiplayer_metadata_live"),
            Some(57)
        );
        assert_eq!(
            id_for_scheduled_name("characters_leaders_cabinet_live"),
            Some(47)
        );
        assert_eq!(
            id_for_scheduled_name("diplomacy_intel_operatives_live"),
            Some(53)
        );
        assert_eq!(id_for_scheduled_name("hub_motorization_live"), Some(62));
        assert_eq!(
            wire_name_for_id(47),
            Some("characters_leaders_cabinet_live")
        );
        assert_eq!(
            wire_name_for_id(53),
            Some("diplomacy_intel_operatives_live")
        );
        assert_eq!(
            wire_name_for_id(57),
            Some("session_multiplayer_metadata_live")
        );
        assert_eq!(wire_name_for_id(62), Some("hub_motorization_live"));
        let production_d70 = assigned_descriptor_by_name("sunk_convoy_history_live")
            .expect("production d70 remains assigned");
        assert_eq!(production_d70.id, 70);
        assert_eq!(
            production_d70.status,
            CompactDomainStatus::ProductionComplete
        );
        assert!(production_d70.schedulable);
        assert!(production_d70.requires_data_record);
        assert_eq!(
            id_for_scheduled_name("sunk_convoy_history_live"),
            Some(70),
            "ordinary production must resolve the exact fail-closed d70 reader"
        );
        assert_eq!(id_for_scheduled_name("air_variant_archetypes_live"), None);
        assert_eq!(wire_name_for_id(46), Some("air_variant_archetypes"));
        assert_eq!(
            assigned_descriptor_by_name("air_variant_archetypes_live").map(|d| d.id),
            Some(46)
        );
        assert!(!requires_data_record(1000));
        assert!(requires_data_record(53));
        assert!(requires_data_record(46));
        assert!(requires_data_record(57));
        assert!(requires_data_record(62));
    }

    #[test]
    fn registry_accepts_lane_decoder_modules() {
        let mut registry = CompactDomainRegistry::<DummySummary>::with_builtin_descriptors()
            .expect("builtins register");
        registry
            .register(CompactDomainModule {
                descriptor: domain(
                    999,
                    "test_lane_live",
                    "test_lane_live",
                    CompactDomainStatus::ActiveIntegration,
                    true,
                    true,
                    &[],
                ),
                data_decoder: Some(dummy_decoder),
            })
            .unwrap();

        let decoder = registry.decoder_for(999).expect("registered decoder");
        let mut summary = DummySummary::default();
        decoder(0, &[], &mut summary);
        assert_eq!(summary.calls, 1);
        assert_eq!(registry.id_for_scheduled_name("test_lane_live"), Some(999));
    }

    #[test]
    fn duplicate_modules_are_rejected() {
        let mut registry = CompactDomainRegistry::<DummySummary>::new();
        registry
            .register_descriptor(domain(
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
            registry.register_descriptor(domain(
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
            registry.register_descriptor(domain(
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
