
use std::collections::BTreeSet;

use replay_core::{ContentManifest, Leaf, ViewStatus, WorldFrame};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snapshot::sha256::Sha256;

pub const MAP_AUTHORITY_INPUT_SCHEMA: &str = "battle-plan-map-authority-input-v4";

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
    Admitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapAuthorityRequirementMetadata {
    pub input_id: &'static str,
    pub payload_schema: Option<&'static str>,
    pub cadence: Option<&'static str>,
    pub identity_order: Option<&'static str>,
    pub blocker: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapAuthorityOwnerProvenAdmission {
    pub input_id: &'static str,
    pub owner_packet: &'static str,
    pub payload_schema: &'static str,
    pub status_accessor: &'static str,
    pub cadence: &'static str,
    pub identity_order: &'static str,
    pub status: fn(&WorldFrame) -> ViewStatus,
    pub can_report_absent: bool,
}

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
        can_report_absent: false,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "map_furniture.current_victory_points",
        owner_packet: "XMAP21",
        payload_schema: "WorldFrame.map_furniture.victory_points",
        status_accessor: "map_furniture_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "strictly ascending province id, every value positive",
        status: map_furniture_current_victory_points_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "weather.province_snow_accumulator",
        owner_packet: "XMAP07",
        payload_schema: "WorldFrame.weather_province_snow",
        status_accessor: "weather_province_snow_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "ascending province id",
        status: province_snow_accumulator_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "mapmode.occupation_resistance_compliance",
        owner_packet: "XMAP18",
        payload_schema: "WorldFrame.occupation_by_state",
        status_accessor: "occupation_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "ascending state id, under D52's own state-root census witness",
        status: occupation_resistance_compliance_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "objects.building_state_damage",
        owner_packet: "XMAP12",
        payload_schema: "WorldFrame.buildings_by_state",
        status_accessor: "d48_buildings_status",
        cadence: "one decoded D48 building view per canonical recorded hour",
        identity_order: "(state_id, province_id, building_token) per row, in recorder census order",
        status: building_state_damage_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "world.calendar_time",
        owner_packet: "XMAP09",
        payload_schema: "WorldFrame.hour_index;WorldFrame.date",
        status_accessor: "date",
        cadence: "one calendar stamp per canonical recorded hour",
        identity_order: "single scalar pair; no census",
        status: calendar_time_status,
        can_report_absent: false,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "units.divisions",
        owner_packet: "XMAP13",
        payload_schema: "WorldFrame.division_units",
        status_accessor: "division_units_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "recorder census order; each row a unique nonzero raw_division_id with a unique nonblank div_uid, a measured nonzero raw_division_type, a measured nonblank owner, a measured nonzero location_province, no dropped source row, and no unaccounted collector problem on the D18 owner-identity domain",
        status: divisions_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "units.taskforces_fleets",
        owner_packet: "XMAP13",
        payload_schema: "WorldFrame.fleets",
        status_accessor: "fleets_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "recorder census order; each row a unique nonblank task_force_uid, a unique nonzero raw_task_force_id, a measured nonzero raw_fleet_id, a measured nonblank owner, a measured nonzero location_province, no dropped source row, and no unaccounted collector problem on the D18 owner-identity or D56 naval-missions-status domains",
        status: taskforces_fleets_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "units.wings",
        owner_packet: "XMAP13",
        payload_schema: "WorldFrame.wings",
        status_accessor: "wings_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "recorder census order; each row a unique nonblank wing_uid with a measured nonblank owner, zero unreadable rows, no dropped source row, and no unaccounted collector problem on the D18 owner-identity domain",
        status: wings_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "units.air_base_location",
        owner_packet: "XMAP13",
        payload_schema: "WorldFrame.wings.air_base_province;WorldFrame.wings.state_id",
        status_accessor: "wings_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "every air-wing row decoded, each carrying a measured nonzero air_base_province and a measured nonzero state_id, with no unaccounted collector problem on the D32 air-wing-loadout or D34 air-base-status domains; the owner tag is units.wings' claim, not this row's",
        status: air_base_location_status,
        can_report_absent: true,
    },
];

pub fn map_authority_observed_status_label(input_id: &str) -> &'static str {
    match input_id {
        "combat.naval_air_epoch" => "naval_combats_status",
        "combat.land_terminal_detail" => "certified ended_combats_status",
        "mapmode.occupation_resistance_compliance" => "certified occupation_status",
        "weather.province_snow_accumulator" => "certified weather_province_snow_status",
        "world.calendar_time" => "derived hour_index;date pair",
        "country.d18_complete_roster" => "d18_country_roster.status",
        "diplomacy.faction_roster" => "faction_roster_status",
        "units.divisions" => "certified division_units_status",
        "units.taskforces_fleets" => "certified fleets_status",
        "units.wings" => "certified wings_status",
        "units.air_base_location" => "certified wings_status air-base join",
        _ => "root status",
    }
}

fn certified_census_status(status: ViewStatus, complete: bool) -> ViewStatus {
    if status == ViewStatus::Present && !complete {
        return ViewStatus::DomainUnavailable;
    }
    status
}

fn occupation_resistance_compliance_status(frame: &WorldFrame) -> ViewStatus {
    let census = frame
        .domain("_r2_occupation_census_live")
        .and_then(serde_json::Value::as_object);
    let producer_census_is_complete = census
        .and_then(|census| census.get("census_complete"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let published_rows = census
        .and_then(|census| census.get("published_state_rows"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|rows| usize::try_from(rows).ok());
    let published_digest = census
        .and_then(|census| census.get("published_state_digest"))
        .and_then(serde_json::Value::as_str)
        .and_then(|digest| digest.parse::<u64>().ok());
    let decoded_rows = frame.occupation_by_state().len();
    let states = frame
        .domain("states")
        .and_then(serde_json::Value::as_object);
    let states_are_structurally_intact = states.is_some_and(|states| {
        states
            .iter()
            .all(|(key, value)| key.parse::<u32>().is_ok() && value.is_object())
    });
    let raw_occupation_ids: Option<Vec<u32>> = states.map(|states| {
        let mut ids: Vec<u32> = states
            .iter()
            .filter(|(_, value)| {
                value
                    .as_object()
                    .is_some_and(|state| state.contains_key("occupation"))
            })
            .map(|(key, _)| key.parse::<u32>().unwrap_or(u32::MAX))
            .collect();
        ids.sort_unstable();
        ids
    });
    let raw_rows = raw_occupation_ids.as_ref().map(Vec::len);
    let every_row_measured = frame.occupation_by_state().values().all(|row| {
        matches!(row.resistance, Leaf::Measured(_)) && matches!(row.compliance, Leaf::Measured(_))
    });
    let membership_matches = match (published_digest, raw_occupation_ids.as_deref()) {
        (Some(digest), Some(ids)) => digest == fnv1a64_over_state_ids(ids),
        _ => false,
    };
    let complete = producer_census_is_complete
        && states_are_structurally_intact
        && every_row_measured
        && raw_rows == Some(decoded_rows)
        && raw_rows == published_rows
        && membership_matches;
    certified_census_status(frame.occupation_status(), complete)
}

fn fnv1a64_over_state_ids(ids: &[u32]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for id in ids {
        for byte in id.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

fn province_snow_accumulator_status(frame: &WorldFrame) -> ViewStatus {
    let view = frame.weather_province_snow();
    let every_province_measured = view
        .by_province
        .values()
        .all(|snow| matches!(snow, Leaf::Measured(_)));
    let weather = frame
        .domain("map")
        .and_then(serde_json::Value::as_object)
        .and_then(|map| map.get("_r2_weather_live"))
        .and_then(serde_json::Value::as_object);
    let declared_rows = weather
        .and_then(|weather| weather.get("observed_province_rows"))
        .and_then(serde_json::Value::as_u64);
    let raw_rows = weather
        .and_then(|weather| weather.get("provinces"))
        .and_then(serde_json::Value::as_object)
        .map(serde_json::Map::len);
    const MIN_MEASURED_PROVINCE_ROWS: u64 = 2;
    const MAX_MEASURED_PROVINCE_ROWS: u64 = 65_536;
    let declared_row_count_is_met = match (declared_rows, raw_rows) {
        (Some(declared), Some(raw)) => {
            let declared_len = declared as usize;
            (MIN_MEASURED_PROVINCE_ROWS..=MAX_MEASURED_PROVINCE_ROWS).contains(&declared)
                && declared_len == raw
                && raw == view.by_province.len()
                && view
                    .by_province
                    .keys()
                    .all(|province_id| (*province_id as u64) < declared)
        }
        _ => false,
    };
    certified_census_status(
        certified_census_status(view.status, every_province_measured),
        declared_row_count_is_met,
    )
}




fn calendar_time_status(frame: &WorldFrame) -> ViewStatus {
    let measured = frame
        .date()
        .is_some_and(|date| engine_calendar_hour_index(date) == Some(frame.hour_index()));
    if measured {
        ViewStatus::Present
    } else {
        ViewStatus::DomainUnavailable
    }
}

const ENGINE_MONTH_LENGTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn engine_calendar_hour_index(date: &str) -> Option<i64> {
    const MIN_HOUR_INDEX: i64 = 1850 * 365 * 24;
    const MAX_HOUR_INDEX: i64 = 2200 * 365 * 24;
    let mut fields = date.split('.');
    let mut next = || {
        let field = fields.next()?;
        if field.is_empty()
            || !field.bytes().all(|byte| byte.is_ascii_digit())
            || (field.len() > 1 && field.starts_with('0'))
        {
            return None;
        }
        field.parse::<u32>().ok()
    };
    let (year, month, day, hour) = (next()?, next()?, next()?, next()?);
    if fields.next().is_some() || !(1..=12).contains(&month) || hour > 23 {
        return None;
    }
    let month_index = usize::try_from(month - 1).ok()?;
    if day == 0 || day > ENGINE_MONTH_LENGTHS[month_index] {
        return None;
    }
    let day_of_year_index: u32 =
        ENGINE_MONTH_LENGTHS[..month_index].iter().sum::<u32>() + (day - 1);
    let hour_index =
        (i64::from(year) * 365 + i64::from(day_of_year_index)) * 24 + i64::from(hour);
    (MIN_HOUR_INDEX..=MAX_HOUR_INDEX)
        .contains(&hour_index)
        .then_some(hour_index)
}

fn land_active_epoch_status(frame: &WorldFrame) -> ViewStatus {
    frame.combats_status()
}

/// XMAP00 row #20, the per-province victory-point value census.
///
/// `MapFurnitureFrame::validate` is the admission and it has already run inside
/// `build_map_furniture`: it pins the exact-a729 executable identity, the v4
/// schema token, `update_index == displayed hour`, the producer's own
/// `complete` attestation, strictly ascending province ids and a positive value
/// on every row. A frame that failed any of those is `DomainUnavailable` with
/// the reason in the frame's coverage warnings, so `Present` here means the
/// whole contract held.
///
/// The extra check this adds is the one the decoder cannot make: that the frame
/// really carries the census rather than merely reporting `Present`. The two
/// cannot disagree today — `Present` is set only on the `Ok` arm — and the guard
/// stays because this function is what a future accessor change would have to
/// keep true, and a status divorced from its payload is exactly what would
/// admit an empty XMAP21 draw.
fn map_furniture_current_victory_points_status(frame: &WorldFrame) -> ViewStatus {
    let status = frame.map_furniture_status();
    if status == ViewStatus::Present && frame.map_furniture().is_none() {
        return ViewStatus::DomainUnavailable;
    }
    status
}

/// `ViewStatus::Present` alone is NOT completeness for the three unit
/// collections. `build_division_units`, `build_fleets` and `build_wings`
/// (`replay_core/src/world.rs`) each `continue` past a malformed source row,
/// push a `... row skipped` coverage warning, and leave the collection status
/// untouched. A census short by a division is exactly what XMAP13's
/// `validate_census` refuses, so admitting the row on the bare accessor would
/// promise a complete all-country census the frame does not carry. This is the
/// same defect `land_terminal_detail_status` already guards for
/// `ended_combats`, applied to the source prefixes those three builders emit.
fn dropped_source_rows(frame: &WorldFrame, prefix: &str) -> bool {
    frame
        .coverage_warnings()
        .iter()
        .any(|warning| warning.starts_with(prefix) && warning.contains("row skipped"))
}

/// Collector domains that CONTRIBUTE a field these rows claim without owning
/// the census that carries the rows.
///
/// `replay_core`'s reconciliation table maps a problem to a typed view and
/// downgrades it. A domain outside that table gets only a warning saying its
/// "views are NOT downgraded and may still present retained state as measured" —
/// so the field is still there, still `Measured`, and possibly carried over from
/// an earlier hour. A predicate that only reads the value cannot tell.
///
/// These three cannot simply be added to the table: each backs one field rather
/// than a collection, so mapping them would invalidate the whole view and take
/// down the census claims that are still sound.
const UNIT_OWNER_IDENTITY_DOMAIN: i64 = 18;
const NAVAL_MISSIONS_STATUS_DOMAIN: i64 = 56;
const AIR_WING_LOADOUT_DOMAIN: i64 = 32;
const AIR_BASE_STATUS_DOMAIN: i64 = 34;

/// The owner tag on every unit row resolves through D18 `country_identity`,
/// which retains its previous map when current-hour identity is unavailable.
const UNIT_OWNER_CONTRIBUTORS: [i64; 1] = [UNIT_OWNER_IDENTITY_DOMAIN];

/// Task forces additionally take their raw identity from the D56 overlay:
/// `exact_overlay_u32_identity` prefers `naval_missions_status_live`'s
/// `task_force_id` / `fleet_id` over the D28 `id`, so a row whose base id is
/// unobtained can still present a `Measured` raw identity sourced entirely from
/// D56.
const TASK_FORCE_CONTRIBUTORS: [i64; 2] =
    [UNIT_OWNER_IDENTITY_DOMAIN, NAVAL_MISSIONS_STATUS_DOMAIN];

/// The air-base join reads D32 `air_wing_loadouts_live` for the wing's base
/// placement and D34 `air_base_status_live` for the base itself. D18 is
/// deliberately NOT here: this row promises `air_base_province` and `state_id`,
/// not the owner tag, and the wing census that carries the rows is D30. Adding
/// D18 would refuse a base join that is fully proved.
const AIR_BASE_CONTRIBUTORS: [i64; 2] = [AIR_WING_LOADOUT_DOMAIN, AIR_BASE_STATUS_DOMAIN];

fn unmapped_problem_on_contributor(frame: &WorldFrame, domains: &[i64]) -> bool {
    frame.coverage_warnings().iter().any(|warning| {
        warning.contains("no typed view is mapped to it")
            && domains.iter().any(|domain| {
                warning.contains(&format!("domain_problems: domain {domain} declared"))
            })
    })
}

fn divisions_status(frame: &WorldFrame) -> ViewStatus {
    let rows = frame.division_units();
    let mut identities = BTreeSet::new();
    let mut uids = BTreeSet::new();
    let complete = !dropped_source_rows(frame, "divisions[")
        && !unmapped_problem_on_contributor(frame, &UNIT_OWNER_CONTRIBUTORS)
        && rows.iter().all(|row| {
            row.raw_division_id != 0
                && !row.div_uid.trim().is_empty()
                && identities.insert(row.raw_division_id)
                && uids.insert(row.div_uid.as_str())
                && row.raw_division_type.measured().is_some_and(|id| *id != 0)
                && row.owner.measured().is_some_and(|tag| !tag.trim().is_empty())
                && row.location_province.measured().is_some_and(|id| *id != 0)
        });
    certified_census_status(frame.division_units_status(), complete)
}

/// The ledger row is "taskforce and fleet enumeration", so the numeric engine
/// identities are part of what it promises, not only the semantic uid string.
/// XMAP13's task-force join checks neither (`xmap13.rs::validate_typed_join`
/// matches on the uid, owner and anchor alone), so nothing downstream would
/// notice a row whose raw task-force or fleet id never resolved.
fn taskforces_fleets_status(frame: &WorldFrame) -> ViewStatus {
    let rows = frame.fleets();
    let mut uids = BTreeSet::new();
    let mut task_force_ids = BTreeSet::new();
    let complete = !dropped_source_rows(frame, "order_of_battle.fleets[")
        && !unmapped_problem_on_contributor(frame, &TASK_FORCE_CONTRIBUTORS)
        && rows.iter().all(|row| {
            !row.task_force_uid.trim().is_empty()
                && uids.insert(row.task_force_uid.as_str())
                && row
                    .raw_task_force_id
                    .measured()
                    .is_some_and(|id| *id != 0 && task_force_ids.insert(*id))
                && row.raw_fleet_id.measured().is_some_and(|id| *id != 0)
                && row.owner.measured().is_some_and(|tag| !tag.trim().is_empty())
                && row.location_province.measured().is_some_and(|id| *id != 0)
        });
    certified_census_status(frame.fleets_status(), complete)
}

/// Every source air-wing row became a `WingUnitRow`.
///
/// Shared by both wing-backed ledger rows because a dropped wing makes the
/// census short for either claim. What is NOT shared is what each row asserts
/// ON TOP of that: `units.wings` owns wing identity, `units.air_base_location`
/// owns the base join. Keeping those separate is what lets one be false while
/// the other is true.
fn wings_rows_all_decoded(frame: &WorldFrame) -> bool {
    frame.wings_unreadable_rows() == 0
        && !dropped_source_rows(frame, "order_of_battle.air_wings[")
}

fn wings_status(frame: &WorldFrame) -> ViewStatus {
    let mut uids = BTreeSet::new();
    let complete = wings_rows_all_decoded(frame)
        && !unmapped_problem_on_contributor(frame, &UNIT_OWNER_CONTRIBUTORS)
        && frame.wings().iter().all(|row| {
            !row.wing_uid.trim().is_empty()
                && uids.insert(row.wing_uid.as_str())
                && row.owner.measured().is_some_and(|tag| !tag.trim().is_empty())
        });
    certified_census_status(frame.wings_status(), complete)
}

/// XMAP13 anchors an air-wing row at `air_base_province`
/// (`xmap13.rs::validate_typed_join`), so a wing whose base province never
/// resolved cannot be placed at all.
fn air_base_location_status(frame: &WorldFrame) -> ViewStatus {
    let complete = wings_rows_all_decoded(frame)
        && !unmapped_problem_on_contributor(frame, &AIR_BASE_CONTRIBUTORS)
        && frame.wings().iter().all(|row| {
            row.air_base_province.measured().is_some_and(|id| *id != 0)
                // Zero is not a state. XMAP13 never re-checks `state_id`, so a
                // zero here would travel as a usable state identity.
                && row.state_id.measured().is_some_and(|id| *id != 0)
        });
    certified_census_status(frame.wings_status(), complete)
}

fn building_state_damage_status(frame: &WorldFrame) -> ViewStatus {
    frame.d48_buildings_status()
}

fn naval_air_epoch_status(frame: &WorldFrame) -> ViewStatus {
    frame.naval_air_combat().naval_combats_status
}

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

fn complete_d18_roster(frame: &WorldFrame) -> Result<BTreeSet<String>, ViewStatus> {
    let roster = frame.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return Err(roster.status);
    }
    if roster.entries.is_empty() {
        return Err(ViewStatus::DomainUnavailable);
    }
    Ok(roster
        .entries
        .iter()
        .map(|entry| entry.tag.clone())
        .collect())
}

fn d18_complete_roster_status(frame: &WorldFrame) -> ViewStatus {
    match complete_d18_roster(frame) {
        Ok(_) => ViewStatus::Present,
        Err(status) => status,
    }
}

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
        return ViewStatus::DomainUnavailable;
    };
    if members.values().flatten().next().is_none() {
        return ViewStatus::Present;
    }
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
    #[serde(skip)]
    frame_derivation_witness_sha256: Option<String>,
}

impl MapAuthorityProjection {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapAuthorityAdmittedInput {
    pub input_id: String,
    pub owner_packet: String,
    pub payload_schema: String,
    pub cadence: String,
    pub identity_order: String,
}

/// Account for the map-authority inputs one packet owns, in either state.
///
/// A packet owes the same accounting whether its inputs are still gaps or have
/// been proved and admitted: the receipt has to name what the layer consumed
/// either way. Returns the subset that is still gapped, which is what a typed
/// unavailable output carries as its `consumed_gap_ids`.
///
/// Refuses if the projection omits an owned input entirely (the projection is
/// no longer the complete census the packet was written against) or reports one
/// as both admitted and gapped (two contradictory dispositions for one input).
pub fn account_owned_map_authority_inputs(
    packet: &str,
    owned: &[&str],
    gaps: &[MapAuthorityGap],
    admitted: &[MapAuthorityAdmittedInput],
) -> Result<Vec<String>, String> {
    let mut consumed_gap_ids = Vec::new();
    for input in owned {
        let is_gap = gaps.iter().any(|candidate| candidate.input_id == *input);
        let is_admitted = admitted
            .iter()
            .any(|candidate| candidate.input_id == *input);
        match (is_gap, is_admitted) {
            (true, true) => {
                return Err(format!(
                    "{packet} authority projection both admitted and gapped owned input {input}"
                ))
            }
            (true, false) => consumed_gap_ids.push((*input).to_string()),
            (false, true) => {}
            (false, false) => {
                return Err(format!(
                    "{packet} authority projection omitted owned input {input}"
                ))
            }
        }
    }
    Ok(consumed_gap_ids)
}

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
        "country.d18_complete_roster" => {
            "the recording carries no complete measured D18 country roster for this hour"
        }
        "terrain.snow_mud_semantic" => {
            "this per-province accumulator is measured (domain 24 snow_raw + mud byte, engine encodings) but is not owner-proven as a map-authority input, and cannot substitute for XMAP07's exact passive global dense texture/fade carrier"
        }
        "weather.province_snow_accumulator" => {
            "the recording carries no complete measured per-province snow accumulator for this hour"
        }
        "lighting.point_light_lifecycle" => {
            "XMAP09 must prove the exact light lifecycle, caster joins, cadence, and authenticated producer contract"
        }
        "world.calendar_time" => {
            "the recording carries no engine calendar stamp for this hour that agrees with its own hour index"
        }
        "objects.building_state_damage" => {
            "the recording carries no measured D48 building state/damage census for this hour"
        }
        "objects.city_population" | "objects.generic_population_lifecycle" => {
            "XMAP12 regenerates these populations from admitted effective content, so no recorded semantic carrier is owed; the row stays unproven because there is nothing on the wire to admit"
        }
        "units.air_base_location"
        | "units.divisions"
        | "units.taskforces_fleets"
        | "units.wings"
        | "orders.theatre_army_hierarchy" => {
            "XMAP13 must prove the exact all-country unit/hierarchy subset, stable identity, order, completeness, and authenticated producer contract"
        }
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
        "country.current_capital_state" => {
            "XMAP21 must prove the exact live furniture value, identity, cadence, completeness, and authenticated producer/content joins"
        }
        // Row #20 is owner-proven, so this text is reached only when the proven
        // status is not `Present` — i.e. the recording genuinely has no census
        // for this hour — and it says that instead of naming work that is done.
        "map_furniture.current_victory_points" => {
            "the recording carries no complete measured victory-point census for this hour"
        }
        "diplomacy.faction_roster" => {
            "the recording carries no measured faction-membership roster for this hour"
        }
        "mapmode.occupation_resistance_compliance" => {
            "the recording carries no complete measured per-state occupation census for this hour"
        }
        "diplomacy.pair_relations"
        | "mapmode.air_superiority"
        | "mapmode.infrastructure_state"
        | "mapmode.intelligence"
        | "mapmode.naval_supremacy"
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

pub fn map_authority_projection_from_world_frame(
    frame: &WorldFrame,
    manifest: &ContentManifest,
) -> Result<MapAuthorityProjection, String> {
    let manifest_digest = manifest
        .manifest_digest_sha256
        .clone()
        .ok_or_else(|| "map authority requires a finalized content manifest".to_string())?;
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
        projector.derive_requirement(
            "map_furniture.current_victory_points",
            Some(frame.map_furniture_status()),
        ),
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
        projector.derive_requirement("world.calendar_time", Some(calendar_time_status(frame))),
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
    projection.frame_derivation_witness_sha256 = Some(projection.admission_witness_sha256()?);
    Ok(projection)
}

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
    let unavailable_statuses: &[ViewStatus] = if proven.is_some() {
        &[ViewStatus::DomainUnavailable]
    } else {
        &[ViewStatus::DomainUnavailable, ViewStatus::Present]
    };
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
