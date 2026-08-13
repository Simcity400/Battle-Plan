//! Wave-5 lane GP-N3: the XUI26 frontend carrier.
//!
//! # This one is not RE-blocked, and that difference matters
//!
//! Four of the five Wave-5 skeletons wait on a Ghidra/live-RE session. GP-N3
//! does not: the gap map lists its live need as "capture", not RE
//! (`extractor/documents/REPLAY_PRODUCER_GAP_MAP.md`, Wave 5 table). Recording
//! that here keeps a future reader from batching this lane into the RE campaign
//! and leaving it parked behind twelve reverse-engineering questions it does
//! not have.
//!
//! What it is blocked on instead is a capture taken through the frontend path,
//! which the owner runs personally. That is a live-session dependency, and the
//! reason this crate is a declaration rather than an implementation.
//!
//! # One carrier, twelve payload families
//!
//! The consumer decodes a single key into a tagged union
//! (`replay_core/src/xui26_frontend.rs:16`, `#[serde(tag = "kind")]` with
//! `deny_unknown_fields`), so a producer emits ONE carrier whose payload
//! discriminates. The `deny_unknown_fields` is worth naming: a producer that
//! ships an extra field for its own bookkeeping is rejected outright, not
//! tolerated.

use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

/// This crate's pending-producer declaration.
pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xui26_frontend",
    ledger_row: "EIR-NE-015",
    blocked_on: "a live capture taken through the XUI26 frontend path (owner-run session)",
    reason: "the frontend payload families are observable but unmeasured: no capture has yet \
             exercised the path, so no producer can be validated against the engine",
};

/// The carrier this lane owes.
pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "_r2_xui26_frontend_v1",
    scope: "root",
    ledger_row: "EIR-NE-015",
    consumer_citation: "replay_core/src/xui26_frontend.rs:14",
    required_schema: RequiredSchema::Token("replay.xui26-frontend.v1"),
    // `xui26_frontend.rs:462` has an explicit `if raw.is_null()` arm returning
    // `DomainUnavailable`, so this carrier is null-tolerant.
    wire_absence: WireAbsence::TypedNull,
    reason: "no frontend producer; blocked on a capture through the frontend path",
}];

/// The collector's install boundary asks every producer this.
pub fn install() -> Result<(), String> {
    Err(PENDING.refusal())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kills: replacing `install()`'s body with `Ok(())`.
    #[test]
    fn an_unbuilt_producer_refuses_installation_by_name() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert!(error.contains("xui26_frontend"), "{error}");
    }

    /// kills: rewriting `blocked_on` to name reverse engineering -- GP-N3 is
    /// the one Wave-5 lane that needs only a capture, and mislabelling it parks
    /// it behind an RE campaign it does not depend on.
    ///
    /// Checked by ABSENCE of the RE vocabulary rather than by presence of a
    /// disclaimer. Two earlier forms were both too weak, and both are worth
    /// recording because they are the same mistake at different strengths:
    ///
    ///  1. Asserting the phrase "no new reverse engineering is required" was
    ///     PRESENT. A blocker reading "...that claim is stale; RE is required"
    ///     contains the phrase while asserting the opposite.
    ///  2. Blacklisting `"reverse engineering"` as a whole phrase.
    ///     `"reverse-engineering required before capture"` slips through the
    ///     hyphen while naming RE and keeping the word `capture`.
    ///
    /// So the check is on the STEM `reverse`, which no spelling of the concept
    /// avoids, plus the tool and artefact vocabulary. Its remaining bound is
    /// honest: a blocker could name RE using only the bare initialism in a form
    /// this does not match. That is why the positive `capture` requirement is
    /// kept alongside — the two together make a wrong blocker hard to write by
    /// accident, which is the realistic failure this guards.
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
        PENDING.validate().expect("the crate declaration must be well formed");
    }

    /// kills: changing `required_schema` away from the token the consumer pins.
    #[test]
    fn the_owed_carrier_records_the_schema_its_consumer_requires() {
        assert_eq!(
            OWED_CARRIERS[0].required_schema.token(),
            Some("replay.xui26-frontend.v1")
        );
        OWED_CARRIERS[0].validate().expect("owed carrier must be well formed");
    }
}
