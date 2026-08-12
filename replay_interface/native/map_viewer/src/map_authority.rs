//! XMAP00's presentation-free product-map authority boundary.
//!
//! XMAP00 has proved the stable input-family identities. For most families it
//! has *not* proved a recording-authenticated canonical payload, enumerator,
//! schema, cadence, or identity order, and those families can only be
//! described as typed gaps.
//!
//! A family whose owning packet *has* supplied owner-proven production
//! admission is different: the authoritative XMAP00 ledger
//! (`re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`)
//! marks it `owner_proven_production_consumer`, and this module resolves its
//! payload/cadence/identity-order from that proof. Its per-hour disposition is
//! then **derived from the authenticated `WorldFrame`**: the input is admitted
//! only when the frame's own status accessor reports `Present`, and otherwise
//! stays a typed, named unavailability exactly like every unproven family. No
//! family is ever admitted because a caller said so.
//!
//! The serial form has `deny_unknown_fields`, so recorder camera, map-mode,
//! selection, GUI, selected-Present, and draw-geometry fields are rejected,
//! and [`admit_map_authority_json`] refuses an admitted row outright: an
//! admission is a statement about frame data the JSON door does not hold.
//!
//! # Where this module's authority ends
//!
//! An admission asserts exactly one thing: *the `WorldFrame` it was derived
//! from reported this input as measured.* It does not, and cannot, assert that
//! the frame itself is genuine. Frame authentication belongs to `replay_core`
//! ingest — container, index, and content-manifest hashing behind
//! `open_recording` — and every other map feature reads the same `WorldFrame`
//! on the same trust. A caller that can fabricate a `WorldFrame` (by supplying
//! its own `ReplaySource`) has already defeated every consumer downstream of
//! ingest, not merely this seam; the frame-derivation witness on
//! [`MapAuthorityProjection`] closes the narrower hole this module owns, namely
//! asserting an admission *without* any frame at all.

use replay_core::{ContentManifest, ViewStatus, WorldFrame};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

pub const MAP_AUTHORITY_INPUT_SCHEMA: &str = "battle-plan-map-authority-input-v4";

/// Stable presentation-independent input-family identities. These names are
/// requirements, not proof that any payload contract has been promoted.
pub const MAP_AUTHORITY_REQUIRED_INPUTS: &[&str] = &[
    "combat.land_active_epoch",
    "combat.land_terminal_detail",
    "combat.naval_air_epoch",
    "control.controller_by_province",
    "control.controller_by_state",
    "control.owner_by_state",
    "country.current_capital_state",
    "country.d18_complete_roster",
    "country.presentation_identity",
    "diplomacy.faction_roster",
    "diplomacy.pair_relations",
    "effects.semantic_lifecycle",
    "fog.province_visibility_by_pov",
    "fog.spotted_land_by_pov",
    "fog.spotted_taskforces_by_pov",
    "fog.spotted_wings_by_pov",
    "lighting.point_light_lifecycle",
    "logistics.rail_hub_frames",
    "logistics.rail_topology_state",
    "map_furniture.current_victory_points",
    "mapmode.air_superiority",
    "mapmode.infrastructure_state",
    "mapmode.intelligence",
    "mapmode.naval_supremacy",
    "mapmode.occupation_resistance_compliance",
    "mapmode.raids",
    "mapmode.resource_deposits",
    "mapmode.scripted_inputs",
    "mapmode.supply_state",
    "objects.building_state_damage",
    "objects.city_population",
    "objects.generic_population_lifecycle",
    "orders.army_battleplans_semantic",
    "orders.theatre_army_hierarchy",
    "postfx.temporal_exposure_state",
    "session.player_country_set",
    "terrain.snow_mud_semantic",
    "units.air_base_location",
    "units.divisions",
    "units.taskforces_fleets",
    "units.wings",
    "weather.province_snow_accumulator",
    "weather.region_gameplay",
    "weather.visual_membership_lifecycle",
    "world.calendar_time",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapAuthorityRequirementDisposition {
    Absent,
    Unavailable,
    /// The owning packet supplied owner-proven production admission *and* the
    /// authenticated `WorldFrame` reported the input as measured this hour.
    /// This disposition is only ever produced by
    /// [`map_authority_projection_from_world_frame`]; it can never be reached
    /// for a family without an entry in [`MAP_AUTHORITY_OWNER_PROVEN_INPUTS`].
    Admitted,
}

/// The metadata XMAP00 can state for a family that has **no** owner-proven
/// admission. `None` means the field is deliberately unresolved, not an
/// invitation for a caller to choose a local value. Families that *are*
/// owner-proven resolve these fields through
/// [`map_authority_owner_proven_admission`] instead; `blocker` is then the
/// named reason the input is missing from a particular hour, not a statement
/// that the owning packet still owes a proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapAuthorityRequirementMetadata {
    pub input_id: &'static str,
    pub payload_schema: Option<&'static str>,
    pub cadence: Option<&'static str>,
    pub identity_order: Option<&'static str>,
    pub blocker: &'static str,
}

/// One XMAP00 ledger row whose `xmap00_disposition` column reads
/// `owner_proven_production_consumer`. Every field is transcribed from the
/// authoritative ledger or from the `replay_core` decoder that builds the
/// named view; nothing here is minted locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapAuthorityOwnerProvenAdmission {
    pub input_id: &'static str,
    /// Ledger `owner_packet` column.
    pub owner_packet: &'static str,
    /// Ledger `portable_path` column — the presentation-free payload identity.
    pub payload_schema: &'static str,
    /// Ledger `status_accessor` column. This is the *only* accessor whose
    /// `Present` may admit this input; it is named here so the derivation
    /// cannot silently drift onto a neighbouring view's status.
    pub status_accessor: &'static str,
    pub cadence: &'static str,
    pub identity_order: &'static str,
}

/// The owner-proven admission table, mirroring the three
/// `owner_proven_production_consumer` rows of
/// `re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`.
///
/// `cadence` is the `WorldFrame` contract: one decoded view per canonical
/// recorded hour (`WorldFrame::hour_index`). `identity_order` is read out of
/// the decoder that builds each view in
/// `replay_interface/native/replay_core/src/world.rs`:
/// `build_combats` (`:7290`) preserves the recorder census order and carries
/// `(combat_id, generation)` per row; `build_ended_combats` (`:7345`) keys a
/// `BTreeMap` by `(combat_id, generation)`, so terminal rows are ascending by
/// that key; `build_naval_air_combat` (`:30947`) keeps the naval census in
/// recorder order and exposes the sunk-convoy and air partitions as separate
/// typed leaves.
pub const MAP_AUTHORITY_OWNER_PROVEN_INPUTS: &[MapAuthorityOwnerProvenAdmission] = &[
    MapAuthorityOwnerProvenAdmission {
        input_id: "combat.land_active_epoch",
        owner_packet: "XMAP14",
        payload_schema: "WorldFrame.combats",
        status_accessor: "combats_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "(combat_id, generation) per row, in recorder census order",
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "combat.land_terminal_detail",
        owner_packet: "XMAP14",
        payload_schema: "WorldFrame.ended_combats",
        status_accessor: "ended_combats_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "(combat_id, generation) ascending",
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "combat.naval_air_epoch",
        owner_packet: "XMAP14",
        payload_schema: "WorldFrame.naval_air_combat",
        status_accessor: "naval_air_combat_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "naval census in recorder order; sunk-convoy and air partitions typed separately",
    },
];

/// The accessor whose value a refusal reports.
///
/// Most inputs are read straight off a `WorldFrame` root accessor and their
/// blocker says so. Two owner-proven inputs are not, and must not claim they
/// are: `combat.naval_air_epoch` binds to a partition inside a composite view,
/// and `combat.land_terminal_detail` binds to a completeness-checked reading of
/// its status rather than the raw one. Both [`unavailable_requirement`] and
/// [`validate_requirement`] resolve the label through this one function, so a
/// row cannot carry a blocker that names an accessor the derivation never read.
pub fn map_authority_observed_status_label(input_id: &str) -> &'static str {
    match input_id {
        "combat.naval_air_epoch" => "naval_combats_status",
        "combat.land_terminal_detail" => "certified ended_combats_status",
        _ => "root status",
    }
}

/// The admission status for `combat.naval_air_epoch`.
///
/// The ledger names `naval_air_combat_status` as this input's accessor, but
/// that root status is a `combine_status` merge in which any measured sibling
/// dominates (`replay_core::world::build_naval_air_combat`), and the view's own
/// doc warns that a `Present` sibling would otherwise hide an explicitly
/// unobtained naval collection behind an empty vector. Admitting on the merged
/// status alone would let `naval_combats: null` plus a measured air partition
/// present an unread census as measured-empty.
///
/// The map-facing epoch is the naval census — exactly what XMAP14 requires
/// `Present` before it will validate its typed census — so this reports the
/// partition-local status instead. That is strictly stronger than the ledger's
/// root accessor and never disagrees with it about admission: `combine_status`
/// returns `Present` whenever any partition is `Present`, so a `Present` naval
/// census implies a `Present` root. Refusals then name the census itself
/// (see [`map_authority_observed_status_label`]) rather than misreporting the
/// root, which may well have been `Present` because of a sibling.
fn naval_air_epoch_status(frame: &WorldFrame) -> ViewStatus {
    frame.naval_air_combat().naval_combats_status
}

/// The admission status for `combat.land_terminal_detail`.
///
/// `build_ended_combats` returns `Present` unconditionally once the collection
/// itself is a well-formed array, even when it skipped rows — a non-object
/// element, a missing or non-positive `(combat_id, generation)`, or a lifecycle
/// that is not exactly `"ended"`. Its active sibling `build_combats` downgrades
/// to `DomainUnavailable` in the same situation. Admitting on the raw status
/// would therefore present a census that lost rows as a complete measurement,
/// which is the measured-empty-for-absent collapse the view's own doc forbids.
///
/// The decoder does record every one of those drops, by name, in
/// [`WorldFrame::coverage_warnings`], so completeness is checkable here: a
/// `Present` status with a recorded terminal-row drop is reported as
/// `DomainUnavailable` and keeps its named blocker. The regression that pins
/// this also pins the warning vocabulary, so if the decoder's wording moves the
/// test fails loudly instead of the check silently going quiet. It becomes
/// redundant the day `build_ended_combats` carries a malformed-row status like
/// its sibling; that one-line `replay_core` change is a named residual for that
/// crate's owner.
fn land_terminal_detail_status(frame: &WorldFrame) -> ViewStatus {
    let status = frame.ended_combats_status();
    if status == ViewStatus::Present && ended_combats_dropped_rows(frame) {
        return ViewStatus::DomainUnavailable;
    }
    status
}

fn ended_combats_dropped_rows(frame: &WorldFrame) -> bool {
    frame.coverage_warnings().iter().any(|warning| {
        warning.starts_with("ended_combats[") && warning.contains("row skipped")
    })
}

/// The owner-proven admission evidence for `input_id`, or `None` when the
/// owning packet still owes its proof.
pub fn map_authority_owner_proven_admission(
    input_id: &str,
) -> Option<MapAuthorityOwnerProvenAdmission> {
    MAP_AUTHORITY_OWNER_PROVEN_INPUTS
        .iter()
        .copied()
        .find(|admission| admission.input_id == input_id)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAuthorityRequirement {
    pub input_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cadence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_order: Option<String>,
    pub disposition: MapAuthorityRequirementDisposition,
    /// The named reason this input is not usable this hour. `None` is legal
    /// only on an [`MapAuthorityRequirementDisposition::Admitted`] row; a
    /// refused row that drops its blocker is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

const MAP_AUTHORITY_ADMISSION_WITNESS_DOMAIN: &str = "battle-plan-map-authority-frame-derivation-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAuthorityProjection {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_digest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub requirements: Vec<MapAuthorityRequirement>,
    /// In-process, tamper-evident witness that these rows were derived from an
    /// authenticated `WorldFrame`.
    ///
    /// It is set only by [`map_authority_projection_from_world_frame`], is
    /// never serialized and never deserialized, and cannot be written by any
    /// caller: the field is private, which also means callers outside this
    /// crate cannot build a projection with struct-literal syntax at all and
    /// must go through [`MapAuthorityProjection::from_untrusted_parts`], which
    /// leaves it `None`. Because the digest covers every requirement row,
    /// editing a derived projection — flipping a row to `Admitted`, altering an
    /// admitted contract, or deleting one — invalidates it too.
    #[serde(skip)]
    frame_derivation_witness_sha256: Option<String>,
}

impl MapAuthorityProjection {
    /// Assemble a projection from caller-supplied parts. Nothing a caller
    /// assembles carries a frame witness, so such a projection can only ever
    /// describe gaps; [`admit_map_authority_projection`] refuses any
    /// `Admitted` row on it.
    pub fn from_untrusted_parts(
        schema: String,
        hour_index: i64,
        content_manifest_digest_sha256: String,
        product_modset_sha256: String,
        effective_vfs_manifest_sha256: String,
        requirements: Vec<MapAuthorityRequirement>,
    ) -> Self {
        MapAuthorityProjection {
            schema,
            hour_index,
            content_manifest_digest_sha256,
            product_modset_sha256,
            effective_vfs_manifest_sha256,
            requirements,
            frame_derivation_witness_sha256: None,
        }
    }

    /// Whether these exact rows still carry the witness stamped when they were
    /// derived from a `WorldFrame`.
    pub fn is_frame_derived(&self) -> bool {
        match (
            self.frame_derivation_witness_sha256.as_deref(),
            self.admission_witness_sha256(),
        ) {
            (Some(stamped), Ok(recomputed)) => stamped == recomputed,
            _ => false,
        }
    }

    fn admission_witness_sha256(&self) -> Result<String, String> {
        let preimage = serde_json::to_vec(&(
            MAP_AUTHORITY_ADMISSION_WITNESS_DOMAIN,
            &self.schema,
            self.hour_index,
            &self.content_manifest_digest_sha256,
            &self.product_modset_sha256,
            &self.effective_vfs_manifest_sha256,
            &self.requirements,
        ))
        .map_err(|error| format!("map authority derivation witness preimage: {error}"))?;
        Ok(Sha256::hex(&preimage))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapAuthorityGap {
    pub input_id: String,
    pub disposition: MapAuthorityRequirementDisposition,
    pub reason: String,
}

/// One XMAP00 input the owning packet has proved and the authenticated
/// `WorldFrame` measured this hour. It carries the resolved contract so a
/// downstream feature can bind the view without re-deriving it, and nothing
/// else: no payload bytes, no caller hash, no receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapAuthorityAdmittedInput {
    pub input_id: String,
    pub owner_packet: String,
    pub payload_schema: String,
    pub cadence: String,
    pub identity_order: String,
}

/// An XMAP00 admission is the exhaustive split of the required inputs into the
/// ones the recording measured under owner-proven contracts and the ones that
/// remain typed gaps. There is intentionally still no `Ready` variant or
/// receipt: hashes supplied by a caller are not authentication, and the
/// admitted set can only be produced from an authenticated `WorldFrame`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapAuthorityAdmission {
    gaps: Vec<MapAuthorityGap>,
    admitted: Vec<MapAuthorityAdmittedInput>,
}

impl MapAuthorityAdmission {
    pub fn gaps(&self) -> &[MapAuthorityGap] {
        &self.gaps
    }

    pub fn admitted(&self) -> &[MapAuthorityAdmittedInput] {
        &self.admitted
    }

    pub fn is_admitted(&self, input_id: &str) -> bool {
        self.admitted
            .iter()
            .any(|input| input.input_id == input_id)
    }
}

/// Typed product refusal used by all shipping routes until a later owning
/// packet proves and authenticates the payload contracts and XMAP01 promotes
/// an independent renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Xmap01ProductMapRefusal {
    pub code: &'static str,
    pub reason: &'static str,
    pub authority_ready: bool,
    pub gap_ids: Vec<String>,
}

impl std::fmt::Display for Xmap01ProductMapRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {} (authority_ready={}, gaps={})",
            self.code,
            self.reason,
            self.authority_ready,
            self.gap_ids.join(",")
        )
    }
}

impl std::error::Error for Xmap01ProductMapRefusal {}

pub fn refuse_product_map_until_xmap01(
    admission: &MapAuthorityAdmission,
) -> Xmap01ProductMapRefusal {
    Xmap01ProductMapRefusal {
        code: "XMAP00_AUTHORITY_UNAVAILABLE",
        reason: "presentation-free map payload contracts are unresolved and XMAP01 is not permitted to render",
        authority_ready: false,
        gap_ids: admission
            .gaps()
            .iter()
            .map(|gap| gap.input_id.clone())
            .collect(),
    }
}

pub fn refuse_missing_xmap00_authority(gap_id: &str) -> Xmap01ProductMapRefusal {
    Xmap01ProductMapRefusal {
        code: "XMAP00_AUTHORITY_UNAVAILABLE",
        reason: "presentation-free map authority cannot be constructed because a structural world-state input is unavailable",
        authority_ready: false,
        gap_ids: vec![gap_id.to_string()],
    }
}

pub fn require_promoted_xmap01_renderer(
    admission: &MapAuthorityAdmission,
) -> Result<(), Xmap01ProductMapRefusal> {
    Err(refuse_product_map_until_xmap01(admission))
}

pub fn map_authority_requirement_metadata(
    input_id: &str,
) -> Option<MapAuthorityRequirementMetadata> {
    let blocker = match input_id {
        "control.controller_by_province"
        | "control.controller_by_state"
        | "control.owner_by_state"
        | "country.d18_complete_roster"
        | "country.presentation_identity"
        | "fog.province_visibility_by_pov"
        | "fog.spotted_land_by_pov"
        | "fog.spotted_taskforces_by_pov"
        | "fog.spotted_wings_by_pov" => {
            "XMAP05 must prove the exact complete identity/POV payload, enumeration, cadence, and authenticated producer contract"
        }
        "terrain.snow_mud_semantic" | "weather.province_snow_accumulator" => {
            "these semantic accumulators remain independently unavailable and cannot substitute for XMAP07's exact passive global dense texture/fade carrier"
        }
        "lighting.point_light_lifecycle" | "world.calendar_time" => {
            "XMAP09 must prove the exact time/light lifecycle, caster joins, cadence, and authenticated producer contract"
        }
        "postfx.temporal_exposure_state" => {
            "XMAP10 must prove the presentation-independent temporal exposure state, cadence, and authenticated producer contract"
        }
        "objects.city_population"
        | "objects.building_state_damage"
        | "objects.generic_population_lifecycle" => {
            "XMAP12 must prove the unconditional object identity/state/lifecycle census and authenticated producer/content joins"
        }
        "units.air_base_location"
        | "units.divisions"
        | "units.taskforces_fleets"
        | "units.wings"
        | "orders.theatre_army_hierarchy" => {
            "XMAP13 must prove the exact all-country unit/hierarchy subset, stable identity, order, completeness, and authenticated producer contract"
        }
        // XMAP14 has proved these three (ledger disposition
        // `owner_proven_production_consumer`), so their blocker is no longer a
        // pending proof: it names the hour's missing measurement.
        "combat.land_active_epoch" => {
            "the recording carries no measured active land-combat census for this hour"
        }
        "combat.land_terminal_detail" => {
            "the recording carries no measured terminal land-combat lifecycle census for this hour"
        }
        "combat.naval_air_epoch" => {
            "the recording carries no measured naval/air combat epoch for this hour"
        }
        "orders.army_battleplans_semantic" => {
            "XMAP15 must prove the semantic order/member/control payload and lifecycle; Replay geometry is derived and never recorder authority"
        }
        "logistics.rail_hub_frames" | "logistics.rail_topology_state" | "mapmode.supply_state" => {
            "XMAP16 must prove exact topology/hub/state identities, order, completeness, and authenticated producer/content joins"
        }
        "country.current_capital_state" | "map_furniture.current_victory_points" => {
            "XMAP21 must prove the exact live furniture value, identity, cadence, completeness, and authenticated producer/content joins"
        }
        "diplomacy.faction_roster"
        | "diplomacy.pair_relations"
        | "mapmode.air_superiority"
        | "mapmode.infrastructure_state"
        | "mapmode.intelligence"
        | "mapmode.naval_supremacy"
        | "mapmode.occupation_resistance_compliance"
        | "mapmode.raids"
        | "mapmode.resource_deposits"
        | "mapmode.scripted_inputs"
        | "session.player_country_set" => {
            "XMAP18 must prove the exact all-country map-mode input, unit, identity order, completeness, and authenticated producer/content joins"
        }
        "weather.region_gameplay" | "weather.visual_membership_lifecycle" => {
            "XMAP19 must prove exact-a729 gameplay weather and presentation-independent visual membership lifecycle contracts"
        }
        "effects.semantic_lifecycle" => {
            "XMAP20 must prove presentation-independent effect identity, lifecycle, anchors, seeds, enumeration, and authenticated producer/content joins"
        }
        _ => return None,
    };
    Some(MapAuthorityRequirementMetadata {
        input_id: MAP_AUTHORITY_REQUIRED_INPUTS
            .iter()
            .copied()
            .find(|known| *known == input_id)
            .expect("metadata match must name a stable family"),
        payload_schema: None,
        cadence: None,
        identity_order: None,
        blocker,
    })
}

fn unavailable_requirement(input_id: &str, status: Option<ViewStatus>) -> MapAuthorityRequirement {
    let metadata =
        map_authority_requirement_metadata(input_id).expect("known map authority requirement");
    let disposition = match status {
        Some(ViewStatus::DomainAbsent) => MapAuthorityRequirementDisposition::Absent,
        _ => MapAuthorityRequirementDisposition::Unavailable,
    };
    let label = map_authority_observed_status_label(input_id);
    let status_suffix = status
        .map(|value| format!("; WorldFrame {label} is {value:?}"))
        .unwrap_or_default();
    MapAuthorityRequirement {
        input_id: input_id.to_string(),
        payload_schema: None,
        cadence: None,
        identity_order: None,
        disposition,
        blocker: Some(format!("{}{status_suffix}", metadata.blocker)),
    }
}

/// Derive one input's disposition from the authenticated frame.
///
/// The single admitting branch of the whole authority: an input is admitted
/// only when its owning packet has supplied owner-proven production admission
/// **and** the frame's own status accessor for that input reports `Present`.
/// `Present` on a `replay_core` view means the recorder measured the
/// collection this hour — a measured-empty collection is `Present` with no
/// rows, which is a fact, while an absent key is `DomainAbsent` and an
/// explicitly unobtained one is `DomainUnavailable`. Both of those fall
/// through to [`unavailable_requirement`] and keep their named blocker, so an
/// absent input is never presented as measured-empty.
fn derive_requirement(input_id: &str, status: Option<ViewStatus>) -> MapAuthorityRequirement {
    if let Some(admission) = map_authority_owner_proven_admission(input_id) {
        if status == Some(ViewStatus::Present) {
            return MapAuthorityRequirement {
                input_id: input_id.to_string(),
                payload_schema: Some(admission.payload_schema.to_string()),
                cadence: Some(admission.cadence.to_string()),
                identity_order: Some(admission.identity_order.to_string()),
                disposition: MapAuthorityRequirementDisposition::Admitted,
                blocker: None,
            };
        }
    }
    unavailable_requirement(input_id, status)
}

/// Project immutable content identity plus each family's derived disposition.
/// A family whose owning packet has proved its production contract is admitted
/// from this frame's own measured view; every other family remains a typed,
/// named unavailability.
pub fn map_authority_projection_from_world_frame(
    frame: &WorldFrame,
    manifest: &ContentManifest,
) -> Result<MapAuthorityProjection, String> {
    let manifest_digest = manifest
        .manifest_digest_sha256
        .clone()
        .ok_or_else(|| "map authority requires a finalized content manifest".to_string())?;
    let faction_roster_status = match frame.diplomacy().faction_roster_status() {
        replay_core::world::FactionRosterStatus::Unavailable => ViewStatus::DomainUnavailable,
        replay_core::world::FactionRosterStatus::MeasuredEmpty
        | replay_core::world::FactionRosterStatus::MeasuredNonEmpty => ViewStatus::Present,
    };
    let mut requirements = vec![
        derive_requirement("combat.land_active_epoch", Some(frame.combats_status())),
        derive_requirement(
            "combat.land_terminal_detail",
            Some(land_terminal_detail_status(frame)),
        ),
        derive_requirement("combat.naval_air_epoch", Some(naval_air_epoch_status(frame))),
        derive_requirement(
            "control.controller_by_province",
            Some(frame.control_by_province_status()),
        ),
        derive_requirement(
            "control.controller_by_state",
            Some(frame.control_by_state_status()),
        ),
        derive_requirement(
            "control.owner_by_state",
            Some(frame.owner_by_state_status()),
        ),
        derive_requirement(
            "country.current_capital_state",
            Some(frame.diplomacy().status),
        ),
        derive_requirement(
            "country.d18_complete_roster",
            Some(frame.d18_country_roster().status),
        ),
        derive_requirement(
            "country.presentation_identity",
            Some(frame.country_identities_status()),
        ),
        derive_requirement("diplomacy.faction_roster", Some(faction_roster_status)),
        derive_requirement("diplomacy.pair_relations", Some(frame.diplomacy().status)),
        derive_requirement(
            "effects.semantic_lifecycle",
            Some(frame.xmap20_effects_status()),
        ),
        derive_requirement(
            "fog.province_visibility_by_pov",
            Some(frame.visibility_status()),
        ),
        derive_requirement("fog.spotted_land_by_pov", Some(frame.visibility_status())),
        derive_requirement(
            "fog.spotted_taskforces_by_pov",
            Some(frame.visibility_status()),
        ),
        derive_requirement("fog.spotted_wings_by_pov", None),
        derive_requirement("lighting.point_light_lifecycle", None),
        derive_requirement(
            "logistics.rail_hub_frames",
            Some(
                if frame.rail_replay().status == ViewStatus::Present
                    && frame.rail_replay().hub_visual_states_complete
                    && frame.rail_replay().hub_effective_cutoffs.is_some()
                    && frame.rail_replay().hub_visual_states.is_some()
                {
                    ViewStatus::Present
                } else {
                    ViewStatus::DomainUnavailable
                },
            ),
        ),
        derive_requirement(
            "logistics.rail_topology_state",
            Some(
                if frame.rail_replay().status == ViewStatus::Present
                    && frame.rail_replay().pair_transitions_complete
                    && frame.rail_replay().pair_transitions.is_some()
                {
                    ViewStatus::Present
                } else {
                    ViewStatus::DomainUnavailable
                },
            ),
        ),
        derive_requirement("map_furniture.current_victory_points", None),
        derive_requirement(
            "mapmode.air_superiority",
            Some(frame.air_superiority_status()),
        ),
        derive_requirement(
            "mapmode.infrastructure_state",
            Some(frame.buildings_status()),
        ),
        derive_requirement("mapmode.intelligence", Some(frame.diplomacy().status)),
        derive_requirement(
            "mapmode.naval_supremacy",
            Some(frame.naval_supremacy_status()),
        ),
        derive_requirement(
            "mapmode.occupation_resistance_compliance",
            Some(frame.occupation_status()),
        ),
        derive_requirement("mapmode.raids", Some(frame.raids_status())),
        derive_requirement("mapmode.resource_deposits", None),
        derive_requirement("mapmode.scripted_inputs", Some(frame.scripted_gui_status())),
        derive_requirement(
            "mapmode.supply_state",
            Some(
                if frame.rail_replay().status == ViewStatus::Present
                    && frame.rail_replay().supply_arrows_complete
                    && frame.rail_replay().supply_arrows.is_some()
                {
                    ViewStatus::Present
                } else {
                    ViewStatus::DomainUnavailable
                },
            ),
        ),
        derive_requirement(
            "objects.building_state_damage",
            Some(frame.d48_buildings_status()),
        ),
        derive_requirement("objects.city_population", None),
        derive_requirement("objects.generic_population_lifecycle", None),
        derive_requirement(
            "orders.army_battleplans_semantic",
            Some(frame.army_battleplans().status),
        ),
        derive_requirement("orders.theatre_army_hierarchy", None),
        derive_requirement("postfx.temporal_exposure_state", None),
        derive_requirement(
            "session.player_country_set",
            Some(frame.session_metadata().status),
        ),
        derive_requirement("terrain.snow_mud_semantic", Some(frame.snow_mud_status())),
        derive_requirement("units.air_base_location", Some(frame.wings_status())),
        derive_requirement("units.divisions", Some(frame.division_units_status())),
        derive_requirement("units.taskforces_fleets", Some(frame.fleets_status())),
        derive_requirement("units.wings", Some(frame.wings_status())),
        derive_requirement(
            "weather.province_snow_accumulator",
            Some(frame.weather_province_snow_status()),
        ),
        derive_requirement("weather.region_gameplay", Some(frame.weather_status())),
        derive_requirement(
            "weather.visual_membership_lifecycle",
            Some(frame.weather_visual_lifecycle().status),
        ),
        derive_requirement("world.calendar_time", None),
    ];
    requirements.sort_by(|left, right| left.input_id.cmp(&right.input_id));
    let mut projection = MapAuthorityProjection::from_untrusted_parts(
        MAP_AUTHORITY_INPUT_SCHEMA.to_string(),
        frame.hour_index(),
        manifest_digest,
        manifest.product_modset_sha256.clone(),
        manifest.effective_vfs_manifest_sha256.clone(),
        requirements,
    );
    // The one place the frame-derivation witness is ever stamped.
    projection.frame_derivation_witness_sha256 = Some(projection.admission_witness_sha256()?);
    Ok(projection)
}

/// Admit a projection that arrived as bytes. Caller-supplied bytes are not a
/// `WorldFrame`, so this door can only ever carry the gap report: an
/// `admitted` row here would be a claim about measured frame data the sender
/// does not hold. Decoding drops the frame-derivation witness, so
/// [`admit_map_authority_projection`] would refuse such a row anyway; this
/// names the boundary explicitly rather than relying on that.
pub fn admit_map_authority_json(bytes: &[u8]) -> Result<MapAuthorityAdmission, String> {
    let projection: MapAuthorityProjection = serde_json::from_slice(bytes)
        .map_err(|error| format!("decode map authority projection: {error}"))?;
    if let Some(row) = projection
        .requirements
        .iter()
        .find(|row| row.disposition == MapAuthorityRequirementDisposition::Admitted)
    {
        return Err(format!(
            "map authority admission is derived from the authenticated WorldFrame; caller-supplied bytes cannot admit {}",
            row.input_id
        ));
    }
    admit_map_authority_projection(&projection)
}

/// Validate the projection and split it into the owner-proven inputs the frame
/// measured and the typed gaps that remain. This function cannot authenticate
/// caller-supplied payloads and therefore still never returns a ready
/// admission — an `Admitted` row states only that a proven contract's view was
/// measured, never that a payload hash is trustworthy.
pub fn admit_map_authority_projection(
    projection: &MapAuthorityProjection,
) -> Result<MapAuthorityAdmission, String> {
    if projection.schema != MAP_AUTHORITY_INPUT_SCHEMA {
        return Err(format!(
            "unsupported map authority schema {:?}",
            projection.schema
        ));
    }
    validate_sha256(
        &projection.content_manifest_digest_sha256,
        "content manifest digest",
    )?;
    validate_sha256(&projection.product_modset_sha256, "product modset")?;
    validate_sha256(
        &projection.effective_vfs_manifest_sha256,
        "effective VFS manifest",
    )?;

    let actual = projection
        .requirements
        .iter()
        .map(|row| row.input_id.as_str())
        .collect::<Vec<_>>();
    if actual != MAP_AUTHORITY_REQUIRED_INPUTS {
        return Err(format!(
            "map authority requirement identity/order mismatch: expected {:?}, got {:?}",
            MAP_AUTHORITY_REQUIRED_INPUTS, actual
        ));
    }

    for row in &projection.requirements {
        validate_requirement(row)?;
    }
    // An admission is a statement that the authenticated frame measured the
    // input. Only `map_authority_projection_from_world_frame` ever saw a frame,
    // and its witness covers every requirement row, so a hand-assembled
    // projection and an edited derived one are both refused here.
    if let Some(row) = projection
        .requirements
        .iter()
        .find(|row| row.disposition == MapAuthorityRequirementDisposition::Admitted)
    {
        if !projection.is_frame_derived() {
            return Err(format!(
                "{} is admitted by a projection that was not derived from an authenticated WorldFrame",
                row.input_id
            ));
        }
    }
    let mut gaps = Vec::new();
    let mut admitted = Vec::new();
    for row in &projection.requirements {
        match row.disposition {
            MapAuthorityRequirementDisposition::Admitted => {
                let proven = map_authority_owner_proven_admission(&row.input_id)
                    .expect("validate_requirement admits only owner-proven inputs");
                admitted.push(MapAuthorityAdmittedInput {
                    input_id: row.input_id.clone(),
                    owner_packet: proven.owner_packet.to_string(),
                    payload_schema: proven.payload_schema.to_string(),
                    cadence: proven.cadence.to_string(),
                    identity_order: proven.identity_order.to_string(),
                });
            }
            MapAuthorityRequirementDisposition::Absent
            | MapAuthorityRequirementDisposition::Unavailable => gaps.push(MapAuthorityGap {
                input_id: row.input_id.clone(),
                disposition: row.disposition,
                reason: row
                    .blocker
                    .clone()
                    .expect("validate_requirement requires a blocker on every refused row"),
            }),
        }
    }
    Ok(MapAuthorityAdmission { gaps, admitted })
}

fn validate_requirement(row: &MapAuthorityRequirement) -> Result<(), String> {
    let expected = map_authority_requirement_metadata(&row.input_id)
        .ok_or_else(|| format!("unknown map authority input {}", row.input_id))?;
    let proven = map_authority_owner_proven_admission(&row.input_id);
    if row.disposition == MapAuthorityRequirementDisposition::Admitted {
        // The only path to an admission: the ledger's own disposition column,
        // mirrored by MAP_AUTHORITY_OWNER_PROVEN_INPUTS, must name this input.
        let Some(proven) = proven else {
            return Err(format!(
                "{} has no owner-proven production admission and cannot be admitted",
                row.input_id
            ));
        };
        if row.payload_schema.as_deref() != Some(proven.payload_schema)
            || row.cadence.as_deref() != Some(proven.cadence)
            || row.identity_order.as_deref() != Some(proven.identity_order)
        {
            return Err(format!(
                "{} does not carry its owner-proven payload/cadence/identity-order contract",
                row.input_id
            ));
        }
        if row.blocker.is_some() {
            return Err(format!("{} is admitted and carries a blocker", row.input_id));
        }
        return Ok(());
    }

    if row.payload_schema.is_some() || row.cadence.is_some() || row.identity_order.is_some() {
        return Err(format!(
            "{} supplies unresolved map authority metadata",
            row.input_id
        ));
    }
    let Some(blocker) = row.blocker.as_deref() else {
        return Err(format!("{} is refused without a blocker", row.input_id));
    };
    // A proven input can only be refused because the frame did not measure it,
    // so `Present` — which the derivation turns into an admission — and the
    // status-free form are both contradictions for those rows.
    let unavailable_statuses: &[ViewStatus] = if proven.is_some() {
        &[ViewStatus::DomainUnavailable]
    } else {
        &[ViewStatus::DomainUnavailable, ViewStatus::Present]
    };
    let label = map_authority_observed_status_label(&row.input_id);
    let blocker_is_canonical = match row.disposition {
        MapAuthorityRequirementDisposition::Absent => {
            blocker
                == format!(
                    "{}; WorldFrame {label} is {:?}",
                    expected.blocker,
                    ViewStatus::DomainAbsent
                )
        }
        MapAuthorityRequirementDisposition::Unavailable => {
            (proven.is_none() && blocker == expected.blocker)
                || unavailable_statuses.iter().any(|status| {
                    blocker == format!("{}; WorldFrame {label} is {status:?}", expected.blocker)
                })
        }
        MapAuthorityRequirementDisposition::Admitted => unreachable!("handled above"),
    };
    if !blocker_is_canonical {
        return Err(format!(
            "{} does not carry the canonical unresolved blocker",
            row.input_id
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not lowercase SHA-256"));
    }
    Ok(())
}
