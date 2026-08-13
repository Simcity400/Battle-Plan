//! `_r2_xui15_trade_catalogue_payload` + `_r2_xui15_trade_catalogue_authority`
//! — ONE atomic carrier, two keys. Owed by EIR-NE-013; filled by lane GP-U4.
//!
//! # Same shape as XUI12, same reasoning
//!
//! `game_ui/src/trade_compositor.rs:522`–`:534` matches on both halves:
//! `(None, None) => Ok(None)`, `(Some, Some) =>` strict decode,
//! everything else `Err(Xui15ProducerRejection::InvalidAuthority)`. Two nulls
//! take the `(Some, Some)` arm and fail the decode, so the pair is omitted when
//! unfed, leaving `Ok(None)` — the consumer's own "named external dependency"
//! state — as the refusal.
//!
//! The consumer's own comment at `:518`–`:521` states why the halves are
//! separate values but one carrier: "The authority record remains separate so a
//! payload cannot authenticate itself by carrying its own receipt." Separate
//! records, single admission decision.
//!
//! # The contract, for the lane that fills it
//!
//! The authority schema is
//! `"replay.xui15.country_trade_catalogue_authority.a729.v2"` (`:34`–`:35`) —
//! version 2, compared as a whole string.
//!
//! Beyond the ordered country census the sibling carriers require, this one adds
//! a filter-catalogue check: `evaluated_filter_token_ids` must match the
//! payload's exactly AND contain no duplicates (`:914`–`:923`), and each
//! catalogue's `country_tag`/`source_ordinal` must line up positionally
//! (`:927`–`:932`). A producer that emits filters as a set, or in a
//! discovery-dependent order, is refused.
//!
//! `xui15_replay_frame_identity_sha256` (`:889`–`:902`) hashes the whole frame
//! minus these two keys; see the lane's frame-identity residual.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_xui15_trade_catalogue_payload",
        scope: "root",
        ledger_row: "EIR-NE-013",
        consumer_citation: "game_ui/src/trade_compositor.rs:522",
        required_schema: RequiredSchema::Token(
            "replay.xui15.country_trade_catalogue_authority.a729.v2",
        ),
        wire_absence: WireAbsence::OmitKey,
        reason: "no XUI15 trade-catalogue producer; both keys are omitted rather than nulled \
                 because (None, None) is this consumer's clean external-dependency state and a \
                 null pair decodes as InvalidAuthority",
    },
    paired_key: Some("_r2_xui15_trade_catalogue_authority"),
    fill,
    scope: SidecarScope::Root,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    match &feed.xui15_trade_catalogue {
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
    feed.xui15_trade_catalogue = produced;
}
