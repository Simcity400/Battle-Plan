use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs, io,
    path::PathBuf,
    time::Instant,
};

use content_reader_layers::ResolveOptions;
#[cfg(test)]
use content_reader_save::Decimal;
use content_reader_save::{self as save, Save};
use replay::{campaign::Replay, campaign::SeekResult};
#[cfg(test)]
use replay_data::rows::TableRows;
use replay_data::{
    BranchId, Hour, identity::SampleBoundary, identity::Verification, rows::TableId, rows::tables,
};
use replay_frame::frame::DecodedFrame;
#[cfg(test)]
use replay_frame::frame::TableView;

use self::support::*;
#[cfg(test)]
use recorder_oracle_support::fixtures;
use recorder_oracle_support::support;

use recorder_oracle_air::air;
use recorder_oracle_air_ledger::air_ledger;
use recorder_oracle_air_missions::air_missions;
use recorder_oracle_characters::characters;
use recorder_oracle_combat_details::combat_details;
use recorder_oracle_combat_log::combat_log;
use recorder_oracle_construction::construction;
use recorder_oracle_core::core;
use recorder_oracle_counters::counters;
#[cfg(test)]
mod country_emissions;
use recorder_oracle_economy::economy;
use recorder_oracle_equipment_destruction::equipment_destruction;
use recorder_oracle_event_feed::{event_feed, map_events};
use recorder_oracle_faction_theatres::faction_theatres;
use recorder_oracle_factories::factories;
use recorder_oracle_fog::fog;
#[cfg(test)]
mod fresh_movement_tests;
use recorder_oracle_game_score::game_score;
use recorder_oracle_governance::governance;
use recorder_oracle_intel::intel;
use recorder_oracle_land::land;
use recorder_oracle_map_inputs::map_inputs;
use recorder_oracle_military::military;
use recorder_oracle_modifier_breakdowns::modifier_breakdowns;
use recorder_oracle_modifiers::modifiers;
use recorder_oracle_naval_history::naval_history;
use recorder_oracle_naval_result_sources::naval_result_sources;
use recorder_oracle_pass_inputs::pass_inputs;
use recorder_oracle_peace::peace;
use recorder_oracle_plans::plans;
use recorder_oracle_politics::politics;
use recorder_oracle_power_ledger::power_ledger;
use recorder_oracle_production::production;
use recorder_oracle_projects::projects;
use recorder_oracle_railway_guns::railway_guns;
use recorder_oracle_relations::relations;
use recorder_oracle_rocket_output::rocket_output;
use recorder_oracle_saved_event_targets::saved_event_targets;
use recorder_oracle_selected_routes::selected_routes;
use recorder_oracle_session::session;
use recorder_oracle_stability::stability;
use recorder_oracle_support::report;
#[cfg(test)]
mod state_history_tests;
use recorder_oracle_supply_trains::supply_trains;
use recorder_oracle_unit_combat_references::unit_combat_references;
use recorder_oracle_visibility::visibility;
use recorder_oracle_wars::wars;
use recorder_oracle_weather::weather;

#[cfg(test)]
use self::relations::saved_country_slot;
use self::relations::{
    compare_country_intel, compare_faction_goals, compare_factions, compare_relation_opinions,
    compare_relation_tuples, compare_war_relations,
};

use self::military::{
    compare_air_wings, compare_army_orders, compare_battle_sounds, compare_country_template_state,
    compare_division_movement, compare_division_panel_stats, compare_division_status,
    compare_division_template_icons, compare_division_templates, compare_divisions,
    compare_land_combats, compare_naval_combat_sources, compare_naval_combats, compare_naval_units,
};

use self::core::{
    compare_boundary_counters, compare_countries, compare_players, compare_province_names,
    compare_provinces, compare_state_flags, compare_states,
};

#[cfg(test)]
use self::report::recorded_table_counts;

use self::report::{Report, is_load_hour, last_read_hours};

pub const USAGE: &str = "recorder oracle <campaign.battleplan> <save.hoi4> [--hour n] [--branch id] [--game-dir <directory>] [--user-dir <directory>]";

pub fn command(args: Vec<OsString>) -> io::Result<()> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let started = Instant::now();
    let mut args = args.into_iter();
    let replay_path = PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?);
    let save_path = PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?);
    let mut requested_hour = None;
    let mut requested_branch = None;
    let mut game_dir = None;
    let mut user_dir = None;
    while let Some(option) = args.next() {
        let value = args.next().ok_or_else(|| invalid(USAGE))?;
        match option.to_str() {
            Some("--hour") if requested_hour.is_none() => {
                requested_hour = Some(
                    value
                        .to_str()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| invalid("--hour requires an engine i32 hour"))?,
                );
            }
            Some("--branch") if requested_branch.is_none() => {
                requested_branch = Some(
                    value
                        .to_str()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| invalid("--branch requires a u64 branch id"))?,
                );
            }
            Some("--game-dir") if game_dir.is_none() => game_dir = Some(PathBuf::from(value)),
            Some("--user-dir") if user_dir.is_none() => user_dir = Some(PathBuf::from(value)),
            _ => return Err(invalid(USAGE)),
        }
    }
    let save_document = save::SaveDocument::read(&fs::read(&save_path)?)?;
    let hour = matching_hour(save_document.date().engine_hour()?, requested_hour)?;
    let replay = Replay::open(fs::read(&replay_path)?)?;
    if save_document.campaign_id() != replay.campaign_id.as_slice() {
        return Err(invalid("save campaign id differs from the replay"));
    }
    let frame = select_frame(&replay, hour, requested_branch)?;
    let branch = replay
        .branches
        .iter()
        .find(|branch| branch.id == frame.branch)
        .ok_or_else(|| invalid("frame branch metadata is missing"))?;
    let session = replay
        .sessions
        .iter()
        .find(|session| session.id == branch.session)
        .ok_or_else(|| invalid("frame session metadata is missing"))?;
    match &session.identity.verification {
        Verification::Verified { .. } => {}
        Verification::Unverified { reason, .. } => {
            return Err(invalid(format!(
                "oracle refuses unverified session content: {reason}"
            )));
        }
        Verification::Unchecked => return Err(invalid("oracle refuses unchecked session content")),
    }
    if !same_version(save_document.version(), &session.identity.game_version) {
        return Err(invalid(format!(
            "save version {} differs from recorded version {}",
            String::from_utf8_lossy(save_document.version()),
            session.identity.game_version
        )));
    }
    let mut save = parse_country_emissions(&save_document, &frame)?;
    validate_identity(&frame, &save)?;
    let load_hour = is_load_hour(branch, hour);
    let saved_roster = save
        .characters
        .manager
        .as_mut()
        .and_then(|manager| characters::fit_saved_roster(&frame, manager, load_hour));
    let options = ResolveOptions {
        game_dir: game_dir
            .map(Ok)
            .unwrap_or_else(crate::directories::default_game_directory)?,
        user_dir: user_dir
            .map(Ok)
            .unwrap_or_else(|| crate::directories::resolve_user_directory().map_err(invalid))?,
        cache_dir: PathBuf::from(
            std::env::var_os("LOCALAPPDATA").ok_or_else(|| invalid("LOCALAPPDATA is not set"))?,
        )
        .join("BattlePlan")
        .join("recorder"),
    };
    println!(
        "Oracle: save {} / hour {hour}, session {}, branch {}",
        save.date, session.id, branch.id
    );
    println!(
        "Sample boundary: {} ({})",
        session.sample_boundary,
        boundary_description(session.sample_boundary)
    );
    println!(
        "Content: game {}, user {}",
        options.game_dir.display(),
        options.user_dir.display()
    );
    let content = content_reader_layers::resolve(&session.identity, &options)?.exact()?;
    engine_executable::executable_tables::initialize(&options.game_dir)?;
    let uncovered_reads = content.layers.report_uncovered_reads();
    println!(
        "Content verified under policy {}: {} files, {} bytes, {} ms",
        session.identity.coverage_policy,
        content.stats.files,
        content.stats.bytes,
        content.stats.elapsed_ms
    );
    let state_history = save::read_partial_state_history(&content.layers);
    let membership = state_history
        .as_ref()
        .map(|(membership, _)| membership)
        .map_err(ToString::to_string);
    let base_resources = state_history
        .as_ref()
        .map_err(ToString::to_string)
        .and_then(|(_, resources)| content_read(resources));
    let railway_adjacency = save::naval_history::RailwayAdjacency::read(&content.layers);
    let buildings = content_reader_key_names::buildings::read(&content.layers);
    let defines = content_reader_defines::defines::read(&content.layers);
    let raid_graph = content_reader_map::province_graph::ProvinceGraph::read(&content.layers);
    let equipment = content_reader_key_names::equipment::Database::load_complete(&content.layers);
    let essential_caps = content_reader_raids::raids::read(&content.layers)
        .map_err(|error| error.to_string())
        .and_then(|raids| raids.essential_caps(equipment.as_ref().map_err(ToString::to_string)?));
    let leader_traits =
        content_reader_military::leader_traits::read(&content.layers).map(|(traits, _)| traits);
    let sub_units = content_reader_key_names::unit_leader::sub_units(&content.layers);
    let regions = content_reader_regions::regions::read(&content.layers);
    let intelligence = content_reader_military::intelligence::read_agency_upgrades(&content.layers);
    let temperature = raid_graph
        .as_ref()
        .map_err(|error| io::Error::other(error.to_string()))
        .and_then(|(graph, geometry)| {
            content_reader_weather::weather::temperature::read_with(&content.layers, graph, geometry)
        });
    let province_modifiers = weather::ProvinceModifierContent::read(&content.layers, &save);
    let read_hours = last_read_hours(&replay, frame.branch, hour)?;
    let mut report = compare_core(
        &frame,
        session.sample_boundary,
        &save,
        membership.clone(),
        content_read(&equipment),
        Some(read_hours),
    )?;
    report.set_load_hour(branch);
    if let Some(roster) = saved_roster {
        characters::note_saved_roster(&mut report, roster);
    }
    report.attempt_table(TableId::NavalMissionTaskForces, |_| {
        bind_raiding_save_fields(&frame, &save_document, &mut save)
    });
    report.attempt_table(TableId::NavalPatrolMissionInputs, |_| {
        recorder_oracle_naval_missions::naval_missions::bind_patrol_save_fields(
            &frame,
            &save_document,
            &mut save,
        )
    });
    compare_extended(
        &frame,
        &save,
        ExtendedContent {
            membership,
            base_resources,
            railway_adjacency: content_read(&railway_adjacency),
            buildings: content_read(&buildings),
            defines: content_read(&defines),
            raid_graph: content_read(&raid_graph),
            equipment: content_read(&equipment),
            essential_caps: essential_caps.as_ref().map_err(Clone::clone),
            leader_traits: content_read(&leader_traits),
            sub_units: content_read(&sub_units),
            intelligence: content_read(&intelligence),
            temperature: content_read(&temperature),
            province_modifiers: content_read(&province_modifiers),
        },
        &mut report,
    );
    report.attempt_table(TableId::ProductionLineOutput, |report| {
        production::compare_line_output(&replay, &frame, report)
    });
    report.attempt_table(TableId::RocketProductionOutput, |report| {
        rocket_output::compare(&replay, &frame, report)
    });
    report.attempt_table(TableId::PeaceBids, |report| {
        peace::compare_bids(&frame, &save.projects, report)
    });
    report.attempt_table(TableId::PeaceSettlements, |report| {
        peace::compare_recorded_settlements(&replay, &frame, &save, report)
    });
    report.attempt_table(TableId::ConstructionLineOutput, |report| {
        construction::compare_output(&replay, &frame, report)
    });
    report.attempt_table(TableId::DivisionEquipmentDeletions, |report| {
        equipment_destruction::compare_deletions(&frame, &save, report)?;
        equipment_destruction::compare_deletion_history(&replay, &frame, &save, report)
    });
    report.attempt_table(TableId::DivisionLandInputs, |report| {
        let saved = land::read_projected_divisions(&save_document, &frame)?;
        land::compare_divisions(&frame, &saved, report)
    });
    report.attempt("LandCombatProgress casualties", |report| {
        let saved = land::read(save_document.items(), land::Comparison::Losses)?;
        land::compare_losses(&frame, &saved, report)
    });
    report.attempt("equipment category families", |report| {
        let families = |tables: engine_executable::executable_tables::Tables| {
            tables
                .auxiliary
                .as_ref()
                .map_err(ToString::to_string)
                .and_then(|auxiliary| auxiliary.category_families.as_ref().map_err(ToString::to_string))
        };
        let recorded = recorded_tables(&replay);
        let installed = engine_executable::executable_tables::current()
            .ok_or_else(|| "the installed executable's tables are not loaded".to_owned())
            .and_then(|installed| match &recorded {
                Ok(recorded) if recorded.identity().sha256 != installed.identity().sha256 => {
                    Err("the installed executable is not the recorded one".to_owned())
                }
                _ => families(installed),
            });
        production::compare_category_families(recorded.and_then(families), installed, report)
    });
    report.attempt_table(TableId::LandCombatResults, |report| {
        let saved = land::read(save_document.items(), land::Comparison::History)?;
        land::compare_events(&replay, &frame, &saved, report)
    });
    report.attempt_table(TableId::NavalResultSources, |report| {
        naval_result_sources::compare(&replay, &frame, report)
    });
    report.attempt_table(TableId::EventFeedHistory, |report| {
        event_feed::compare(&replay, &frame, &save_document, report)
    });
    report.attempt_table(TableId::MapEvents, |report| {
        map_events::compare(&replay, &frame, report)
    });
    report.attempt_table(TableId::AirLossEvents, |report| {
        air_ledger::compare_air_losses(&replay, &frame, &save.air, content_read(&regions), report)
    });
    report.attempt_table(TableId::BuildingBombingDamage, |report| {
        air_ledger::compare_bombing_damage(&frame, content_read(&buildings), report)
    });
    report.content_reads = Some(uncovered_reads);
    report.print(replay.format);
    println!("Oracle elapsed: {:.3} s", started.elapsed().as_secs_f64());
    report.outcome()
}

fn bind_raiding_save_fields(
    frame: &DecodedFrame,
    save_document: &save::SaveDocument,
    save: &mut Save,
) -> io::Result<()> {
    if !frame
        .complete::<tables::NavalMissionTaskForces>()?
        .iter()
        .any(|row| row.raiding_spotting.is_some() || row.raiding_region.is_some())
    {
        return Ok(());
    }
    let lexical = frame.complete::<tables::LexicalTokens>()?;
    let mut keys = [b"".as_slice(); 2];
    for (key, token) in keys.iter_mut().zip([
        engine_native_naval::naval_mission_inputs::RAID_SPOTTING_SAVE_TOKEN,
        engine_native_naval::naval_mission_inputs::RAID_REGION_SAVE_TOKEN,
    ]) {
        let mut rows = lexical.iter().filter(|row| row.token == token);
        *key = rows
            .next()
            .map(|row| row.literal.as_slice())
            .ok_or_else(|| invalid("retained raiding save token unavailable"))?;
        if rows.next().is_some() {
            return Err(invalid("retained raiding save token duplicated"));
        }
    }
    save.naval_missions.raiding_grid = Some(match country_emission_layout(frame) {
        Ok(Some(layout)) => save_document.project(&layout)?.mission_raiding_grid(keys)?,
        _ => save_document.mission_raiding_grid(keys)?,
    });
    Ok(())
}

fn matching_hour(saved: Hour, requested: Option<Hour>) -> io::Result<Hour> {
    if requested.is_some_and(|hour| hour != saved) {
        return Err(invalid(format!(
            "save hour {saved} differs from requested hour {}; pause and save at the captured hour",
            requested.unwrap()
        )));
    }
    Ok(saved)
}

fn same_version(saved: &[u8], recorded: &str) -> bool {
    saved == recorded.as_bytes()
        || saved
            .strip_prefix(recorded.as_bytes())
            .is_some_and(|suffix| {
                suffix.len() == 7
                    && suffix.starts_with(b" (")
                    && suffix.ends_with(b")")
                    && suffix[2..6].iter().all(u8::is_ascii_hexdigit)
            })
}

fn select_frame(
    replay: &Replay,
    hour: Hour,
    explicit: Option<BranchId>,
) -> io::Result<Box<DecodedFrame>> {
    if let Some(branch) = explicit {
        if !replay.branches.iter().any(|entry| entry.id == branch) {
            return Err(invalid(format!("unknown branch {branch}")));
        }
        return match replay.seek(branch, hour)? {
            SeekResult::Frame(frame) => Ok(frame),
            SeekResult::Gap { reason, .. } => Err(invalid(format!(
                "branch {branch}, hour {hour} is a gap: {reason}"
            ))),
        };
    }
    let candidates = replay
        .index
        .iter()
        .filter(|block| block.first_hour <= hour && hour <= block.last_hour)
        .map(|block| block.branch)
        .collect::<BTreeSet<_>>();
    let mut frames = BTreeMap::new();
    let mut gaps = Vec::new();
    for branch in candidates {
        match replay.seek(branch, hour)? {
            SeekResult::Frame(frame) => {
                frames.insert(branch, frame);
            }
            SeekResult::Gap { reason, .. } => gaps.push(format!("branch {branch}: {reason}")),
        }
    }
    let branch = unique_branch(&frames.keys().copied().collect::<Vec<_>>()).map_err(|error| {
        invalid(format!(
            "hour {hour}: {error}{}",
            if gaps.is_empty() {
                String::new()
            } else {
                format!("; {}", gaps.join("; "))
            }
        ))
    })?;
    Ok(frames.remove(&branch).unwrap())
}

fn unique_branch(branches: &[BranchId]) -> io::Result<BranchId> {
    match branches {
        [branch] => Ok(*branch),
        [] => Err(invalid(
            "no recorded frame; select --branch to report that branch's gap reason",
        )),
        _ => Err(invalid(format!(
            "recorded on branches {branches:?}; pass --branch explicitly"
        ))),
    }
}

fn validate_identity(frame: &DecodedFrame, save: &Save) -> io::Result<()> {
    matching_hour(save.date.engine_hour()?, Some(frame.hour))?;
    let [clock] = frame.complete::<tables::Clock>()? else {
        return Err(invalid("clock table must contain one clock"));
    };
    let [campaign] = frame.complete::<tables::Campaign>()? else {
        return Err(invalid("campaign table must contain one identity"));
    };
    if clock.hour != frame.hour || clock.seed != save.seed || campaign.id != save.campaign_id {
        return Err(invalid(
            "frame clock, seed or campaign identity differs from the save",
        ));
    }
    Ok(())
}

fn boundary_description(boundary: SampleBoundary) -> &'static str {
    match boundary {
        SampleBoundary::AfterHourlyUpdate => {
            "frames sample the state after hour H's hourly update and, where the command batch is observed, the commands its batch ran behind it"
        }
        SampleBoundary::EndOfHour => {
            "frames sample the state at the end of hour H, at the entry of the next hourly update before its clock increment"
        }
    }
}

fn boundary_limitation(boundary: SampleBoundary) -> &'static str {
    match boundary {
        SampleBoundary::AfterHourlyUpdate => {
            "frames are read after their hour's update and, where the command batch is observed, the commands its batch ran behind it, but the frame compared with a save is taken to be one a refresh settled, read as the next hourly update is entered: it follows every command of hour H, including any executed after the save while paused or after unpausing and before the next tick; matching hours do not prove that no command ran after the save. The boundary witness below measures that gap from the engine's own allocation counters; differences still fail either way"
        }
        SampleBoundary::EndOfHour => {
            "the end-of-hour sample follows every command of hour H, including any executed after the save while paused (the session drain does not check pause) or after unpausing and before the next tick; matching hours do not prove that no command ran after the save. The boundary witness below measures that gap from the engine's own allocation counters; differences still fail either way"
        }
    }
}

fn compare_core(
    frame: &DecodedFrame,
    boundary: SampleBoundary,
    save: &Save,
    membership: ContentRead<'_, save::PartialMapMembership>,
    equipment: ContentRead<'_, content_reader_key_names::equipment::Database>,
    read_hours: Option<BTreeMap<TableId, Hour>>,
) -> io::Result<Report> {
    validate_identity(frame, save)?;
    let mut report = Report {
        read_hours,
        ..Report::default()
    };
    report.record_tables(frame);
    report.record_country_emissions(save);
    report.refuse_uncaptured_reserved_operands();
    report.attempt("content map membership", |report| {
        let membership = required_context("known map membership", &membership)?;
        report.membership_content(membership);
        Ok(())
    });
    report.attempt("content equipment", |_| {
        required_context("verified equipment", &equipment).map(|_| ())
    });
    report.sample_boundary = Some(boundary);
    report.limitation(boundary_limitation(boundary), "all captured tables");
    report.attempt("boundary counters", |report| {
        compare_boundary_counters(frame, save, report)
    });
    report.limitation(
        "save session time includes elapsed wall time; it is not the captured persisted play_time",
        "clock.play_time",
    );
    report.limitation(
        "process-local load timestamp is absent from saves",
        "clock.play_session_started_at",
    );
    report.limitation(
        "saves retain country tags but erase raw country-link and canonical-slot identities",
        "countries / country links / province, state, division, war relation and faction member country references",
    );
    report.limitation("save serializer omits owner-equivalent and nonpositive controller references; these compare by resolved control", "provinces with no explicit saved controller");
    report.attempt("map country context", |report| {
        let context = country_context(frame)?;
        let by_tag = &context.by_tag;
        let links = &context.links;
        report.attempt("countries", |report| {
            compare_countries(save, &equipment, by_tag, report);
            Ok(())
        });
        report.attempt("provinces", |report| {
            compare_provinces(
                frame,
                save,
                required_context("known map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("states", |report| {
            compare_states(
                frame,
                save,
                required_context("known map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("divisions", |report| {
            compare_divisions(
                frame,
                save,
                required_context("known map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("war relations", |report| {
            compare_war_relations(frame, save, &context, report)
        });
        report.attempt("RelationOpinions", |report| {
            compare_relation_opinions(frame, save, &context, report)
        });
        report.attempt("CountryIntel", |report| {
            compare_country_intel(frame, save, &context, report)
        });
        report.attempt("factions", |report| {
            compare_factions(frame, save, &context, report)
        });
        report.attempt("FactionGoals", |report| {
            compare_faction_goals(frame, save, report)
        });
        report.attempt("FactionMinimumUpgrades", |report| {
            relations::faction_companions::minimum_upgrades(frame, save, report)
        });
        report.attempt("FactionDoctrine", |report| {
            relations::faction_companions::doctrine(frame, save, report)
        });
        report.attempt("FactionResearch", |report| {
            relations::faction_companions::research(frame, save, &context, report)
        });
        report.attempt("FactionProjection", |report| {
            relations::faction_pools::projection(frame, save, report)
        });
        report.attempt("FactionResources", |report| {
            relations::faction_pools::resources(frame, save, report)
        });
        report.attempt("FactionAdvisorModifiers", |report| {
            relations::faction_advisors::compare(frame, save, &context, report)
        });
        report.attempt("FactionAccounts", |report| {
            relations::faction_accounts::compare(frame, save, &context, report)
        });
        report.attempt("FactionVariables", |report| {
            relations::faction_companions::variables(frame, save, report)
        });
        report.attempt("FactionRules", |report| {
            relations::faction_companions::rules(frame, save, report)
        });
        report.attempt("FactionPrograms", |report| {
            relations::faction_companions::programmes(frame, save, report)
        });
        report.attempt("FactionUnlocks", |report| {
            relations::faction_companions::unlocks(frame, save, report)
        });
        report.attempt("FactionManpower", |report| {
            relations::faction_companions::manpower(frame, save, &context, report)
        });
        report.attempt("FactionTheatres", |report| {
            faction_theatres::compare(frame, save, &context, report)
        });
        report.attempt("players", |report| {
            compare_players(frame, save, links, report)
        });
        report.attempt("naval units", |report| {
            compare_naval_units(
                frame,
                save,
                required_context("known map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("naval combats", |report| {
            compare_naval_combats(
                frame,
                save,
                required_context("known map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("naval combat sources", |report| {
            compare_naval_combat_sources(frame, save, report)
        });
        report.attempt("air wings", |report| {
            compare_air_wings(frame, save, links, report)
        });
        report.attempt("army orders", |report| {
            compare_army_orders(frame, save, links, report)
        });
        Ok(())
    });
    report.attempt("land combats", |report| {
        compare_land_combats(
            frame,
            save,
            required_context("known map membership", &membership)?,
            report,
        )
    });
    report.attempt_table(TableId::BattleSounds, |report| {
        compare_battle_sounds(
            frame,
            required_context("known map membership", &membership)?,
            report,
        )
    });
    report.attempt_table(TableId::ProvinceNames, |report| {
        compare_province_names(frame, save, report)
    });
    Ok(report)
}

fn port_types(buildings: &content_reader_key_names::buildings::Buildings) -> BTreeSet<Vec<u8>> {
    buildings
        .types
        .values()
        .filter(|building| building.is_port)
        .map(|building| building.name.clone().into_bytes())
        .collect()
}

struct ExtendedContent<'a> {
    membership: ContentRead<'a, save::PartialMapMembership>,
    base_resources: ContentRead<'a, save::construction::PartialBaseResources>,
    railway_adjacency: ContentRead<'a, save::naval_history::RailwayAdjacency>,
    buildings: ContentRead<'a, content_reader_key_names::buildings::Buildings>,
    defines: ContentRead<'a, content_reader_defines::defines::Defines>,
    raid_graph: ContentRead<'a, projects::RaidGraph>,
    equipment: ContentRead<'a, content_reader_key_names::equipment::Database>,
    essential_caps: ContentRead<'a, BTreeMap<Vec<u8>, i32>>,
    leader_traits: ContentRead<'a, content_reader_military::leader_traits::LeaderTraits>,
    sub_units: ContentRead<'a, content_reader_script::sub_units::SubUnits>,
    intelligence: ContentRead<'a, content_reader_military::intelligence::AgencyUpgrades>,
    temperature: ContentRead<'a, replay_data::temperature::Content>,
    province_modifiers: ContentRead<'a, weather::ProvinceModifierContent>,
}

fn compare_extended(
    frame: &DecodedFrame,
    save: &Save,
    content: ExtendedContent<'_>,
    report: &mut Report,
) {
    report.record_country_emissions(save);
    report.refuse_uncaptured_reserved_operands();
    let ExtendedContent {
        membership,
        base_resources,
        railway_adjacency,
        buildings,
        defines,
        raid_graph,
        equipment,
        essential_caps,
        leader_traits,
        sub_units,
        intelligence,
        temperature,
        province_modifiers,
    } = content;
    let port_types = buildings.clone().map(port_types);
    report.attempt("reserved country intel sample", |report| {
        relations::compare_reserved_country_intel(frame, save, report)
    });
    report.attempt("reserved reverse visibility", |report| {
        relations::compare_reserved_reverse_visibility(frame, report)
    });
    plans::compare(frame, save, report);
    map_inputs::compare(frame, save, report);
    counters::compare(frame, save, report);
    railway_guns::compare(frame, save, report);
    unit_combat_references::compare(frame, report);
    selected_routes::compare(
        frame,
        save,
        raid_graph.clone().map(|(_, geometry)| geometry),
        report,
    );
    pass_inputs::compare(frame, save, report);
    supply_trains::compare(frame, save, &membership, report);
    combat_details::compare(frame, save, report);
    report.attempt_table(TableId::PowerBalanceModifierBreakdowns, |r| {
        modifier_breakdowns::balance(frame, &save.governance, r)
    });
    report.attempt_table(TableId::StateFlags, |report| {
        compare_state_flags(frame, save, report)
    });
    report.attempt_table(TableId::DivisionMovement, |report| {
        compare_division_movement(frame, save, report)
    });
    let countries = country_context(frame);
    let tokens = lexical_tokens(frame, report);
    report.attempt("AirWings transport inputs", |report| {
        military::compare_air_transport_inputs(frame, save, &sub_units, &tokens, report)
    });
    report.attempt_table(TableId::CountryGameScoreInputs, |report| {
        game_score::rosters(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("CountryGameScoreInputs.land_count", |report| {
        game_score::land(
            frame,
            save,
            required_context("known map membership", &membership)?,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::AirWingGameScoreStats, |report| {
        game_score::wings(frame, save, report)
    });
    report.attempt_table(TableId::GameScoreMultipliers, |report| {
        game_score::multipliers(frame, report)
    });
    report.attempt("war overview operands", |report| {
        wars::compare(
            frame,
            save,
            required_context("country links", &countries)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("war-goal inventories", |report| {
        recorder_oracle_wars::wargoals::compare(
            frame,
            save,
            required_context("country links", &countries)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("politics", |report| {
        politics::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        );
        Ok(())
    });
    let camo_show_at = defines.clone().and_then(|defines| {
        defines
            .fixed5("NGraphics", "ACCLIMATIZATION_CAMO_SHOW_AT")
            .map_err(|error| error.to_string())
    });
    let supply_grace = defines.clone().and_then(|defines| {
        defines
            .int32("NMilitary", "SUPPLY_GRACE")
            .map_err(|error| error.to_string())
    });
    report.attempt_table(TableId::DivisionPanelStats, |report| {
        compare_division_panel_stats(frame, save, report)
    });
    report.attempt_table(TableId::DivisionStatus, |report| {
        compare_division_status(
            frame,
            save,
            required_context("lexical tokens", &tokens)?,
            &camo_show_at,
            &supply_grace,
            report,
        )
    });
    military::compare_movement_operands(frame, report);
    report.attempt("movement States", |report| {
        core::compare_railway_infrastructure(
            frame,
            save,
            required_context("verified building templates", &buildings)?,
            required_context("verified movement defines", &defines)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("movement RailCooldowns", |report| {
        naval_history::compare_railway_buildings(
            frame,
            &save.construction,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("air missions", |report| {
        air_missions::compare(
            frame,
            save,
            required_context("country links", &countries)?,
            &defines.clone().and_then(|defines| {
                defines
                    .fixed5_array("NNavy", "AIR_MISSION_SPOTTING_FACTORS")
                    .map_err(|error| error.to_string())
            }),
            report,
        );
        Ok(())
    });
    report.attempt_table(TableId::CountryTemplateState, |report| {
        compare_country_template_state(
            frame,
            save,
            required_context("country links", &countries)?,
            report,
        )
    });
    report.attempt_table(TableId::CountryUnitBonuses, |report| {
        military::compare_country_unit_bonuses(
            frame,
            save,
            required_context("country links", &countries)?,
            required_context("lexical tokens", &tokens)?,
            report,
        )
    });
    report.attempt_table(TableId::DivisionTemplates, |report| {
        compare_division_templates(
            frame,
            save,
            required_context("country links", &countries)?,
            report,
        )
    });
    report.attempt_table(TableId::DivisionTemplateIcons, |report| {
        compare_division_template_icons(
            frame,
            save,
            required_context("country links", &countries)?,
            required_context("lexical tokens", &tokens)?,
            report,
        )
    });
    report.attempt("content base resources", |_| {
        required_context("verified base resources", &base_resources).map(|_| ())
    });
    report.attempt("content railway adjacency", |_| {
        required_context("verified railway adjacency", &railway_adjacency).map(|_| ())
    });
    report.attempt("governance", |report| {
        governance::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("production", |report| {
        production::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("production VariantDefinitions.archetype_token", |report| {
        production::compare_variant_archetypes(
            frame,
            required_context("verified equipment", &equipment)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("economy", |report| {
        economy::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("politics ledger", |report| {
        power_ledger::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        );
        Ok(())
    });
    report.attempt_table(TableId::CombatLogEquipment, |report| {
        combat_log::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::CombatLogGroups, |report| {
        recorder_oracle_combat_log::retained::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::CombatLogObservers, |report| {
        recorder_oracle_combat_log::observers::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::CombatLogState, |report| {
        combat_log::state(frame, report)
    });
    report.attempt_table(TableId::DivisionCensus, |report| {
        combat_log::division_census(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::LendLeaseEquipment, |report| {
        combat_log::lend_lease(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("CountryFactories", |report| {
        factories::compare(
            frame,
            save,
            required_context("known map membership", &membership)?,
            required_context("verified building types", &buildings)?,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("construction StateBuildings", |report| {
        construction::buildings(
            complete::<tables::StateBuildings>(frame)?,
            &save.construction.state_buildings,
            false,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction ProvinceBuildings", |report| {
        construction::buildings(
            complete::<tables::ProvinceBuildings>(frame)?,
            &save.construction.province_buildings,
            true,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction NavalBases", |report| {
        construction::naval_bases(
            complete::<tables::NavalBases>(frame)?,
            &save.construction.province_buildings,
            required_context("verified port building types", &port_types)?,
            &save.country_naval_launch,
            &complete::<tables::Countries>(frame)?
                .iter()
                .map(|row| (row.id, row.tag.as_slice()))
                .chain(
                    complete::<tables::ReservedCountryNaming>(frame)
                        .unwrap_or(&[])
                        .iter()
                        .map(|row| (0, row.tag.as_slice())),
                )
                .collect(),
            report,
        )
    });
    report.attempt("construction StateBaseResources", |report| {
        construction::partial_base_resources(
            complete::<tables::StateBaseResources>(frame)?,
            required_context("verified base resources", &base_resources)?,
            required_context("verified map membership", &membership)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction StateResources", |report| {
        construction::state_pools(
            complete::<tables::StateResources>(frame)?,
            &save.construction.state_resources,
            required_context("lexical dictionary", &tokens)?,
            &sampled_states(frame)?,
            report,
        )
    });
    report.attempt("construction StateDynamicModifiers", |report| {
        construction::state_modifiers(
            complete::<tables::StateDynamicModifiers>(frame)?,
            &save.construction.state_dynamic_modifiers,
            &required_context("country links", &countries)?.links,
            &sampled_states(frame)?,
            report,
        )
    });
    report.attempt("construction StateCores", |report| {
        construction::state_cores(
            complete::<tables::StateCores>(frame)?,
            &save.countries,
            &required_context("country links", &countries)?.links,
            complete::<tables::SliceReads>(frame)?,
            frame.hour,
            report,
        )
    });
    report.attempt("construction StateCores strategic locations", |report| {
        let state_of = complete::<tables::Provinces>(frame)?
            .iter()
            .map(|province| (province.id, province.state))
            .collect();
        construction::state_strategic_locations(
            complete::<tables::StateCores>(frame)?,
            &save.states,
            &save.provinces,
            &state_of,
            required_context("lexical dictionary", &tokens)?,
            complete::<tables::SliceReads>(frame)?,
            frame.hour,
            report,
        )
    });
    report.attempt("construction StateResourceInputs", |report| {
        construction::resource_inputs(
            complete::<tables::StateResourceInputs>(frame)?,
            complete::<tables::States>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            &sampled_states(frame)?,
            report,
        )
    });
    report.attempt("construction CountryBuildingResources", |report| {
        construction::building_overrides(
            complete::<tables::CountryBuildingResources>(frame)?,
            complete::<tables::SliceReads>(frame)?,
            complete::<tables::ResourceDefinitions>(frame)?,
            complete::<tables::States>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            frame.hour,
            report,
        )
    });
    report.attempt("construction resources map", |report| {
        let threshold = defines
            .clone()
            .and_then(|defines| {
                defines
                    .fixed5("NGraphics", "TRADE_ROUTE_BAD_EFFICIENCY_THRESHOLD")
                    .map_err(|error| error.to_string())
            })
            .map_err(|error| invalid(format!("requires the bad-efficiency threshold: {error}")))?;
        construction::resource_map(
            &construction::ResourceMapRows {
                origins: complete::<tables::ResourceOrigins>(frame)?,
                rights: complete::<tables::ResourceRights>(frame)?,
                clients: complete::<tables::ConvoyClients>(frame)?,
                routes: complete::<tables::TradeRoutes>(frame)?,
                regional: complete::<tables::RegionalConvoyEfficiency>(frame)?,
                reads: complete::<tables::SliceReads>(frame)?,
                resources: complete::<tables::ResourceDefinitions>(frame)?,
            },
            &save.construction,
            &save.economy,
            &save.countries,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            threshold,
            frame.hour,
            report,
        )
    });
    report.attempt("construction BuildingConstruction", |report| {
        construction::building_construction(
            complete::<tables::BuildingConstruction>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction RailwayConstruction", |report| {
        construction::railway_construction(
            complete::<tables::RailwayConstruction>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    let occupation_tags = complete::<tables::Countries>(frame).map(|countries| {
        let mut tags = countries
            .iter()
            .map(|country| (country.id, country.tag.as_slice()))
            .collect::<BTreeMap<_, _>>();
        if let Ok([reserved]) = complete::<tables::ReservedCountryNaming>(frame) {
            tags.insert(0, reserved.tag.as_slice());
        }
        tags
    });
    let occupation_settings = complete::<tables::OccupationSettings>(frame).ok();
    report.attempt("construction OccupationSettings", |report| {
        construction::occupation_settings(
            complete::<tables::OccupationSettings>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            required_context("raw occupation country tags", &occupation_tags)?,
            report,
        )
    });
    report.attempt("construction StateOccupationLaws", |report| {
        construction::occupation_laws(
            complete::<tables::StateOccupationLaws>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            occupation_settings,
            required_context("raw occupation country tags", &occupation_tags)?,
            report,
        )
    });
    report.attempt("construction StateGarrisons", |report| {
        construction::garrisons(
            complete::<tables::StateGarrisons>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            occupation_settings,
            required_context("raw occupation country tags", &occupation_tags)?,
            report,
        )
    });
    report.attempt("construction StateAssignedManpower", |report| {
        construction::assigned_manpower(
            complete::<tables::StateAssignedManpower>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            occupation_settings,
            required_context("raw occupation country tags", &occupation_tags)?,
            report,
        )
    });
    report.attempt("construction equipment StateGarrisons", |report| {
        construction::garrison_equipment(
            frame,
            &save.construction,
            &required_context("country links", &countries)?.links,
            required_context("lexical tokens", &tokens)?,
            report,
        )
    });
    report.attempt("construction CountryOccupationReserveManpower", |report| {
        construction::reserve_manpower(
            complete::<tables::CountryOccupationReserveManpower>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("construction StateResistance", |report| {
        construction::resistance(
            complete::<tables::StateResistance>(frame)?,
            &save.construction,
            required_context("raw occupation country tags", &occupation_tags)?,
            report,
        )
    });
    report.attempt("construction map StateResistance", |report| {
        construction::resistance_map(
            complete::<tables::StateResistance>(frame)?,
            &save.construction,
            &save.states,
            complete::<tables::States>(frame)?,
            &required_context("country links", &countries)?
                .links
                .iter()
                .map(|(&id, country)| (id, country.tag.as_slice()))
                .collect(),
            required_context("raw occupation country tags", &occupation_tags)?,
            report,
        )
    });
    report.attempt("construction StateResistance hover", |report| {
        construction::resistance_hover(
            complete::<tables::StateResistance>(frame)?,
            &save.construction,
            &save.intel,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("construction CountryCollaboration", |report| {
        construction::collaboration(
            complete::<tables::CountryCollaboration>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("characters", |report| {
        characters::compare(
            frame,
            &save.characters,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            leader_traits.as_deref().map_err(String::as_str),
            report,
        )
    });
    report.attempt("operative codenames", |report| {
        characters::compare_codenames(frame, &save.intel, report)
    });
    report.attempt("RelationTuples", |report| {
        compare_relation_tuples(
            frame,
            save,
            required_context("country links", &countries)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("intel", |report| {
        intel::compare(
            frame,
            &save.intel,
            &save.country_rosters,
            save.characters.manager.as_ref(),
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            intelligence.as_deref().map_err(String::as_str),
            intel::ScopeKinds {
                global: 0,
                country: 1,
                state: 2,
                character: 3,
            },
            report,
        )
    });
    report.attempt("AgencyOperationBuffers", |report| {
        intel::compare_operation_buffers(
            frame,
            &save.intel,
            &save.governance,
            &save.states.keys().copied().collect(),
            membership.as_ref().copied().map_err(String::as_str),
            report,
        )
    });
    report.attempt("IntelSourcePools", |report| {
        intel::compare_sources(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("air", |report| {
        air::compare(
            frame,
            &save.air,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::AirRegionCombat, |report| {
        let timeout = required_context("verified air defines", &defines)?
            .int32("NGame", "AIR_LOG_TIMEOUT_HOURS")?;
        air::compare_region_combat(
            frame,
            &save.air,
            &required_context("country links", &countries)?.links,
            timeout,
            report,
        )
    });
    report.attempt("naval_history", |report| {
        naval_history::compare(
            frame,
            &save.naval_history,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("naval management", |report| {
        recorder_oracle_naval_history::naval_management::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("army order of battle", |report| {
        naval_history::compare_order_of_battle(
            frame,
            &save.divisions,
            &save.naval_history,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("RailEdges", |report| {
        naval_history::compare_rail_edges(
            frame,
            &save.naval_history,
            required_context("verified railway adjacency", &railway_adjacency)?,
            report,
        )
    });
    report.attempt("projects", |report| {
        projects::compare(
            frame,
            &save.projects,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt_table(TableId::RaidPathDistances, |report| {
        projects::compare_raid_paths(
            frame,
            required_context("verified province graph", &raid_graph)?,
            report,
        )
    });
    report.attempt("projects RaidTooltipOperands", |report| {
        projects::compare_raid_tooltip_operands(
            frame,
            &save.projects,
            &save.divisions,
            &save.air,
            report,
        )
    });
    report.attempt_table(TableId::GlobalSavedEventTargets, |report| {
        saved_event_targets::compare(
            frame,
            &save.saved_event_targets,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt_table(TableId::EquipmentDestruction, |report| {
        equipment_destruction::compare(frame, report)
    });
    report.attempt("session", |report| {
        session::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("projects Scientists", |report| {
        projects::compare_scientists(
            frame,
            &save.characters,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("modifiers", |report| {
        modifiers::compare(
            frame,
            &save.characters,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens),
            report,
        )
    });
    report.attempt("stability", |report| {
        stability::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("weather", |report| {
        weather::compare(frame, save, temperature, province_modifiers, report)
    });
    report.attempt("fog", |report| {
        fog::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("Air volunteer stockpile", |report| {
        let database = required_context("verified equipment", &equipment)?;
        let names = required_context("lexical dictionary", &tokens)?;
        visibility::compare_volunteer_stockpile(
            frame,
            save,
            required_context("country links", &countries)?,
            &|token| {
                let name = names
                    .get(&token)
                    .ok_or_else(|| invalid("volunteer archetype token is missing"))?;
                let mut current = database.get(name).ok_or_else(|| {
                    invalid("volunteer archetype is absent from recorded content")
                })?;
                let mut seen = BTreeSet::new();
                loop {
                    if !seen.insert(current) {
                        return Err(invalid("volunteer archetype content cycle"));
                    }
                    let definition = database
                        .equipment
                        .get(current)
                        .ok_or_else(|| invalid("volunteer archetype content index invalid"))?;
                    if database.category(definition) & engine_native_parts::cequipment_type::PLANE_MASK
                        != 0
                    {
                        return Ok(true);
                    }
                    match definition.archetype {
                        Some(next) if next != current => current = next,
                        _ => return Ok(false),
                    }
                }
            },
            report,
        )
    });
    report.attempt("visibility", |report| {
        visibility::compare(
            frame,
            save,
            required_context("country links", &countries)?,
            required_context("lexical dictionary", &tokens)?,
            essential_caps.clone(),
            report,
        )
    });
}

#[cfg(test)]
#[path = "oracle/report_tests.rs"]
mod reporting_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[test]
fn validation_refusal_is_incomplete_even_when_publication_was_deferred() {
    let (mut frame, _, _, _) = fixtures::fixture();
    frame.tables[TableId::OperationCompletions.index()] = TableView::Unavailable {
        reason: "spool validation failed: OperationCompletions: row 7 column country: missing or not recorded row field".into(),
    }.into();
    let mut report = Report::default();
    report.record_tables(&frame);
    report
        .deferred_tables
        .insert(TableId::OperationCompletions, "publication pending".into());
    assert_eq!(report.unavailable_tables(), 1);
    assert!(
        report
            .outcome()
            .unwrap_err()
            .to_string()
            .contains("comparison incomplete")
    );
}

fn recorded_tables(replay: &Replay) -> Result<engine_executable::executable_tables::Tables, String> {
    let at = replay
        .fact_packs
        .len()
        .checked_sub(1)
        .ok_or_else(|| "the recording carries no game facts".to_owned())?;
    let bytes = replay.facts(at).map_err(|error| error.to_string())?;
    let length = bytes.len() as u64;
    content_reader_layers::content::facts::Pack::open(std::sync::Arc::new(bytes), 0, length)
        .map_err(|error| error.to_string())?
        .tables()
}
