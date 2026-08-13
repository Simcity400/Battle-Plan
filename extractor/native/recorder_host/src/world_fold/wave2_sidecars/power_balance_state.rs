//! `_r2_power_balance_state_live` — the all-country power-balance state carrier.
//! Owed by EIR-NE-006; filled by lane GP-U1.
//!
//! # Not the same carrier as the one the fold already emits
//!
//! `world_fold.rs::build_snapshot` already emits `_r2_power_balance_presentation_live`
//! and the two names are one word apart. They are different contracts:
//! `..._presentation_live` is the existing open-panel presentation carrier;
//! `..._state_live` is the passive all-country STATE this row demands. A lane
//! that renamed the existing key would ship POV-local panel data under a
//! contract that promises all-country coverage.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:29643`; the gate at `:29677` demands
//! `schema_version == 1` and authority
//! `exact_a729_all_country_power_balance_manager`. Note the authority does NOT
//! carry the `passive_` infix its siblings use.
//!
//! It also pins `completeness_record_identity` to the literal
//! `format!("d74-state:{semantic_receipt_sha256}")` (`:29683`) and requires the
//! embedded D18 roster's `frame_hour` and `expected_country_count` to agree with
//! the carrier's own (`:29685`–`:29688`).
//!
//! # Null tolerance
//!
//! `:29660` returns `DomainUnavailable` for an explicit null.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_power_balance_state_live",
        scope: "root",
        ledger_row: "EIR-NE-006",
        consumer_citation: "replay_core/src/world.rs:29677",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "no all-country power-balance state producer; d74 is Reserved because its \
                 open-panel-only contract cannot represent ordinary passive closed hours",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.power_balance_state.clone().unwrap_or(Value::Null))
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U1 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U1's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.power_balance_state = produced;
}
