
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{
    ccharacter,
    ccountry,
    cproduction_status::equipment_license as cequipment_production_license,
    cfaction,
    cgame_state,
    crelation::{self, status as crelation_status},
    cstate,
};
use engine_native_politics::cpolitical_party;
use crate::{
    cnational_focus_progress,
    cresearch_slot,
    ctechnology,
    ctechnology_sharing_group,
    ctechnology_status,
    ctechnology_template,
    nproject_cprogram_status,
    nproject_cproject,
};
use engine_sites::Rva;

pub use crate::advisor_mutations::MutationSite;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tables {
    pub stability: bool,
    pub research: bool,
    pub technologies: bool,
    pub focus: bool,
    pub intel_networks: bool,
    pub projects: bool,
    pub appearance: bool,
    pub counters: bool,
    pub visibility: bool,
    pub weather: bool,
}

impl Tables {
    pub const NONE: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const STABILITY: Self = Self {
        stability: true,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const RESEARCH: Self = Self {
        stability: false,
        research: true,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const BOTH: Self = Self {
        stability: true,
        research: true,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const TECHNOLOGY: Self = Self {
        stability: false,
        research: true,
        technologies: true,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const FOCUS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: true,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const INTEL_NETWORKS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: true,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const PROJECTS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: true,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const APPEARANCE: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: true,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const COUNTERS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: true,
        visibility: false,
        weather: false,
    };
    pub const VISIBILITY: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: true,
        weather: false,
    };
    pub const WEATHER: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: true,
    };
    pub const ALL: Self = Self {
        stability: true,
        research: true,
        technologies: true,
        focus: true,
        intel_networks: false,
        projects: true,
        appearance: true,
        counters: true,
        visibility: true,
        weather: false,
    };

    pub const fn with(self, other: Self) -> Self {
        Self {
            stability: self.stability || other.stability,
            research: self.research || other.research,
            technologies: self.technologies || other.technologies,
            focus: self.focus || other.focus,
            intel_networks: self.intel_networks || other.intel_networks,
            projects: self.projects || other.projects,
            appearance: self.appearance || other.appearance,
            counters: self.counters || other.counters,
            visibility: self.visibility || other.visibility,
            weather: self.weather || other.weather,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Country,
    Interception,
    Manpower,
    RelationStatus,
    WarRelation,
    CountryCharacters,
    PoliticalStatus,
    Party,
    Character,
    Scientist,
    ResearchSlot,
    SharedResearchSlot,
    TechnologyStatus,
    Technology,
    SharedTechnology,
    SharedTechnologyStatus,
    CompletedTechnology,
    ProgramStatus,
    FocusProgress,
    Project,
    FactionSharing,
    License,
    DiplomacyStatus,
    Subjects,
    StateOwner,
    StateController,
    TagPointer,
    Every,
}

impl Receiver {
    pub const fn own(self) -> Self {
        match self {
            Self::SharedTechnology => Self::Technology,
            Self::SharedTechnologyStatus => Self::TechnologyStatus,
            Self::SharedResearchSlot => Self::ResearchSlot,
            Self::CompletedTechnology => Self::TechnologyStatus,
            other => other,
        }
    }

    pub const fn renames(self) -> bool {
        matches!(self, Self::Subjects)
    }
}

pub const INTERCEPTION_COLUMN: u32 = 1;
pub const SCIENTIST_CHARACTER: usize = 0x08;
pub const SHARING_ROSTER: usize = 0x118;
pub const SLOT_ROSTER: usize = ctechnology_status::RESEARCH_SLOTS_DATA;
const GROUP_MEMBERS: usize = ctechnology_sharing_group::MEMBERS;
const FACTION_SHARING_VTABLE_RVA: Rva = Rva(0x02ac_66b8);

const UNAPPOINT_ADVISOR_RVA: Rva = Rva(0x117_2b90);
const SET_LEVEL_RVA: Rva = Rva(0xf5_ffe0);
const RESOLVE_GRAPHICAL_CULTURE_RVA: Rva = Rva(0x75_a3e0);
const REBUILD_LOCALIZED_NAME_RVA: Rva = Rva(0x75_e6d0);

pub const MUTATIONS: [(MutationSite, Receiver, Tables); 98] = [
    (
        MutationSite {
            name: "recalculate_modifiers",
            rva: Rva(0x72_2930),
        },
        Receiver::Country,
        Tables::BOTH.with(Tables::COUNTERS).with(Tables::VISIBILITY),
    ),
    (
        MutationSite {
            name: "add_stability",
            rva: ccountry::ADD_STABILITY_RVA,
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "add_war_support",
            rva: ccountry::ADD_WAR_SUPPORT_RVA,
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_stability",
            rva: Rva(0x75_aa30),
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_war_support",
            rva: Rva(0x75_aa80),
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "weekly_update",
            rva: ccountry::WEEKLY_UPDATE_RVA,
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "add_air_statistic",
            rva: Rva(0x71_94a0),
        },
        Receiver::Interception,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "replace_relation",
            rva: Rva(0xd7_8e60),
        },
        Receiver::RelationStatus,
        Tables::STABILITY.with(Tables::COUNTERS),
    ),
    (
        MutationSite {
            name: "set_instigator",
            rva: Rva(0xd8_ea80),
        },
        Receiver::WarRelation,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_defender",
            rva: Rva(0xd8_e930),
        },
        Receiver::WarRelation,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "become_exiled",
            rva: Rva(0xd9_8790),
        },
        Receiver::Every,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "clear_exile_links",
            rva: Rva(0xda_9380),
        },
        Receiver::Every,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "return_from_exile",
            rva: Rva(0xda_9430),
        },
        Receiver::Every,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "unappoint_advisor",
            rva: UNAPPOINT_ADVISOR_RVA,
        },
        Receiver::CountryCharacters,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_technology",
            rva: Rva(0xf6_0170),
        },
        Receiver::SharedResearchSlot,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "sync_research_slots",
            rva: Rva(0xf6_15c0),
        },
        Receiver::TechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "set_level",
            rva: SET_LEVEL_RVA,
        },
        Receiver::SharedTechnology,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "claim_limited_use_bonuses",
            rva: Rva(0xf5_5f70),
        },
        Receiver::Technology,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "release_limited_use_bonuses",
            rva: Rva(0xf5_6460),
        },
        Receiver::Technology,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "complete_research_slot",
            rva: Rva(0xf5_dd50),
        },
        Receiver::TechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "reset_research",
            rva: Rva(0xf5_f820),
        },
        Receiver::SharedTechnology,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "set_design_team",
            rva: Rva(0xf5_fd00),
        },
        Receiver::Technology,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "set_boosted_by_xp",
            rva: Rva(0xf6_19b0),
        },
        Receiver::Technology,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "research_completed",
            rva: ctechnology_status::RESEARCH_COMPLETED_RVA,
        },
        Receiver::CompletedTechnology,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "rebuild_or_transfer_b",
            rva: ctechnology_status::REBUILD_OR_TRANSFER_B_RVA,
        },
        Receiver::SharedTechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "rebuild_or_transfer_a",
            rva: ctechnology_status::REBUILD_OR_TRANSFER_A_RVA,
        },
        Receiver::Every,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "add_limited_use_bonus",
            rva: Rva(0xf5_1ea0),
        },
        Receiver::TechnologyStatus,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "add_cost_reduction",
            rva: Rva(0xf5_1f10),
        },
        Receiver::TechnologyStatus,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "sharing_roster_add",
            rva: Rva(0xf5_23d0),
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "sharing_roster_remove",
            rva: Rva(0xf5_f9d0),
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "sharing_add_member",
            rva: Rva(0xde_a060),
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "sharing_remove_member",
            rva: Rva(0xde_c210),
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "sharing_bonus",
            rva: Rva(0xde_bd60),
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "register_license",
            rva: engine_native_parts::clicensed_production_status::REGISTER_LICENSE_RVA,
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "scientist_categories",
            rva: Rva(0xee_7530),
        },
        Receiver::ProgramStatus,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "research_console",
            rva: Rva(0x29_4310),
        },
        Receiver::Every,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "fresh_init",
            rva: ctechnology_status::FRESH_INIT_RVA,
        },
        Receiver::SharedTechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "focus_reset",
            rva: Rva(0x2e_7c70),
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "focus_database_clear",
            rva: Rva(0x2e_79f0),
        },
        Receiver::Every,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "continuous_focus",
            rva: Rva(0x2f_3650),
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "focus_completed",
            rva: cnational_focus_progress::COMPLETED_RVA,
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "focus_copy",
            rva: Rva(0x2e_8540),
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "focus_remove_completed",
            rva: Rva(0x2f_3380),
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "set_focus_tree",
            rva: Rva(0x75_9be0),
        },
        Receiver::Country,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "add_completed_focus",
            rva: Rva(0x2e_5eb0),
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    (
        MutationSite {
            name: "reload_technologies",
            rva: Rva(0x29_2770),
        },
        Receiver::Every,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "grow_intel_network",
            rva: Rva(0x27_8400),
        },
        Receiver::Every,
        Tables::INTEL_NETWORKS,
    ),
    (
        MutationSite {
            name: "start_project",
            rva: Rva(0x106_b290),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "stop_project",
            rva: Rva(0x106_b4a0),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "cancel_project",
            rva: Rva(0x106_a410),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "complete_project",
            rva: Rva(0x106_9c20),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "add_project_progress",
            rva: Rva(0x106_66c0),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "project_reward_option",
            rva: Rva(0x106_6a90),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "forced_project_update",
            rva: Rva(0x106_a5a0),
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "project_database_resync",
            rva: Rva(0x151_d070),
        },
        Receiver::Every,
        Tables::PROJECTS,
    ),
    (
        MutationSite {
            name: "license_removal",
            rva: Rva(0x14d_6620),
        },
        Receiver::License,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "license_extension",
            rva: Rva(0x14d_3920),
        },
        Receiver::License,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "faction_set_leader",
            rva: Rva(0xdf_92c0),
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "set_cosmetic_tag",
            rva: Rva(0x75_93e0),
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    (
        MutationSite {
            name: "copy_country_colors",
            rva: Rva(0x75_93b0),
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    (
        MutationSite {
            name: "resolve_graphical_culture",
            rva: RESOLVE_GRAPHICAL_CULTURE_RVA,
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    (
        MutationSite {
            name: "resolve_graphical_culture_2d",
            rva: Rva(0x75_a380),
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    (
        MutationSite {
            name: "rebuild_localized_name",
            rva: REBUILD_LOCALIZED_NAME_RVA,
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    (
        MutationSite {
            name: "rebuild_subjects",
            rva: Rva(0xda_bf10),
        },
        Receiver::Subjects,
        Tables::APPEARANCE,
    ),
    (
        MutationSite {
            name: "add_army",
            rva: Rva(0x71_ab10),
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "remove_army",
            rva: Rva(0x74_7880),
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "insert_unit_leader",
            rva: Rva(0x117_6eb0),
        },
        Receiver::CountryCharacters,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "remove_unit_leader",
            rva: Rva(0x117_5010),
        },
        Receiver::CountryCharacters,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "refresh_neighbors",
            rva: Rva(0x75_30b0),
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "reload_mio_policies",
            rva: Rva(0x20_06d0),
        },
        Receiver::Every,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "faction_sharing_base",
            rva: Rva(0x1c7_d5e0),
        },
        Receiver::FactionSharing,
        Tables::RESEARCH,
    ),
    (
        MutationSite {
            name: "weather_reinitialize",
            rva: Rva(0xfa_2600),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_reset",
            rva: Rva(0x24_5730),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_add_snow",
            rva: Rva(0x25_3e20),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_set_mud",
            rva: Rva(0x29_c5b0),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_set_region",
            rva: Rva(0x29_eaa0),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_read",
            rva: Rva(0xfa_2670),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_initial_passes",
            rva: Rva(0xf9_f6d0),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_reload",
            rva: Rva(0x29_2ea0),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_console_hours",
            rva: Rva(0x25_b290),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_add_province_modifier",
            rva: Rva(0xf9_88b0),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_remove_province_modifier",
            rva: Rva(0xfa_3770),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "weather_expiry_rebuild",
            rva: Rva(0xf9_b6c0),
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    (
        MutationSite {
            name: "research_progress",
            rva: Rva(0xf5_2040),
        },
        Receiver::Technology,
        Tables::TECHNOLOGY,
    ),
    (
        MutationSite {
            name: "set_recruitment_target",
            rva: Rva(0xd5fbe0),
        },
        Receiver::Manpower,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_recruitment_ratio",
            rva: Rva(0xd5ff90),
        },
        Receiver::Manpower,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "add_exile_manpower",
            rva: Rva(0xd5b8f0),
        },
        Receiver::Manpower,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "restore_exile_manpower",
            rva: Rva(0xd5f970),
        },
        Receiver::Manpower,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "use_exile_manpower",
            rva: Rva(0xd5ffa0),
        },
        Receiver::Manpower,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_exile_manpower_cap",
            rva: Rva(0xd5fe90),
        },
        Receiver::Manpower,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "add_exile_legitimacy",
            rva: Rva(0xd97e00),
        },
        Receiver::DiplomacyStatus,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "set_exile_legitimacy",
            rva: Rva(0xdaab80),
        },
        Receiver::DiplomacyStatus,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "exile_equipment_legitimacy",
            rva: Rva(0xda5810),
        },
        Receiver::DiplomacyStatus,
        Tables::STABILITY,
    ),
    (
        MutationSite {
            name: "refresh_wing_crews",
            rva: Rva(0xfec940),
        },
        Receiver::Every,
        Tables::NONE,
    ),
    (
        MutationSite {
            name: "refresh_ship_crews",
            rva: Rva(0xc94c40),
        },
        Receiver::Every,
        Tables::NONE,
    ),
    (
        MutationSite {
            name: "remove_ship_crews",
            rva: Rva(0xc94820),
        },
        Receiver::Every,
        Tables::NONE,
    ),
    (
        MutationSite {
            name: "add_sent_expeditionary",
            rva: Rva(0x719d20),
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    (
        MutationSite {
            name: "remove_sent_expeditionary",
            rva: Rva(0x7580e0),
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
];

pub const APPEARANCE_SITES: std::ops::Range<usize> = 58..64;
pub const COUNTER_SITES: std::ops::Range<usize> = 64..70;

pub const REMOVE_UNIT_LEADER: usize = 67;
pub const REBUILD_LOCALIZED_NAME: usize = 62;
const _: () = assert!(matches!(
    MUTATIONS[REBUILD_LOCALIZED_NAME].0.name.as_bytes(),
    b"rebuild_localized_name"
));
pub const SYNC_RESEARCH_SLOTS: usize = 15;
pub const FACTION_SHARING_BASE: usize = 70;
pub const WEATHER_WRITERS: std::ops::Range<usize> = 71..83;
pub const WEATHER_EXPIRY_REBUILD: usize = 82;
pub const PROVINCE_WEATHER_REBUILD_RVA: Rva = Rva(0xfa_4b50);
pub const RESEARCH_PROGRESS: usize = 83;

pub const SET_RECRUITMENT_TARGET: usize = 84;
pub const SET_RECRUITMENT_RATIO: usize = 85;
pub const ADD_EXILE_MANPOWER: usize = 86;
pub const RESTORE_EXILE_MANPOWER: usize = 87;
pub const USE_EXILE_MANPOWER: usize = 88;
pub const SET_EXILE_MANPOWER_CAP: usize = 89;
pub const ADD_EXILE_LEGITIMACY: usize = 90;
pub const SET_EXILE_LEGITIMACY: usize = 91;
pub const EXILE_EQUIPMENT_LEGITIMACY: usize = 92;
pub const REFRESH_WING_CREWS: usize = 93;
pub const REFRESH_SHIP_CREWS: usize = 94;
pub const REMOVE_SHIP_CREWS: usize = 95;
pub const ADD_SENT_EXPEDITIONARY: usize = 96;
pub const REMOVE_SENT_EXPEDITIONARY: usize = 97;
pub const CREW_WRITERS: std::ops::Range<usize> = REFRESH_WING_CREWS..ADD_SENT_EXPEDITIONARY;

pub const EFFECT_PROGRESS_SOURCE: u32 = 2;
pub const GROW_INTEL_NETWORK: usize = 46;
pub const PROJECT_REWARD_OPTION: usize = 52;
pub const PROJECT_DATABASE_RESYNC: usize = 54;

pub fn countries(
    memory: &impl Memory,
    base: usize,
    receiver: Receiver,
    first: usize,
    second: usize,
    visit: &mut dyn FnMut(u32),
) -> bool {
    let mut named = |id: Option<u32>| match id {
        Some(id) => {
            visit(id);
            true
        }
        None => false,
    };
    match receiver {
        Receiver::Every => false,
        Receiver::Country => named(of_country(memory, first)),
        Receiver::Manpower => named(link_at(
            memory,
            base,
            first,
            engine_native_parts::ccountry_manpower::COUNTRY,
        )),
        Receiver::Interception => {
            second as u32 != INTERCEPTION_COLUMN || named(of_country(memory, first))
        }
        Receiver::RelationStatus => named(
            pointer(memory, first, crelation_status::DIPLOMACY_STATUS)
                .and_then(|diplomacy| pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY))
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::WarRelation => [crelation::FIRST_COUNTRY, crelation::SECOND_COUNTRY]
            .into_iter()
            .all(|offset| {
                named(
                    first
                        .checked_add(offset)
                        .and_then(|address| read::<u32>(memory, address))
                        .and_then(|raw| of_link(memory, base, raw)),
                )
            }),
        Receiver::CountryCharacters => named(
            pointer(memory, first, engine_native::ccountry::rosters::OWNER)
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::PoliticalStatus => named(
            pointer(memory, first, engine_native_parts::cpolitical_status_layout::COUNTRY)
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::Party => named(link_at(memory, base, first, cpolitical_party::TAG)),
        Receiver::Character => named(link_at(memory, base, first, ccharacter::COUNTRY)),
        Receiver::Scientist => named(
            pointer(memory, first, SCIENTIST_CHARACTER)
                .and_then(|character| link_at(memory, base, character, ccharacter::COUNTRY)),
        ),
        Receiver::ResearchSlot => named(
            pointer(memory, first, cresearch_slot::STATUS)
                .and_then(|status| status_country(memory, status)),
        ),
        Receiver::SharedResearchSlot => match pointer(memory, first, cresearch_slot::STATUS) {
            Some(status) => {
                let written = [
                    pointer(memory, first, cresearch_slot::TECHNOLOGY),
                    Some(second),
                ];
                named(status_country(memory, status))
                    && sharing_affected(memory, base, status, &written, visit)
            }
            None => false,
        },
        Receiver::TechnologyStatus => named(status_country(memory, first)),
        Receiver::Technology => named(
            pointer(memory, first, ctechnology::STATUS)
                .and_then(|status| status_country(memory, status)),
        ),
        Receiver::SharedTechnology => match pointer(memory, first, ctechnology::STATUS) {
            Some(status) => {
                named(status_country(memory, status))
                    && sharing_affected(memory, base, status, &[Some(first)], visit)
            }
            None => false,
        },
        Receiver::CompletedTechnology => {
            named(status_country(memory, first))
                && sharing_affected(memory, base, first, &[Some(second)], visit)
        }
        Receiver::SharedTechnologyStatus => {
            named(status_country(memory, first)) && sharing(memory, base, first, visit)
        }
        Receiver::ProgramStatus => named(link_at(
            memory,
            base,
            first,
            nproject_cprogram_status::OWNER,
        )),
        Receiver::FocusProgress => named(
            pointer(memory, first, cnational_focus_progress::COUNTRY)
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::Project => named(link_at(memory, base, first, nproject_cproject::OWNER)),
        Receiver::FactionSharing => {
            members(memory, base, first, visit)
                && match pointer(memory, first, FACTION_SHARING_FACTION) {
                    Some(faction) => faction_members(memory, faction, visit),
                    None => false,
                }
        }
        Receiver::License => named(link_at(
            memory,
            base,
            first,
            cequipment_production_license::OWNER,
        )),
        Receiver::DiplomacyStatus => named(diplomacy_country(memory, first)),
        Receiver::Subjects => {
            named(diplomacy_country(memory, first))
                && positive_link(
                    memory,
                    base,
                    first
                        .checked_add(engine_native_parts::cdiplomacy_status_layout::OVERLORD)
                        .and_then(|address| read::<u32>(memory, address)),
                    visit,
                )
        }
        Receiver::StateOwner | Receiver::StateController => {
            let offset = if receiver == Receiver::StateOwner {
                cstate::OWNER
            } else {
                cstate::CONTROLLER
            };
            let old = (first != 0)
                .then(|| first.checked_add(offset))
                .flatten()
                .and_then(|address| read::<u32>(memory, address));
            positive_link(memory, base, old, visit)
                && positive_link(memory, base, Some(second as u32), visit)
        }
        Receiver::TagPointer => positive_link(
            memory,
            base,
            (second != 0).then(|| read::<u32>(memory, second)).flatten(),
            visit,
        ),
    }
}

fn positive_link(
    memory: &impl Memory,
    base: usize,
    raw: Option<u32>,
    visit: &mut dyn FnMut(u32),
) -> bool {
    match raw {
        None => false,
        Some(raw) if raw as i32 <= 0 => true,
        Some(raw) => match of_link(memory, base, raw) {
            Some(id) => {
                visit(id);
                true
            }
            None => false,
        },
    }
}

fn diplomacy_country(memory: &impl Memory, diplomacy: usize) -> Option<u32> {
    pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)
        .and_then(|country| of_country(memory, country))
}

pub fn slot_roster(memory: &impl Memory, status: usize) -> Option<[u8; 16]> {
    read::<[u8; 16]>(memory, status.checked_add(SLOT_ROSTER)?)
}

pub fn unit_leader_counts(memory: &impl Memory, characters: usize) -> Option<[u8; 16]> {
    let count = |roster: usize| {
        read::<[u8; 4]>(
            memory,
            characters.checked_add(roster)?.checked_add(LEADER_COUNT)?,
        )
    };
    let mut counts = [0; 16];
    counts[..4].copy_from_slice(&count(LEADER_ROSTERS[0])?);
    counts[4..8].copy_from_slice(&count(LEADER_ROSTERS[1])?);
    Some(counts)
}

const LEADER_ROSTERS: [usize; 2] = [0x70, 0x88];
const LEADER_COUNT: usize = 0xC;

pub fn faction_sharing_base(memory: &impl Memory, group: usize) -> Option<[u8; 16]> {
    read::<[u8; 16]>(memory, group.checked_add(FACTION_SHARING_BASE_OFFSET)?)
}

const FACTION_SHARING_BASE_OFFSET: usize = 0x10;
const FACTION_SHARING_FACTION: usize = 0x48;

pub fn sharing(
    memory: &impl Memory,
    base: usize,
    status: usize,
    visit: &mut dyn FnMut(u32),
) -> bool {
    let (Some(group_vtable), Some(faction_vtable)) = (
        engine_sites::at(base, ctechnology_sharing_group::VTABLE_RVA),
        engine_sites::at(base, FACTION_SHARING_VTABLE_RVA),
    ) else {
        return false;
    };
    let Some(roster) = status
        .checked_add(SHARING_ROSTER)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for index in 0..roster.count {
        let Some(group) = roster
            .element(index, 8)
            .and_then(|address| pointer(memory, address, 0))
        else {
            return false;
        };
        let Some(vtable) = read::<u64>(memory, group) else {
            return false;
        };
        if vtable != group_vtable as u64 && vtable != faction_vtable as u64 {
            return false;
        }
        if !members(memory, base, group, visit) {
            return false;
        }
    }
    let Some(diplomacy) = pointer(memory, status, ctechnology_status::COUNTRY)
        .and_then(|country| pointer(memory, country, ccountry::DIPLOMACY_STATUS))
    else {
        return false;
    };
    let Some(faction) = diplomacy
        .checked_add(engine_native_parts::cdiplomacy_status_layout::FACTION)
        .and_then(|address| read::<u64>(memory, address))
    else {
        return false;
    };
    match usize::try_from(faction) {
        Ok(0) => true,
        Ok(faction) => faction_members(memory, faction, visit),
        Err(_) => false,
    }
}

fn sharing_partners(
    memory: &impl Memory,
    base: usize,
    status: usize,
    visit: &mut dyn FnMut(usize),
) -> bool {
    let (Some(group_vtable), Some(faction_vtable)) = (
        engine_sites::at(base, ctechnology_sharing_group::VTABLE_RVA),
        engine_sites::at(base, FACTION_SHARING_VTABLE_RVA),
    ) else {
        return false;
    };
    let Some(roster) = status
        .checked_add(SHARING_ROSTER)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for index in 0..roster.count {
        let Some(group) = roster
            .element(index, 8)
            .and_then(|address| pointer(memory, address, 0))
        else {
            return false;
        };
        let Some(vtable) = read::<u64>(memory, group) else {
            return false;
        };
        if vtable != group_vtable as u64 && vtable != faction_vtable as u64 {
            return false;
        }
        if !member_countries(memory, base, group, visit) {
            return false;
        }
    }
    let Some(diplomacy) = pointer(memory, status, ctechnology_status::COUNTRY)
        .and_then(|country| pointer(memory, country, ccountry::DIPLOMACY_STATUS))
    else {
        return false;
    };
    let Some(faction) = diplomacy
        .checked_add(engine_native_parts::cdiplomacy_status_layout::FACTION)
        .and_then(|address| read::<u64>(memory, address))
    else {
        return false;
    };
    match usize::try_from(faction) {
        Ok(0) => true,
        Ok(faction) => faction_countries(memory, faction, visit),
        Err(_) => false,
    }
}

const TECHNOLOGY_DEPENDENTS: usize = 0xb0;
const TECHNOLOGY_PREDECESSORS: usize = 0xc8;

pub fn sharing_affected(
    memory: &impl Memory,
    base: usize,
    status: usize,
    written: &[Option<usize>],
    visit: &mut dyn FnMut(u32),
) -> bool {
    match affected_partners(memory, base, status, written) {
        Some(Some(affected)) => {
            for id in affected {
                visit(id);
            }
            true
        }
        Some(None) => false,
        None => sharing(memory, base, status, visit),
    }
}

fn affected_partners(
    memory: &impl Memory,
    base: usize,
    status: usize,
    written: &[Option<usize>],
) -> Option<Option<Vec<u32>>> {
    fn push<T>(values: &mut Vec<T>, value: T) -> Option<()> {
        crate::allocation::reserve_push_batch(values, 1).ok()?;
        values.push(value);
        Some(())
    }
    let roster = Array::read(
        memory,
        status.checked_add(ctechnology_status::TECHNOLOGIES_DATA)?,
        8,
    )?;
    let mut partners: Vec<usize> = Vec::new();
    let mut failed = false;
    if !sharing_partners(memory, base, status, &mut |country| {
        failed |= push(&mut partners, country).is_none();
    }) {
        return Some(None);
    }
    if failed {
        return None;
    }
    let mut affected: Vec<u32> = Vec::new();
    let mut candidates: Vec<(usize, u32)> = Vec::new();
    for &country in &partners {
        let Some(id) = of_country(memory, country) else {
            return Some(None);
        };
        for instance in partner_reads(memory, country, roster)? {
            if written.contains(&Some(instance)) {
                push(&mut affected, id)?;
                continue;
            }
            let predecessors =
                Array::read(memory, instance.checked_add(TECHNOLOGY_PREDECESSORS)?, 8)?;
            for index in 0..predecessors.count {
                let predecessor = read::<u64>(memory, predecessors.element(index, 8)?)
                    .and_then(|p| usize::try_from(p).ok())?;
                push(&mut candidates, (predecessor, id))?;
            }
        }
    }
    if !candidates.is_empty() {
        let listener_vtable = engine_sites::at(base, ctechnology::LISTENER_VTABLE_RVA)? as u64;
        let mut reached: Vec<usize> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for &technology in written.iter().flatten() {
            if technology != 0 {
                push(&mut stack, technology)?;
            }
        }
        while let Some(technology) = stack.pop() {
            if reached.contains(&technology) {
                continue;
            }
            if reached.len() > roster.count as usize {
                return None;
            }
            push(&mut reached, technology)?;
            let dependents =
                Array::read(memory, technology.checked_add(TECHNOLOGY_DEPENDENTS)?, 8)?;
            for index in 0..dependents.count {
                let dependent = read::<u64>(memory, dependents.element(index, 8)?)
                    .and_then(|p| usize::try_from(p).ok())?;
                if dependent != 0 {
                    push(&mut stack, dependent)?;
                }
            }
            if read::<u64>(
                memory,
                technology.checked_add(ctechnology::PENDING_LISTENERS)?,
            )? != 0
            {
                return None;
            }
            let listeners =
                Array::read(memory, technology.checked_add(ctechnology::LISTENERS)?, 8)?;
            for index in 0..listeners.count {
                let listener = read::<u64>(memory, listeners.element(index, 8)?)
                    .and_then(|p| usize::try_from(p).ok())?;
                if read::<u64>(memory, listener)? != listener_vtable {
                    return None;
                }
                push(&mut stack, listener.checked_sub(ctechnology::LISTENER)?)?;
            }
        }
        for &(predecessor, id) in &candidates {
            if reached.contains(&predecessor) {
                push(&mut affected, id)?;
            }
        }
    }
    affected.sort_unstable();
    affected.dedup();
    Some(Some(affected))
}

fn partner_reads(memory: &impl Memory, country: usize, roster: Array) -> Option<Vec<usize>> {
    let status = pointer(memory, country, ccountry::TECHNOLOGY_STATUS)?;
    let slots = Array::read(
        memory,
        status.checked_add(ctechnology_status::RESEARCH_SLOTS_DATA)?,
        8,
    )?;
    let mut instances = Vec::new();
    for index in 0..slots.count {
        let slot = pointer(memory, slots.element(index, 8)?, 0)?;
        let technology = nullable(memory, slot.checked_add(cresearch_slot::TECHNOLOGY)?)?;
        if technology == 0 {
            continue;
        }
        let template = pointer(memory, technology, ctechnology::TEMPLATE)?;
        let database = read::<i32>(
            memory,
            template.checked_add(ctechnology_template::DATABASE_INDEX)?,
        )?;
        let position = database
            .checked_sub(1)
            .and_then(|i| u32::try_from(i).ok())?;
        if position >= roster.count {
            continue;
        }
        let instance = nullable(memory, roster.element(position, 8)?)?;
        if instance != 0 {
            crate::allocation::reserve_push_batch(&mut instances, 1).ok()?;
            instances.push(instance);
        }
    }
    Some(instances)
}

fn nullable(memory: &impl Memory, address: usize) -> Option<usize> {
    usize::try_from(read::<u64>(memory, address)?).ok()
}

fn members(memory: &impl Memory, base: usize, group: usize, visit: &mut dyn FnMut(u32)) -> bool {
    let Some(members) = group
        .checked_add(GROUP_MEMBERS)
        .and_then(|address| Array::read(memory, address, 4))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 4)
            .and_then(|address| read::<u32>(memory, address))
            .and_then(|raw| of_link(memory, base, raw))
        else {
            return false;
        };
        visit(id);
    }
    true
}

fn member_countries(
    memory: &impl Memory,
    base: usize,
    group: usize,
    visit: &mut dyn FnMut(usize),
) -> bool {
    let Some(members) = group
        .checked_add(GROUP_MEMBERS)
        .and_then(|address| Array::read(memory, address, 4))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 4)
            .and_then(|address| read::<u32>(memory, address))
            .and_then(|raw| country_of_link(memory, base, raw))
        else {
            return false;
        };
        visit(id);
    }
    true
}

fn faction_members(memory: &impl Memory, faction: usize, visit: &mut dyn FnMut(u32)) -> bool {
    let Some(members) = faction
        .checked_add(cfaction::MEMBERS)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 8)
            .and_then(|address| pointer(memory, address, 0))
            .and_then(|country| of_country(memory, country))
        else {
            return false;
        };
        visit(id);
    }
    true
}

fn faction_countries(memory: &impl Memory, faction: usize, visit: &mut dyn FnMut(usize)) -> bool {
    let Some(members) = faction
        .checked_add(cfaction::MEMBERS)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 8)
            .and_then(|address| pointer(memory, address, 0))
            .filter(|&country| of_country(memory, country).is_some())
        else {
            return false;
        };
        visit(id);
    }
    true
}

pub fn of_link(memory: &impl Memory, base: usize, raw: u32) -> Option<u32> {
    if raw == 0 {
        return None;
    }
    let state = pointer(
        memory,
        engine_sites::at(base, cgame_state::SINGLETON_RVA)?,
        0,
    )?;
    let links = Array::read(memory, state.checked_add(cgame_state::COUNTRY_LINKS)?, 4)?;
    let slot = read::<u32>(memory, links.element(raw, 4)?)?;
    if slot == 0 {
        return None;
    }
    let countries = Array::read(memory, state.checked_add(cgame_state::COUNTRIES)?, 8)?;
    of_country(memory, pointer(memory, countries.element(slot, 8)?, 0)?)
}

fn country_of_link(memory: &impl Memory, base: usize, raw: u32) -> Option<usize> {
    if raw == 0 {
        return None;
    }
    let state = pointer(
        memory,
        engine_sites::at(base, cgame_state::SINGLETON_RVA)?,
        0,
    )?;
    let links = Array::read(memory, state.checked_add(cgame_state::COUNTRY_LINKS)?, 4)?;
    let slot = read::<u32>(memory, links.element(raw, 4)?)?;
    if slot == 0 {
        return None;
    }
    let countries = Array::read(memory, state.checked_add(cgame_state::COUNTRIES)?, 8)?;
    pointer(memory, countries.element(slot, 8)?, 0)
}

fn of_country(memory: &impl Memory, country: usize) -> Option<u32> {
    if country == 0 {
        return None;
    }
    read::<u32>(memory, country.checked_add(ccountry::ID)?)
}

fn status_country(memory: &impl Memory, status: usize) -> Option<u32> {
    pointer(memory, status, ctechnology_status::COUNTRY)
        .and_then(|country| of_country(memory, country))
}

fn link_at(memory: &impl Memory, base: usize, object: usize, offset: usize) -> Option<u32> {
    if object == 0 {
        return None;
    }
    let raw = read::<u32>(memory, object.checked_add(offset)?)?;
    of_link(memory, base, raw)
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Option<usize> {
    if object == 0 {
        return None;
    }
    usize::try_from(read::<u64>(memory, object.checked_add(offset)?)?)
        .ok()
        .filter(|&pointer| pointer != 0)
}

#[cfg(test)]
mod tests;
