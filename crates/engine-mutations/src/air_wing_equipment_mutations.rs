
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const POOL: usize = 0x1d8;

pub const TRAINING_STOP: usize = 0x10c;
pub const ATTACHMENT_COVERAGE: usize = 0x9c0;
pub const MISSION: usize = 0x90;
pub const RAID_REFERENCE: usize = engine_native_air::cair_wing::transport_operands::RAID_REFERENCE;
pub const RAID_LIFETIME_KEY: usize = 1;

pub const MAP_MUTATIONS: [MutationSite; 6] = [
    MutationSite {
        name: "wing_training_stop",
        rva: Rva(0x100_9c80),
    },
    MutationSite {
        name: "wing_attachment_coverage",
        rva: Rva(0xc4_8760),
    },
    MutationSite {
        name: "wing_raid_reference",
        rva: Rva(0xfe_b740),
    },
    MutationSite {
        name: "raid_reference_lifetime",
        rva: Rva(0x106_fc60),
    },
    MutationSite {
        name: "append_incoming_air_wing",
        rva: Rva(0xca_9030),
    },
    MutationSite {
        name: "erase_incoming_air_wing",
        rva: Rva(0xcc_0490),
    },
];

pub const SUBTRACT_VARIANT_RVA: Rva = Rva(0x109_8700);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Wing,
    Wings,
    Pool,
}

pub const MUTATIONS: [(MutationSite, Receiver); 7] = [
    (
        MutationSite {
            name: "add_airplanes",
            rva: Rva(0xfd_f340),
        },
        Receiver::Wing,
    ),
    (
        MutationSite {
            name: "add_equipment_pool",
            rva: Rva(0xfd_f1c0),
        },
        Receiver::Wing,
    ),
    (
        MutationSite {
            name: "move_airplanes",
            rva: Rva(0xfe_8020),
        },
        Receiver::Wings,
    ),
    (
        MutationSite {
            name: "subtract_equipment_pool",
            rva: Rva(0xfe_ad80),
        },
        Receiver::Wing,
    ),
    (
        MutationSite {
            name: "trim_airplanes",
            rva: Rva(0xfe_c83a),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "return_airplanes",
            rva: Rva(0xee_1510),
        },
        Receiver::Pool,
    ),
    (
        MutationSite {
            name: "destroy_wing",
            rva: Rva(0xfd_eb30),
        },
        Receiver::Wing,
    ),
];

pub fn pools(receiver: Receiver, first: usize, second: usize) -> [usize; 2] {
    match receiver {
        Receiver::Wing => [first.wrapping_add(POOL), 0],
        Receiver::Wings => [first.wrapping_add(POOL), second.wrapping_add(POOL)],
        Receiver::Pool => [first, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receivers_name_the_pools() {
        assert_eq!(pools(Receiver::Wing, 0x5000, 7), [0x51d8, 0]);
        assert_eq!(pools(Receiver::Wings, 0x5000, 0x6000), [0x51d8, 0x61d8]);
        assert_eq!(pools(Receiver::Pool, 0x51d8, 0), [0x51d8, 0]);
    }
}
