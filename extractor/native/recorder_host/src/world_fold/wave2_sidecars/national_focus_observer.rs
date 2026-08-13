//! `_r2_national_focus_observer_exact` — the passive all-country national-focus
//! scene carrier. Owed by EIR-NE-002; filled by lane GP-U3.
//!
//! # This key was found by a sweep, not by the ledger
//!
//! Worth recording, because it is why the scaffold exists in the form it does.
//! No XUI27 ledger row names this key. It surfaced only when
//! `runs/gp_ledger_map/r2_key_diff.py` diffed every prefixed carrier literal on
//! both sides and found `_r2_national_focus_observer_exact` on the replay side
//! with no producer counterpart. A row-driven scaffold would have registered
//! EIR-NE-002's carrier under some other name and left this one absent forever.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:33390`; the gate demands `schema_version == 1` and
//! authority `exact_a729_passive_all_country_national_focus_scene_v1`
//! (`:33391`).
//!
//! # Null tolerance
//!
//! `:33404` returns `DomainUnavailable` for an explicit null.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_national_focus_observer_exact",
        scope: "root",
        ledger_row: "EIR-NE-002",
        consumer_citation: "replay_core/src/world.rs:33390",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "no passive all-country national-focus scene producer; d89's carrier is \
                 descheduled POV-local presentation and is not a substitute",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.national_focus_observer.clone().unwrap_or(Value::Null))
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
    feed.national_focus_observer = produced;
}
