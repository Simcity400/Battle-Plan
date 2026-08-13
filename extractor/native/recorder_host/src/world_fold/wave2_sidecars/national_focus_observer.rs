//! `_r2_national_focus_observer_exact` — the passive all-country national-focus
//! scene carrier. Owed by EIR-NE-002; owned by lane GP-U3.
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
//! `replay_core/src/world.rs::build_national_focus_observer_exact` names the key
//! and holds the whole gate: it demands `schema_version == 1`, authority
//! `exact_a729_passive_all_country_national_focus_scene_v1` and the literal
//! `producer_contract` `"EIR-NE-002"`.
//!
//! # The producer-side read law (lane GP-U3)
//!
//! [`CONTRACT_FAMILIES`] is this file's substance. Wave 2 could only assert
//! that d89's descheduled carrier "is not a substitute"; a producer lane has to
//! answer the narrower, checkable question — for each family of values the
//! consumer requires, is there an all-country, panel-independent source the
//! recorder can read passively?
//!
//! The split is sharp, and it is not the one the row's name suggests:
//!
//! * The SEMANTIC half is HALF produced, all-country, and the missing half is
//!   this lane's second finding. The consumer cross-checks the carrier against
//!   two d12 leaves — `row.tree_id != *d12_tree` and
//!   `row.nodes.iter().map(|node| &node.focus_id).ne(d12_visible.iter())`
//!   (both inside `world.rs::build_national_focus_observer_exact`) — and
//!   requires both to be `Leaf::Measured`.
//!   `world_fold.rs::fold_focus` emits the first and not the
//!   second: `focus.visible_order` (and `focus.available` beside it,
//!   `world.rs::build_focus`) appear NOWHERE in the extractor, and the d12
//!   country row the fold parses carries no field either could come from — it
//!   is tag, flags, completed_count, progress_raw, focus_tree, current,
//!   current_continuous, plus completed tokens. So EIR-NE-002 carries a second,
//!   independent blocker that the gap map does not record: a seam key of the
//!   GP-F1/GP-F2/GP-F9 class, on d12, needing an engine read the extractor does
//!   not have. Even a perfect scene carrier is refused while it is missing.
//! * The SCENE half is a product of the client's own focus panel. Every value
//!   the carrier adds on top of the D12 join — each node's presentation-state
//!   code, display-progress percent, glow and historical-marker flags and
//!   joint-view pair, every node's and link tile's `canvas_position_pixels`,
//!   and each link tile's child operations with their sprite names and frame
//!   indices — is written onto `CFocusTreeView` items while that view is built.
//!   The lane's own d89 producer records exactly those offsets:
//!   `a729_focus_item_producer.rs` `VIEW_VECTOR_DATA_OFFSET` (0x2058) for the
//!   item vector, `ITEM_PRESENTATION_STATE_OFFSET` (0x540) for the state code,
//!   and `DISPLAY_PROGRESS_PRODUCER_RVA` for the percent. There is ONE such
//!   view, for the country the client is looking at, and only while the focus
//!   panel is open. That is precisely why d89 is descheduled
//!   (`compact_contract/src/lib.rs::DESCHEDULED_CAPTURE_DOMAIN_IDS`, ruling
//!   `documents/DECISIONS.md`).
//!
//! So EIR-NE-002 does not want a wider read of the same thing; it wants the
//! GUI scene of every country's focus tree, and no such scene exists unless
//! something builds it. Building 200 of them per hour is the game working for
//! the recorder; opening each country's panel in turn is staged client
//! interaction. Both are closed by law, which is what makes the refusal below
//! the correct output rather than an unfinished one.
//!
//! Replay's own XUI10 contract says where the remaining route is: the
//! carrier-independent seam `game_ui::focus_observer::prepare_observer_focus_scene`
//! already joins a scene back to the D12/FILE model
//! (`rendering_laws/XUI10/D12_FOCUS_OBSERVER_CONTRACT.md:23`-`:26`), and the
//! authored focus definitions own tree membership, icons and sprites. Deriving
//! the scene at replay from D12 plus the effective files is therefore a real
//! option and a replay-side one — an owner routing decision, recorded as this
//! lane's residual.
//!
//! # Null tolerance
//!
//! `world.rs::build_national_focus_observer_exact` returns `DomainUnavailable`
//! for an explicit null, on the branch immediately after the absent branch.

use serde_json::Value;

use compact_contract::pending_producer::{PendingProducerSlot, RequiredSchema, WireAbsence};

use super::{SidecarEmission, SidecarScope, Wave2Carrier, Wave2Feed, WorldFold};

/// Identity the consumer pins, transcribed from its own constants.
///
/// These are the producer half of the `exact_a729_passive_*` vocabulary split
/// recorded in the gap map's section 2.2: until an extractor file spells them,
/// `authority_vocab_diff.py` reports the name replay-only. Transcription is not
/// evidence on its own, so `runs/ie_gp_u3/xui_identity_crosscheck.py` re-reads
/// each value from the consumer's own source.
pub(super) const AUTHORITY: &str = "exact_a729_passive_all_country_national_focus_scene_v1";
/// The pinned a729 image, compared in
/// `replay_core/src/world.rs::build_national_focus_observer_exact`.
pub(super) const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
/// The consumer compares `presentation.producer_contract` against this literal
/// ledger row rather than a hash, in
/// `replay_core/src/world.rs::build_national_focus_observer_exact`.
pub(super) const PRODUCER_CONTRACT: &str = "EIR-NE-002";
/// The consumer compares an integer `schema_version` field, not a token
/// (`replay_core/src/world.rs::build_national_focus_observer_exact`).
pub(super) const SCHEMA_VERSION: u32 = 1;

/// The identity block is evaluated at compile time rather than merely written
/// down. A `const` nothing reads is a note, and a note disagrees with its
/// source silently; this makes a malformed digest a build failure and keeps the
/// constants live in a build with the tests compiled out.
const _: () = {
    assert!(!AUTHORITY.is_empty());
    assert!(EXECUTABLE_SHA256.len() == 64);
    assert!(!PRODUCER_CONTRACT.is_empty());
    assert!(SCHEMA_VERSION == 1);
};

/// Where one family of contract values lives in the running process.
///
/// The five variants are not shades of the same thing. The first two are
/// obtainable today; the last three are DISTINCT ways a value can fail to be,
/// and they close differently. `UnreadEngineState` closes with an RE session
/// and a wire field; `OnDemandComputation` could only close by proving a
/// resident cache; `PanelProductOnly` could only close by staging client
/// interaction, which attach-and-play forbids outright. A later lane that
/// closes one therefore learns nothing about the others, which is why they are
/// not collapsed into one "not available" variant.
///
/// The enum is declared here and again in `decision_evaluation.rs` rather than
/// shared from `mod.rs`. That is the scaffold's whole design: a carrier file
/// must be editable by its lane without touching a site another lane also
/// edits, and `mod.rs` is exactly that site. `emit` is duplicated thirteen
/// times for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassiveSource {
    /// Engine-resident, all-country, maintained whether or not any panel is
    /// open, and reachable by an ordinary passive read.
    EngineResident {
        /// Why the family is maintained for every country: the a729 function
        /// that maintains it, or the extractor domain that already carries it.
        evidence: &'static str,
        /// Where the extractor reads or emits it today.
        extractor_reader: &'static str,
    },
    /// Not an engine read at all: an identity over the effective game and mod
    /// files, which the recorder already packages for every capture.
    EffectiveContent { extractor_reader: &'static str },
    /// Replay reads a key the extractor has never emitted, and the domain's
    /// wire row carries no field it could be emitted from. Distinct from the
    /// two below because it is not closed by a law or a ruling: it needs an
    /// engine read that does not exist yet, which is an RE session and a wire
    /// field, not a decision.
    UnreadEngineState {
        /// The replay site that reads the key.
        consumer_read: &'static str,
        /// What the extractor would have to obtain, and why it cannot today.
        needed_read: &'static str,
    },
    /// Computed on demand by a producer that returns a value and stores it only
    /// on the transient view it was called for.
    OnDemandComputation { evidence: &'static str },
    /// Materialized only while the client builds the panel, for the one country
    /// the client is viewing.
    PanelProductOnly { evidence: &'static str },
}

impl PassiveSource {
    const fn is_passively_readable(&self) -> bool {
        matches!(
            self,
            PassiveSource::EngineResident { .. } | PassiveSource::EffectiveContent { .. }
        )
    }
}

/// One family of values the consumer requires, and where it lives.
#[derive(Debug, Clone, Copy)]
pub(super) struct ContractFamily {
    /// The name used in the refusal, so prose and inventory cannot drift.
    pub(super) name: &'static str,
    /// The consumer ITEM that demands it, as `path::item`.
    pub(super) consumer_citation: &'static str,
    pub(super) source: PassiveSource,
}

/// The consumer's contract, decomposed into families by where each family's
/// value lives. This is the lane's finding in machine-readable form.
pub(super) const CONTRACT_FAMILIES: [ContractFamily; 12] = [
    ContractFamily {
        name: "complete D18 country roster and per-country identity",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        source: PassiveSource::EngineResident {
            evidence: "d18 country_identity is production_complete and all-country",
            extractor_reader: "domain_registry.json d18 country_identity",
        },
    },
    ContractFamily {
        name: "per-country focus tree id",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        source: PassiveSource::EngineResident {
            evidence: "d12 focus_live is a country-scoped compact domain read for every \
                            country, not for the viewed one",
            extractor_reader: "world_fold.rs::fold_focus emits the per-country `focus_tree` key",
        },
    },
    ContractFamily {
        name: "per-country visible focus order",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        source: PassiveSource::UnreadEngineState {
            consumer_read: "replay_core/src/world.rs::build_focus reads focus.visible_order as a string \
                            list, and reads focus.available beside it the same way",
            needed_read: "world_fold.rs::fold_focus emits neither key, and the d12 country row it \
                          parses carries no such field -- the row is tag, flags, completed_count, \
                          progress_raw, focus_tree, current, current_continuous plus completed \
                          tokens. The engine's per-country visible/available focus enumeration \
                          has to be read and added to the d12 wire row before any XUI10 carrier \
                          can be admitted, because the consumer joins every node against this \
                          leaf and refuses an unavailable one",
        },
    },
    ContractFamily {
        name: "effective content manifest and authored GUI identity",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        source: PassiveSource::EffectiveContent {
            extractor_reader: "recorder_host/src/campaign_identity.rs effective-content manifest",
        },
    },
    ContractFamily {
        name: "per-country focus progress raw value",
        consumer_citation: "replay_core/src/world.rs::NationalFocusObserverProgressExact",
        source: PassiveSource::EngineResident {
            evidence: "the country's focus progress is engine state, ticked for every country",
            extractor_reader: "world_fold.rs::fold_focus reads `progress_raw` out of the d12 \
                               country row; it publishes the normalised quotient under \
                               `progress`, but the raw value is on the wire",
        },
    },
    ContractFamily {
        // The scale is a SEPARATE row from the raw value and from the required
        // progress, because all three are in different states. Finding it took
        // two review rounds: a name-based `progress_scale` probe misses it
        // entirely, since d12 spells it with the generic fixed-point constant.
        name: "per-country focus progress scale",
        consumer_citation: "replay_core/src/world.rs::NationalFocusObserverProgressExact",
        source: PassiveSource::EngineResident {
            evidence: "d12's focus payload carries COMPACT_FIXED_SCALE_RAW in its header, and \
                       that is the scale `progress_raw` is encoded at -- \
                       collector/src/attach_log.rs writes the constant into the header and \
                       encodes progress through `compact_scaled_i32(.., \
                       COMPACT_FIXED_SCALE_RAW as f64)`",
            extractor_reader: "recorder_host/src/domain_data_authority.rs reads the header \
                               scale at payload offset 20 and rejects the record when it is \
                               not COMPACT_FIXED_SCALE_RAW. NOTE: world_fold.rs::fold_focus \
                               reads offsets 8, 16 and 24 only, so the value is captured and \
                               validated but not re-emitted into the snapshot; a producer for \
                               this carrier reads it from the payload, not from `focus`",
        },
    },
    ContractFamily {
        // Split from the raw value and the scale rather than bundled with
        // either, because the three are NOT in the same state and bundling them
        // read as "progress is available". The consumer's progress tuple is
        // four fields; exactly one of them has no producer.
        name: "per-country effective required progress",
        consumer_citation: "replay_core/src/world.rs::NationalFocusObserverProgressExact",
        source: PassiveSource::UnreadEngineState {
            consumer_read: "world.rs::NationalFocusObserverProgressExact requires \
                            effective_required_progress_raw beside progress_raw and \
                            progress_scale_raw, and \
                            world.rs::build_national_focus_observer_exact rejects the row \
                            unless it is positive",
            needed_read: "`effective_required_progress` appears NOWHERE in the extractor once \
                          this file is excluded from the probe -- it is named here, so a \
                          probe that does not exclude `wave2_sidecars/` matches the very \
                          claim it is meant to test, the same self-contamination this lane \
                          records as IE-F2 one directory over. Exact commands and output: \
                          runs/ie_gp_u3/IE_F6_negative_proof.txt. The d12 payload carries the \
                          progress numerator and its scale and nothing about the denominator, \
                          so the focus cost after modifiers has to be read from the engine \
                          before the tuple can be emitted",
        },
    },
    ContractFamily {
        name: "per-node display progress percent",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        source: PassiveSource::OnDemandComputation {
            evidence: "lanes/focus_research_decisions_depth_live/src/\
                            a729_focus_item_producer.rs DISPLAY_PROGRESS_PRODUCER_RVA runs per \
                            item of a built view and stores its result on that item only",
        },
    },
    ContractFamily {
        name: "per-node presentation state, glow and historical-marker flags and joint-view pair",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        source: PassiveSource::PanelProductOnly {
            evidence: "lanes/focus_research_decisions_depth_live/src/\
                            a729_focus_item_producer.rs ITEM_PRESENTATION_STATE_OFFSET 0x540 on \
                            CFocusTreeView items reached through VIEW_VECTOR_DATA_OFFSET 0x2058; \
                            one view, the viewed country, only while the panel is open",
        },
    },
    ContractFamily {
        // Link TOPOLOGY, kept apart from the laid-out geometry below. Which
        // focus prerequisites exist, and whether a set is mutually exclusive,
        // is authored content; where the tiles ended up on a canvas is not.
        name: "link tile topology: from/to focus ids and mutual exclusivity",
        consumer_citation: "replay_core/src/world.rs::NationalFocusObserverLinkExact",
        source: PassiveSource::EffectiveContent {
            extractor_reader: "the authored focus definitions carry each focus's prerequisite \
                               sets and which of them are mutually exclusive; the recorder \
                               already packages the effective content they live in",
        },
    },
    ContractFamily {
        // Every remaining field of NationalFocusObserverLinkExact and
        // NationalFocusObserverLinkChildExact, named individually because an
        // earlier revision said "canvas positions and child sprite operations"
        // and the review found `container_width_pixels`, child `visible` and
        // child `local_position_pixels` covered by no family at all.
        name: "laid-out link geometry: container width, canvas and local positions, child \
               visibility, sprite name and frame index",
        consumer_citation: "replay_core/src/world.rs::NationalFocusObserverLinkChildExact",
        source: PassiveSource::PanelProductOnly {
            evidence: "the consumer requires container_width_pixels present exactly when the \
                       link is mutually exclusive, one child operation per authored widget \
                       (link1/link2/left/right/mid, or a single `link`), and for each of them \
                       a visibility flag, a sprite name, a frame index and a local pixel \
                       offset. Every one of those is written while the GUI lays the scene \
                       out; none exists before a view is built, and there is one view, for \
                       the country the client is viewing",
        },
    },
    ContractFamily {
        name: "per-node canvas position",
        consumer_citation: "replay_core/src/world.rs::NationalFocusObserverNodeExact",
        source: PassiveSource::PanelProductOnly {
            evidence: "`canvas_position_pixels` is the node's laid-out position in the built \
                       focus scene, written onto the CFocusTreeView item alongside the \
                       presentation state at ITEM_PRESENTATION_STATE_OFFSET",
        },
    },
];

/// True only when every family in [`CONTRACT_FAMILIES`] is passively readable.
///
/// Derived, not asserted: the constant is what `fill` refuses on, and the test
/// below is what stops the refusal from outliving its own reason.
pub(super) const CARRIER_PRODUCIBLE: bool = all_families_passively_readable();

const fn all_families_passively_readable() -> bool {
    let mut index = 0;
    while index < CONTRACT_FAMILIES.len() {
        let family = &CONTRACT_FAMILIES[index];
        // A family that cannot say what it is or which consumer line demands it
        // is not a record of anything, so it can never count as proven however
        // its `source` reads. Checking it here rather than only in a test means
        // the carrier stays refused instead of being unblocked by a blank row.
        if family.name.is_empty() || family.consumer_citation.is_empty() {
            return false;
        }
        if !family.source.is_passively_readable() {
            return false;
        }
        index += 1;
    }
    true
}

pub(super) const CARRIER: Wave2Carrier = Wave2Carrier {
    slot: PendingProducerSlot {
        wire_key: "_r2_national_focus_observer_exact",
        scope: "root",
        ledger_row: "EIR-NE-002",
        consumer_citation: "replay_core/src/world.rs::build_national_focus_observer_exact",
        // The LITERAL, not `SCHEMA_VERSION`: see the note in `decision_evaluation.rs`.
        // `wave2_scope_enumerate.py` reads this field by pattern and reported "no
        // required schema recorded" when it held a constant.
        required_schema: RequiredSchema::Version(1),
        wire_absence: WireAbsence::TypedNull,
        reason: "d12 already produces this carrier's tree identity and its per-country focus \
                 progress raw value and progress scale, and the effective content carries the \
                 link tile topology, but six families do not exist on the producer side. The \
                 per-country visible focus order and the per-country effective required \
                 progress are values replay reads and no extractor code produces. The \
                 per-node display progress percent, the per-node presentation state, glow and \
                 historical-marker flags and joint-view pair, the per-node canvas position, \
                 and the laid-out link geometry: container width, canvas and local positions, \
                 child visibility, sprite name and frame index have no passive all-country \
                 source: the last three are products of the client's own focus panel, which \
                 is why d89 is descheduled rather than widened, and the display percent is \
                 computed per item of a built view and stored nowhere else",
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
/// Fail closed. [`CARRIER_PRODUCIBLE`] is false while any family in
/// [`CONTRACT_FAMILIES`] lacks a passive all-country source, so no payload is
/// produced and `emit` publishes the typed null the consumer maps to
/// `DomainUnavailable`. The lane that proves the last family writes the
/// assembly here; the test below is what makes forgetting impossible to land
/// quietly.
fn fill(fold: &WorldFold, feed: &mut Wave2Feed) {
    let _ = fold;
    // The invariant, not a formality: once the inventory says every family has a
    // passive all-country source, refusing here is a defect rather than a
    // conservative default, and a debug build says so at the point of refusal
    // instead of leaving it to a reader to notice.
    debug_assert!(
        !CARRIER_PRODUCIBLE,
        "every contract family is passively readable, so this hour owes an assembled carrier"
    );
    feed.national_focus_observer = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The producer half of EIR-NE-002's acceptance gate, over one hour's
    /// snapshot.
    ///
    /// Written against a decoded snapshot rather than the feed because the feed
    /// cannot express the failure this exists to catch — a key that never
    /// reached the wire at all. It is ONE function so that a fixture hour and a
    /// recorded hour are judged identically; two copies is how those drift.
    fn carrier_law(snapshot: &Value) -> Result<(), String> {
        let key = CARRIER.slot.wire_key;
        let object = snapshot
            .as_object()
            .ok_or_else(|| "an hour's snapshot is a JSON object".to_owned())?;
        let value = object.get(key).ok_or_else(|| {
            format!(
                "{key} is absent from the hour. This consumer's clean unobtained state is a \
                 typed null (`replay_core/src/world.rs::build_national_focus_observer_exact`); absence instead tells replay \
                 the recorder has never heard of the carrier"
            )
        })?;
        if !CARRIER_PRODUCIBLE {
            if value.is_null() {
                return Ok(());
            }
            return Err(format!(
                "{key} carries a payload while {} contract families still have no passive \
                 all-country source; a scene assembled from anything else is forged",
                CONTRACT_FAMILIES
                    .iter()
                    .filter(|family| !family.source.is_passively_readable())
                    .count()
            ));
        }
        let payload = value
            .as_object()
            .ok_or_else(|| format!("{key} is producible this build, so it must carry an object"))?;
        payload_identity_law(payload)
    }

    /// The header identity the consumer compares before it looks at a single
    /// row (`replay_core/src/world.rs::build_national_focus_observer_exact`–`:33455`).
    ///
    /// Separate from [`carrier_law`] so it can be exercised in both directions
    /// today, rather than first evaluated against the recording it is meant to
    /// judge. Note the shape difference from XUI19: this consumer compares an
    /// integer `schema_version` and a literal `producer_contract`, not a schema
    /// token — a producer that satisfies the number and misses the row name is
    /// still refused.
    fn payload_identity_law(payload: &serde_json::Map<String, Value>) -> Result<(), String> {
        let version = payload.get("schema_version").and_then(Value::as_u64);
        if version != Some(u64::from(SCHEMA_VERSION)) {
            return Err(format!(
                "schema_version is {version:?}, and the consumer compares it against \
                 {SCHEMA_VERSION}"
            ));
        }
        for (field, expected) in [
            ("authority", AUTHORITY),
            ("executable_sha256", EXECUTABLE_SHA256),
            ("producer_contract", PRODUCER_CONTRACT),
        ] {
            let found = payload.get(field).and_then(Value::as_str);
            if found != Some(expected) {
                return Err(format!(
                    "{field} is {found:?}, and the consumer compares it whole against {expected:?}"
                ));
            }
        }
        for flag in [
            "passive_all_country",
            "ui_open_independent",
            "recorder_pov_independent",
            "producer_complete",
        ] {
            if payload.get(flag).and_then(Value::as_bool) != Some(true) {
                return Err(format!(
                    "{flag} must be true; the consumer rejects the whole hour otherwise"
                ));
            }
        }
        Ok(())
    }

    /// One hour's snapshot as the scaffold actually assembles it, through the
    /// production merge rather than a hand-written object.
    fn scaffold_snapshot(feed: &Wave2Feed) -> Value {
        let batch = super::super::build_wave2_sidecars(feed);
        let mut snapshot = json!({"provenance": {"notes": []}});
        super::super::merge_root_into_snapshot(&mut snapshot, batch.root);
        snapshot
    }

    fn well_formed_payload() -> Value {
        json!({
            "schema_version": SCHEMA_VERSION,
            "authority": AUTHORITY,
            "executable_sha256": EXECUTABLE_SHA256,
            "producer_contract": PRODUCER_CONTRACT,
            "passive_all_country": true,
            "ui_open_independent": true,
            "recorder_pov_independent": true,
            "producer_complete": true,
        })
    }

    /// The capture-wired behavior gate for EIR-NE-002 on the producer side.
    ///
    /// Point it at the next real capture with
    /// `BATTLE_PLAN_IE_GP_U3_CAPTURE_SNAPSHOT=<path to an hour's snapshot JSON>`. With
    /// the variable unset it runs the IDENTICAL law over the snapshot this
    /// scaffold assembles, so it is a live assertion either way rather than a
    /// skip that reports green on an empty run.
    ///
    /// kills: making `emit` omit this key when unfed. Replay would report
    /// `DomainAbsent` — "this recorder has never heard of the carrier" —
    /// instead of the `DomainUnavailable` the consumer defines at
    /// `world.rs::build_national_focus_observer_exact`, and the XUI10 shipping route's named gap
    /// (`production_renderer.rs::prepare_xui10_focus_route_from_admitted_topology`,
    /// which prints the status)
    /// would stop distinguishing a deliberate refusal from an older recorder.
    /// Against a real capture it additionally kills a producer that ships a
    /// payload while the read law still refuses.
    #[test]
    fn the_hour_carries_this_key_and_never_a_silent_absence() {
        let snapshot = match std::env::var("BATTLE_PLAN_IE_GP_U3_CAPTURE_SNAPSHOT").or_else(|_| std::env::var("HOI4_IE_GP_U3_CAPTURE_SNAPSHOT")) {
            Ok(path) => {
                let bytes = std::fs::read(&path)
                    .unwrap_or_else(|error| panic!("capture snapshot {path}: {error}"));
                serde_json::from_slice::<Value>(&bytes)
                    .unwrap_or_else(|error| panic!("capture snapshot {path} is not JSON: {error}"))
            }
            Err(_) => scaffold_snapshot(&Wave2Feed::unfed()),
        };
        carrier_law(&snapshot).unwrap_or_else(|error| panic!("{error}"));
    }

    /// kills: a `payload_identity_law` that accepts a payload the consumer
    /// would reject -- dropping a field, comparing the integer version without
    /// the `producer_contract` row name, or letting a `false` completeness flag
    /// through. That is the branch a real capture runs the day the read law
    /// closes, so it is exercised in both directions now.
    #[test]
    fn the_capture_law_reads_the_pinned_identity_rather_than_merely_the_key() {
        let good = well_formed_payload();
        payload_identity_law(good.as_object().unwrap()).expect("the pinned identity passes");

        for (field, wrong) in [
            ("schema_version", json!(2)),
            // A near-miss rather than another carrier's real authority: see the same
            // note in `decision_evaluation.rs`.
            ("authority", json!("passive_all_country_national_focus_scene_v1")),
            ("executable_sha256", json!("0".repeat(64))),
            ("producer_contract", json!("EIR-NE-007")),
            ("passive_all_country", json!(false)),
            ("ui_open_independent", json!(false)),
            ("recorder_pov_independent", json!(false)),
            ("producer_complete", json!(false)),
        ] {
            let mut payload = good.clone();
            payload[field] = wrong;
            let error = payload_identity_law(payload.as_object().unwrap())
                .expect_err("a value the consumer rejects must not pass here");
            assert!(error.contains(field), "{error}");
        }

        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("authority");
        payload_identity_law(missing.as_object().unwrap())
            .expect_err("an absent field is not a passing field");
    }

    /// kills: a `carrier_law` that ignores the read law and accepts a payload
    /// while contract families are still unproven, or that treats an absent key
    /// as clean. Both are the shapes a capture is judged on.
    #[test]
    fn the_capture_law_refuses_a_payload_the_read_law_has_not_earned() {
        assert!(!CARRIER_PRODUCIBLE);
        let refused = carrier_law(&json!({ CARRIER.slot.wire_key: well_formed_payload() }))
            .expect_err("a payload contradicts the read law");
        assert!(refused.contains("forged"), "{refused}");

        assert!(carrier_law(&json!({}))
            .expect_err("absence is not a clean state for this consumer")
            .contains("absent from the hour"));

        carrier_law(&json!({ CARRIER.slot.wire_key: Value::Null }))
            .expect("the typed null is the refusal the consumer defines");
    }

    /// kills: proving the last unproven family in `CONTRACT_FAMILIES` without
    /// writing the carrier assembly in `fill`. `CARRIER_PRODUCIBLE` becomes
    /// true, this assertion fails, and the typed null cannot outlive the reason
    /// it was derived from. Without it a later lane could record that every
    /// value is now readable and still ship an hour that says the carrier was
    /// unobtainable.
    #[test]
    fn the_refusal_lasts_exactly_as_long_as_a_family_has_no_passive_source() {
        assert!(
            !CARRIER_PRODUCIBLE,
            "every contract family is now passively readable, so `fill` must assemble the \
             carrier instead of leaving the typed null in place"
        );
        let mut feed = Wave2Feed::unfed();
        feed.national_focus_observer = None;
        assert_eq!(emit(&feed), SidecarEmission::Single(Value::Null));
    }

    /// The complete inventory, as a manifest that must match exactly.
    ///
    /// kills: DELETING a family and shortening the array. Every other test here
    /// iterates `CONTRACT_FAMILIES` and so is satisfied by any shorter list —
    /// drop "per-country focus progress scale", set the length to 11, and the
    /// reason test, the evidence test and `CARRIER_PRODUCIBLE` are all still
    /// green while the carrier has quietly stopped accounting for a value the
    /// consumer requires. It also kills reclassifying a family in place, which
    /// is how an inventory drifts toward "producible" one row at a time.
    #[test]
    fn the_inventory_is_exactly_this_manifest() {
        let manifest: Vec<(&str, &str)> = CONTRACT_FAMILIES
            .iter()
            .map(|family| {
                let kind = match family.source {
                    PassiveSource::EngineResident { .. } => "EngineResident",
                    PassiveSource::EffectiveContent { .. } => "EffectiveContent",
                    PassiveSource::UnreadEngineState { .. } => "UnreadEngineState",
                    PassiveSource::OnDemandComputation { .. } => "OnDemandComputation",
                    PassiveSource::PanelProductOnly { .. } => "PanelProductOnly",
                };
                (family.name, kind)
            })
            .collect();
        assert_eq!(
            manifest,
            vec![
                (
                    "complete D18 country roster and per-country identity",
                    "EngineResident"
                ),
                ("per-country focus tree id", "EngineResident"),
                ("per-country visible focus order", "UnreadEngineState"),
                (
                    "effective content manifest and authored GUI identity",
                    "EffectiveContent"
                ),
                ("per-country focus progress raw value", "EngineResident"),
                ("per-country focus progress scale", "EngineResident"),
                (
                    "per-country effective required progress",
                    "UnreadEngineState"
                ),
                ("per-node display progress percent", "OnDemandComputation"),
                (
                    "per-node presentation state, glow and historical-marker flags and \
                     joint-view pair",
                    "PanelProductOnly"
                ),
                (
                    "link tile topology: from/to focus ids and mutual exclusivity",
                    "EffectiveContent"
                ),
                (
                    concat!(
                        "laid-out link geometry: container width, canvas and local ",
                        "positions, child visibility, sprite name and frame index"
                    ),
                    "PanelProductOnly"
                ),
                ("per-node canvas position", "PanelProductOnly"),
            ],
            "the inventory changed; if that is deliberate, update this manifest AND the slot \
             reason in the same change"
        );
    }

    /// kills: the refusal's own arithmetic drifting away from the inventory.
    ///
    /// Same guard as the one in `decision_evaluation.rs`, for the same reason:
    /// the reason sentence counts the families with no passive source, that
    /// number went stale twice while families were being split, and nothing
    /// checked it.
    #[test]
    fn the_refusal_counts_the_unproven_families_correctly() {
        let unproven = CONTRACT_FAMILIES
            .iter()
            .filter(|family| !family.source.is_passively_readable())
            .count();
        assert_eq!(unproven, 6, "six families have no passive all-country source");
        assert!(
            CARRIER.slot.reason.contains("six families do not exist on the"),
            "the refusal must state the count the inventory actually holds; it says: {}",
            CARRIER.slot.reason
        );
        assert_eq!(
            CONTRACT_FAMILIES.len() - unproven,
            6,
            "six families are obtainable"
        );
    }

    /// kills: adding a family to `CONTRACT_FAMILIES` (or changing one's name)
    /// without naming it in the slot's `reason`. The wire carries a typed null
    /// and nothing else, so the declaration IS the answer to "why"; a reason
    /// that has silently stopped listing everything unproven is a refusal that
    /// understates itself.
    #[test]
    fn the_declared_reason_names_every_family_that_is_not_passively_readable() {
        for family in CONTRACT_FAMILIES {
            if family.source.is_passively_readable() {
                continue;
            }
            assert!(
                CARRIER.slot.reason.contains(family.name),
                "the refusal does not name the unproven family {:?}",
                family.name
            );
        }
    }

    /// kills: recording an unproven family with an empty citation, or a proven
    /// one with no extractor reader. `EngineResident` and `EffectiveContent`
    /// are the variants that claim a value is already obtainable, so they are
    /// the ones that have to name where it is read; the other THREE have to
    /// name the evidence for why they are not. All five variants are matched
    /// exhaustively, so adding a SIXTH without giving it required fields fails
    /// to compile here rather than passing unchecked.
    #[test]
    fn every_family_carries_the_evidence_its_verdict_rests_on() {
        for family in CONTRACT_FAMILIES {
            assert!(!family.name.is_empty());
            assert!(
                family.consumer_citation.starts_with("replay_core/"),
                "{} cites {:?}",
                family.name,
                family.consumer_citation
            );
            // An ITEM name, not a line number. The repository's citation
            // integrity gate refuses a new `file.rs:NNN` reference into another
            // file outright: a line number in a file every lane edits is stale
            // within a day, while an item name survives insertions above it.
            let (path, item) = family
                .consumer_citation
                .split_once("::")
                .expect("a citation names an item, as path::item");
            assert!(path.ends_with(".rs"), "{:?}", family.consumer_citation);
            assert!(!item.is_empty(), "{:?}", family.consumer_citation);
            assert!(
                item.parse::<u32>().is_err(),
                "{:?} names a line number where an item belongs",
                family.consumer_citation
            );
            match family.source {
                PassiveSource::EngineResident {
                    evidence,
                    extractor_reader,
                } => {
                    assert!(!evidence.is_empty(), "{}", family.name);
                    assert!(!extractor_reader.is_empty(), "{}", family.name);
                }
                PassiveSource::EffectiveContent { extractor_reader } => {
                    assert!(!extractor_reader.is_empty(), "{}", family.name);
                }
                PassiveSource::UnreadEngineState {
                    consumer_read,
                    needed_read,
                } => {
                    assert!(!consumer_read.is_empty(), "{}", family.name);
                    assert!(!needed_read.is_empty(), "{}", family.name);
                }
                PassiveSource::OnDemandComputation { evidence }
                | PassiveSource::PanelProductOnly { evidence } => {
                    assert!(!evidence.is_empty(), "{}", family.name);
                }
            }
        }
    }

    /// kills: rescheduling d89 and re-pointing its POV-local carrier at this
    /// key -- the mistake the gap map's section 4.3 warns each brief about by
    /// name. The THREE families d89's view produces — the node presentation
    /// state, the node canvas position and the laid-out link geometry — must
    /// stay recorded as panel products; recording any of them as obtainable
    /// would move
    /// `CARRIER_PRODUCIBLE` one step closer to true on evidence that is about
    /// the viewed country only, and the descheduling ruling
    /// (`compact_contract/src/lib.rs::DESCHEDULED_CAPTURE_DOMAIN_IDS`) would have
    /// been "fixed" by mistake.
    #[test]
    fn the_panel_produced_families_are_never_recorded_as_engine_resident() {
        let panel_products = CONTRACT_FAMILIES
            .iter()
            .filter(|family| matches!(family.source, PassiveSource::PanelProductOnly { .. }))
            .count();
        assert_eq!(
            panel_products, 3,
            "the node presentation state, the node canvas position and the laid-out link              geometry are the panel's products"
        );
    }

    /// kills: collapsing the progress tuple back into fewer families than the
    /// number of distinct states its fields are in.
    ///
    /// The consumer's `NationalFocusObserverProgressExact` is four fields in
    /// THREE states, and getting that wrong took two review rounds to settle:
    ///
    /// * `progress_raw` — captured. `fold_focus` reads it off the d12 row.
    /// * `progress_scale_raw` — captured. d12's payload header carries
    ///   `COMPACT_FIXED_SCALE_RAW`, which is the scale `progress_raw` is
    ///   encoded at, and `domain_data_authority.rs` rejects the record if the
    ///   header disagrees. A name-based `progress_scale` grep misses this
    ///   entirely, which is how it was first mis-filed as absent.
    /// * `effective_required_progress_raw` — absent. Nothing in the extractor
    ///   names it, with this file excluded from the probe; see
    ///   `runs/ie_gp_u3/IE_F6_negative_proof.txt`.
    /// * `display_progress_percent` — its own family, an on-demand computation
    ///   of the built view.
    ///
    /// One family over all four reads as "progress is available" and moves
    /// `CARRIER_PRODUCIBLE` closer to true on a value that does not exist. One
    /// family over the last three understates what d12 already carries and
    /// would send a later lane re-reading a scale it already has. Both errors
    /// were made here in turn; the split is what this test pins.
    ///
    /// Every field is asserted BY NAME, not by counting the array. An earlier
    /// revision counted, and deleting the display-progress family while
    /// shortening the array to nine left this test and every other count-based
    /// test green — an inventory can always be made consistent by dropping the
    /// row that was inconvenient.
    #[test]
    fn the_progress_numerator_and_its_denominator_are_recorded_separately() {
        let raw = CONTRACT_FAMILIES
            .iter()
            .find(|family| family.name == "per-country focus progress raw value")
            .expect("the raw-progress family is declared");
        assert!(
            raw.source.is_passively_readable(),
            "d12 does carry progress_raw"
        );
        let scale = CONTRACT_FAMILIES
            .iter()
            .find(|family| family.name == "per-country focus progress scale")
            .expect("the progress-scale family is declared");
        assert!(
            scale.source.is_passively_readable(),
            "d12's payload header carries the fixed-point scale progress_raw is encoded at"
        );
        let denominator = CONTRACT_FAMILIES
            .iter()
            .find(|family| family.name == "per-country effective required progress")
            .expect("the required-progress family is declared");
        assert!(
            matches!(denominator.source, PassiveSource::UnreadEngineState { .. }),
            "the effective required progress has no producer read; the scale beside it does"
        );
        assert!(
            CARRIER.slot.reason.contains(denominator.name),
            "the refusal must name the missing denominator"
        );
        // The fourth field, by NAME rather than by counting. Without this the
        // test passed with the display-progress family deleted and the array
        // shortened to nine, because every other assertion here and in the
        // count-based tests is satisfied by a shorter inventory that simply
        // stops mentioning a required value.
        let display = CONTRACT_FAMILIES
            .iter()
            .find(|family| family.name == "per-node display progress percent")
            .expect("the display-progress family is declared");
        assert!(
            matches!(display.source, PassiveSource::OnDemandComputation { .. }),
            "the display percent is computed per item of a built view; recording it as \
             readable or as absent both misstate what closing it would take"
        );
        assert!(
            CARRIER.slot.reason.contains(display.name),
            "the refusal must name the display percent"
        );
    }

    /// kills: recording the visible-focus-order family as already produced.
    ///
    /// This is the lane's second finding and the easiest one to get wrong,
    /// because the neighbouring `focus_tree` key IS emitted and the two are
    /// read side by side (`world.rs::build_focus`). They are not in the same
    /// state: `world_fold.rs::fold_focus` emits `focus_tree` and never
    /// `visible_order` or `available`, and the d12 wire row carries no field
    /// either could come from. The consumer requires
    /// `Leaf::Measured(d12_visible)` for every roster country, so this alone
    /// refuses the frame even if the whole scene half were solved. Recording it
    /// as readable would send the next lane looking only at the panel problem
    /// and leave a carrier that can never be admitted.
    #[test]
    fn the_visible_focus_order_leaf_is_recorded_as_a_key_no_producer_emits() {
        let readable = CONTRACT_FAMILIES
            .iter()
            .filter(|family| family.source.is_passively_readable())
            .count();
        assert_eq!(
            readable, 6,
            "roster, tree id, content identity, progress raw value, progress scale and link              topology"
        );
        let visible_order = CONTRACT_FAMILIES
            .iter()
            .find(|family| family.name.contains("visible focus order"))
            .expect("the visible-order family is declared");
        assert!(
            matches!(
                visible_order.source,
                PassiveSource::UnreadEngineState { .. }
            ),
            "the visible-order leaf has no producer read; it is not a panel product and not \
             already emitted"
        );
        assert!(CONTRACT_FAMILIES.iter().any(|family| family.name
            == "per-country focus tree id"
            && family.source.is_passively_readable()));
    }

    /// kills: editing the pinned identity to a value the consumer does not
    /// hold. `runs/ie_gp_u3/xui_identity_crosscheck.py` is the half that
    /// re-reads them from the consumer's source, which a producer-local test
    /// cannot do across cargo workspaces.
    #[test]
    fn the_pinned_identity_is_the_consumers_own() {
        assert_eq!(
            AUTHORITY,
            "exact_a729_passive_all_country_national_focus_scene_v1"
        );
        assert_eq!(PRODUCER_CONTRACT, CARRIER.slot.ledger_row);
        assert_eq!(
            CARRIER.slot.required_schema,
            RequiredSchema::Version(SCHEMA_VERSION)
        );
        assert_eq!(EXECUTABLE_SHA256.len(), 64);
        assert!(EXECUTABLE_SHA256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
}
