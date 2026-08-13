//! `map._r2_weather_visual_lifecycle` — the weather visual lifecycle envelope.
//! Owed by XMAP00 #44; filled by lane GP-U16.
//!
//! # Map-scoped, and null-tolerant unlike its map-scoped sibling
//!
//! `replay_core/src/world.rs:9274` reads it from the snapshot's `map` object,
//! and `:9278` has an explicit `if value.is_null()` arm returning
//! `DomainUnavailable`. That is the difference from `_r2_xmap05_control_authority`,
//! which sits in the same object and has no null arm at all — proof that
//! null-tolerance is a per-consumer fact and cannot be inferred from a carrier's
//! scope.
//!
//! # The contract, for the lane that fills it
//!
//! The root object is checked for EXACT field-set equality, not just presence:
//! `:9284`–`:9300` builds a twelve-name list and refuses if `root.len()` differs
//! or any name is missing. An extra field for producer bookkeeping is a
//! rejection, and so is a missing one.
//!
//! Identity requires `schema == "hoi4.weather-visual-lifecycle.a729.v1"`
//! (`:9259`) AND `schema_version == 1` (`:9305`), plus a pinned producer RVA and
//! cadence. Per-epoch checks are stricter still: signed-i32 expiry, an epoch
//! that is not in the future, ordered and non-duplicated producer identities
//! with a pinned prefix, and — the one most likely to be missed — clear weather
//! must carry NO producer identities (`:9500`).
//!
//! `REGION_ROWS` is 305 (`:9270`), so the envelope's `complete_region_count` has
//! a fixed expected value on this build.
//!
//! The envelope is minted over `RenderSubmit.weather_membership`, which the
//! render-submit lane already populates; this lane is an envelope, not a new
//! read.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

/// The `schema` token the consumer pins alongside `schema_version == 1`.
///
/// Read only by the registry's regression test, deliberately: it is there so a
/// change to the consumer's token is caught before the GP-U16 lane writes a
/// producer against the old one.
#[allow(dead_code)]
pub(super) const REQUIRED_SCHEMA_TOKEN: &str = "hoi4.weather-visual-lifecycle.a729.v1";

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_weather_visual_lifecycle",
        scope: "map",
        ledger_row: "XMAP00 #44",
        consumer_citation: "replay_core/src/world.rs:9305",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "no lifecycle envelope over the already-populated RenderSubmit.weather_membership; \
                 the epochs exist but nothing mints the identity/digest/completeness witness \
                 replay admits",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Map,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.weather_visual_lifecycle.clone().unwrap_or(Value::Null))
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U16 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U16's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.weather_visual_lifecycle = produced;
}
