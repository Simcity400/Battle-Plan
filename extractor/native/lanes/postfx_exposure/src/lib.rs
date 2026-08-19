use compact_contract::pending_producer::{PendingProducerCrate, PendingProducerSlot};

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

pub const OWED_CARRIERS: [PendingProducerSlot; 0] = [];

pub const REQUIRED_SCHEMA_UNREADABLE: &str =
    "no replay-side decoder names a carrier key or pins a schema for \
     postfx.temporal_exposure_state; the row exists only as an admission requirement \
     (map_viewer/src/map_authority.rs:79, :483). CLOSING STEP: settle whether a \
     presentation-independent exposure state exists during the batched RE campaign; if it does \
     and packet XMAP10 mints the carrier, read its key and version off that decoder and record \
     them here before writing any producer. If it does not, this lane is an owner ruling on \
     scope, not a producer.";

pub fn install() -> Result<(), String> {
    Err(PENDING.refusal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unbuilt_producer_refuses_installation_by_name() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert!(error.contains("postfx_exposure"), "{error}");
        assert!(error.contains("XMAP00 #35"), "{error}");
    }

    #[test]
    fn no_carrier_is_declared_and_the_prior_question_is_named() {
        assert!(OWED_CARRIERS.is_empty());
        assert!(REQUIRED_SCHEMA_UNREADABLE.contains("CLOSING STEP:"));
        assert!(PENDING.blocked_on.contains("presentation-INDEPENDENT"));
        PENDING
            .validate()
            .expect("the crate declaration must be well formed");
    }
}
