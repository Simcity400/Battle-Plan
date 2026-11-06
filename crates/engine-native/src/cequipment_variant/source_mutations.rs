//! Narrow source/provider writer boundaries. Construction is baselined on first
//! demand; reset/destruction invalidate the previous native lifetime.
use engine_sites::Rva;
pub struct Mutation {
    pub name: &'static str,
    pub rva: Rva,
}
pub const BONUS_BODY: usize = 0x38;
pub const IDEA_SOURCES: usize = 0xa88;
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        name: "ReadAppliedSource",
        rva: Rva(0x00679390),
    },
    Mutation {
        name: "ResetAppliedSource",
        rva: Rva(0x00678e80),
    },
    Mutation {
        name: "PostprocessIdeaSources",
        rva: Rva(0x01058c70),
    },
    Mutation {
        name: "DestroyAppliedSource",
        rva: Rva(0x006782c0),
    },
    Mutation {
        name: "AppendNamedSource",
        rva: Rva(0x00ed0d80),
    },
    Mutation {
        name: "RemoveNamedSource",
        rva: Rva(0x00ee0580),
    },
];

pub const ORGANISATIONS: &[Mutation] = &[
    Mutation {
        name: "ReconcileOrganisations",
        rva: Rva(0x00ec94d0),
    },
    Mutation {
        name: "InitializeOrganisations",
        rva: Rva(0x00ee21d0),
    },
    Mutation {
        name: "RepairOrganisationReference",
        rva: Rva(0x00e28570),
    },
    Mutation {
        name: "DestroyOrganisation",
        rva: Rva(0x00e21d60),
    },
];
