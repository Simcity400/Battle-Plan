//! Runtime occupation hover writers. Resistance methods name their embedded state;
//! legitimacy methods name the exile diplomacy; relation rebuild names its relation.
//! Country modifier rebuild/reset, state resource-accumulator rebuild, state lifetime and
//! exile relationship/reset writers are observed through existing patch owners.
//! Save member loading and world construction are covered by the load boundary.
//!
//! Level integration and expiry run in occupation updates. Effects and script setters can
//! change them between those updates. Spread reset/addition and speed rebuilding run in
//! the garrison refresh. Local modifier rebuilding writes the stored law scaling ratio.
//! Added targets have one appender, one explicit remover, reset and daily expiry.
//! Daily diplomacy expires war goals and updates exile legitimacy; this family
//! owns that shared update and forwards it to the goal observer.
//! Exile legitimacy has daily, script, convoy and setter writers; country inputs own
//! the script/convoy/setter and exile entry/return patches and forward their receivers. Relation accumulator clear/copy
//! rebuilding is enclosed by its rebuild method; relation reset is forwarded by opinions.

use engine_sites::Rva;
#[derive(Clone, Copy)]
pub enum Receiver {
    Resistance,
    Diplomacy,
    Relation,
    OccupationData,
    OccupationHolder,
}
#[derive(Clone, Copy)]
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
    pub receiver: Receiver,
}

pub const SITES: [Site; 25] = [
    Site {
        name: "update_levels",
        rva: Rva(0x101c5e0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "update_speeds",
        rva: Rva(0x1020520),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "reset",
        rva: Rva(0x101c1f0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "stop",
        rva: Rva(0x101be20),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "start",
        rva: Rva(0x101f3f0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "change_occupied_country",
        rva: Rva(0x101fba0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "transfer_compliance",
        rva: Rva(0x101fc70),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "reset_target",
        rva: Rva(0x10204d0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "add_spread",
        rva: Rva(0x10167a0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "update_effects",
        rva: Rva(0x101bbd0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "build_local_modifiers",
        rva: Rva(0x1015e50),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "change_compliance",
        rva: Rva(0x1016580),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "change_resistance",
        rva: Rva(0x1016650),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "set_compliance",
        rva: Rva(0x1020760),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "set_resistance",
        rva: Rva(0x10209e0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "remove_added_target",
        rva: Rva(0x1020700),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "add_target",
        rva: Rva(0x10166b0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "daily_diplomacy_update",
        rva: Rva(0xd9f420),
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "rebuild_relation_modifiers",
        rva: Rva(0xd8dd80),
        receiver: Receiver::Relation,
    },
    Site {
        name: "check_activation",
        rva: Rva(0x1020d90),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "initialize_occupation",
        rva: Rva(0x101f5e0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "rebuild_state_active_definitions",
        rva: Rva(0x0102_0e70),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "clear_state_active_definitions",
        rva: Rva(0x0101_c3f0),
        receiver: Receiver::Resistance,
    },
    Site {
        name: "rebuild_core_active_definitions",
        rva: Rva(0x0108_4a30),
        receiver: Receiver::OccupationData,
    },
    Site {
        name: "clear_holder_active_definitions",
        rva: Rva(0x0107_cb70),
        receiver: Receiver::OccupationHolder,
    },
];
