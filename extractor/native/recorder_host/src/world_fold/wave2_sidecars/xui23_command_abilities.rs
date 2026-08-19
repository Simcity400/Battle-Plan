//! `_r2_xui23_passive_command_abilities_authority` — the XUI23 passive
//! all-country command-ability authority. Owed by EIR-NE-009; filled by lane
//! GP-U7.
//!
//! # This carrier must NOT be registered as null, and this is the clearest case
//!
//! The Wave-2 plan's gate says "each replay decoder already treats null as
//! `DomainUnavailable`". This consumer does not, and here the cost of a null is
//! not merely a different status — it is a WORSE and untrue reason.
//!
//! `game_ui/src/active_abilities.rs:1091`–`:1101`:
//!
//! ```text
//! frame.domain(FIELD).cloned()
//!     .ok_or_else(|| ActiveAbilityRejection(XUI23_PASSIVE_CARRIER_UNAVAILABLE_REASON.into()))
//!     .and_then(|value| serde_json::from_value::<DecodedObserverAbilityAuthority>(value)
//!         .map_err(|_| ActiveAbilityRejection("XUI23 passive carrier authority record is malformed".into())))
//! ```
//!
//! With the key ABSENT, replay reports the exact residual this row exists to
//! surface: `"EIR-NE-009: canonical UI-open-independent all-country
//! command-ability carrier is absent"` (`:115`). That string names the ledger
//! row, the contract and the fact. `game_ui/src/active_abilities.rs:2681`
//! asserts it, and `game_ui/tests/production_renderer.rs:429`–`:431` asserts
//! that the constant and its use site both still exist.
//!
//! With the key set to null, `from_value` fails and replay instead reports the
//! authority record is MALFORMED. Nothing was produced; claiming corruption is a
//! false statement about a carrier that does not exist, and it would break the
//! two tests above.
//!
//! So the wire keeps the consumer's own clean-absence spelling, and the named
//! absence is the one the consumer already produces — which for this carrier is
//! the best available: it quotes the ledger row itself.
//!
//! # The contract, for the lane that fills it
//!
//! `schema` must equal `XUI23_PRODUCER_AUTHORITY_SCHEMA` =
//! `"replay.xui23-passive-command-abilities-authority.v1"`; the consumer is the
//! authority.
//!
//! The payload also carries `replay_frame_identity_sha256`, computed over the
//! SHARED base-frame projection under this carrier's own domain separator
//! (`portable_replay_format::replay_frame_identity`, domain
//! `hoi4.xui23.replay-base-frame.v2`). The producer must call that same
//! function: it removes every registered sidecar key at both scopes, so the
//! digest is the same whether or not XUI12's and XUI15's carriers ship in the
//! same hour. Until GP-F10 this digest covered the whole frame with only this
//! key removed, which — with XUI12's and XUI15's digests doing the mirror image
//! — made the three a fixed-point cycle no emission order could satisfy. The
//! retired `hoi4.xui23.replay-frame.v1` domain is refused by name.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_xui23_passive_command_abilities_authority",
        scope: "root",
        ledger_row: "EIR-NE-009",
        consumer_citation: "game_ui/src/active_abilities.rs:1091",
        required_schema: RequiredSchema::Token(
            "replay.xui23-passive-command-abilities-authority.v1",
        ),
        wire_absence: WireAbsence::OmitKey,
        reason: "no passive all-country command-ability producer; d88 is descheduled POV-local \
                 presentation. The key is omitted rather than nulled because this consumer maps \
                 absence to the exact EIR-NE-009 residual and a null to a false 'malformed' claim",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    match &feed.xui23_command_abilities {
        Some(value) => SidecarEmission::Single(value.clone()),
        None => SidecarEmission::NoKey,
    }
}

/// Read this carrier's producer off the fold.
///
/// THE ONE LINE LANE GP-U7 CHANGES. It assigns `None` today because no producer
/// exists; when one does, this reads it from `fold` and assigns `Some(..)`. No
/// other file in the scaffold moves -- that is what makes GP-U7's footprint
/// disjoint from every other producer lane's, and it is the reason the feed is
/// assembled from these per-carrier functions rather than at one central site.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    let produced: Option<Value> = None;
    feed.xui23_command_abilities = produced;
}
