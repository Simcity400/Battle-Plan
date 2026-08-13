//! Wave-5 lane GP-N7: XMAP00 #35, the temporal exposure state producer.
//!
//! # Same shape as `victory_points`: a carrier with no consumer yet
//!
//! There is no replay-side decoder for `postfx.temporal_exposure_state`, only
//! an admission requirement (`map_viewer/src/map_authority.rs:79`, requirement
//! text at `:483`). [`OWED_CARRIERS`] is therefore empty and
//! [`REQUIRED_SCHEMA_UNREADABLE`] names the step that would let a carrier be
//! declared. Inventing a key and a version to fill the row would hand the
//! Wave-5 lane a contract nothing agreed to.
//!
//! # The word "presentation-independent" is the whole difficulty
//!
//! The requirement is that XMAP10 prove a PRESENTATION-INDEPENDENT temporal
//! exposure state (`map_authority.rs:483`–`:485`). Exposure is, by default, a
//! property of how a frame is being drawn — which is exactly what this recorder
//! is forbidden to record, since it captures host-visible authoritative state
//! and not client rendering. A producer therefore has to find exposure state
//! that exists independently of the local presentation, or establish that none
//! does. That question is upstream of any offset, and it is what the RE session
//! must settle first.

use compact_contract::pending_producer::{PendingProducerCrate, PendingProducerSlot};

/// This crate's pending-producer declaration.
pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "postfx_exposure",
    ledger_row: "XMAP00 #35 (postfx.temporal_exposure_state)",
    blocked_on: "RE establishing whether a presentation-INDEPENDENT temporal exposure state \
                 exists at all, then packet XMAP10's proof of that state, its cadence and its \
                 authenticated producer contract",
    reason: "exposure is ordinarily a property of local presentation, which this recorder does \
             not capture; no presentation-independent exposure state has been shown to exist, \
             and the row has no replay-side decoder",
};

/// Empty ON PURPOSE — see the module docs and `victory_points`.
pub const OWED_CARRIERS: [PendingProducerSlot; 0] = [];

/// Why no required schema is recorded, and the single step that closes it.
pub const REQUIRED_SCHEMA_UNREADABLE: &str =
    "no replay-side decoder names a carrier key or pins a schema for \
     postfx.temporal_exposure_state; the row exists only as an admission requirement \
     (map_viewer/src/map_authority.rs:79, :483). CLOSING STEP: settle whether a \
     presentation-independent exposure state exists during the batched RE campaign; if it does \
     and packet XMAP10 mints the carrier, read its key and version off that decoder and record \
     them here before writing any producer. If it does not, this lane is an owner ruling on \
     scope, not a producer.";

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
        assert!(error.contains("postfx_exposure"), "{error}");
        assert!(error.contains("XMAP00 #35"), "{error}");
    }

    /// kills: populating `OWED_CARRIERS` with an invented key/version, or
    /// dropping the "does a presentation-independent state exist at all"
    /// question from the blocker -- the lane would then be scoped as an
    /// ordinary read when its first question is whether the read is possible.
    #[test]
    fn no_carrier_is_declared_and_the_prior_question_is_named() {
        assert!(OWED_CARRIERS.is_empty());
        assert!(REQUIRED_SCHEMA_UNREADABLE.contains("CLOSING STEP:"));
        assert!(PENDING.blocked_on.contains("presentation-INDEPENDENT"));
        PENDING.validate().expect("the crate declaration must be well formed");
    }
}
