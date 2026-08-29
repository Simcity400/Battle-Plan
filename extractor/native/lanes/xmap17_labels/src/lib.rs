use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xmap17_labels",
    ledger_row: "EIR-XMAP17-FACTION-LABEL-TEXT",
    blocked_on: "collector emission for the built faction-label producer in \
                 map_passive_carriers_live_lane::xmap17",
    reason: "the read law is proven and the producer is written and joint-certified against the \
             consumer's own bytes; what is missing is the wire slot -- nothing calls \
             build_faction_labels_carrier from the collector's D53 faction walk, so the key never \
             reaches a recording",
};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "_xmap17_faction_labels_exact",
    scope: "root",
    ledger_row: "EIR-XMAP17-FACTION-LABEL-TEXT",
    consumer_citation: "replay_core/src/xmap17_player_labels.rs:21",
    required_schema: RequiredSchema::Token("xmap17.faction_labels_exact.a729.v1"),
    wire_absence: WireAbsence::OmitKey,
    reason: "the faction-label producer is built but unwired; no collector path emits the key",
}];

pub fn install() -> Result<(), String> {
    Err(PENDING.refusal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unbuilt_producer_refuses_installation_by_name() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert!(error.contains("xmap17_labels"), "{error}");
        assert!(error.contains("map_passive_carriers_live_lane"), "{error}");
    }

    /// kills: re-declaring the player row as an owed producer carrier. It is
    /// derived at replay from the D18 roster and D57's `players_by_country`,
    /// both already recorded, so a producer slot for it would send the next
    /// lane at a carrier that must not exist.
    #[test]
    fn only_the_faction_row_is_owed_and_it_is_owed_a_wire_slot_not_research() {
        assert_eq!(OWED_CARRIERS.len(), 1);
        assert_eq!(
            OWED_CARRIERS[0].required_schema.token(),
            Some("xmap17.faction_labels_exact.a729.v1")
        );
        assert!(
            !OWED_CARRIERS
                .iter()
                .any(|carrier| carrier.wire_key.contains("player")),
            "the player label layer is derived, not produced"
        );
        // The blocker is emission, not reverse engineering: a blocker naming a
        // function would send the next lane back to the disassembler for a law
        // that is already proven and already implemented.
        let blocked = PENDING.blocked_on.to_ascii_lowercase();
        for re_word in ["fun_", "reverse", "ghidra", "decompil", "offset"] {
            assert!(!blocked.contains(re_word), "{}", PENDING.blocked_on);
        }
        for carrier in &OWED_CARRIERS {
            carrier
                .validate()
                .expect("owed carrier must be well formed");
        }
        PENDING
            .validate()
            .expect("the crate declaration must be well formed");
    }
}
