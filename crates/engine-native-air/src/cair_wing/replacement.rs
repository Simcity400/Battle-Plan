use engine_sites::Rva;
pub const SETTING: usize = 0x9cc;
pub const COMMAND_WINGS: usize = 0x28;
pub struct Mutation {
    pub name: &'static str,
    pub rva: Rva,
}
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        name: "DeployWing",
        rva: Rva(0x00cad040),
    },
    Mutation {
        name: "SetWingReplacement",
        rva: Rva(0x019d18b0),
    },
];
