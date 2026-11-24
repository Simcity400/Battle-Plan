
use engine_sites::Rva;
pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

const RESET_RVA: Rva = Rva(0xd7fdf0);
const LOAD_TOKEN_RVA: Rva = Rva(0xd8cd80);

pub const MUTATIONS: [MutationSite; 4] = [
    MutationSite {
        name: "recompute_opinion",
        rva: Rva(0xd8f350),
    },
    MutationSite {
        name: "reset",
        rva: RESET_RVA,
    },
    MutationSite {
        name: "load_token",
        rva: LOAD_TOKEN_RVA,
    },
    MutationSite {
        name: "construct",
        rva: engine_native::crelation::status::CONSTRUCTOR_RVA,
    },
];

pub const RELATIONS: [MutationSite; 15] = [
    MutationSite {
        name: "air_relation",
        rva: Rva(0xd7_6610),
    },
    MutationSite {
        name: "attache_relation",
        rva: Rva(0xd7_68c0),
    },
    MutationSite {
        name: "boost_party_relation",
        rva: Rva(0xd7_6b70),
    },
    MutationSite {
        name: "subject_relation",
        rva: Rva(0xd7_6e20),
    },
    MutationSite {
        name: "docking_relation",
        rva: Rva(0xd7_70d0),
    },
    MutationSite {
        name: "embargo_relation",
        rva: Rva(0xd7_7380),
    },
    MutationSite {
        name: "equipment_purchase_relation",
        rva: Rva(0xd7_7630),
    },
    MutationSite {
        name: "guarantee_relation",
        rva: Rva(0xd7_78e0),
    },
    MutationSite {
        name: "improve_relation",
        rva: Rva(0xd7_7b90),
    },
    MutationSite {
        name: "lend_lease_relation",
        rva: Rva(0xd7_7e40),
    },
    MutationSite {
        name: "licence_relation",
        rva: Rva(0xd7_80f0),
    },
    MutationSite {
        name: "market_access_relation",
        rva: Rva(0xd7_83a0),
    },
    MutationSite {
        name: "military_access_relation",
        rva: Rva(0xd7_8650),
    },
    MutationSite {
        name: "blockade_relation",
        rva: Rva(0xd7_8900),
    },
    MutationSite {
        name: "non_aggression_relation",
        rva: Rva(0xd7_8bb0),
    },
];

