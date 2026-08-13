//! Wave-5 lane GP-N6: XMAP00 #20, the current victory-point value producer.
//!
//! # This crate owes a carrier whose CONSUMER DOES NOT EXIST YET
//!
//! That makes it different from `xmap17_labels`, `xmap06_border` and
//! `xui26_frontend`, and the difference is recorded rather than smoothed over.
//! Those three owe a carrier whose replay-side decoder is already written, so
//! the schema each producer must emit is readable today. XMAP00 #20 has no
//! decoder: what exists is an admission REQUIREMENT in `map_viewer`'s map
//! authority (`map_viewer/src/map_authority.rs:64`, requirement text at `:516`)
//! that says the row is unavailable until a packet proves it.
//!
//! Consequently [`OWED_CARRIERS`] is EMPTY and [`REQUIRED_SCHEMA_UNREADABLE`]
//! states why, naming the one step that closes it. This is deliberate. The
//! alternative — inventing a plausible key and version so the table looks
//! uniform — would hand the Wave-5 lane a fabricated contract to build against,
//! which is precisely the failure mode this scaffold exists to prevent. An
//! honest "not readable yet, here is what would make it readable" is a finding;
//! a fabricated version is a defect.
//!
//! # What the row actually demands
//!
//! The requirement is owned by packet XMAP21 and reads: prove the exact live
//! furniture value, identity, cadence, completeness, and authenticated
//! producer/content joins (`map_authority.rs:516`–`:518`). A runtime read is
//! needed for the value; the rest is certification.

use compact_contract::pending_producer::{PendingProducerCrate, PendingProducerSlot};

/// This crate's pending-producer declaration.
pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "victory_points",
    ledger_row: "XMAP00 #20 (map_furniture.current_victory_points)",
    blocked_on: "RE of the runtime victory-point value producer, then packet XMAP21's proof of \
                 exact live furniture value, identity, cadence and completeness",
    reason: "the live furniture value is runtime state with no proven passive read, and the row \
             has no replay-side decoder yet, so neither the read nor the carrier contract exists",
};

/// Empty ON PURPOSE — see the module docs.
///
/// A carrier may only be declared here once a consumer names its key and pins
/// its schema. Until then the honest declaration is that there is none.
pub const OWED_CARRIERS: [PendingProducerSlot; 0] = [];

/// Why no required schema is recorded, and the single step that would let one
/// be recorded.
///
/// "Could not determine X" is a finding that names the step which closes it,
/// never a conclusion.
pub const REQUIRED_SCHEMA_UNREADABLE: &str =
    "no replay-side decoder names a carrier key or pins a schema for \
     map_furniture.current_victory_points; the row exists only as an admission requirement \
     (map_viewer/src/map_authority.rs:64, :516). CLOSING STEP: when packet XMAP21 mints the \
     carrier, read its key and version off that decoder and record them here before writing \
     any producer.";

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
        assert!(error.contains("victory_points"), "{error}");
        assert!(error.contains("XMAP00 #20"), "{error}");
    }

    /// kills: populating `OWED_CARRIERS` with an invented key/version to make
    /// the table look uniform -- the Wave-5 lane would build against a contract
    /// no consumer agreed to. If a decoder later appears, this test is updated
    /// together with the entry that the decoder justifies.
    #[test]
    fn no_carrier_is_declared_while_no_consumer_pins_one() {
        assert!(OWED_CARRIERS.is_empty());
        assert!(REQUIRED_SCHEMA_UNREADABLE.contains("CLOSING STEP:"));
        PENDING.validate().expect("the crate declaration must be well formed");
    }
}
