//! `_r2_occupation_ui_supplement_live` — the XUI16 occupation UI supplement.
//! Owed by EIR-NE-010; filled by lane GP-U6.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:29083`; the gate at `:29107` compares
//! `schema_version` against `OCCUPATION_UI_SUPPLEMENT_SCHEMA_VERSION`, which is
//! **1** (`replay_core/src/world.rs:19936`), with authority
//! `a729_xui16_occupation_ui_supplement_v1` (`:19937`).
//!
//! Two requirements a producer will not guess:
//!
//! * `evidence_manifest_sha256s` is a LIST that must equal
//!   `OCCUPATION_UI_SUPPLEMENT_EVIDENCE_MANIFEST_SHA256S` element for element
//!   (`:29111`–`:29115`) — five hashes, in order, not a set.
//! * `localisation_language` must start with `l_` and be non-blank control-free
//!   text (`:29119`–`:29120`). The carrier is localisation-bearing, so the hour
//!   records which language it transcribed.
//!
//! `source_record_id` is recomputed by the consumer from the supplement itself
//! (`:29106`, `compute_occupation_ui_source_record_id`), so it is a derived
//! identity and not a free-form label.
//!
//! # Null tolerance
//!
//! `:29095` returns `DomainUnavailable` for an explicit null.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_occupation_ui_supplement_live",
        scope: "root",
        ledger_row: "EIR-NE-010",
        consumer_citation: "replay_core/src/world.rs:29108",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "no occupation UI supplement producer; d52/d66 carry the resistance/compliance \
                 state but not the localisation-bearing controller supplement replay reads",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.occupation_ui_supplement.clone().unwrap_or(Value::Null))
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U6 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U6's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.occupation_ui_supplement = produced;
}
