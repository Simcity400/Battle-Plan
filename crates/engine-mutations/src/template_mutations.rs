//! Native writers of retained division-template data and country template
//! configuration. Assignment, token loading, reset, grid writes and resizing,
//! derived-cache rebuilding, scalar/equipment/model setters and AI-role
//! rebuilding certify the template object. Effect writers are scoped to their
//! canonical country, so inline lock/cap stores also revoke that owner's rows.
//! The deleting destructor prevents address reuse from preserving a certificate.
//! The custom-icon command resolves its template natively and stores the icon
//! fields directly, so it revokes every template.
//!
//! Deployment selection and occupation reset/default/group/state setters
//! certify their holders. Occupation insertion and group/status destruction revoke
//! membership. Country loading and world replacement also invalidate capture epochs.
//! Deployment reset uses the existing local-subunit writer certificate.
//! The occupation insertion forwards its fifth argument explicitly.
//! Nothing here calls the game or obtains a fact by evaluating native code.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// How a site's RCX names the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// The contained data, `template+DATA`.
    Data,
    /// The template.
    Template,
    /// A deployment or occupation holder, watched by its own address.
    Object,
    /// An occupation group whose parent status is stored at its second word.
    OccupationGroup,
    /// The embedded occupation membership table, relative to its status.
    OccupationTable,
    /// An effect's second argument holds its country scope.
    ScopeCountry,
    /// A command whose receiver retains the raw owning-country link.
    CommandCountry,
    /// No receiver is resolved: the writer revokes every template.
    Every,
}

/// The contained data's offset in the template
/// (`creferenced_division_template::COMPOSITION`).
pub const DATA: usize = crate::creferenced_division_template::COMPOSITION;
/// Embedded occupation membership map, before its allocator-owned data header.
pub const OCCUPATION_MEMBERSHIP: usize = 0x40;
/// Occupation group's parent status pointer.
pub const OCCUPATION_PARENT: usize = 0x08;
/// Country tag in an authored effect's execution scope.
pub const EFFECT_SCOPE_COUNTRY: usize = 0x08;
pub const COMMAND_COUNTRY: usize = 0x28;

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 60] = [
    // Editable-data assignment (dest, source): saving RSI and reserving 0x20
    // stack bytes occupy six relocated bytes.
    (
        MutationSite {
            name: "assign_editable_template_data",
            rva: Rva(0xbf_18e0),
        },
        Receiver::Data,
    ),
    // Full-data assignment (dest, source): the RBX spill to RCX's home slot relocates.
    (
        MutationSite {
            name: "assign_template_data",
            rva: Rva(0xbe_92a0),
        },
        Receiver::Data,
    ),
    // Data reader (data, reader, token): the RBX spill to RCX's home slot relocates.
    (
        MutationSite {
            name: "read_template_data",
            rva: Rva(0xbf_5d60),
        },
        Receiver::Data,
    ),
    // Primary deleting destructor (template, flags): the RBX spill to RCX's home slot
    // relocates.
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
    // Role edits can rewrite the excluded-variant payload without changing its header.
    (
        MutationSite {
            name: "update_template_tracked_variant",
            rva: Rva(0xbf_87d0),
        },
        Receiver::Data,
    ),
    // Custom-icon command (command): writes the icon index and its flag past every other site.
    (
        MutationSite {
            name: "set_template_symbol",
            rva: Rva(0x1c2_cce0),
        },
        Receiver::Every,
    ),
    // Scripted template additions (effect, scope) rename an existing template whose name the new one takes.
    (
        MutationSite {
            name: "add_template_effect",
            rva: Rva(0x36_2650),
        },
        Receiver::Every,
    ),
    (
        MutationSite {
            name: "add_colonial_template_effect",
            rva: Rva(0x36_8a50),
        },
        Receiver::Every,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The data receiver is the template's contained data, and the sites are
    /// distinct.
    #[test]
    fn data_sites_name_the_contained_data() {
        assert_eq!(DATA, 0x18);
        assert_eq!(crate::creferenced_division_template::NAME_KEY, DATA + 0x28);
        let mut rvas = MUTATIONS.map(|(site, _)| site.rva);
        rvas.sort_unstable();
        assert!(rvas.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
