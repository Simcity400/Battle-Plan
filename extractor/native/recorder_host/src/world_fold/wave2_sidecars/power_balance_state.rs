//! `_r2_power_balance_state_live` — passive all-country power-balance state.
//!
//! Domain 74 copies the exact-a729 global manager independently of player POV
//! and panel state. The fold delegates PBS1 parsing to `d74_grammar`, computes
//! checked aggregates and the frozen D74S1 receipt, and stores the strict
//! schema-v1 value emitted here. Any failed input leaves a typed JSON null.
//!
//! `_r2_power_balance_presentation_live` remains a separate typed-null carrier;
//! manager state is never projected into presentation fields.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed};
use crate::world_fold::{WorldFold, POWER_BALANCE_STATE_DOMAIN_ID};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_power_balance_state_live",
        scope: "root",
        ledger_row: "EIR-NE-006",
        consumer_citation: "replay_core/src/world.rs:30575",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "domain 74 passively produces the exact-a729 all-country power-balance manager census; an affected hour remains typed-null when its PBS1 payload, D18 custody, checked aggregate arithmetic, or mod-aware effective-content manifest is unavailable",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.power_balance_state.clone().unwrap_or(Value::Null))
}

fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    // The unknown verdict outranks the accumulator. A domain-hour that any
    // record marked unknown publishes the typed null even if an earlier record
    // in the same hour had left a complete state behind.
    feed.power_balance_state = if fold.fold_domain_unknown(POWER_BALANCE_STATE_DOMAIN_ID) {
        None
    } else {
        fold.power_balance_state_this_hour.clone()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // kills: emitting anything but the passive manager state through this
    // carrier — the panel-shaped `_r2_power_balance_presentation_live` key is a
    // separate, always-null carrier and must never be fed from here.
    #[test]
    fn fill_reads_only_the_passive_manager_state_field() {
        let mut fold = WorldFold::new("d74-sidecar-separation");
        fold.power_balance_state_this_hour = Some(serde_json::json!({"schema_version": 1}));
        let mut feed = Wave2Feed::unfed();
        (CARRIER.fill)(&fold, &mut feed);
        assert_eq!(
            emit(&feed),
            SidecarEmission::Single(serde_json::json!({"schema_version": 1}))
        );
    }

    // kills: turning missing/failed d74 state into key absence changes this
    // from the consumer's typed DomainUnavailable sentinel.
    #[test]
    fn an_unfed_state_is_a_typed_json_null() {
        assert_eq!(
            emit(&Wave2Feed::unfed()),
            SidecarEmission::Single(Value::Null)
        );
    }

    // kills: reading the accumulator without the unknown verdict. The stream
    // admits several DOMAIN_DATA records per domain per hour, so a hour that a
    // later record declared unknown would otherwise publish the earlier
    // record's state as if it were the whole hour.
    #[test]
    fn an_unknown_domain_hour_publishes_the_typed_null_not_the_accumulator() {
        let mut fold = WorldFold::new("d74-sidecar-unknown");
        fold.power_balance_state_this_hour = Some(serde_json::json!({"schema_version": 1}));
        fold.mark_fold_domain_unknown(
            POWER_BALANCE_STATE_DOMAIN_ID,
            "compact_status_failure",
            "d74 declared unknown by a later record this hour",
        );
        let mut feed = Wave2Feed::unfed();
        (CARRIER.fill)(&fold, &mut feed);
        assert_eq!(emit(&feed), SidecarEmission::Single(Value::Null));
    }
}
