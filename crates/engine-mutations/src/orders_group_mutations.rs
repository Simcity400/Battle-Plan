
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    First,
    Second,
}

pub const DAILY_UPDATE: usize = 3;

pub const MUTATIONS: [(MutationSite, Receiver); 6] = [
    (site("set_leader", Rva(0xc4_b460)), Receiver::First),
    (site("set_training", Rva(0xc4_bd10)), Receiver::First),
    (
        site("assign_to_army_group", Rva(0xc4_03a0)),
        Receiver::Second,
    ),
    (site("daily_update", Rva(0xc4_1cd0)), Receiver::First),
    (site("set_deployed_leader", Rva(0xc4_b600)), Receiver::First),
    (site("clear_hq_provider", Rva(0xc4_98c0)), Receiver::First),
];

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
}

