//! `_r2_politics_presentation_exact` — the XUI11 passive all-country politics
//! carrier. Owed by EIR-NE-003; filled by lane GP-U1.
//!
//! # The contract, read from the consumer
//!
//! `replay_core/src/world.rs:23833` is the decoder. Its envelope gate at `:23873`
//! demands `schema_version == 7` together with authority
//! `exact_a729_passive_all_country_politics` and a self-hash: the carrier's own
//! `record_sha256` must equal the SHA-256 of the carrier with that field cleared
//! (`:23868`–`:23872`). A producer cannot compute the hash last and paste it in
//! afterwards without clearing the field first.
//!
//! The gate also requires the row count to equal the D18 country roster's
//! (`:23893`) and every row's ordinal to match its roster position (`:23911`),
//! so this is an ORDERED all-country census, not a map.
//!
//! # Null tolerance
//!
//! `:23847` returns `DomainUnavailable` for an explicit null and `:23843`
//! returns `DomainAbsent` for a missing key. The typed null is therefore the
//! better unfed spelling: it says the recorder knows this carrier exists.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_politics_presentation_exact",
        scope: "root",
        ledger_row: "EIR-NE-003",
        consumer_citation: "replay_core/src/world.rs:23873",
        required_schema: RequiredSchema::Version(7),
        wire_absence: WireAbsence::TypedNull,
        reason: "no passive all-country politics producer; the extractor's politics carriers are \
                 POV-local/open-panel by design and cannot satisfy the passive contract",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.politics_presentation.clone().unwrap_or(Value::Null))
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
    feed.politics_presentation = produced;
}
