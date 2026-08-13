//! `_r2_xui12_production_lines_payload` + `_r2_xui12_producer_canvas_authority`
//! — ONE atomic carrier, two keys. Owed by EIR-NE-014; filled by lane GP-U4.
//!
//! # Why the two keys are one carrier
//!
//! `game_ui/src/production_lines.rs:748`–`:761`:
//!
//! ```text
//! match (frame.domain(PAYLOAD_FIELD), frame.domain(AUTHORITY_FIELD)) {
//!     (None, None) => Ok(None),
//!     (Some(payload), Some(_)) => serde_json::from_value(payload.clone())
//!         .map(Some).map_err(|_| InvalidProducerAuthority),
//!     _ => Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority),
//! }
//! ```
//!
//! The wildcard arm rejects one-sided presence outright. A scaffold that exposed
//! two independently fillable slots would let a lane ship half a carrier and get
//! a rejection instead of a gap. The feed therefore holds `Option<(payload,
//! authority)>` — the type makes one-sidedness unrepresentable.
//!
//! The comment above that match states the intent the type now enforces:
//! "Missing is a named external dependency; a one-sided or malformed carrier is
//! a feature-local rejection and can never authenticate measured-empty."
//!
//! # Why neither key may be registered as null
//!
//! `(None, None)` is the consumer's CLEAN state — a named external dependency,
//! which is exactly what an unbuilt producer is. Two nulls take the
//! `(Some, Some)` arm, fail `from_value` against a struct, and become
//! `InvalidProducerAuthority`: a feature-local rejection claiming the carrier is
//! corrupt. Even a null in the authority half alone would be rejected later by
//! the strict authority decode at `:453`.
//!
//! # The contract, for the lane that fills it
//!
//! The authority record's schema is
//! `"replay.xui12.production-lines-authority.a729.v2"` (`:30`) — a whole-string
//! token, version 2. The payload must carry an ORDERED all-country census whose
//! entries match the D18 roster by tag, `tag_db_id` and `source_ordinal`
//! (`:764`–`:786`), and the authority's census must agree with the payload's
//! element for element (`:789`–`:800`).
//!
//! The payload also carries `replay_frame_identity_sha256` over the whole frame
//! minus these two keys (`:915`–`:917`); see the lane's frame-identity residual.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_xui12_production_lines_payload",
        scope: "root",
        ledger_row: "EIR-NE-014",
        consumer_citation: "game_ui/src/production_lines.rs:748",
        required_schema: RequiredSchema::Token("replay.xui12.production-lines-authority.a729.v2"),
        wire_absence: WireAbsence::OmitKey,
        reason: "no XUI12 production-lines producer; both keys are omitted rather than nulled \
                 because (None, None) is this consumer's clean external-dependency state and a \
                 null pair decodes as InvalidProducerAuthority",
    },
    paired_key: Some("_r2_xui12_producer_canvas_authority"),
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    match &feed.xui12_production_lines {
        Some((payload, authority)) => SidecarEmission::Pair {
            payload: payload.clone(),
            authority: authority.clone(),
        },
        None => SidecarEmission::NoKey,
    }
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U4 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U4's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<(Value, Value)> = None;
    feed.xui12_production_lines = produced;
}
