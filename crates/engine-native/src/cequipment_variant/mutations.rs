use engine_sites::Rva;
#[derive(Clone, Copy)]
pub enum Receiver {
    Variant,
    Command,
    Published,
    Removed,
    Roster,
    CreateCommand,
}
pub struct Mutation {
    pub name: &'static str,
    pub rva: Rva,
    pub receiver: Receiver,
    pub witness: Option<usize>,
}
pub const COMMAND_VARIANT: usize = 0x28;
pub const REBUILD_STATS_RVA: Rva = Rva(0x00c35f90);
pub const FORWARDED_SITES: [usize; 3] = [2, 3, 9];
pub const WRITER_AUDIT_REFUSAL: Option<&str> = None;
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        name: "RebuildStats",
        rva: REBUILD_STATS_RVA,
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "RefreshDerivedState",
        rva: Rva(0x00c2e110),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "RefreshOrganisation",
        rva: Rva(0x00c37c60),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "RefreshOrganisationBonus",
        rva: Rva(0x00c29860),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "SetObsolete",
        rva: Rva(0x00c37850),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "PropagateObsolete",
        rva: Rva(0x00c378c0),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "SetHighlight",
        rva: Rva(0x00c37570),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "EnableUpgradeVariant",
        rva: Rva(0x00c383a0),
        receiver: Receiver::Variant,
        witness: Some(0x428),
    },
    Mutation {
        name: "RefreshNiches",
        rva: Rva(0x00c37dc0),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "MoveAssign",
        rva: Rva(0x00673bc0),
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "Publish",
        rva: Rva(0x001e0dc0),
        receiver: Receiver::Published,
        witness: None,
    },
    Mutation {
        name: "Destroy",
        rva: Rva(0x00c27030),
        receiver: Receiver::Removed,
        witness: None,
    },
    Mutation {
        name: "Rename",
        rva: Rva(0x01a39650),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetAutoUpgraded",
        rva: Rva(0x01a397e0),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetNamesGroup",
        rva: Rva(0x01a399e0),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetRoleIcon",
        rva: Rva(0x01a39b50),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetModel",
        rva: Rva(0x01a39d90),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetIcon",
        rva: Rva(0x01a39ee0),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "Update",
        rva: Rva(0x01a3a3c0),
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "BulkUpgradeModules",
        rva: Rva(0x00c37ed0),
        receiver: Receiver::Roster,
        witness: None,
    },
    Mutation {
        name: "BulkUpgradeTypes",
        rva: Rva(0x00c38280),
        receiver: Receiver::Roster,
        witness: None,
    },
    Mutation {
        name: "BulkUpgradeVariants",
        rva: Rva(0x00c34e60),
        receiver: Receiver::Roster,
        witness: None,
    },
    Mutation {
        name: "CreateCommand",
        rva: Rva(0x01a38540),
        receiver: Receiver::CreateCommand,
        witness: None,
    },
];

