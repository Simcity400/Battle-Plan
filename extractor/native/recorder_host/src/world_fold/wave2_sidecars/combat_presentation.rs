//! `_r2_combat_presentation_exact` — the XUI20 passive GLOBAL combat
//! presentation carrier. Owed by EIR-CW-002; filled by lane GP-U5.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:24016`; the envelope gate at `:24048` compares
//! `schema_version` against `combat_presentation::XUI20_SCHEMA_VERSION`, which
//! is **1** (`replay_core/src/combat_presentation.rs:12`).
//!
//! Two details that will bite a producer written from the other carriers'
//! shape:
//!
//! * This one is `passive_global`, not `passive_all_country` (`:24055`). The
//!   census is over combats, not countries.
//! * `:24033` runs `xui20_required_nullable_members_present` BEFORE the strict
//!   decode. A member that is nullable in the struct still has to be PRESENT as
//!   an explicit null; omitting it is rejected separately from decoding. Serde's
//!   default `Option` handling would happily omit it, so a producer must
//!   serialize nulls explicitly.
//!
//! It also refuses unless `combats_status` and `naval_combats_status` are both
//! `Present` (`:24067`–`:24068`) — the carrier cannot be admitted for an hour
//! whose ordinary combat views are not.
//!
//! # Null tolerance
//!
//! `:24028` returns `DomainUnavailable` for an explicit null.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_combat_presentation_exact",
        scope: "root",
        ledger_row: "EIR-CW-002",
        consumer_citation: "replay_core/src/world.rs:24048",
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "no passive global combat-presentation producer; the existing combat carriers are \
                 POV-local panel state and cannot represent a closed passive hour",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.combat_presentation.clone().unwrap_or(Value::Null))
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U5 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U5's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.combat_presentation = produced;
}
