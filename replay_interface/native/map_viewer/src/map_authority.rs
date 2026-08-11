//! XMAP00's presentation-free product-map authority boundary.
//!
//! XMAP00 has proved the stable input-family identities, but it has not proved
//! a recording-authenticated canonical payload, enumerator, schema, cadence,
//! or identity order for any family. Consequently this module can describe
//! typed gaps, but it cannot produce a product-map-ready admission.
//!
//! The serial form has `deny_unknown_fields`, so recorder camera, map-mode,
//! selection, GUI, selected-Present, and draw-geometry fields are rejected.

use replay_core::{ContentManifest, ViewStatus, WorldFrame};
use serde::{Deserialize, Serialize};

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
}

/// The only metadata XMAP00 can currently state for a family. `None` means the
/// field is deliberately unresolved, not an invitation for a caller to choose
/// a local value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapAuthorityRequirementMetadata {
    pub input_id: &'static str,
    pub payload_schema: Option<&'static str>,
    pub cadence: Option<&'static str>,
    pub identity_order: Option<&'static str>,
    pub blocker: &'static str,
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
    pub blocker: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAuthorityProjection {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_digest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub requirements: Vec<MapAuthorityRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapAuthorityGap {
    pub input_id: String,
    pub disposition: MapAuthorityRequirementDisposition,
    pub reason: String,
}

/// XMAP00 admissions are gap reports only. There is intentionally no `Ready`
/// variant or receipt: hashes supplied by a caller are not authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapAuthorityAdmission {
    gaps: Vec<MapAuthorityGap>,
}

impl MapAuthorityAdmission {
    pub fn gaps(&self) -> &[MapAuthorityGap] {
        &self.gaps
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
        "combat.land_active_epoch"
        | "combat.land_terminal_detail"
        | "combat.naval_air_epoch" => {
            "XMAP14 must prove the exact cross-domain combat epoch, lifecycle, member order, completeness, and authenticated producer contract"
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
    let status_suffix = status
        .map(|value| format!("; WorldFrame root status is {value:?}"))
        .unwrap_or_default();
    MapAuthorityRequirement {
        input_id: input_id.to_string(),
        payload_schema: None,
        cadence: None,
        identity_order: None,
        disposition,
        blocker: format!("{}{status_suffix}", metadata.blocker),
    }
}

/// Project immutable content identity plus typed unavailability. Current
/// `WorldFrame` views are decoded status/views, not authenticated canonical
/// map payloads, so every family remains non-admissible.
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
        unavailable_requirement("combat.land_active_epoch", Some(frame.combats_status())),
        unavailable_requirement(
            "combat.land_terminal_detail",
            Some(frame.ended_combats_status()),
        ),
        unavailable_requirement(
            "combat.naval_air_epoch",
            Some(frame.naval_air_combat_status()),
        ),
        unavailable_requirement(
            "control.controller_by_province",
            Some(frame.control_by_province_status()),
        ),
        unavailable_requirement(
            "control.controller_by_state",
            Some(frame.control_by_state_status()),
        ),
        unavailable_requirement(
            "control.owner_by_state",
            Some(frame.owner_by_state_status()),
        ),
        unavailable_requirement(
            "country.current_capital_state",
            Some(frame.diplomacy().status),
        ),
        unavailable_requirement(
            "country.d18_complete_roster",
            Some(frame.d18_country_roster().status),
        ),
        unavailable_requirement(
            "country.presentation_identity",
            Some(frame.country_identities_status()),
        ),
        unavailable_requirement("diplomacy.faction_roster", Some(faction_roster_status)),
        unavailable_requirement("diplomacy.pair_relations", Some(frame.diplomacy().status)),
        unavailable_requirement(
            "effects.semantic_lifecycle",
            Some(frame.xmap20_effects_status()),
        ),
        unavailable_requirement(
            "fog.province_visibility_by_pov",
            Some(frame.visibility_status()),
        ),
        unavailable_requirement("fog.spotted_land_by_pov", Some(frame.visibility_status())),
        unavailable_requirement(
            "fog.spotted_taskforces_by_pov",
            Some(frame.visibility_status()),
        ),
        unavailable_requirement("fog.spotted_wings_by_pov", None),
        unavailable_requirement("lighting.point_light_lifecycle", None),
        unavailable_requirement(
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
        unavailable_requirement(
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
        unavailable_requirement("map_furniture.current_victory_points", None),
        unavailable_requirement(
            "mapmode.air_superiority",
            Some(frame.air_superiority_status()),
        ),
        unavailable_requirement(
            "mapmode.infrastructure_state",
            Some(frame.buildings_status()),
        ),
        unavailable_requirement("mapmode.intelligence", Some(frame.diplomacy().status)),
        unavailable_requirement(
            "mapmode.naval_supremacy",
            Some(frame.naval_supremacy_status()),
        ),
        unavailable_requirement(
            "mapmode.occupation_resistance_compliance",
            Some(frame.occupation_status()),
        ),
        unavailable_requirement("mapmode.raids", Some(frame.raids_status())),
        unavailable_requirement("mapmode.resource_deposits", None),
        unavailable_requirement("mapmode.scripted_inputs", Some(frame.scripted_gui_status())),
        unavailable_requirement(
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
        unavailable_requirement(
            "objects.building_state_damage",
            Some(frame.d48_buildings_status()),
        ),
        unavailable_requirement("objects.city_population", None),
        unavailable_requirement("objects.generic_population_lifecycle", None),
        unavailable_requirement(
            "orders.army_battleplans_semantic",
            Some(frame.army_battleplans().status),
        ),
        unavailable_requirement("orders.theatre_army_hierarchy", None),
        unavailable_requirement("postfx.temporal_exposure_state", None),
        unavailable_requirement(
            "session.player_country_set",
            Some(frame.session_metadata().status),
        ),
        unavailable_requirement("terrain.snow_mud_semantic", Some(frame.snow_mud_status())),
        unavailable_requirement("units.air_base_location", Some(frame.wings_status())),
        unavailable_requirement("units.divisions", Some(frame.division_units_status())),
        unavailable_requirement("units.taskforces_fleets", Some(frame.fleets_status())),
        unavailable_requirement("units.wings", Some(frame.wings_status())),
        unavailable_requirement(
            "weather.province_snow_accumulator",
            Some(frame.weather_province_snow_status()),
        ),
        unavailable_requirement("weather.region_gameplay", Some(frame.weather_status())),
        unavailable_requirement(
            "weather.visual_membership_lifecycle",
            Some(frame.weather_visual_lifecycle().status),
        ),
        unavailable_requirement("world.calendar_time", None),
    ];
    requirements.sort_by(|left, right| left.input_id.cmp(&right.input_id));
    Ok(MapAuthorityProjection {
        schema: MAP_AUTHORITY_INPUT_SCHEMA.to_string(),
        hour_index: frame.hour_index(),
        content_manifest_digest_sha256: manifest_digest,
        product_modset_sha256: manifest.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
        requirements,
    })
}

pub fn admit_map_authority_json(bytes: &[u8]) -> Result<MapAuthorityAdmission, String> {
    let projection: MapAuthorityProjection = serde_json::from_slice(bytes)
        .map_err(|error| format!("decode map authority projection: {error}"))?;
    admit_map_authority_projection(&projection)
}

/// Validate the gap report. This function cannot authenticate caller-supplied
/// payloads and therefore never returns a ready admission.
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
    Ok(MapAuthorityAdmission {
        gaps: projection
            .requirements
            .iter()
            .map(|row| MapAuthorityGap {
                input_id: row.input_id.clone(),
                disposition: row.disposition,
                reason: row.blocker.clone(),
            })
            .collect(),
    })
}

fn validate_requirement(row: &MapAuthorityRequirement) -> Result<(), String> {
    let expected = map_authority_requirement_metadata(&row.input_id)
        .ok_or_else(|| format!("unknown map authority input {}", row.input_id))?;
    if row.payload_schema.is_some() || row.cadence.is_some() || row.identity_order.is_some() {
        return Err(format!(
            "{} supplies unresolved map authority metadata",
            row.input_id
        ));
    }
    let blocker_is_canonical = match row.disposition {
        MapAuthorityRequirementDisposition::Absent => {
            row.blocker
                == format!(
                    "{}; WorldFrame root status is {:?}",
                    expected.blocker,
                    ViewStatus::DomainAbsent
                )
        }
        MapAuthorityRequirementDisposition::Unavailable => {
            row.blocker == expected.blocker
                || [ViewStatus::DomainUnavailable, ViewStatus::Present]
                    .into_iter()
                    .any(|status| {
                        row.blocker
                            == format!("{}; WorldFrame root status is {status:?}", expected.blocker)
                    })
        }
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
