use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xui26_frontend",
    ledger_row: "EIR-NE-015",
    blocked_on: "a live capture taken through the XUI26 frontend path (owner-run session)",
    reason: "the frontend payload families are observable but unmeasured: no capture has yet \
             exercised the path, so no producer can be validated against the engine",
};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "_r2_xui26_frontend_v1",
    scope: "root",
    ledger_row: "EIR-NE-015",
    consumer_citation: "replay_core/src/xui26_frontend.rs:14",
    required_schema: RequiredSchema::Token("replay.xui26-frontend.v1"),
    wire_absence: WireAbsence::TypedNull,
    reason: "no frontend producer; blocked on a capture through the frontend path",
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
        assert!(error.contains("xui26_frontend"), "{error}");
    }

    #[test]
    fn this_lane_is_capture_blocked_not_re_blocked() {
        let blocked = PENDING.blocked_on.to_ascii_lowercase();
        assert!(blocked.contains("capture"), "{}", PENDING.blocked_on);
        for re_word in [
            "reverse",
            "ghidra",
            "fun_",
            "offset",
            "disassembl",
            "decompil",
            "rva",
        ] {
            assert!(
                !blocked.contains(re_word),
                "GP-N3 is capture-blocked, but its blocker names {re_word:?}: {}",
                PENDING.blocked_on
            );
        }
        PENDING
            .validate()
            .expect("the crate declaration must be well formed");
    }

    #[test]
    fn the_owed_carrier_records_the_schema_its_consumer_requires() {
        assert_eq!(
            OWED_CARRIERS[0].required_schema.token(),
            Some("replay.xui26-frontend.v1")
        );
        OWED_CARRIERS[0]
            .validate()
            .expect("owed carrier must be well formed");
    }
}
