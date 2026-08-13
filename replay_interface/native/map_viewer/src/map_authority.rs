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

use std::collections::BTreeSet;

use replay_core::{ContentManifest, ViewStatus, WorldFrame};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    /// The certified admission predicate, carried by the row itself.
    ///
    /// # Why the predicate lives in the table
    ///
    /// [`map_authority_projection_from_world_frame`] holds a hand-written arm
    /// per required input, and until this field existed a proven row's
    /// disposition came from whatever `ViewStatus` that arm happened to pass.
    /// The `status_accessor` string above was then only a comment: nothing
    /// connected it to the value actually consulted, so a proven row could be
    /// wired to a neighbouring view — or to a composite root that is `Present`
    /// while this row's own payload is absent — and no test could see it.
    ///
    /// [`derive_requirement`] now ignores the arm's status entirely for a
    /// proven input and calls this function instead, so the admission table is
    /// the single place a proven row's measurement is decided. Most of these
    /// are partition-local or completeness-checked readings rather than the
    /// ledger's root accessor, for the reason each one's doc comment states;
    /// [`map_authority_observed_status_label`] names what was really read so a
    /// refusal can never claim a root status it did not consult.
    pub status: fn(&WorldFrame) -> ViewStatus,
    /// Whether [`Self::status`] can ever return `ViewStatus::DomainAbsent`.
    ///
    /// [`validate_requirement`] otherwise accepts an `Absent` disposition with
    /// the canonical `DomainAbsent` blocker for ANY proven row, and a gap-only
    /// projection needs no frame witness — so a caller could hand back a
    /// refusal claiming an observation the row's predicate is incapable of
    /// making. `diplomacy.faction_roster` is the case that motivates the flag:
    /// `build_diplomacy` forces a missing faction carrier to `Leaf::Unobtained`
    /// rather than `Leaf::Absent`, so its predicate reports only `Present` or
    /// `DomainUnavailable`, and an `Absent` row for it is a fabricated reading.
    pub can_report_absent: bool,
}

/// The owner-proven admission table, mirroring the
/// `owner_proven_production_consumer` rows of
/// `re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`.
///
/// `cadence` is the `WorldFrame` contract and is the same sentence for every
/// row: one decoded view per canonical recorded hour (`WorldFrame::hour_index`).
/// It is spelled out per row rather than shared through a constant because
/// `Audit-NonUiVisualInputLedger.ps1` reads these entries out of the source
/// text and can only see a literal.
///
/// `identity_order` is read out of the decoder that builds each view in
/// `replay_interface/native/replay_core/src/world.rs`, never invented here:
/// `build_combats` preserves the recorder census order and carries
/// `(combat_id, generation)` per row; `build_ended_combats` keys a `BTreeMap`
/// by `(combat_id, generation)`, so terminal rows are ascending by that key;
/// `build_naval_air_combat` keeps the naval census in recorder order and
/// exposes the sunk-convoy and air partitions as separate typed leaves;
/// `exact_d18_country_roster` walks the `countries` object in TAG order and
/// refuses any partial prefix; `build_country_identities`,
/// `build_diplomacy` and `build_rail_replay` all key their censuses by
/// `BTreeMap`, so each is ascending by its own key.
pub const MAP_AUTHORITY_OWNER_PROVEN_INPUTS: &[MapAuthorityOwnerProvenAdmission] = &[
    MapAuthorityOwnerProvenAdmission {
        input_id: "combat.land_active_epoch",
        owner_packet: "XMAP14",
        payload_schema: "WorldFrame.combats",
        status_accessor: "combats_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "(combat_id, generation) per row, in recorder census order",
        status: land_active_epoch_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "combat.land_terminal_detail",
        owner_packet: "XMAP14",
        payload_schema: "WorldFrame.ended_combats",
        status_accessor: "ended_combats_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "(combat_id, generation) ascending",
        status: land_terminal_detail_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "combat.naval_air_epoch",
        owner_packet: "XMAP14",
        payload_schema: "WorldFrame.naval_air_combat",
        status_accessor: "naval_air_combat_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "naval census in recorder order; sunk-convoy and air partitions typed separately",
        status: naval_air_epoch_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "country.d18_complete_roster",
        owner_packet: "XMAP05",
        payload_schema: "WorldFrame.countries._r2_tag_db_id",
        status_accessor: "d18_country_roster.status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "TAG-ascending, each with a unique nonzero tag_db_id; no partial prefix",
        status: d18_complete_roster_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "diplomacy.faction_roster",
        owner_packet: "XMAP18",
        payload_schema: "WorldFrame.diplomacy.faction_members",
        status_accessor: "faction_roster_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order:
            "faction id ascending; member TAGs in recorder order, each in at most one faction and each present in the complete D18 roster, under the producer's own D53 completeness witness",
        status: faction_roster_status,
        // `build_diplomacy` forces a missing faction carrier to
        // `Leaf::Unobtained`, so `faction_roster_status` reports only `Present`
        // or `DomainUnavailable` and can never observe `DomainAbsent`.
        can_report_absent: false,
    },
];

/// The accessor whose value a refusal reports.
///
/// An unproven family is read straight off a `WorldFrame` root accessor and its
/// blocker says so. Most owner-proven families are NOT, and must not claim they
/// are: the ledger's `status_accessor` names a composite that can be `Present`
/// while the admitted payload is absent, unobtained, incomplete, or a retained
/// observation of an earlier hour. Each such row's `status` predicate reads
/// something strictly stronger, and the label below is what that predicate
/// actually consulted. Both [`unavailable_requirement`] and
/// [`validate_requirement`] resolve the label through this one function, so a
/// row cannot carry a blocker that names an accessor the derivation never read.
pub fn map_authority_observed_status_label(input_id: &str) -> &'static str {
    match input_id {
        "combat.naval_air_epoch" => "naval_combats_status",
        "combat.land_terminal_detail" => "certified ended_combats_status",
        // These two ARE read off the accessor the ledger names, but "root
        // status" would be a lie about which one: `countries_status` and
        // `diplomacy.status` are both `Present` for any `countries` object,
        // while these predicates are the fail-closed verdicts one level in.
        "country.d18_complete_roster" => "d18_country_roster.status",
        "diplomacy.faction_roster" => "faction_roster_status",
        _ => "root status",
    }
}

/// The admission status for `combat.land_active_epoch`.
///
/// The only proven row whose ledger root accessor is already exactly its own
/// payload's verdict: `build_combats` downgrades to `DomainUnavailable` the
/// moment it skips a malformed row, so `Present` is a complete census.
fn land_active_epoch_status(frame: &WorldFrame) -> ViewStatus {
    frame.combats_status()
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

/// The complete D18 country roster, or the status that refuses it.
///
/// This is the only view in the frame that states, fail-closed, *which*
/// countries the hour measured, so it is the completeness reference any
/// per-country census must join to. `exact_d18_country_roster` already refuses
/// a partial prefix — a missing, zero, duplicate or malformed `_r2_tag_db_id`,
/// or a roster that does not cover the whole typed `countries` view, makes the
/// roster unavailable rather than a shorter list — so a census checked against
/// it cannot be complete against a roster that silently lost members.
///
/// It returns the TAG set rather than a bare status because that is what a
/// joining census needs; `country.current_capital_state` was the first such
/// census and is currently refused for a producer-side reason recorded in
/// `extractor/native/runs/hw_admission_a/ROW_PROOFS.md` (HW-F6).
fn complete_d18_roster(frame: &WorldFrame) -> Result<BTreeSet<String>, ViewStatus> {
    let roster = frame.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return Err(roster.status);
    }
    // A `Present` roster is never empty (`exact_d18_country_roster` refuses an
    // empty `countries` object), but a census joined to an empty roster would
    // be vacuously complete, so this refuses rather than relying on that.
    if roster.entries.is_empty() {
        return Err(ViewStatus::DomainUnavailable);
    }
    Ok(roster
        .entries
        .iter()
        .map(|entry| entry.tag.clone())
        .collect())
}

/// The admission status for `country.d18_complete_roster`.
///
/// This is the one row whose ledger accessor is already exactly its payload's
/// own fail-closed completeness verdict, so the predicate is the accessor.
fn d18_complete_roster_status(frame: &WorldFrame) -> ViewStatus {
    match complete_d18_roster(frame) {
        Ok(_) => ViewStatus::Present,
        Err(status) => status,
    }
}

/// The admission status for `diplomacy.faction_roster`.
///
/// `DiplomacyView::faction_roster_status` is the atomic verdict over the single
/// `faction_members` leaf — an absent or unobtained carrier is unavailable, and
/// a measured map distinguishes empty from nonempty. That is necessary but NOT
/// sufficient, because the leaf has two producers.
///
/// `build_diplomacy` prefers the modern D53 witness
/// (`read_domain53_faction_roster`) and falls back to the legacy
/// `diplomacy_live.faction_members` object when a recording predates it. The
/// legacy reader accepts any well-formed nonempty object as `Leaf::Measured`:
/// it validates each row's shape and member exclusivity but has **no expected
/// faction count and no completeness witness**, so a recording that captured
/// one faction and missed another is indistinguishable from a complete one. The
/// modern route is the opposite — `faction_roster_status` is a stated
/// `measured_empty` / `measured_nonempty` / `unavailable` claim the producer
/// made about its own coverage.
///
/// So this admits only a roster the producer WITNESSED as complete: the modern
/// claim must be present and state a measurement, and `replay_core` must
/// independently have turned it into a measured leaf. The second half is what
/// keeps this from re-implementing the parse — a witness that says "measured"
/// over rows that do not parse leaves `faction_roster_status` `Unavailable` and
/// is refused here.
///
/// A third conjunct covers identity. A faction row's members are TAG strings,
/// and neither side joins them to the country census: when the recorder cannot
/// map a member's `tag_db_id` it emits the numeric id as the key rather than
/// fabricating a TAG (`world_fold.rs`'s `resolve_country_key`), and
/// `parse_domain53_faction_rows` accepts any nonempty string. A witnessed row
/// naming `"999"` would otherwise be admitted as a measured membership census
/// while that member is explicitly unresolved. So every member TAG must appear
/// in the complete D18 roster — the frame's own statement of which countries
/// this hour identified. A measured-empty roster has no members and joins
/// vacuously, which is correct: it asserts no identity.
fn faction_roster_status(frame: &WorldFrame) -> ViewStatus {
    let diplomacy = frame.diplomacy();
    let measured = match diplomacy.faction_roster_status() {
        replay_core::world::FactionRosterStatus::Unavailable => false,
        replay_core::world::FactionRosterStatus::MeasuredEmpty
        | replay_core::world::FactionRosterStatus::MeasuredNonEmpty => true,
    };
    if !measured {
        return ViewStatus::DomainUnavailable;
    }
    let witnessed = frame
        .domain("diplomacy_world")
        .and_then(Value::as_object)
        .and_then(|world| world.get("domain53_extra"))
        .and_then(Value::as_object)
        .and_then(|extra| extra.get("faction_roster_status"))
        .and_then(Value::as_str)
        .is_some_and(|claim| claim == "measured_empty" || claim == "measured_nonempty");
    if !witnessed {
        return ViewStatus::DomainUnavailable;
    }
    let Some(members) = diplomacy.faction_members.measured() else {
        // Unreachable while `measured` is true, but this is a refusal rather
        // than an `expect`: an admission must never rest on an invariant this
        // module does not own.
        return ViewStatus::DomainUnavailable;
    };
    if members.values().flatten().next().is_none() {
        // Measured-empty: no membership identity is being asserted, so there is
        // nothing to join and the D18 roster is not required.
        return ViewStatus::Present;
    }
    // A failing roster is NOT propagated verbatim. `complete_d18_roster` reports
    // `DomainAbsent` when the frame carries no `countries` key at all, and this
    // row's carrier is independent of that key — a witnessed faction roster can
    // be measured on a frame with no country census. Reporting `DomainAbsent`
    // here would therefore describe a different view's absence, and because
    // `can_report_absent` is false for this row `validate_requirement` would
    // reject the whole projection rather than record a gap. What is true is
    // that this row's identity cannot be established, which is an
    // unavailability of this row.
    let roster = match complete_d18_roster(frame) {
        Ok(roster) => roster,
        Err(_) => return ViewStatus::DomainUnavailable,
    };
    let identified = members
        .values()
        .flatten()
        .all(|member| roster.contains(member));
    if identified {
        ViewStatus::Present
    } else {
        ViewStatus::DomainUnavailable
    }
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
        | "country.presentation_identity"
        | "fog.province_visibility_by_pov"
        | "fog.spotted_land_by_pov"
        | "fog.spotted_taskforces_by_pov"
        | "fog.spotted_wings_by_pov" => {
            "XMAP05 must prove the exact complete identity/POV payload, enumeration, cadence, and authenticated producer contract"
        }
        // XMAP05 has proved its roster row (ledger disposition
        // `owner_proven_production_consumer`), so its blocker names the hour's
        // missing measurement instead of a pending proof. Its sibling rows above
        // keep theirs, each for a producer-side reason recorded in
        // `extractor/native/runs/hw_admission_a/ROW_PROOFS.md`: the POV rows need
        // the all-country `complete_pov_roster` FOW1 census no producer emits,
        // and `country.presentation_identity` needs a cosmetic-tag spelling that
        // distinguishes "legitimately absent" from "could not read" across the
        // seam, which the current `cosmetic_tag: null` emission does not.
        "country.d18_complete_roster" => {
            "the recording carries no complete measured D18 country roster for this hour"
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
        // XMAP16's three rows stay unproven: `build_rail_replay` admits only the
        // `exact_a729_crailwaygfx_carrier` document, and the recorder's D81 fold
        // emits `exact_a729_supply_simulation_tables` with a different field set,
        // so no capture can make `rail_replay` anything but unavailable. Recorded
        // in `extractor/native/runs/hw_admission_a/ROW_PROOFS.md`.
        "logistics.rail_hub_frames" | "logistics.rail_topology_state" | "mapmode.supply_state" => {
            "XMAP16 must prove exact topology/hub/state identities, order, completeness, and authenticated producer/content joins"
        }
        // `country.current_capital_state` stays unproven for a producer reason,
        // not a replay one: `capital_state` is minted only by the collector's
        // direct-JSON country renderer and appears nowhere in `recorder_host/`,
        // so the fold never writes it into the hourly snapshot. HW-F6 in
        // `extractor/native/runs/hw_admission_a/ROW_PROOFS.md`.
        "country.current_capital_state" | "map_furniture.current_victory_points" => {
            "XMAP21 must prove the exact live furniture value, identity, cadence, completeness, and authenticated producer/content joins"
        }
        // XMAP18 has proved the faction roster; its other rows still owe a proof.
        "diplomacy.faction_roster" => {
            "the recording carries no measured faction-membership roster for this hour"
        }
        "diplomacy.pair_relations"
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
/// **and** the admission table's own certified predicate reports `Present` for
/// this frame. `Present` on a `replay_core` view means the recorder measured
/// the collection this hour — a measured-empty collection is `Present` with no
/// rows, which is a fact, while an absent key is `DomainAbsent` and an
/// explicitly unobtained one is `DomainUnavailable`. Both of those fall
/// through to [`unavailable_requirement`] and keep their named blocker, so an
/// absent input is never presented as measured-empty.
///
/// `status` is the hand-written arm's reading in
/// [`map_authority_projection_from_world_frame`]. It is used for UNPROVEN
/// families only, which is all it can honestly describe: for a proven family
/// the arm's composite root can be `Present` while the admitted payload is
/// not, so the table's predicate is consulted instead and the arm's value is
/// deliberately discarded. That is what makes it structurally impossible to
/// admit a proven row off a neighbouring or too-weak accessor.
/// The frame one projection is derived from.
///
/// This exists so `derive_requirement` can reach the frame while keeping the
/// input id as its first literal argument at every call site.
/// `Audit-NonUiVisualInputLedger.ps1` proves the projection covers all 45
/// semantic ledger rows by matching `derive_requirement(\s*"<id>"` — and the
/// missing-status census by matching `derive_requirement(\s*"<id>",\s*None\s*)`
/// — against this file's source text, so threading the frame through a leading
/// parameter or a wrapper closure would silently blind that audit rather than
/// fail it.
struct RequirementProjector<'a> {
    frame: &'a WorldFrame,
}

impl RequirementProjector<'_> {
    fn derive_requirement(
        &self,
        input_id: &str,
        status: Option<ViewStatus>,
    ) -> MapAuthorityRequirement {
        let Some(admission) = map_authority_owner_proven_admission(input_id) else {
            return unavailable_requirement(input_id, status);
        };
        let derived = (admission.status)(self.frame);
        if derived != ViewStatus::Present {
            return unavailable_requirement(input_id, Some(derived));
        }
        MapAuthorityRequirement {
            input_id: input_id.to_string(),
            payload_schema: Some(admission.payload_schema.to_string()),
            cadence: Some(admission.cadence.to_string()),
            identity_order: Some(admission.identity_order.to_string()),
            disposition: MapAuthorityRequirementDisposition::Admitted,
            blocker: None,
        }
    }
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
    // Every arm below passes the accessor the XMAP00 ledger's `status_accessor`
    // column names for that input, and nothing else. For an UNPROVEN family
    // that reading is the whole story. For a PROVEN one it is discarded:
    // `derive_requirement` consults the admission table's own certified
    // predicate instead, because these root accessors are exactly the
    // composites that can be `Present` while the admitted payload is not.
    let projector = RequirementProjector { frame };
    let mut requirements = vec![
        projector.derive_requirement("combat.land_active_epoch", Some(frame.combats_status())),
        projector.derive_requirement(
            "combat.land_terminal_detail",
            Some(frame.ended_combats_status()),
        ),
        projector.derive_requirement(
            "combat.naval_air_epoch",
            Some(frame.naval_air_combat_status()),
        ),
        projector.derive_requirement(
            "control.controller_by_province",
            Some(frame.control_by_province_status()),
        ),
        projector.derive_requirement(
            "control.controller_by_state",
            Some(frame.control_by_state_status()),
        ),
        projector.derive_requirement("control.owner_by_state", Some(frame.owner_by_state_status())),
        projector.derive_requirement("country.current_capital_state", Some(frame.diplomacy().status)),
        projector.derive_requirement(
            "country.d18_complete_roster",
            Some(frame.d18_country_roster().status),
        ),
        projector.derive_requirement(
            "country.presentation_identity",
            Some(frame.country_identities_status()),
        ),
        projector.derive_requirement("diplomacy.faction_roster", Some(frame.diplomacy().status)),
        projector.derive_requirement("diplomacy.pair_relations", Some(frame.diplomacy().status)),
        projector.derive_requirement(
            "effects.semantic_lifecycle",
            Some(frame.xmap20_effects_status()),
        ),
        projector.derive_requirement(
            "fog.province_visibility_by_pov",
            Some(frame.visibility_status()),
        ),
        projector.derive_requirement("fog.spotted_land_by_pov", Some(frame.visibility_status())),
        projector.derive_requirement(
            "fog.spotted_taskforces_by_pov",
            Some(frame.visibility_status()),
        ),
        projector.derive_requirement("fog.spotted_wings_by_pov", None),
        projector.derive_requirement("lighting.point_light_lifecycle", None),
        projector.derive_requirement("logistics.rail_hub_frames", Some(frame.rail_replay().status)),
        projector.derive_requirement(
            "logistics.rail_topology_state",
            Some(frame.rail_replay().status),
        ),
        projector.derive_requirement("map_furniture.current_victory_points", None),
        projector.derive_requirement("mapmode.air_superiority", Some(frame.air_superiority_status())),
        projector.derive_requirement("mapmode.infrastructure_state", Some(frame.buildings_status())),
        projector.derive_requirement("mapmode.intelligence", Some(frame.diplomacy().status)),
        projector.derive_requirement("mapmode.naval_supremacy", Some(frame.naval_supremacy_status())),
        projector.derive_requirement(
            "mapmode.occupation_resistance_compliance",
            Some(frame.occupation_status()),
        ),
        projector.derive_requirement("mapmode.raids", Some(frame.raids_status())),
        projector.derive_requirement("mapmode.resource_deposits", None),
        projector.derive_requirement("mapmode.scripted_inputs", Some(frame.scripted_gui_status())),
        projector.derive_requirement("mapmode.supply_state", Some(frame.rail_replay().status)),
        projector.derive_requirement(
            "objects.building_state_damage",
            Some(frame.d48_buildings_status()),
        ),
        projector.derive_requirement("objects.city_population", None),
        projector.derive_requirement("objects.generic_population_lifecycle", None),
        projector.derive_requirement(
            "orders.army_battleplans_semantic",
            Some(frame.army_battleplans().status),
        ),
        projector.derive_requirement("orders.theatre_army_hierarchy", None),
        projector.derive_requirement("postfx.temporal_exposure_state", None),
        projector.derive_requirement(
            "session.player_country_set",
            Some(frame.session_metadata().status),
        ),
        projector.derive_requirement("terrain.snow_mud_semantic", Some(frame.snow_mud_status())),
        projector.derive_requirement("units.air_base_location", Some(frame.wings_status())),
        projector.derive_requirement("units.divisions", Some(frame.division_units_status())),
        projector.derive_requirement("units.taskforces_fleets", Some(frame.fleets_status())),
        projector.derive_requirement("units.wings", Some(frame.wings_status())),
        projector.derive_requirement(
            "weather.province_snow_accumulator",
            Some(frame.weather_province_snow_status()),
        ),
        projector.derive_requirement("weather.region_gameplay", Some(frame.weather_status())),
        projector.derive_requirement(
            "weather.visual_membership_lifecycle",
            Some(frame.weather_visual_lifecycle().status),
        ),
        projector.derive_requirement("world.calendar_time", None),
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
    // A refusal states what was OBSERVED, so it may not claim an observation
    // the row's own predicate cannot make. A gap-only projection carries no
    // frame witness and is otherwise accepted on its blocker text alone, so
    // without this a caller could hand back `diplomacy.faction_roster` as
    // `Absent` — a reading `faction_roster_status` never produces, because
    // `build_diplomacy` forces a missing carrier to `Leaf::Unobtained`.
    if row.disposition == MapAuthorityRequirementDisposition::Absent
        && proven.is_some_and(|admission| !admission.can_report_absent)
    {
        return Err(format!(
            "{} cannot observe DomainAbsent and must not be refused as absent",
            row.input_id
        ));
    }
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
