
use engine_sites::Rva;
pub use crate::advisor_mutations::MutationSite;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    ProvinceNames,
    RegionAccess,
    Tension,
    DynamicModifiers,
    IndustrialOrganisationRosters,
    IndustrialOrganisationPolicies,
    DesignIndustrialOrganisation,
    GameFog,
}

pub const DOMAINS: usize = 8;

pub const CONTAINER_COUNTRY: usize = crate::cdynamic_modifier_container::COUNTRY;

pub const MUTATIONS: [(MutationSite, Domain); 27] = [
    (
        MutationSite {
            name: "rename_province",
            rva: Rva(0x37_7d50),
        },
        Domain::ProvinceNames,
    ),
    (
        MutationSite {
            name: "reset_province_name",
            rva: Rva(0x37_7fa0),
        },
        Domain::ProvinceNames,
    ),
    (
        MutationSite {
            name: "set_region_access_modes",
            rva: Rva(0xf2_3440),
        },
        Domain::RegionAccess,
    ),
    (
        MutationSite {
            name: "reset_navy",
            rva: Rva(0xf1_ee30),
        },
        Domain::RegionAccess,
    ),
    (
        MutationSite {
            name: "update_threat_total",
            rva: crate::cworld_threat::UPDATE_TOTAL_RVA,
        },
        Domain::Tension,
    ),
    (
        MutationSite {
            name: "add_dynamic_modifier",
            rva: crate::cdynamic_modifier_container::ADD_RVA,
        },
        Domain::DynamicModifiers,
    ),
    (
        MutationSite {
            name: "renew_dynamic_modifier",
            rva: Rva(0x65_93d0),
        },
        Domain::DynamicModifiers,
    ),
    (
        MutationSite {
            name: "remove_dynamic_modifier",
            rva: Rva(0x65_8650),
        },
        Domain::DynamicModifiers,
    ),
    (
        MutationSite {
            name: "dynamic_modifier_daily_update",
            rva: crate::cdynamic_modifier_container::DAILY_UPDATE_RVA,
        },
        Domain::DynamicModifiers,
    ),
    (
        MutationSite {
            name: "mio_queue_append",
            rva: Rva(0xe22680),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_state_copy",
            rva: Rva(0xe22a60),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_allowed_policies",
            rva: Rva(0xe24fd0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_move",
            rva: Rva(0xe28ba0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_read_state",
            rva: Rva(0xe28ce0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_remove",
            rva: Rva(0xe29230),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_queue_replace",
            rva: Rva(0xe293b0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "mio_unlock_trait",
            rva: Rva(0xe2a1f0),
        },
        Domain::IndustrialOrganisationRosters,
    ),
    (
        MutationSite {
            name: "design_mio_bonuses",
            rva: Rva(0xc29860),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_link",
            rva: Rva(0xc37c60),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_move",
            rva: Rva(0x673bc0),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_clone",
            rva: Rva(0xc2d4e0),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "design_mio_read",
            rva: Rva(0xc341e0),
        },
        Domain::DesignIndustrialOrganisation,
    ),
    (
        MutationSite {
            name: "mio_policy_score_refresh",
            rva: Rva(0xa9a610),
        },
        Domain::IndustrialOrganisationPolicies,
    ),
    (
        MutationSite {
            name: "fog_enter_game",
            rva: Rva(0x00e300e0),
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_enter_frontend",
            rva: Rva(0x00b8d1f0),
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_leave_nudge",
            rva: Rva(0x013647d0),
        },
        Domain::GameFog,
    ),
    (
        MutationSite {
            name: "fog_toggle",
            rva: Rva(0x0026ff10),
        },
        Domain::GameFog,
    ),
];

