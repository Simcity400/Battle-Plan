//! Exact-hour comparison against an independently parsed text save.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs, io,
    path::PathBuf,
    time::Instant,
};

use installed_content::{
    ResolveOptions,
    save::{self, Decimal, Save},
};
use replay::{
    BranchId, Hour,
    container::{Replay, SeekResult},
    frame::{DecodedFrame, TableView},
    identity::{SampleBoundary, Verification},
    rows::{CountryRow, TableId, TableRows, TableSchema, tables},
};

mod air;
mod air_ledger;
mod air_missions;
mod characters;
mod combat_details;
mod combat_log;
mod construction;
mod core;
mod counters;
mod country_emissions;
mod country_rosters;
mod economy;
mod equipment_destruction;
mod event_feed;
mod faction_theatres;
mod factories;
mod fog;
#[cfg(test)]
mod fresh_movement_tests;
mod fuel_demand;
mod game_score;
mod governance;
mod intel;
mod land;
mod map_inputs;
mod military;
mod modifier_breakdowns;
mod modifiers;
mod naval_history;
mod naval_missions;
mod naval_result_sources;
mod pass_inputs;
mod peace;
mod plans;
mod politics;
mod power_ledger;
mod production;
mod projects;
mod railway_guns;
mod relations;
mod report;
mod rocket_output;
mod saved_event_targets;
mod selected_routes;
mod session;
mod stability;
#[cfg(test)]
mod state_history_tests;
mod supply_trains;
mod unit_combat_references;
mod visibility;
mod wars;
mod weather;

use self::relations::{
    compare_country_intel, compare_factions, compare_relation_opinions, compare_relation_tuples,
    compare_war_relations, saved_country_slot,
};

use self::military::{
    compare_air_wings, compare_army_orders, compare_division_movement, compare_division_status,
    compare_division_template_icons, compare_division_templates, compare_divisions,
    compare_land_combats, compare_naval_combat_sources, compare_naval_combats, compare_naval_units,
};

#[cfg(test)]
use self::core::{CONVOY_LIMIT, convoy_add, convoy_variant};

use self::core::{
    compare_boundary_counters, compare_countries, compare_players, compare_province_names,
    compare_provinces, compare_state_flags, compare_states,
};

#[cfg(test)]
use self::report::recorded_table_counts;

pub(crate) use self::report::daily_read;

use self::report::{Report, last_read_hours};

type TokenNames<'a> = BTreeMap<u32, &'a [u8]>;
type Spellings<'a> = BTreeMap<installed_content::tokens::Token, Option<&'a [u8]>>;
type ContentRead<'a, T> = Result<&'a T, String>;

/// Text writers scan the first-NUL prefix before escaping ordinary text.
/// Stored length does not determine the emitted prefix.
/// Raw strings use the same first-NUL prefix in archive
/// and memory streams.
fn serialized_text<'a>(report: &mut Report, field: &str, bytes: &'a [u8]) -> &'a [u8] {
    let text = installed_content::parser::c_string(bytes);
    if text.len() < bytes.len() {
        report.limitation("native text serialization ends at the first NUL; captured trailing bytes are not save-comparable", field);
    }
    text
}

/// Token insertion and lookup reuse ASCII-case aliases
/// without changing the first spelling. Token text lookup returns
/// the recorded reverse bytes, which text writers emit.
fn reverse_spellings<'a>(tokens: &TokenNames<'a>) -> Spellings<'a> {
    let mut spellings = BTreeMap::new();
    for literal in tokens.values() {
        let literal = installed_content::parser::c_string(literal);
        spellings
            .entry(installed_content::tokens::Token::c_str(literal))
            .and_modify(|spelling| {
                // Reserved reverse names can alias a later dynamic token.
                // Equal writer bytes are certain; a disagreement stays so.
                if *spelling != Some(literal) {
                    *spelling = None;
                }
            })
            .or_insert(Some(literal));
    }
    spellings
}

fn token_spelling<'a>(
    tokens: &TokenNames<'a>,
    spellings: &Spellings<'a>,
    name: &[u8],
    subject: &str,
) -> io::Result<&'a [u8]> {
    let identity = installed_content::tokens::Token::c_str(name);
    let spelling = if identity.is_rejected() {
        tokens.get(&0).copied().map(installed_content::parser::c_string)
    } else {
        spellings.get(&identity).copied().ok_or_else(|| {
            invalid(format!(
                "{subject} {:?} is absent from the recorded lexical dictionary",
                String::from_utf8_lossy(name),
            ))
        })?
    };
    spelling.ok_or_else(|| {
        invalid(format!(
            "{subject} {:?} has missing or ambiguous recorded lexical spelling",
            String::from_utf8_lossy(name),
        ))
    })
}

fn content_read<T>(result: &io::Result<T>) -> ContentRead<'_, T> {
    result.as_ref().map_err(ToString::to_string)
}

pub const USAGE: &str = "recorder oracle <campaign.battleplan> <save.hoi4> [--hour n] [--branch id] [--game-dir <directory>] [--user-dir <directory>]";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

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
    for reason in crate::replay_io::compatibility_notes(&replay) {
        println!("Compatibility gap: {reason}");
    }
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
    let save = country_emissions::parse(&save_document, &frame)?;
    validate_identity(&frame, &save)?;
    let options = ResolveOptions {
        game_dir: game_dir.map(Ok).unwrap_or_else(default_game_directory)?,
        user_dir: user_dir
            .map(Ok)
            .unwrap_or_else(|| crate::recording::resolve_user_directory().map_err(invalid))?,
        cache_dir: PathBuf::from(
            std::env::var_os("LOCALAPPDATA").ok_or_else(|| invalid("LOCALAPPDATA is not set"))?,
        )
        .join("HOI4Replay")
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
    let content = installed_content::resolve(&session.identity, &options)?;
    engine::executable_tables::initialize(&options.game_dir)?;
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
    let buildings = installed_content::buildings::read(&content.layers);
    let defines = installed_content::defines::read(&content.layers);
    let raid_graph = installed_content::province_graph::ProvinceGraph::read(&content.layers);
    let equipment = installed_content::equipment::Database::load_complete(&content.layers);
    let essential_caps = installed_content::raids::read(&content.layers)
        .map_err(|error| error.to_string())
        .and_then(|raids| raids.essential_caps(equipment.as_ref().map_err(ToString::to_string)?));
    let leader_traits = installed_content::leader_traits::read(&content.layers).map(|(traits, _)| traits);
    let sub_units = installed_content::unit_leader::sub_units(&content.layers);
    let regions = installed_content::regions::read(&content.layers);
    let intelligence = installed_content::intelligence::read_agency_upgrades(&content.layers);
    let temperature = raid_graph
        .as_ref()
        .map_err(|error| io::Error::other(error.to_string()))
        .and_then(|(graph, geometry)| {
            installed_content::weather::temperature::read_with(&content.layers, graph, geometry)
        });
    let read_hours = last_read_hours(&replay, frame.branch, hour)?;
    let mut report = compare_core_in(
        replay.format,
        &frame,
        session.sample_boundary,
        &save,
        membership.clone(),
        content_read(&equipment),
        Some(read_hours),
    )?;
    report.set_load_hour(branch);
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
        },
        &mut report,
    );
    // The per-line output is checked along the branch, not against the save.
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
    // So is each construction line's output, against the recorded queues.
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
    report.attempt_table(TableId::LandCombatResults, |report| {
        let saved = land::read(save_document.items(), land::Comparison::History)?;
        land::compare_events(&replay, &frame, &saved, report)
    });
    report.attempt_table(TableId::NavalResultSources, |report| {
        naval_result_sources::compare(&replay, &frame, report)
    });
    report.attempt_table(TableId::EventFeedHistory, |report| {
        event_feed::compare(&replay, &frame, report)
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

fn complete<T: TableSchema>(frame: &DecodedFrame) -> io::Result<&[T::Row]> {
    frame.complete::<T>()
}

/// Whether a state's sliced raid-state rows were read at the frame's hour,
/// from the family's hour source, StateResourceInputs (one row per state),
/// in SliceReads.
fn sampled_states(frame: &DecodedFrame) -> io::Result<impl Fn(u32) -> bool + '_> {
    let reads = complete::<tables::SliceReads>(frame)?;
    let hour = frame.hour;
    Ok(move |state| {
        replay::sliced::read_hour(reads, TableId::StateResourceInputs, state) == Some(hour)
    })
}

/// The raw owners whose rows of the owner-sliced `table` this frame holds as
/// read at its own hour, or None when every owner's are: a full hour, or a
/// recording from before the table was sliced, whose SliceReads dates none
/// of its owners because every frame read it whole. A sliced read records
/// every owner it covered, with rows or without (`Campaign::merge_sliced_inputs`).
fn current_slice_owners(
    frame: &DecodedFrame,
    table: TableId,
    report: &Report,
) -> io::Result<Option<BTreeSet<u32>>> {
    if report.full_refresh() {
        return Ok(None);
    }
    let id = table.index() as u32;
    let mut dated = false;
    let mut current = BTreeSet::new();
    for read in complete::<tables::SliceReads>(frame)?
        .iter()
        .filter(|read| read.table == id)
    {
        dated = true;
        if read.sample_hour == frame.hour {
            current.insert(read.owner);
        }
    }
    Ok(dated.then_some(current))
}

/// States that an owner-sliced comparison covered only the owners read at
/// the oracle hour, with the oldest earlier read among the others.
fn slice_owner_limitation(
    frame: &DecodedFrame,
    table: TableId,
    current: &Option<BTreeSet<u32>>,
    report: &mut Report,
) {
    let Some(current) = current else {
        return;
    };
    let oldest = complete::<tables::SliceReads>(frame)
        .ok()
        .and_then(|reads| {
            reads
                .iter()
                .filter(|row| row.table == table.index() as u32 && row.sample_hour != frame.hour)
                .map(|row| row.sample_hour)
                .min()
        });
    if oldest.is_some() {
        report.carried_owners(table, oldest);
    }
    report.limitation(
        "panel-only tables are read once a game day, one owner slice per hour (owner decision 2026-10-11); only the owners read at the oracle hour are compared with the save, and the others hold earlier reads (their hours are in SliceReads)",
        format!("{table:?}: {} owners read at the oracle hour", current.len()),
    );
}

fn complete_table(frame: &DecodedFrame, table: TableId) -> io::Result<&TableRows> {
    match &*frame.tables[table.index()] {
        TableView::Complete(rows) if rows.table() == table => Ok(rows),
        TableView::Complete(_) => Err(invalid(format!("wrong table rows in {table:?}"))),
        TableView::Truncated { missing, .. } => Err(invalid(format!(
            "hour {} {table:?} is truncated ({missing} missing rows)",
            frame.hour
        ))),
        TableView::Unavailable { reason } => Err(invalid(format!(
            "hour {} {table:?} is unavailable: {reason}",
            frame.hour
        ))),
    }
}

fn lexical_tokens<'a>(frame: &'a DecodedFrame, report: &mut Report) -> io::Result<TokenNames<'a>> {
    if let TableView::Unavailable { reason } = &*frame.tables[TableId::LexicalTokens.index()] {
        report.limitation(
            "recorded lexical dictionary is unavailable; text-to-token comparisons name their missing coverage and do not infer token identities from game content",
            reason.clone(),
        );
        return Ok(BTreeMap::new());
    }
    let mut tokens = BTreeMap::new();
    for row in complete::<tables::LexicalTokens>(frame)? {
        if tokens.insert(row.token, row.literal.as_slice()).is_some() {
            return Err(invalid("duplicate lexical token id"));
        }
    }
    Ok(tokens)
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

struct CountryContext<'a> {
    by_tag: BTreeMap<&'a [u8], &'a CountryRow>,
    links: BTreeMap<u32, &'a CountryRow>,
}

fn country_context(frame: &DecodedFrame) -> io::Result<CountryContext<'_>> {
    let mut by_tag = BTreeMap::new();
    let mut by_slot = BTreeMap::new();
    for country in frame.complete::<tables::Countries>()? {
        if by_tag.insert(country.tag.as_slice(), country).is_some()
            || by_slot.insert(country.canonical_slot, country).is_some()
        {
            return Err(invalid("ambiguous country tag or canonical slot"));
        }
    }
    let mut links = BTreeMap::new();
    for link in frame.complete::<tables::CountryLinks>()? {
        let country = by_slot
            .get(&link.canonical_slot)
            .ok_or_else(|| invalid(format!("country link {} has no canonical country", link.id)))?;
        if links.insert(link.id, *country).is_some() {
            return Err(invalid("duplicate country link"));
        }
    }
    for (&tag, captured) in &by_tag {
        if links
            .get(&captured.id)
            .map(|country| country.canonical_slot)
            != Some(captured.canonical_slot)
        {
            return Err(invalid(format!(
                "country {} has an inconsistent identity link",
                String::from_utf8_lossy(tag)
            )));
        }
    }
    Ok(CountryContext { by_tag, links })
}

/// What the frame labelled with the save's hour sampled. Hour H is compared
/// with save hour H in both cases; only the printed limitation differs.
fn boundary_description(boundary: SampleBoundary) -> &'static str {
    match boundary {
        SampleBoundary::AfterHourlyUpdate => {
            "frames sample the state just after hour H's hourly update returned"
        }
        SampleBoundary::EndOfHour => {
            "frames sample the state at the end of hour H, at the entry of the next hourly update before its clock increment"
        }
    }
}

fn boundary_limitation(boundary: SampleBoundary) -> &'static str {
    match boundary {
        SampleBoundary::AfterHourlyUpdate => {
            "commands after the hourly callback can change state before saving without advancing the hour; matching hours do not prove the same command boundary. The boundary witness below measures that gap from the engine's own allocation counters; differences still fail either way"
        }
        SampleBoundary::EndOfHour => {
            "the end-of-hour sample follows every command of hour H, including any executed after the save while paused (the session drain does not check pause) or after unpausing and before the next tick; matching hours do not prove that no command ran after the save. The boundary witness below measures that gap from the engine's own allocation counters; differences still fail either way"
        }
    }
}

/// [`compare_core_in`] for a recording in the current format.
#[cfg(test)]
fn compare_core(
    frame: &DecodedFrame,
    boundary: SampleBoundary,
    save: &Save,
    membership: ContentRead<'_, save::PartialMapMembership>,
    equipment: ContentRead<'_, installed_content::equipment::Database>,
    read_hours: Option<BTreeMap<TableId, Hour>>,
) -> io::Result<Report> {
    compare_core_in(
        replay::format::FormatVersion::CURRENT,
        frame,
        boundary,
        save,
        membership,
        equipment,
        read_hours,
    )
}

/// `format` is the recording's: from [`replay::display::DISPLAY_PRECISION_FORMAT`]
/// its stepped columns hold display precision, and their comparisons round
/// the save's exact values the same way.
fn compare_core_in(
    format: replay::format::FormatVersion,
    frame: &DecodedFrame,
    boundary: SampleBoundary,
    save: &Save,
    membership: ContentRead<'_, save::PartialMapMembership>,
    equipment: ContentRead<'_, installed_content::equipment::Database>,
    read_hours: Option<BTreeMap<TableId, Hour>>,
) -> io::Result<Report> {
    validate_identity(frame, save)?;
    let mut report = Report {
        read_hours,
        display_steps: replay::display::stepped_format(format.version()),
        format,
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
        compare_boundary_counters(frame, save, boundary, report)
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
    report.attempt_table(TableId::ProvinceNames, |report| {
        compare_province_names(frame, save, report)
    });
    Ok(report)
}

fn required_context<'a, T, E: std::fmt::Display>(
    name: &str,
    context: &'a Result<T, E>,
) -> io::Result<&'a T> {
    context
        .as_ref()
        .map_err(|error| invalid(format!("requires {name}: {error}")))
}

/// The content's `is_port` building types, whose levels `naval-base cache` sums.
fn port_types(buildings: &installed_content::buildings::Buildings) -> BTreeSet<Vec<u8>> {
    buildings
        .types
        .values()
        .filter(|building| building.is_port)
        .map(|building| building.name.clone().into_bytes())
        .collect()
}

/// The content reads the extended comparisons need, each failing on its own.
struct ExtendedContent<'a> {
    membership: ContentRead<'a, save::PartialMapMembership>,
    base_resources: ContentRead<'a, save::construction::PartialBaseResources>,
    railway_adjacency: ContentRead<'a, save::naval_history::RailwayAdjacency>,
    buildings: ContentRead<'a, installed_content::buildings::Buildings>,
    defines: ContentRead<'a, installed_content::defines::Defines>,
    raid_graph: ContentRead<'a, projects::RaidGraph>,
    equipment: ContentRead<'a, installed_content::equipment::Database>,
    /// Each essential archetype's largest raid requirement, by name
    /// ([`installed_content::raids::Catalog::essential_caps`]).
    essential_caps: ContentRead<'a, BTreeMap<Vec<u8>, i32>>,
    leader_traits: ContentRead<'a, installed_content::leader_traits::LeaderTraits>,
    sub_units: ContentRead<'a, installed_content::unit_leader::SubUnits>,
    intelligence: ContentRead<'a, installed_content::intelligence::AgencyUpgrades>,
    /// What the hourly temperature pass reads, which recordings that store only its inputs need.
    temperature: ContentRead<'a, replay::temperature::Content>,
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
            report,
        );
        Ok(())
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
    report.attempt("construction OccupationSettings", |report| {
        construction::occupation_settings(
            complete::<tables::OccupationSettings>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("construction StateOccupationLaws", |report| {
        construction::occupation_laws(
            complete::<tables::StateOccupationLaws>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("construction StateGarrisons", |report| {
        construction::garrisons(
            complete::<tables::StateGarrisons>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
    report.attempt("construction StateAssignedManpower", |report| {
        construction::assigned_manpower(
            complete::<tables::StateAssignedManpower>(frame)?,
            &save.construction,
            &required_context("country links", &countries)?.links,
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
    report.attempt("projects RaidPathDistances", |report| {
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
            // Only the definitions need the dictionary; appointments do not.
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
        weather::compare(frame, save, temperature, report)
    });
    report.attempt("fog", |report| {
        fog::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
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

fn required_country_tag<'a>(
    reference: u32,
    links: &BTreeMap<u32, &'a CountryRow>,
) -> io::Result<&'a [u8]> {
    country_tag(reference, links)?.ok_or_else(|| invalid("null country reference"))
}

fn country_tag<'a>(
    reference: u32,
    links: &BTreeMap<u32, &'a CountryRow>,
) -> io::Result<Option<&'a [u8]>> {
    if reference == 0 {
        return Ok(None);
    }
    links
        .get(&reference)
        .map(|country| Some(country.tag.as_slice()))
        .ok_or_else(|| invalid(format!("unresolved country reference {reference}")))
}

fn tag_text(tag: Option<&[u8]>) -> String {
    tag.map(|tag| String::from_utf8_lossy(tag).into_owned())
        .unwrap_or_else(|| "<none>".into())
}

pub(crate) fn default_game_directory() -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    for key in [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 394360",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 394360",
    ] {
        let key = crate::windows::wide(OsStr::new(key));
        let name = crate::windows::wide(OsStr::new("InstallLocation"));
        let mut buffer = vec![0_u16; 32768];
        let mut bytes = (buffer.len() * 2) as u32;
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if result == 0 {
            let length = buffer
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(buffer.len());
            let path = PathBuf::from(OsString::from_wide(&buffer[..length]));
            if path.join("hoi4.exe").is_file() {
                return Ok(path);
            }
        }
    }
    Err(invalid(
        "Steam the game installation was not found in the registry; pass --game-dir",
    ))
}

#[cfg(test)]
#[path = "oracle/report_tests.rs"]
mod reporting_tests;

#[cfg(test)]
mod tests;
