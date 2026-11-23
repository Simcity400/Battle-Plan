
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Data,
    Template,
    Object,
    OccupationGroup,
    OccupationTable,
    ScopeCountry,
    CommandCountry,
    Every,
}

pub const DATA: usize = engine_native::creferenced_division_template::COMPOSITION;
pub const OCCUPATION_MEMBERSHIP: usize = 0x40;
pub const OCCUPATION_PARENT: usize = 0x08;
pub const EFFECT_SCOPE_COUNTRY: usize = 0x08;
pub const COMMAND_COUNTRY: usize = 0x28;

pub const MUTATIONS: [(MutationSite, Receiver); 60] = [
    (
        MutationSite {
            name: "assign_editable_template_data",
            rva: Rva(0xbf_18e0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "assign_template_data",
            rva: Rva(0xbe_92a0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "read_template_data",
            rva: Rva(0xbf_5d60),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "destroy_template",
            rva: Rva(0xbe_a590),
        },
        Receiver::Template,
    ),
    (
        MutationSite {
            name: "reset_template_data",
            rva: Rva(0xbf1210),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "rebuild_template_caches",
            rva: Rva(0xbf7580),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_foreign_equipment_permission",
            rva: Rva(0xbf7fe0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_new_equipment_permission",
            rva: Rva(0xbf8570),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regiment_cell",
            rva: Rva(0xbf8580),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regiment_cell_and_rebuild",
            rva: Rva(0xbf8590),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_country",
            rva: Rva(0xbf85d0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_name_group",
            rva: Rva(0xbf86d0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_support_cell",
            rva: Rva(0xbf8780),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_support_cell_and_rebuild",
            rva: Rva(0xbf8790),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_equipment_niche",
            rva: Rva(0xbf89f0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_foreign_origin",
            rva: Rva(0xbf91f0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_hq_kind",
            rva: Rva(0xbf92a0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_obsolete",
            rva: Rva(0xbf92b0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_model_entity",
            rva: Rva(0xbf93d0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_model_category",
            rva: Rva(0xbf93f0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regimental_support_cell",
            rva: Rva(0xbf94c0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regimental_support_cell_and_rebuild",
            rva: Rva(0xbf94e0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_priority",
            rva: Rva(0xbf9520),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_unit_type",
            rva: Rva(0xbf9550),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "toggle_template_variant",
            rva: Rva(0xbf95d0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "rebuild_template_ai_role",
            rva: Rva(0xbf99d0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "resize_template_grids",
            rva: Rva(0xbf9d20),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "refresh_template_name_group",
            rva: Rva(0xbf7130),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_force_recruiting",
            rva: Rva(0xbf91e0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "effect_template_lock",
            rva: Rva(0x379f60),
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_template_cap",
            rva: Rva(0x37cd70),
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_clear_template_cap",
            rva: Rva(0x368710),
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_remove_template",
            rva: Rva(0x36ca60),
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_country_template_lock",
            rva: Rva(0x373540),
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "set_staff_template",
            rva: Rva(0xd74f10),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_default_garrison_template",
            rva: Rva(0x1083d10),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_group_garrison_template",
            rva: Rva(0x1084250),
        },
        Receiver::OccupationGroup,
    ),
    (
        MutationSite {
            name: "set_state_garrison_template",
            rva: Rva(0x10842b0),
        },
        Receiver::OccupationGroup,
    ),
    (
        MutationSite {
            name: "reset_occupation_status",
            rva: Rva(0x107beb0),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "insert_occupation_group",
            rva: Rva(0x10762e0),
        },
        Receiver::OccupationTable,
    ),
    (
        MutationSite {
            name: "destroy_occupation_group",
            rva: Rva(0x1079370),
        },
        Receiver::OccupationGroup,
    ),
    (
        MutationSite {
            name: "destroy_occupation_status",
            rva: Rva(0x1079450),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "allocate_hq_deployment_order",
            rva: Rva(0xd7_2b20),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_hq_target",
            rva: Rva(0xc4b3a0),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "begin_hq_deployment",
            rva: Rva(0xc4bdb0),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "cancel_hq_deployment",
            rva: Rva(0xc41550),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "destroy_hq_group",
            rva: Rva(0xc3f800),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "requisition_hq_from_army",
            rva: Rva(0xc4a420),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "rebuild_hq_requirements",
            rva: Rva(0x19c9f80),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "distribute_hq_equipment",
            rva: Rva(0x19c9d40),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "add_hq_manpower",
            rva: Rva(0x19c9a20),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "accept_hq_manpower",
            rva: Rva(0x19c9a30),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "take_hq_equipment",
            rva: Rva(0x19c9ae0),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "refund_hq_equipment",
            rva: Rva(0x19c9f00),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "destroy_hq_deployment",
            rva: Rva(0x19c9950),
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_garrison_priority",
            rva: Rva(0x11e59f0),
        },
        Receiver::CommandCountry,
    ),
    (
        MutationSite {
            name: "update_template_tracked_variant",
            rva: Rva(0xbf_87d0),
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_symbol",
            rva: Rva(0x1c2_cce0),
        },
        Receiver::Every,
    ),
    (
        MutationSite {
            name: "add_template_effect",
            rva: Rva(0x36_2650),
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "add_colonial_template_effect",
            rva: Rva(0x36_8a50),
        },
        Receiver::ScopeCountry,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_sites_name_the_contained_data() {
        assert_eq!(DATA, 0x18);
        assert_eq!(engine_native::creferenced_division_template::NAME_KEY, DATA + 0x28);
        let mut rvas = MUTATIONS.map(|(site, _)| site.rva);
        rvas.sort_unstable();
        assert!(rvas.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
