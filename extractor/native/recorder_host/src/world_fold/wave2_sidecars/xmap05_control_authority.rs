//! `map._r2_xmap05_control_authority` — the MAPC1 owner/controller authority
//! envelope. Owed by XMAP00 #4–#6; filled by lane GP-U15.
//!
//! # Map-scoped, not root-scoped
//!
//! This carrier lives under the snapshot's `map` object
//! (`map_viewer/src/production_features/xmap05.rs:1146`–`:1152` reads
//! `frame.domain("map")` and then `map.get("_r2_xmap05_control_authority")`),
//! which is why the scaffold has two merge points rather than one.
//!
//! # Why this one is omitted rather than nulled
//!
//! Unlike its nine null-tolerant siblings, this consumer has no null arm at all:
//!
//! ```text
//! frame.domain("map").and_then(as_object)
//!     .and_then(|map| map.get("_r2_xmap05_control_authority"))
//!     .and_then(serde_json::Value::as_object)
//!     .ok_or_else(|| "XMAP05 MAPC1 control authority is absent".into())
//! ```
//!
//! `Value::Null.as_object()` is `None`, so a null and a missing key produce the
//! IDENTICAL error text. Registering a null therefore conveys nothing to replay
//! that absence did not already convey — while still adding a key to the frame,
//! which perturbs the whole-frame identity hashes XUI12/XUI15/XUI23 compute (see
//! the scaffold module docs). A registration that buys no information and costs
//! a hash perturbation is not worth making, so the wire is left alone and the
//! consumer's own "XMAP05 MAPC1 control authority is absent" stands as the
//! named refusal.
//!
//! This is a third case the Wave-2 plan's blanket "every decoder treats null as
//! DomainUnavailable" did not anticipate: not harmful like XUI12/15/23, simply
//! pointless.
//!
//! # The contract, for the lane that fills it
//!
//! `validate_mapc1_authority_object` (`:1156`) requires `schema == "MAPC1"` AND
//! `schema_version == 1`, plus a set of exact string fields that are RE facts,
//! not free text: `province_controller_field_offset == "0x188"`,
//! `country_remap_rva == "0x00ba6240"`, a pinned Ghidra manifest SHA, and the
//! two identity orders `state_id_ascending` / `province_id_ascending`. The
//! envelope is minted over the owner/controller rows the fold already emits, so
//! this lane is an envelope, not a new read.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

/// The `schema` token the consumer pins alongside `schema_version == 1`.
///
/// Recorded separately because [`RequiredSchema`] holds one spelling and this
/// consumer checks both: a producer satisfying only the integer is refused.
///
/// Its only reader today is the registry's regression test, which is the point —
/// the constant exists so that a change to the consumer's token is caught here
/// rather than discovered by the GP-U15 lane after it has written a producer.
#[allow(dead_code)]
pub(super) const REQUIRED_SCHEMA_TOKEN: &str = "MAPC1";

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_xmap05_control_authority",
        scope: "map",
        ledger_row: "XMAP00 #4-#6",
        consumer_citation: "map_viewer/src/production_features/xmap05.rs:1162",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::OmitKey,
        reason: "no MAPC1 control-authority envelope over d22's owner/controller rows; the key is \
                 omitted rather than nulled because this consumer collapses null and absence to \
                 the same error, so a null would add no information and only perturb the \
                 whole-frame identity hashes",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Map,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    match &feed.xmap05_control_authority {
        Some(value) => SidecarEmission::Single(value.clone()),
        None => SidecarEmission::NoKey,
    }
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U15 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U15's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.xmap05_control_authority = produced;
}
