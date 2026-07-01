pub type CompactDomainId = u16;

pub type CompactDataDecoder<S> = fn(offset: usize, payload: &[u8], summary: &mut S);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDomainStatus {
    Existing,
    ProductionComplete,
    ProductionCompleteBase,
    ValidatedSlice,
    ValidatedPartialSlice,
    ValidatedSliceOpenGaps,
    ActiveIntegration,
    Reserved,
    Infrastructure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactDomainDescriptor {
    pub id: CompactDomainId,
    pub canonical_name: &'static str,
    pub wire_name: &'static str,
    pub status: CompactDomainStatus,
    pub schedulable: bool,
    pub requires_data_record: bool,
    pub aliases: &'static [&'static str],
}

impl CompactDomainDescriptor {
    pub fn matches_any_name(&self, name: &str) -> bool {
        self.canonical_name == name
            || self.wire_name == name
            || self.aliases.iter().any(|alias| *alias == name)
    }

    pub fn matches_scheduled_name(&self, name: &str) -> bool {
        self.schedulable
            && (self.wire_name == name || self.aliases.iter().any(|alias| *alias == name))
    }
}

#[derive(Clone, Copy)]
pub struct CompactDomainModule<S> {
    pub descriptor: CompactDomainDescriptor,
    pub data_decoder: Option<CompactDataDecoder<S>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainRegistryError {
    InvalidId {
        id: CompactDomainId,
    },
    InvalidName {
        name: &'static str,
    },
    DuplicateId {
        id: CompactDomainId,
        existing_name: &'static str,
        new_name: &'static str,
    },
    DuplicateName {
        name: &'static str,
        existing_id: CompactDomainId,
        new_id: CompactDomainId,
    },
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

pub fn assigned_descriptor_by_id(id: CompactDomainId) -> Option<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .find(|descriptor| descriptor.id == id)
}

pub fn assigned_descriptor_by_name(name: &str) -> Option<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .find(|descriptor| descriptor.matches_any_name(name))
}

pub fn id_for_scheduled_name(name: &str) -> Option<CompactDomainId> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .find(|descriptor| descriptor.matches_scheduled_name(name))
        .map(|descriptor| descriptor.id)
}

pub fn wire_name_for_id(id: CompactDomainId) -> Option<&'static str> {
    assigned_descriptor_by_id(id).map(|descriptor| descriptor.wire_name)
}

pub fn requires_data_record(id: CompactDomainId) -> bool {
    assigned_descriptor_by_id(id)
        .map(|descriptor| descriptor.requires_data_record)
        .unwrap_or(false)
}

fn validate_descriptor(descriptor: CompactDomainDescriptor) -> Result<(), DomainRegistryError> {
    if descriptor.id == 0 {
        return Err(DomainRegistryError::InvalidId { id: descriptor.id });
    }
    for name in descriptor_names(descriptor) {
        if !is_valid_domain_name(name) {
            return Err(DomainRegistryError::InvalidName { name });
        }
    }
    Ok(())
}

fn descriptor_names(descriptor: CompactDomainDescriptor) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(2 + descriptor.aliases.len());
    names.push(descriptor.canonical_name);
    names.push(descriptor.wire_name);
    names.extend(descriptor.aliases.iter().copied());
    names
}

fn is_valid_domain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub const ASSIGNED_COMPACT_DOMAINS: &[CompactDomainDescriptor] = &[
    domain(
        1,
        "clock",
        "clock",
        CompactDomainStatus::Infrastructure,
        false,
        false,
        &[],
    ),
    domain(
        10,
        "fuel_live",
        "fuel_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        11,
        "production_live",
        "production_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        12,
        "focus_live",
        "focus_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        13,
        "tech_live",
        "tech_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        14,
        "politics_live",
        "politics_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        15,
        "politics_residuals",
        "politics_residuals",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        16,
        "decisions_live",
        "decisions_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        17,
        "diplomacy_live",
        "diplomacy_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        20,
        "divisions_live",
        "divisions",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        21,
        "land_combats_live",
        "land_combats",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        22,
        "map_control_live",
        "map_control",
        CompactDomainStatus::Existing,
        true,
        true,
        &["state_ownership", "state_control", "province_control"],
    ),
    domain(
        23,
        "map_detail_live",
        "map_detail",
        CompactDomainStatus::Existing,
        true,
        true,
        &["state_detail", "buildings_live"],
    ),
    domain(
        24,
        "weather_live",
        "weather_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        26,
        "division_equipment_live",
        "division_equipment",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        27,
        "army_orders_live",
        "army_orders",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        28,
        "oob_command_live",
        "oob_command",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        29,
        "ships_live",
        "ships",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        30,
        "air_wings_live",
        "air_wings",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        31,
        "air_region_activity_live",
        "air_region_activity",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        32,
        "air_wing_loadouts_live",
        "air_wing_loadouts",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        33,
        "air_variant_defs",
        "air_variant_defs",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        34,
        "air_base_status_live",
        "air_base_status",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        35,
        "air_variant_stats",
        "air_variant_stats",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        36,
        "air_variant_reserves",
        "air_variant_reserves",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        37,
        "stockpile_variant_defs",
        "stockpile_variant_defs",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        38,
        "air_superiority_inputs",
        "air_superiority_inputs",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        39,
        "air_variant_upgrades",
        "air_variant_upgrades",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        40,
        "stockpile_variant_upgrades",
        "stockpile_variant_upgrades",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        41,
        "projects_live",
        "projects_live",
        CompactDomainStatus::ValidatedPartialSlice,
        true,
        true,
        &[],
    ),
    domain(
        42,
        "air_combat_losses_live",
        "air_combat_losses_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        43,
        "market_trade_live",
        "market_trade_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        44,
        "supply_logistics_attrition_live",
        "supply_logistics_attrition_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        45,
        "focus_research_decisions_depth_live",
        "focus_research_decisions_depth_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        46,
        "air_variant_archetypes_live",
        "air_variant_archetypes",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        47,
        "characters_leaders_cabinet_live",
        "characters_leaders_cabinet_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        48,
        "construction_buildings_state_damage_live",
        "construction_buildings_state_damage_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        49,
        "naval_combat_lifecycle_live",
        "naval_combat_lifecycle_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        50,
        "naval_combat_members_live",
        "naval_combat_members_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        51,
        "scripted_state_flags_variables_live",
        "scripted_state_flags_variables_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        52,
        "occupation_resistance_compliance_live",
        "occupation_resistance_compliance_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        53,
        "diplomacy_intel_operatives_live",
        "diplomacy_intel_operatives_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        54,
        "army_battleplans_depth_live",
        "army_battleplans_depth_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        1000,
        "frame",
        "frame",
        CompactDomainStatus::Infrastructure,
        false,
        false,
        &[],
    ),
];

const fn domain(
    id: CompactDomainId,
    canonical_name: &'static str,
    wire_name: &'static str,
    status: CompactDomainStatus,
    schedulable: bool,
    requires_data_record: bool,
    aliases: &'static [&'static str],
) -> CompactDomainDescriptor {
    CompactDomainDescriptor {
        id,
        canonical_name,
        wire_name,
        status,
        schedulable,
        requires_data_record,
        aliases,
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
        assert_eq!(id_for_scheduled_name("air_variant_archetypes_live"), None);
        assert_eq!(wire_name_for_id(46), Some("air_variant_archetypes"));
        assert_eq!(
            assigned_descriptor_by_name("air_variant_archetypes_live").map(|d| d.id),
            Some(46)
        );
        assert!(!requires_data_record(1000));
        assert!(requires_data_record(46));
    }

    #[test]
    fn registry_accepts_lane_decoder_modules() {
        let mut registry = CompactDomainRegistry::<DummySummary>::with_builtin_descriptors()
            .expect("builtins register");
        registry
            .register(CompactDomainModule {
                descriptor: domain(
                    60,
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

        let decoder = registry.decoder_for(60).expect("registered decoder");
        let mut summary = DummySummary::default();
        decoder(0, &[], &mut summary);
        assert_eq!(summary.calls, 1);
        assert_eq!(registry.id_for_scheduled_name("test_lane_live"), Some(60));
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
