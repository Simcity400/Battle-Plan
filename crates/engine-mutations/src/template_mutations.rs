//! Native writers of retained division-template data and country template
//! configuration. Assignment, token loading, reset, grid writes and resizing,
//! derived-cache rebuilding, scalar/equipment/model setters and AI-role
//! rebuilding certify the template object. Effect writers are scoped to their
//! canonical country, so inline lock/cap stores also revoke that owner's rows.
//! The deleting destructor prevents address reuse from preserving a certificate.
//!
//! Deployment selection and occupation reset/default/group/state setters
//! certify their holders. Occupation insertion and group/status destruction revoke
//! membership. Country loading and world replacement also invalidate capture epochs.
//! Deployment reset uses the existing local-subunit writer certificate.
//! The occupation insertion forwards its fifth argument explicitly.
//! Nothing here calls the game or obtains a fact by evaluating native code.

pub use crate::advisor_mutations::MutationSite;

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
pub const MUTATIONS: [(MutationSite, Receiver); 56] = [
    // Editable-data assignment (dest, source): saving RSI and reserving 0x20
    // stack bytes occupy six relocated bytes.
    (
        MutationSite {
            name: "assign_editable_template_data",
            rva: 0xb9_e550,
            sentinel: [
                0x40, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x89, 0x6c, 0x24, 0x38, 0x48, 0x8b, 0xf1,
                0x48, 0x8b,
            ],
        },
        Receiver::Data,
    ),
    // Full-data assignment (dest, source): the RBX spill to RCX's home slot relocates.
    (
        MutationSite {
            name: "assign_template_data",
            rva: 0xb9_5e30,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Receiver::Data,
    ),
    // Data reader (data, reader, token): the RBX spill to RCX's home slot relocates.
    (
        MutationSite {
            name: "read_template_data",
            rva: 0xba_29f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Data,
    ),
    // Primary deleting destructor (template, flags): the RBX spill to RCX's home slot
    // relocates.
    (
        MutationSite {
            name: "destroy_template",
            rva: 0xb9_70f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xaf,
                0x84, 0xdb,
            ],
        },
        Receiver::Template,
    ),
    (
        MutationSite {
            name: "reset_template_data",
            rva: 0xb9de80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "rebuild_template_caches",
            rva: 0xba41f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_foreign_equipment_permission",
            rva: 0xba4c50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_new_equipment_permission",
            rva: 0xba51e0,
            sentinel: [
                0x88, 0x91, 0xad, 0x01, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regiment_cell",
            rva: 0xba51f0,
            sentinel: [
                0x45, 0x8b, 0xc8, 0x48, 0x83, 0xc1, 0x48, 0x41, 0xc1, 0xe9, 0x10, 0xe9, 0x50, 0xa8,
                0x41, 0x00,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regiment_cell_and_rebuild",
            rva: 0xba5200,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x0f, 0xb6, 0xd9,
                0x48, 0x8b,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_country",
            rva: 0xba5240,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_name_group",
            rva: 0xba5340,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x01, 0xc8,
                0xa3, 0x02,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_support_cell",
            rva: 0xba53f0,
            sentinel: [
                0x45, 0x8b, 0xc8, 0x48, 0x83, 0xc1, 0x68, 0x41, 0xc1, 0xe9, 0x10, 0xe9, 0x50, 0xa6,
                0x41, 0x00,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_support_cell_and_rebuild",
            rva: 0xba5400,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x0f, 0xb6, 0xd9,
                0x48, 0x8b,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_equipment_niche",
            rva: 0xba5660,
            sentinel: [
                0x88, 0x91, 0x44, 0x02, 0x00, 0x00, 0xe9, 0x05, 0x00, 0x00, 0x00, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_foreign_origin",
            rva: 0xba5e50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x80,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_hq_kind",
            rva: 0xba5f00,
            sentinel: [
                0x88, 0x91, 0xb4, 0x01, 0x00, 0x00, 0xe9, 0x75, 0x0a, 0x00, 0x00, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_obsolete",
            rva: 0xba5f10,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x88, 0x91, 0x1e, 0x02, 0x00, 0x00, 0x48, 0x8b,
                0xd9, 0x80,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_model_entity",
            rva: 0xba6030,
            sentinel: [
                0x48, 0x81, 0xc1, 0xf8, 0x01, 0x00, 0x00, 0x48, 0x3b, 0xca, 0x74, 0x13, 0x48, 0x83,
                0x7a, 0x18,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_model_category",
            rva: 0xba6050,
            sentinel: [
                0x0f, 0x10, 0x02, 0x0f, 0x11, 0x81, 0xe8, 0x01, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regimental_support_cell",
            rva: 0xba6120,
            sentinel: [
                0x45, 0x8b, 0xc8, 0x48, 0x81, 0xc1, 0x88, 0x00, 0x00, 0x00, 0x41, 0xc1, 0xe9, 0x10,
                0xe9, 0x1d,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_regimental_support_cell_and_rebuild",
            rva: 0xba6140,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x0f, 0xb6, 0xd9,
                0x48, 0x8b,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_priority",
            rva: 0xba6180,
            sentinel: [
                0x8b, 0x05, 0xb2, 0x11, 0x79, 0x02, 0xff, 0xc8, 0x85, 0xd2, 0x79, 0x0b, 0xc7, 0x81,
                0x8c, 0x01,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_unit_type",
            rva: 0xba61b0,
            sentinel: [
                0x89, 0x91, 0x18, 0x02, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "toggle_template_variant",
            rva: 0xba6230,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8d, 0x99, 0x28,
                0x01, 0x00,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "rebuild_template_ai_role",
            rva: 0xba6630,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "resize_template_grids",
            rva: 0xba6980,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x80, 0xb9, 0xb4, 0x01,
                0x00, 0x00,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "refresh_template_name_group",
            rva: 0xba3da0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56,
                0x48, 0x8d,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "set_template_force_recruiting",
            rva: 0x1fc350,
            sentinel: [
                0x88, 0x91, 0x38, 0x02, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Data,
    ),
    (
        MutationSite {
            name: "effect_template_lock",
            rva: 0x361990,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x81, 0xec,
                0x30, 0x01,
            ],
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_template_cap",
            rva: 0x3647a0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x81,
            ],
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_clear_template_cap",
            rva: 0x350130,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec, 0x30, 0x01, 0x00, 0x00, 0x48,
                0x8b, 0xd9,
            ],
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_remove_template",
            rva: 0x354480,
            sentinel: [
                0x40, 0x55, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x8d, 0x72, 0x08, 0x48,
                0x8b, 0xe9,
            ],
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "effect_country_template_lock",
            rva: 0x35af70,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8d, 0x4a, 0x08, 0xe8,
                0x0e, 0x94,
            ],
        },
        Receiver::ScopeCountry,
    ),
    (
        MutationSite {
            name: "set_staff_template",
            rva: 0xd13630,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x05, 0xeb, 0x9e, 0x62, 0x02, 0x48,
                0x8b, 0xd9,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_default_garrison_template",
            rva: 0xffdde0,
            sentinel: [
                0x48, 0x89, 0x51, 0x78, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_group_garrison_template",
            rva: 0xffe3c0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x89, 0x91, 0xc8,
                0x00, 0x00,
            ],
        },
        Receiver::OccupationGroup,
    ),
    (
        MutationSite {
            name: "set_state_garrison_template",
            rva: 0xffe420,
            sentinel: [
                0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x83, 0xec, 0x48, 0x44, 0x8b, 0x4a, 0x58, 0x4c,
                0x8d, 0x91,
            ],
        },
        Receiver::OccupationGroup,
    ),
    (
        MutationSite {
            name: "reset_occupation_status",
            rva: 0xff5ee0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "insert_occupation_group",
            rva: 0xff0730,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x48, 0x8b,
                0xec, 0x48,
            ],
        },
        Receiver::OccupationTable,
    ),
    (
        MutationSite {
            name: "destroy_occupation_group",
            rva: 0xff3390,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::OccupationGroup,
    ),
    (
        MutationSite {
            name: "destroy_occupation_status",
            rva: 0xff3470,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "allocate_hq_deployment_order",
            rva: 0xd1_12d0,
            sentinel: [
                0x8b, 0x81, 0xa8, 0x00, 0x00, 0x00, 0x8d, 0x50, 0x01, 0x89, 0x91, 0xa8, 0x00, 0x00,
                0x00, 0xc3,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_hq_target",
            rva: 0xbf5960,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x05, 0xbb, 0x7b, 0x74, 0x02, 0x48,
                0x8b, 0xd9,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "begin_hq_deployment",
            rva: 0xbf6370,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "cancel_hq_deployment",
            rva: 0xbebce0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x80, 0xb9, 0x99, 0x01,
                0x00, 0x00,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "destroy_hq_group",
            rva: 0xbe9f90,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "requisition_hq_from_army",
            rva: 0xbf49e0,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "rebuild_hq_requirements",
            rva: 0x193ecc0,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "distribute_hq_equipment",
            rva: 0x193ea80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "add_hq_manpower",
            rva: 0x193e760,
            sentinel: [
                0x85, 0xd2, 0x7e, 0x06, 0x01, 0x91, 0x88, 0x00, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "accept_hq_manpower",
            rva: 0x193e770,
            sentinel: [
                0x4c, 0x8b, 0xc1, 0x8b, 0x89, 0x88, 0x00, 0x00, 0x00, 0x41, 0x8b, 0x80, 0x8c, 0x00,
                0x00, 0x00,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "take_hq_equipment",
            rva: 0x193e820,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "refund_hq_equipment",
            rva: 0x193ec40,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x83, 0xc1, 0x20, 0x83,
                0x39, 0x00,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "destroy_hq_deployment",
            rva: 0x193e690,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x8b,
            ],
        },
        Receiver::Object,
    ),
    (
        MutationSite {
            name: "set_garrison_priority",
            rva: 0x115bc80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x79, 0x28,
                0x48, 0x8b,
            ],
        },
        Receiver::CommandCountry,
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
