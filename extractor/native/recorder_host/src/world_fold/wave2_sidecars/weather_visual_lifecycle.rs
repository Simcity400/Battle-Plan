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
//! render-submit lane already populates.
//!
//! # It was not only an envelope (lane IN)
//!
//! The paragraph above used to end "this lane is an envelope, not a new read".
//! That was wrong on three fields. The carrier owes a per-region visual epoch —
//! the game hour the current entity set was instantiated, and an ordinal
//! advancing on every airborne-code change — plus the two selector seed words,
//! and none of the three was on the wire or in the engine: `0x14166ec50` stores
//! only the last code it computed. All three are now captured
//! (`a729_hook::RawWeatherVisualEpoch`, `WeatherMembership::selection_seed_words`)
//! and this file only decides whether an hour has a census to mint from.

use serde::Deserialize;
use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};
use portable_render_stream::WeatherMembership;
use production_render_submit_live_lane::weather_visual_lifecycle::mint_weather_visual_lifecycle;

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
        // The producer landed with lane IN (GP-U16), so this is no longer
        // "nothing mints it". It names the cases in which a LANDED producer
        // still has nothing to publish. It stays a summary: the hour's actual
        // cause — how many regions were unwitnessed, or which integrity rule
        // failed — is pushed per hour into `Wave2Feed::dynamic_refusals` by
        // `fill`, because that detail is not derivable from any static string.
        reason: "the hour has no domain-77 render frame with a verified hour, or its weather \
                 census carries at least one strategic region whose current visual epoch this \
                 recorder never witnessed being instantiated, so that epoch's game hour and \
                 ordinal are unknowable and the census cannot be completed; the hour's own \
                 reason is filed in provenance.notes",
    },
    paired_key: None,
    fill,
    scope: SidecarScope::Map,
    emit,
};

fn emit(feed: &Wave2Feed) -> SidecarEmission {
    SidecarEmission::Single(feed.weather_visual_lifecycle.clone().unwrap_or(Value::Null))
}

/// Read this carrier's producer off the fold (lane GP-U16).
///
/// The producer is domain 77's retained render frame for THIS hour: its
/// checkpoint holds the per-region weather-membership census, and
/// `mint_weather_visual_lifecycle` turns that census into the envelope. Every
/// rule about what the envelope may say lives in the lane crate beside the
/// capture that produced the rows; this function only decides whether an hour
/// has a census at all.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    match produce(fold) {
        Ok(envelope) => feed.weather_visual_lifecycle = Some(envelope),
        // A reason is filed only when it is this carrier's to give. "There was
        // no render frame" is already recorded, structurally, by domain 77's
        // own problem entry, and repeating it every hour would put a line in
        // every snapshot that carries nothing the hour does not already say —
        // the exact per-hour noise the scaffold removed static reasons for.
        // What IS this carrier's alone is how much of the map it could not
        // witness, and any integrity failure in the census.
        Err(reason) => {
            feed.weather_visual_lifecycle = None;
            if let Some(reason) = reason {
                feed.dynamic_refusals
                    .push(format!("map._r2_weather_visual_lifecycle: {reason}"));
            }
        }
    }
}

/// `Err(None)` is an hour with no census to mint from, already explained by
/// domain 77. `Err(Some(..))` is a reason only this carrier can give.
fn produce(fold: &WorldFold) -> Result<Value, Option<String>> {
    // A domain the fold could not read this hour publishes nothing — the same
    // admission the snapshot's own `production_render` projection makes
    // (`world_fold.rs::build_snapshot`, the domain-77 arm).
    if fold.fold_domain_unknown(super::super::PRODUCTION_RENDER_SUBMITS_DOMAIN_ID) {
        // Domain 77 already named its own failure through the fold's
        // domain-problem channel this hour, so repeating it here would file the
        // same fact twice under two names.
        return Err(None);
    }
    // That projection additionally compares the retained frame's hour with the
    // hour being built, and nulls the frame when they differ. A `fill` is not
    // given the snapshot hour — deliberately, so that adding a producer never
    // reopens the shared scaffold — so the equivalent guarantee is taken from
    // where it can actually be established: `fold_production_render` records
    // whether the frame's own `update_index` was checked against the H4CR hour
    // of the record it arrived on. Unverified means the frame's hour is its own
    // unchecked claim, and this carrier PUBLISHES that hour, so it refuses.
    if !fold.production_render_hour_verified {
        return Err(None);
    }
    let Some((frame_hour, frame)) = fold.production_render_this_hour.as_ref() else {
        return Err(None);
    };
    let Ok(frame_hour) = i64::try_from(*frame_hour) else {
        // Not an ordinary absence: a retained frame whose hour is not a
        // snapshot hour index is a frame that should never have been retained.
        return Err(Some(format!(
            "the retained domain-77 render frame's hour {frame_hour} is not a snapshot hour index"
        )));
    };
    // The checkpoint census is the deduplicated per-region view the collector
    // folds from the selected submit, so it is the one place a whole-map census
    // exists. Only that array is decoded: the frame itself carries texture and
    // draw payloads that this carrier has no use for and that no hour should
    // pay to re-materialise.
    let Some(census) = census_from_frame(frame) else {
        return Err(Some(
            "the retained domain-77 render frame carries no decodable weather census".into(),
        ));
    };
    mint_weather_visual_lifecycle(&census, frame_hour)
        .map_err(|refusal| Some(refusal.to_string()))
}

/// The census the collector folded into this frame's checkpoint.
///
/// Separate from [`produce`] so the path into the projected frame — two key
/// names that exist nowhere else in this crate — can be tested against the
/// wire type itself rather than against a hand-written object that would agree
/// with a typo.
fn census_from_frame(frame: &Value) -> Option<Vec<WeatherMembership>> {
    let census = frame
        .get("checkpoint")?
        .get("weather_state_and_ordered_membership")?;
    Vec::<WeatherMembership>::deserialize(census).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kills: a typo or a rename in either key of the census path. The frame
    /// this walks is `serde_json::to_value` of the REAL wire type, exactly as
    /// `world_fold.rs::fold_production_render` projects it, so renaming
    /// `RenderCheckpoint::weather_state_and_ordered_membership` upstream fails
    /// here instead of turning every hour's carrier silently absent.
    ///
    /// It deliberately does NOT assert an envelope: what the census may say is
    /// the lane crate's contract and is tested there. This test owns exactly
    /// the two key names.
    #[test]
    fn the_census_path_matches_the_projected_frame() {
        let checkpoint = portable_render_stream::RenderCheckpoint {
            last_sequence: 1,
            last_update_index: 1,
            last_submit_index: None,
            next_stable_ids: Default::default(),
            active_effect_lifecycle: Vec::new(),
            active_light_lifecycle: Vec::new(),
            weather_state_and_ordered_membership: Vec::new(),
            active_effect_set_sha256: String::new(),
            ordered_particle_packets_sha256: String::new(),
            ordered_entity_submissions_sha256: String::new(),
            ordered_light_submissions_sha256: String::new(),
            weather_state_sha256: String::new(),
            weather_membership_sha256: String::new(),
            weather_lifecycle_sha256: String::new(),
            weather_visibility_sha256: String::new(),
            ordered_weather_packets_sha256: String::new(),
            weather_composition_sha256: String::new(),
        };
        let frame = serde_json::json!({
            "checkpoint": serde_json::to_value(&checkpoint).expect("checkpoint projects"),
        });
        assert_eq!(census_from_frame(&frame), Some(Vec::new()));
        assert_eq!(census_from_frame(&serde_json::json!({})), None);
    }
}
