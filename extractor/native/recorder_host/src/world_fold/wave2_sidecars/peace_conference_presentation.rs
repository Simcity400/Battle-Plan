//! `_r2_peace_conference_presentation_exact` — the XUI08 passive global
//! peace-conference carrier. Owed by EIR-NE-001; filled by lane GP-U8.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:34673`; the envelope at `:34714` compares
//! `schema_version` against `peace_conference_presentation::XUI08_SCHEMA_VERSION`,
//! which is **3** (`replay_core/src/peace_conference_presentation.rs:12`).
//!
//! Unlike its siblings this gate is written as a `let base_valid = ... && ...`
//! chain rather than an `if ... { reject }`, so a reader grepping for
//! `schema_version !=` does not find it. The requirement is identical.
//!
//! Every authored text in the carrier carries its own SHA-256, and the consumer
//! recomputes it (`:34705`–`:34710`, `text_sha256`): a producer that transcribes
//! engine text must hash exactly the bytes it shipped.
//!
//! # Null tolerance
//!
//! `:34685` returns `DomainUnavailable` for an explicit null.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_peace_conference_presentation_exact",
        scope: "root",
        ledger_row: "EIR-NE-001",
        consumer_citation: "replay_core/src/world.rs:34714",
        required_schema: RequiredSchema::Version(3),
        wire_absence: WireAbsence::TypedNull,
        reason: "no passive global peace-conference producer; the existing d97 carrier is \
                 open-panel and cannot represent a closed passive hour",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(
        feed.peace_conference_presentation
            .clone()
            .unwrap_or(Value::Null),
    )
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U8 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U8's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.peace_conference_presentation = produced;
}
