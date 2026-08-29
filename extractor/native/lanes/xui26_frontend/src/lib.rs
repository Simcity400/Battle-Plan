use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xui26_frontend",
    ledger_row: "EIR-NE-015",
    blocked_on: "an owner decision on EIR-NE-015's element-semantics requirement, which asks the \
                 extractor for resolved per-element panel presentation",
    reason: "the blocker is not the frontend capture this slot used to name. Two of the eleven \
             payload families are already carried by domain 57 on captures that exist, and a \
             third's file hash is recorded every hour, so a capture is not what those are \
             missing; and no family can be admitted at all without a complete resolved \
             element-semantic map covering the panel's authored roles, which is POV-local \
             presentation the owner ruled out of capture scope on 2026-07-28",
};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "_r2_xui26_frontend_v1",
    scope: "root",
    ledger_row: "EIR-NE-015",
    consumer_citation: "replay_core/src/xui26_frontend.rs:14",
    required_schema: RequiredSchema::Token("replay.xui26-frontend.v1"),
    wire_absence: WireAbsence::TypedNull,
    reason: "no frontend producer, and the carrier cannot be sealed without resolved element \
             semantics for the panel's whole authored role set",
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

    /// kills: restoring the "no capture has yet exercised the path" blocker.
    /// Two of the eleven payload families -- hotjoin and matchmaking chat --
    /// are already folded from domain 57 on captures that exist, so a blocker
    /// naming a capture as the missing thing would send the next lane to ask
    /// the owner for a session it does not need, and would still not name the
    /// element-semantic map that blocks all eleven.
    #[test]
    fn the_blocker_is_the_owner_scope_question_not_a_missing_capture() {
        let blocked = PENDING.blocked_on.to_ascii_lowercase();
        assert!(
            !blocked.contains("capture"),
            "the frontend-capture blocker was refuted: {}",
            PENDING.blocked_on
        );
        assert!(blocked.contains("owner decision"), "{}", PENDING.blocked_on);
        assert!(
            PENDING.reason.contains("element-semantic"),
            "{}",
            PENDING.reason
        );
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
