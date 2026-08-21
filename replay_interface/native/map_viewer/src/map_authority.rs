
use std::collections::{BTreeMap, BTreeSet};

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
        input_id: "mapmode.raids",
        owner_packet: "XMAP18",
        payload_schema: "WorldFrame.raids",
        status_accessor: "raids_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "engine parent-then-child producer order under the producer's own instances_complete claim, re-validated structurally by the decoder; each instance a unique (id, id_type) with a measured phase, and every non-dummy country status row carrying a three-letter tag with a nonzero tag_db_id",
        status: raids_status,
        can_report_absent: true,
    },
    MapAuthorityOwnerProvenAdmission {
        input_id: "session.player_country_set",
        owner_packet: "XMAP18",
        payload_schema: "WorldFrame.session_metadata.player_countries",
        status_accessor: "session_metadata.player_countries",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "a non-empty set in the producer's non-decreasing (country_tag, tag_db_id, human_name) order, that order re-checked at decode together with a nonzero engine tag_db_id and the human-controlled marker on every row; every row names a three-letter country and no row's name is a failed read -- a human-held slot the engine named nobody for is measured absence. A TAG MAY REPEAT: two humans co-oping one country is a lawful engine state and the map mode paints set membership.",
        status: player_country_set_status,
        can_report_absent: true,
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
    // POSITIVE WITNESS: captures/oz_tip (lane PA, 2026-08-21).
    //
    // Lane OW recorded that this row had never been positively Admitted on any recording, and
    // read that as "no recording carries the fleets domain". captures/oz_tip does carry it -
    // 473 of its 535 hours - and the row still refused every one of them, so the missing
    // witness was half a data gap and half this predicate.
    //
    // The predicate demanded a measured nonzero location_province on EVERY row. CHI task
    // force 601 (fleet 2377, one destroyer, d56 naval mission 8 RESERVE_FLEET) has none, on
    // every hour, because the engine holds none for it - not because the read failed. The
    // recording proves which: the run manifest's bound_trips ledger is empty across the whole
    // capture, so no map-province bound trip ever refused a value. One lawfully locationless
    // task force therefore turned a Present fleets collection into DomainUnavailable, for the
    // whole family, on every hour of every sealed recording.
    //
    // With the engine's own absence admitted and only an unreadable location counted as a gap,
    // the census admits all 473 of those hours: 99,354 measured locations, 473 engine-absent,
    // zero unreadable. Evidence:
    // extractor/native/runs/pa_capture_defects/oz_tip_verdicts.json
    MapAuthorityOwnerProvenAdmission {
        input_id: "units.taskforces_fleets",
        owner_packet: "XMAP13",
        payload_schema: "WorldFrame.fleets",
        status_accessor: "fleets_status",
        cadence: "one decoded view per canonical recorded hour",
        identity_order: "recorder census order; each row a unique nonblank task_force_uid, a unique nonzero raw_task_force_id, a measured nonzero raw_fleet_id, a measured nonblank owner, a location_province that is either a measured nonzero province or the engine's own measured absence for a task force not deployed to one, no dropped source row, and no unaccounted collector problem on the D18 owner-identity or D56 naval-missions-status domains",
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
    // NO POSITIVE WITNESS YET (lane OW, checked 2026-08-21).
    //
    // This row and `units.air_base_location` were promoted to owner-proven without one real
    // recorded hour where they are positively Admitted, and a wider check did not find one.
    // Over ALL 533 hours of captures/mu_t83_settings_partition - 177x lane OK's three-hour
    // sample - both rows refuse in every single hour, each for the same one reason
    // ("WorldFrame certified fleets_status is DomainUnavailable" /
    // "... certified wings_status air-base join is DomainUnavailable"). The other candidate
    // recording, captures/ob_first_bmp, cannot be opened at all: its only artifact is
    // native_session.failed.battleplan, which the reader refuses as recovery-only.
    //
    // The predicates are NOT vacuous and this is NOT a defect in them: on the same 533 hours
    // units.divisions admits in 364 and units.wings in 366, so the machinery demonstrably
    // says yes when the data is there. What is missing is a recording that carries the fleets
    // domain and the wings air-base join at all - which needs a live game, so it is left
    // annotated rather than demoted. Census:
    // extractor/native/runs/ow_viewer_sweep/POSITIVE_WITNESS_mu_t83.json
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
        "mapmode.raids" => "certified raids_status",
        "session.player_country_set" => "certified D57 player-country leaf",
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

/// The raid census the raids map mode paints.
///
/// `build_raids` (`replay_core/src/world.rs`) hardcodes `ViewStatus::Present`
/// whenever the `raids` domain is an object at all, so the bare accessor says
/// nothing about the payload: on an injured hour it is still `Present` with both
/// collections `Unobtained`. The completeness this adds is the producer's own
/// `instances_complete` claim AND the decoder's structural re-validation of it
/// (`parse_complete_raid_instances` returns `None` and the builder clears the
/// flag when the parent/child ordering claim does not replay), plus unique
/// instance identity.
///
/// `next_target` is deliberately NOT required. `raids.counters` is spelled
/// `null` by the producer on every hour of `captures/oz_tip` — it is the engine's
/// raid-id allocator, not a map-mode input, and the raids map mode never reads
/// it. Requiring it would refuse every hour of every recording for a field the
/// map does not draw.
///
/// The engine's own `missing_tag_dummy` status row (tag `null`, tag_db_id 0,
/// child_count 0) is admitted as measured absence for the same reason: it is
/// what the engine holds, not a read that failed.
fn raids_status(frame: &WorldFrame) -> ViewStatus {
    let raids = frame.raids();
    // A frame that never heard of the raids domain reports absence, not a
    // failed read: `DomainAbsent` and `DomainUnavailable` are different answers
    // and the gap report spells them differently.
    if raids.status != ViewStatus::Present {
        return raids.status;
    }
    let (Some(statuses), Some(instances)) =
        (raids.country_statuses.measured(), raids.instances.measured())
    else {
        return ViewStatus::DomainUnavailable;
    };
    // The producer requires EXACTLY ONE `missing_tag_dummy`, at ordinal zero,
    // whenever the parent census is non-empty
    // (`extractor/native/compact_contract/src/raid_instances.rs`: the post-loop
    // `if dummy_count != 1 || next_child != children.len()` refusal; the
    // both-empty population returns Ok before ever reaching it).
    //
    // `parse_complete_raid_instances` re-checks the dummy's SHAPE if one is
    // there — unique ordinal zero, null tag, tag_db_id zero — but never checks
    // that one EXISTS, so a census whose first parent is an ordinary country
    // replays structurally and would otherwise be admitted as complete. That is
    // a population the producer cannot emit, and admitting it would mean
    // admitting a raid census short by the engine's untagged parent slot.
    // `world_fold` derives the priority list and the parent census from the
    // same engine vector, so the two must agree: a MEASURED list (present
    // array, every element parsed — `priorities` alone reads missing/null/
    // malformed as empty) whose tag set is exactly the non-dummy parent tag
    // set. Anything else is a shape the producer cannot emit.
    if !raids.priorities_measured {
        return certified_census_status(raids.status, false);
    }
    // The producer's both-empty population (no parents, no children) returns
    // Ok before its dummy refusal ever runs, so a raid-free hour is a valid
    // measured-empty census, not a failed claim.
    if statuses.is_empty() && instances.is_empty() {
        return certified_census_status(
            raids.status,
            raids.instances_complete && raids.priorities.is_empty(),
        );
    }
    // Row-for-row agreement, values included: each non-dummy parent's
    // (tag, priority) pair must be the priority list's entry, and the list
    // must carry nothing else. Tag-set equality alone would admit a
    // producer-impossible list with the right names and the wrong values.
    let non_dummy_parents = statuses
        .iter()
        .filter(|row| row.tag.is_some())
        .count();
    let priorities_agree = raids.priorities.len() == non_dummy_parents
        && statuses.iter().all(|row| match row.tag.as_deref() {
            None => true,
            Some(tag) => raids.priorities.get(tag) == Some(&row.priority),
        });
    if !priorities_agree {
        return certified_census_status(raids.status, false);
    }
    let dummy_population_is_exact = statuses
        .iter()
        .filter(|row| row.classification == "missing_tag_dummy")
        .count()
        == 1
        && statuses.first().is_some_and(|row| {
            row.classification == "missing_tag_dummy" && row.tag.is_none() && row.tag_db_id == 0
        });
    // EVERY country identity in the census must join the resolved roster on
    // the exact (tag, tag_db_id) pair: `world_fold` renders an unresolved
    // numeric id as its decimal string ("123".len() == 3, so shape alone
    // admits it), and a roster tag paired with ANOTHER country's db id would
    // resolve to the wrong country downstream. That covers parents, instance
    // owners, victims, and raid sources alike. A census that exists but
    // cannot be authenticated (no usable roster) is UNAVAILABLE, never
    // measured absence.
    let identity_map = match resolved_d18_identity_map(frame) {
        Ok(map) => map,
        Err(_) => return ViewStatus::DomainUnavailable,
    };
    let resolved_pair = |tag: Option<&str>, id: u32| match tag {
        // An absent optional identity carries id 0; anything else is a claim.
        None => id == 0,
        Some(tag) => identity_map.get(tag) == Some(&id),
    };
    let mut identities = BTreeSet::new();
    let complete = raids.instances_complete
        && dummy_population_is_exact
        && instances.iter().all(|row| {
            identities.insert((row.id, row.id_type))
                && !row.phase.trim().is_empty()
                && row.owner_tag.is_some()
                && resolved_pair(row.owner_tag.as_deref(), row.owner_tag_db_id)
                && resolved_pair(row.victim_country.as_deref(), row.victim_country_tag_db_id)
                && resolved_pair(row.raid_source.tag.as_deref(), row.raid_source.tag_db_id)
        })
        && statuses.iter().all(|row| match row.tag.as_deref() {
            None => row.classification == "missing_tag_dummy" && row.tag_db_id == 0,
            Some(tag) => identity_map.get(tag) == Some(&row.tag_db_id),
        });
    certified_census_status(raids.status, complete)
}

/// The human-held country set XMAP18's Players map mode paints.
///
/// `prepare_render_political`'s `HardcodedMapMode::Players` arm refuses on
/// exactly three things: an unmeasured set, a row with no measured tag, and a
/// blank or duplicate tag. This predicate is those three, moved to the door so
/// the authority row cannot promise a set the renderer will reject.
///
/// Duplicates are checked on `(tag, name)` rather than on the tag alone: the D57
/// producer sorts by `(country_tag, tag_db_id, human_name)` precisely because
/// one country can hold more than one human in co-op, so two rows for one TAG
/// are a real engine state, not a defect. The renderer folds them into one
/// `player_tags` entry, which is correct for painting; what would be wrong is
/// the SAME human appearing twice on the same country.
///
/// An empty known set is refused. `state: "known"` means the collector read the
/// engine's player vector, and a recording made by attaching to a running host
/// always has at least that host in it — an empty read is the shape of a
/// successful call against the wrong object, not of a game nobody is playing.
fn player_country_set_status(frame: &WorldFrame) -> ViewStatus {
    match player_country_set_leaf_status(frame) {
        // The leaf itself is this row's ledger accessor, NOT
        // `session_metadata.status`. That root is combined from `session` and
        // `world.scenario`, neither of which this carrier lives in: the D57
        // producer can publish `players_by_country` on an hour where the
        // pause/speed section is unavailable, and gating on the root would have
        // refused a measured player set for a reason that has nothing to do
        // with it.
        ViewStatus::Present => {}
        other => return other,
    }
    let players = frame
        .session_metadata()
        .player_countries
        .measured()
        .expect("Present is set only when the leaf is measured");
    // NO uniqueness check, deliberately.
    //
    // There is no stable player identity in this view to be unique ON.
    // `PlayerCountry` carries a TAG and a display name; the engine's actual
    // player key (`player_key_hash`) is not projected into it. And every
    // candidate key is legitimately repeatable: two humans co-oping one country
    // repeat the TAG, two whom the engine named nobody for repeat `(TAG, None)`
    // — a pair the producer's own sort explicitly permits by comparing equal —
    // and two different people can choose the same display name, because a name
    // is not an identity. A uniqueness rule over any of those refuses a lawful
    // multiplayer capture and proves nothing about the one shape that would be
    // a real defect.
    //
    // What this row promises is therefore what it can check: a non-empty set in
    // which every row names a country and no row's name is a failed read. The
    // rest of the D57 contract — the nonzero engine identity, the
    // human-controlled marker and the producer's ordering — is enforced at
    // decode (`leaf_domain57_player_countries`), where the fields still exist.
    let complete = !players.is_empty()
        && players.iter().all(|player| {
            let Some(tag) = player.tag.measured() else {
                return false;
            };
            // Measured, or the engine's own "nobody is named here". A failed
            // read is not either of those.
            if matches!(player.player_name, Leaf::Unobtained) {
                return false;
            }
            tag.len() == 3 && !tag.trim().is_empty()
        });
    if complete {
        ViewStatus::Present
    } else {
        ViewStatus::DomainUnavailable
    }
}

/// Did the EXACT D57 carrier supply this frame's player-country leaf?
///
/// This is the row's ledger accessor, and it deliberately asks about the
/// carrier rather than about the leaf alone. `build_session_metadata` still
/// falls back to two legacy keys (`session.player_countries`,
/// `world.scenario.players`) that this recorder has never emitted; a frame
/// carrying `{"session":{"player_countries":["GER"]}}` produces a perfectly
/// measured one-row leaf with no `tag_db_id`, no human-controlled marker and no
/// producer order — none of which the legacy shape even has. Certifying off the
/// leaf alone would let that frame establish an admission whose `identity_order`
/// advertises the whole D57 contract.
///
/// So the accessor is the carrier's own three-way answer: the sidecar is
/// missing (absent), present but refused or unreadable (unavailable), or known
/// and decoded into a measured leaf (present).
pub fn player_country_set_leaf_status(frame: &WorldFrame) -> ViewStatus {
    let sidecar = match frame.domain("_r2_session_multiplayer_metadata_live") {
        // No D57 sidecar at all. Whatever the legacy fallback produced, this
        // row has nothing of its own to certify.
        None => return ViewStatus::DomainAbsent,
        // Spelled null, or wrong-typed: the producer had the domain and could
        // not deliver it. That is a failed read, not an absent domain.
        Some(Value::Null) => return ViewStatus::DomainUnavailable,
        Some(value) => match value.as_object() {
            None => return ViewStatus::DomainUnavailable,
            Some(sidecar) => sidecar,
        },
    };
    let state = match sidecar.get("players_by_country") {
        None => return ViewStatus::DomainAbsent,
        Some(Value::Null) => return ViewStatus::DomainUnavailable,
        Some(value) => value
            .as_object()
            .and_then(|players| players.get("state"))
            .and_then(Value::as_str),
    };
    if state != Some("known") {
        return ViewStatus::DomainUnavailable;
    }
    // The carrier says known; the decoder is what decides whether it really
    // decoded. Every producer-contract check lives there.
    match &frame.session_metadata().player_countries {
        Leaf::Measured(_) => ViewStatus::Present,
        _ => ViewStatus::DomainUnavailable,
    }
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

/// A task force's location is accounted for when the recorder either measured a
/// real province for it, or said the engine holds none.
///
/// The engine's absence marker at `CTaskForce+0x18` is a measurement, not a gap: a
/// task force that is not deployed to a province genuinely has no province, and a
/// reserve-fleet one is the observed case. Demanding a measured nonzero province
/// on every row made a single such task force -- CHI 601, one destroyer, on all
/// 535 hours of `captures/oz_tip` -- turn a Present fleets collection into
/// `DomainUnavailable`, so `units.taskforces_fleets` refused every hour of every
/// sealed recording over one row. `Leaf::Unobtained` is still a gap and still
/// costs the census: that is the read that actually failed.
fn task_force_location_is_accounted(location: &Leaf<u32>) -> bool {
    match location {
        Leaf::Measured(province_id) => *province_id != 0,
        Leaf::Absent => true,
        Leaf::Unobtained => false,
    }
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
                && task_force_location_is_accounted(&row.location_province)
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

/// The resolved `(tag, tag_db_id)` identity map. Same availability rules as
/// [`complete_d18_roster`]; consumers that must authenticate an id pair (not
/// just membership) join through this.
fn resolved_d18_identity_map(
    frame: &WorldFrame,
) -> Result<BTreeMap<String, u32>, ViewStatus> {
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
        .map(|entry| (entry.tag.clone(), entry.tag_db_id))
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
        // The five rows below are owner-proven, so these texts are reached only
        // when the proven status is not `Present` — i.e. the recording genuinely
        // has no complete carrier for this hour — and each says that instead of
        // naming work that is already done.
        // ---- XMAP18's seven remaining refusals, and why each is refused ----
        //
        // Lane PL measured these on `captures/oz_tip` (2026-08-21;
        // extractor/native/runs/pl_xmap18/). The COMMITTED evidence is the
        // authority-disposition witness over all 534 hours
        // (witness_oz_tip_all.json) plus a detailed raw-carrier census of 14
        // hours at stride 40 (census_oz_tip_stride40.json); raw-carrier
        // hour-counts below beyond that sample come from the lane's full scan,
        // whose detail was not committed, and carry that caveat.
        // These blocker STRINGS say
        // ONLY which contract is unmet. A blocker is rendered on every refusing
        // hour, including hours nobody has measured, so a string that said "the
        // census IS on the wire" or "the scopes ARE measured" would be false on
        // exactly the hours where the carrier is missing -- which are the hours
        // most likely to be read. Every measurement therefore lives in these
        // comments, where it is dated and attached to its evidence, and none of
        // it lives in a string that gets printed about an unmeasured frame.

        // MEASURED PRESENT, RENDERER CANNOT USE IT YET. On the committed
        // 14-hour stride-40 sample, `captures/oz_tip` carries the exact D31
        // census at `naval_air._r2_air_superiority_inputs` (173-248 rows over
        // 16-22 strategic regions and 27-32 owners) and the exact D56 ordered
        // occurrence list at `naval_air._r2_naval_supremacy`, under its own
        // `row_count` witness and the `C` recorder marker. (The lane's full
        // scan put the carriers at 469 and 475 of 534 hours respectively;
        // uncommitted, see the caveat above.)
        //
        // Neither is admissible yet, and the reason is NOT the carrier. The
        // recorder deliberately spells the two COLLAPSED views
        // (`naval_air.air_superiority`, root `naval_supremacy`) as null because
        // which country HOLDS a region is an engine rule nobody has proven
        // (`world_fold.rs::build_naval_air` and `insert_naval_supremacy` say so
        // at length) -- and XMAP18's own renderer reads NEITHER form: its
        // strategic-air and strategic-navy modes require the per-observer
        // `map_mode_runtime` matrix (`xmap18.rs::raw_runtime_layers`), which no
        // producer in this repo emits. Admitting these rows off the uncollapsed
        // census would tell the map layer an input is ready and then fail the
        // moment either mode was selected.
        //
        // What closes them is one of two things, both real work: emit the
        // `map_mode_runtime` carrier, or implement the viewer-side per-observer
        // derivation from the uncollapsed census that is already on the wire.
        "mapmode.air_superiority" | "mapmode.naval_supremacy" => {
            "XMAP18 must prove the exact all-country map-mode input, unit, identity order, completeness, and authenticated producer/content joins; no per-observer map-mode carrier or viewer-side derivation exists to consume a D31/D56 census"
        }

        // MEASURED PRESENT, NO COMPLETENESS WITNESS. `captures/oz_tip` carries
        // 1,081 per-state building rows with 1,045 measured infrastructure
        // levels (the other 36 states have never built one, which is the
        // engine's own answer and what the renderer's `None => 0.0` arm draws),
        // and on its snapshot hour a D48 resource map over all 1,081 states.
        //
        // Neither is admissible, for the same reason: nothing in the recording
        // says the state set in the frame IS the state set the effective VFS
        // defines. XMAP18's infrastructure mode iterates the VFS state list and
        // refuses on the first state the frame does not carry
        // (`xmap18.rs::prepare_render_political`), so "every state I was given
        // is fine" is not the claim the renderer needs -- a frame carrying ONE
        // state would pass it. `mapmode.occupation_resistance_compliance` is
        // admitted precisely because D52 ships a `published_state_digest` over
        // its own membership; these two rows have no equivalent.
        //
        // What closes them: a producer-side state-membership witness of the D52
        // kind, or an authenticated comparison against the effective VFS state
        // list.
        "mapmode.infrastructure_state" | "mapmode.resource_deposits" => {
            "XMAP18 must prove the exact all-country map-mode input, unit, identity order, completeness, and authenticated producer/content joins; no producer census witness or effective-VFS join proves the per-state membership is complete"
        }

        // MEASURED PRESENT, SCOPE UNIVERSE NEVER ENUMERATED. On every healthy
        // hour of the committed 14-hour stride-40 sample of `captures/oz_tip`
        // the scripted variable/flag surface carries no unobtained scope at
        // all: 104 measured country scopes and 336 measured-absent ones, 308
        // measured state scopes and 773 measured-absent. On the one injured
        // hour in the sample every measured scope flips to unobtained, so the
        // distinction is real.
        //
        // It is still not completeness. A scope map that is simply EMPTY also
        // has no unobtained entry, so "no scope failed" cannot tell a fully
        // measured world from one where the scopes were never enumerated, and
        // the country/state universes themselves are not witnessed anywhere in
        // the frame. Separately, XMAP18's scripted modes render from
        // `map_mode_scripted_runtime` (`xmap18.rs::prepare_scripted_render`),
        // which no producer emits.
        //
        // What closes it: a witnessed country/state scope universe, and the
        // scripted map-mode runtime carrier.
        "mapmode.scripted_inputs" => {
            "XMAP18 must prove the exact all-country map-mode input, unit, identity order, completeness, and authenticated producer/content joins; the scripted scope universe is not witnessed and no scripted map-mode runtime carrier exists"
        }

        // Owner-proven, so this text is reached only when the proven predicate
        // is not Present. `captures/oz_tip` admits it on 470 of 534 hours.
        "mapmode.raids" => {
            "the recording carries no complete measured raid instance/status census for this hour"
        }

        // Owner-proven; admitted on 470 of 534 hours of `captures/oz_tip`.
        "session.player_country_set" => {
            "the recording carries no complete measured human player-country set for this hour"
        }

        // SURFACE COMPLETE, CONTENT EMPTY. On the healthiest hour of
        // `captures/oz_tip` all 96,141 unordered pairs over the 439-tag D18
        // roster are answerable -- and every one of them answers `at_war`
        // unobtained and `same_faction` unobtained, with the other five relation
        // fields measured-absent
        // (extractor/native/runs/pl_xmap18/census_oz_tip_pairfields.json).
        // Enumerability is not measurement: admitting on "every pair answers"
        // would promise a relation surface in which no relation was ever read.
        "diplomacy.pair_relations" => {
            "XMAP18 must prove the exact all-country map-mode input, unit, identity order, completeness, and authenticated producer/content joins; the D53 war-edge and faction-membership carriers that give a country pair its content are not proven"
        }

        // NOTHING ON THE WIRE. Over the committed 14-hour stride-40 sample of
        // `captures/oz_tip`, including its healthiest hour, `intel_levels`,
        // `intel_network_graphs`, `operation_instances` and `air_disclosure`
        // are every one of them `Leaf::Unobtained` -- so no predicate written
        // here could admit this row on those hours. (The lane's full scan saw
        // the same on all 534 hours; uncommitted, see the caveat above.) The string still states the contract rather
        // than that measurement, because it is rendered on hours nobody has
        // measured.
        "mapmode.intelligence" => {
            "XMAP18 must prove the exact all-country map-mode input, unit, identity order, completeness, and authenticated producer/content joins; the D53 intel-level, intel-network, operation-instance and air-disclosure carriers it would prove against are not owner-proven"
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
        projector.derive_requirement("session.player_country_set", None),
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
