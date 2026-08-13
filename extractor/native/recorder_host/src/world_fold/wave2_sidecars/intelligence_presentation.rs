//! `_r2_intelligence_presentation_exact` — the XUI14 passive all-country
//! intelligence carrier. Owed by EIR-NE-004; filled by lane GP-U2.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:24418`; the envelope gate at `:24445` compares
//! `schema_version` against `intelligence_presentation::XUI14_SCHEMA`, which is
//! **8** (`replay_core/src/intelligence_presentation.rs:12`).
//!
//! Eight, not one. Of the fifteen carriers in this scaffold this is the highest
//! required version by a wide margin, and it is exactly the shape of the drift
//! that produced this wave: a producer written from the sibling carriers' habits
//! would emit 1 and be refused. The constant is spelled `XUI14_SCHEMA` rather
//! than `XUI14_SCHEMA_VERSION`, which is why a grep for the usual name finds
//! nothing here.
//!
//! The rest of the gate is the standard passive census: authority, executable
//! SHA, producer/evidence manifests, `frame_hour`, a non-zero capture sequence,
//! the three independence flags, a row count equal to the D18 roster's, and a
//! self-hash (`:24445`–`:24465`).
//!
//! # Null tolerance
//!
//! `:24430` returns `DomainUnavailable` for an explicit null.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_intelligence_presentation_exact",
        scope: "root",
        ledger_row: "EIR-NE-004",
        consumer_citation: "replay_core/src/world.rs:24445",
        required_schema: RequiredSchema::Version(8),
        wire_absence: WireAbsence::TypedNull,
        reason: "no passive all-country intelligence producer; d53's relations root is \
                 SOURCE_UNAVAILABLE on a729 and the existing intelligence reads are POV-local",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(
        feed.intelligence_presentation
            .clone()
            .unwrap_or(Value::Null),
    )
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U2 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U2's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.intelligence_presentation = produced;
}
