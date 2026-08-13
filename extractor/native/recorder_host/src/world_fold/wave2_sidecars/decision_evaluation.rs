//! `_r2_decision_evaluation_exact` — the XUI19 passive all-country decision
//! evaluation carrier. Owed by EIR-NE-007; filled by lane GP-U3.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/decision_evaluation.rs:14` names the key; the gate at `:397`
//! compares `carrier.schema` against `XUI19_SCHEMA`, which is the TOKEN
//! `"xui19.decision_evaluation_exact.v1"` (`:15`) — a string compared whole, not
//! an integer field. A producer emitting `schema_version: 1` satisfies nothing
//! here; the field is `schema` and it is text.
//!
//! # What the consumer refuses to be
//!
//! The module header states it plainly (`:1`–`:6`): this is a replay-file
//! admission boundary, it accepts only an atomic current-hour census keyed to
//! the complete D18 roster, and it never evaluates Clausewitz triggers, reads
//! recorder UI state, or adapts the retired selected-Present decision-panel
//! carrier. The extractor's existing `_r2_decision_panel_live` is exactly that
//! retired carrier, so it cannot be re-pointed at this key.
//!
//! # Null tolerance
//!
//! `:384` returns an explicitly worded unavailable — "XUI19 passive decision
//! carrier was explicitly unobtained" — for a null. This is the one consumer in
//! the set whose null arm carries its own prose reason, so the typed null is
//! meaningfully better than absence here rather than merely tidier.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_decision_evaluation_exact",
        scope: "root",
        ledger_row: "EIR-NE-007",
        consumer_citation: "replay_core/src/decision_evaluation.rs:397",
        required_schema: RequiredSchema::Token("xui19.decision_evaluation_exact.v1"),
        wire_absence: WireAbsence::TypedNull,
        reason: "no passive all-country decision-evaluation producer; the retired \
                 selected-Present decision panel is explicitly not adaptable to this contract",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.decision_evaluation.clone().unwrap_or(Value::Null))
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U3 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U3's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.decision_evaluation = produced;
}
