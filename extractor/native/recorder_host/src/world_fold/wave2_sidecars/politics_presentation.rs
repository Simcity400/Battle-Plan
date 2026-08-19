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
//!
//! # Why `fill` stays `None` — lane IK (GP-U1), 2026-08-14
//!
//! Settled: the schema-7 row needs four presentation families D14/D15 do not carry
//! — engine-evaluated final text with its SHA, six tooltip families, authored
//! visibility/enabled state and sprite keys, and the active goal's identity and
//! progress leaves -- plus the category/slot topology (`category_order`,
//! `slot_order`, each idea's `category_id`/`slot_id`), which is authored,
//! mod-aware content D14/D15 also do not supply. (The row's identity hashes
//! and the idea KIND partition ARE derivable from what the recorder has;
//! they are not blockers.) D14/D15 carry exactly the state leaves the consumer
//! cross-checks and none of these, so the carrier cannot be produced from
//! anything this recorder captures today. Fail-closed, that is a typed null.
//!
//! NOT settled, and this is the honest limit: whether those families are
//! reachable by SOME passive producer. The evidence points at panel-build-only
//! — they are written into the authored widget tree of one constructed
//! `CCountryPoliticsView`, and the recorder's generic tooltip producer (d72) is
//! keyed by the HOVERED element and was descheduled for that reason. But the
//! per-family source closure over the engine's politics text and tooltip
//! FORMATTERS has not been run, and the tree already contains formatter-level
//! tooltip producers for other domains (province supply, land combat), which is
//! exactly the shape that would reopen this row. So EIR-NE-003 is a bounded
//! DEFERRAL, not a proof of impossibility.
//!
//! If that closure comes back negative, the remaining route is deriving the
//! text at replay, which is a REPLAY-side change (decoder, port spec and the
//! pinned `producer_contract_manifest.json`) and therefore an owner decision.
//!
//! The closure list below is enforced by the producer and consumer implementations.

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
        reason: "EIR-NE-003 deferred, NOT refused (lane IK): schema 7 needs four \
                 presentation families D14/D15 do not carry -- engine-evaluated final text \
                 with its SHA, six tooltip families, authored visibility/enabled state and \
                 sprite keys, and the active goal's identity and progress leaves -- plus the \
                 authored, mod-aware category/slot topology they also do not supply. So no \
                 producer can be built from what this recorder captures today. Whether those \
                 leaves are reachable by SOME passive producer is UNSETTLED: the evidence \
                 points at panel-build-only (one CCountryPoliticsView, and d72 is hover-bound \
                 and descheduled), but the source closure over the engine's politics text and \
                 tooltip formatters has not been run and formatter-level tooltip producers \
                 exist for other domains.",
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
    // Deferred fail-closed by lane IK; cause recorded in `CARRIER.slot.reason`.
    let produced: Option<Value> = None;
    feed.politics_presentation = produced;
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::pending_producer::WireAbsence;

    /// kills: reducing the slot's `reason` to a bare absence such as
    /// "no passive all-country politics producer", AND upgrading it to a
    /// refusal.
    ///
    /// `PendingProducerSlot::validate` accepts any non-empty string, so nothing
    /// else in the tree notices when a derived verdict is replaced by a
    /// restatement of the symptom -- or when a deferral is quietly promoted to
    /// "refused", which is how a row stops being staffed. Both directions
    /// matter here: this lane's own first draft called the row refused, and its
    /// reviewer was right that the evidence did not carry that. The `UNSETTLED`
    /// clause and the pointer to the closure list are what keep the next lane
    /// running the closure instead of trusting this sentence.
    #[test]
    fn the_recorded_reason_carries_the_derived_cause_and_its_open_question() {
        let reason = CARRIER.slot.reason;
        for fragment in [
            "final text",
            "tooltip",
            "UNSETTLED",
            "NOT refused",
        ] {
            assert!(
                reason.contains(fragment),
                "the EIR-NE-003 deferral must still name {fragment:?}: {reason}"
            );
        }
    }

    /// kills: giving `fill` an unconditional payload (a stub census or an
    /// all-empty transaction), and specifically a conditional clone of d72's
    /// hover-bound tooltip carrier, which the fixture below populates.
    ///
    /// This runs the real `fill` against a fold with no producer in it, which is
    /// every hour today, and requires it to leave the carrier unfed. The
    /// mutation it catches is the one this row's contract punishes hardest: the
    /// consumer checks `passive_all_country`, `ui_open_independent`,
    /// `recorder_pov_independent` and `producer_complete`, so any payload
    /// manufactured without an all-country presentation source is the recorder
    /// asserting four things it cannot back.
    ///
    /// The fold is given d72's resolved-tooltip carrier POPULATED, because that
    /// is the nearest wrong source for family (b) and because a conditional
    /// miswire (`fold.resolved_tooltip_instances_this_hour.clone()`) would
    /// return `None` on an empty fold and pass a weaker test.
    ///
    /// Two LIMITS, stated so nobody over-reads it. It cannot catch a `fill` that
    /// reads a genuine all-country producer off a populated fold, because no
    /// such producer exists to populate; when one does, this test is replaced by
    /// that lane, not kept. And it covers only the ONE wrong predecessor it
    /// populates -- a conditional clone of some other `Option<Value>` field the
    /// fixture leaves `None`, such as `power_balance_state_this_hour`, would
    /// still pass here.
    #[test]
    fn fill_refuses_the_hover_bound_tooltip_carrier_and_the_wire_carries_the_typed_null() {
        let mut fold = WorldFold::new("ik-gp-u1-politics");
        // d72: what one client's cursor actually resolved. Hover-bound and
        // owner-descheduled; routing it here would publish POV-local tooltips
        // under a carrier asserting `recorder_pov_independent`.
        fold.resolved_tooltip_instances_this_hour = Some(serde_json::json!({
            "schema_version": 2,
            "frame_hour": 1,
            "instances": [],
        }));
        let mut feed = Wave2Feed::unfed();
        (CARRIER.fill)(&fold, &mut feed);
        assert!(
            feed.politics_presentation.is_none(),
            "no all-country politics producer exists, so fill must not invent one \
             -- least of all out of hover-bound tooltip instances"
        );
        assert_eq!(emit(&feed), SidecarEmission::Single(Value::Null));
        assert_eq!(
            CARRIER.slot.wire_absence,
            WireAbsence::TypedNull,
            "XUI11 has an explicit is_null arm, so absence must stay the typed null"
        );
    }
}
