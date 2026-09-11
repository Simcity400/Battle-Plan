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
    identity::Verification,
    rows::{CountryRow, TableId, TableRows, TableSchema, tables},
};

mod air;
mod characters;
mod construction;
mod economy;
mod fog;
mod governance;
mod intel;
mod naval_history;
mod production;
mod projects;
mod session;
mod weather;

type TokenNames<'a> = BTreeMap<u32, &'a [u8]>;
type ContentRead<'a, T> = Result<&'a T, String>;

fn content_read<T>(result: &io::Result<T>) -> ContentRead<'_, T> {
    result.as_ref().map_err(ToString::to_string)
}

pub const USAGE: &str = "recorder oracle <campaign.battleplan> <save.hoi4> [--hour n] [--branch id] [--game-dir <directory>] [--user-dir <directory>]";
const CONVOY_LIMIT: i64 = 0x53e2_d620_fb40;

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
    let save = save::parse(&fs::read(&save_path)?)?;
    let hour = matching_hour(save.date.engine_hour()?, requested_hour)?;
    let replay = Replay::open(fs::read(&replay_path)?)?;
    if save.campaign_id != replay.campaign_id {
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
    if !same_version(&save.version, &session.identity.game_version) {
        return Err(invalid(format!(
            "save version {} differs from recorded version {}",
            String::from_utf8_lossy(&save.version),
            session.identity.game_version
        )));
    }
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
        "Content: game {}, user {}",
        options.game_dir.display(),
        options.user_dir.display()
    );
    let content = installed_content::resolve(&session.identity, &options)?;
    println!(
        "Content verified under policy {}: {} files, {} bytes, {} ms",
        session.identity.coverage_policy,
        content.stats.files,
        content.stats.bytes,
        content.stats.elapsed_ms
    );
    let membership = save::read_map_membership(&content.layers);
    let equipment_types = save::read_equipment_types(&content.layers);
    let base_resources = save::construction::BaseResources::read(&content.layers);
    let railway_adjacency = save::naval_history::RailwayAdjacency::read(&content.layers);
    let mut report = compare_core(
        &frame,
        &save,
        content_read(&membership),
        content_read(&equipment_types),
    )?;
    compare_extended(
        &frame,
        &save,
        content_read(&membership),
        content_read(&base_resources),
        content_read(&railway_adjacency),
        &mut report,
    );
    report.print();
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

fn complete_table(frame: &DecodedFrame, table: TableId) -> io::Result<&TableRows> {
    match &frame.tables[table.index()] {
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

fn recorded_table_counts(frame: &DecodedFrame) -> BTreeMap<TableId, Result<usize, String>> {
    TableId::ALL
        .into_iter()
        .map(|table| {
            let count = complete_table(frame, table)
                .map(TableRows::len)
                .map_err(|error| error.to_string());
            (table, count)
        })
        .collect()
}

fn lexical_tokens<'a>(frame: &'a DecodedFrame, report: &mut Report) -> io::Result<TokenNames<'a>> {
    if let TableView::Unavailable { reason } = &frame.tables[TableId::LexicalTokens.index()] {
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

#[derive(Debug, Default)]
struct Report {
    countries: usize,
    provinces: usize,
    states: usize,
    divisions: usize,
    war_relations: usize,
    factions: usize,
    faction_members: usize,
    players: usize,
    land_combats: usize,
    fleets: usize,
    task_forces: usize,
    ships: usize,
    naval_combats: usize,
    naval_combat_participants: usize,
    air_wings: usize,
    army_orders: usize,
    scalar_checks: usize,
    differences: Vec<String>,
    /// The comparison group each difference belongs to, in lockstep with
    /// `differences`, so every group can print all of its own rows.
    difference_owners: Vec<String>,
    limitations: BTreeMap<String, Vec<String>>,
    compared_tables: BTreeMap<String, usize>,
    recorded_tables: BTreeMap<TableId, Result<usize, String>>,
    failed_comparisons: BTreeMap<String, String>,
    difference_groups: BTreeMap<String, usize>,
    active_comparison: Option<String>,
    /// One line per allocation counter: recorded value, saved value and gap.
    boundary_witness: Vec<String>,
    /// The explicit verdict on whether the save was taken at the sampled tick.
    boundary_verdict: Option<String>,
}

impl Report {
    fn attempt(&mut self, name: &str, compare: impl FnOnce(&mut Self) -> io::Result<()>) -> bool {
        let previous = self.active_comparison.replace(name.into());
        let result = compare(self);
        self.active_comparison = previous;
        match result {
            Ok(()) => true,
            Err(error) => {
                self.failed_comparisons
                    .insert(name.into(), error.to_string());
                false
            }
        }
    }

    fn unavailable_tables(&self) -> usize {
        self.recorded_tables
            .values()
            .filter(|rows| rows.is_err())
            .count()
    }

    fn outcome(&self) -> io::Result<()> {
        let unavailable = self.unavailable_tables();
        if self.differences.is_empty() && unavailable == 0 && self.failed_comparisons.is_empty() {
            Ok(())
        } else {
            Err(invalid(format!(
                "oracle: {} differences; {} unavailable, truncated or invalid tables; {} failed or skipped comparison groups; comparison {}",
                self.differences.len(),
                unavailable,
                self.failed_comparisons.len(),
                if unavailable == 0 && self.failed_comparisons.is_empty() {
                    "completed"
                } else {
                    "incomplete"
                },
            )))
        }
    }

    fn record_tables(&mut self, frame: &DecodedFrame) {
        self.recorded_tables = recorded_table_counts(frame);
        for table in [TableId::LexicalTokens, TableId::ScriptVariables] {
            if let TableView::Unavailable { reason } = &frame.tables[table.index()] {
                self.limitation(
                    "explicitly unavailable source remains unverified and makes the overall comparison incomplete; the recorded reason identifies the missing proof or failed read",
                    format!("{table:?}: {reason}"),
                );
            }
        }
    }

    fn compared(&mut self, table: &str, rows: usize) {
        *self.compared_tables.entry(table.to_owned()).or_default() += rows;
    }
    fn difference(
        &mut self,
        field: impl AsRef<str>,
        replay: impl std::fmt::Debug,
        save: impl std::fmt::Debug,
    ) {
        let group = self
            .active_comparison
            .as_deref()
            .unwrap_or("direct comparisons");
        if let Some(count) = self.difference_groups.get_mut(group) {
            *count += 1;
        } else {
            self.difference_groups.insert(group.into(), 1);
        }
        self.difference_owners.push(group.to_owned());
        self.differences.push(format!(
            "{}: replay {replay:?}, save {save:?}",
            field.as_ref()
        ));
    }

    /// Every difference is printed, grouped by its comparison, with a count
    /// line per group. The former hard hundred-line cap silently dropped whole
    /// groups, including ones whose only difference it hid, which left them
    /// undiagnosable from the report alone. There is no cap now.
    fn difference_lines(&self) -> Vec<String> {
        if self.differences.is_empty() {
            return Vec::new();
        }
        let mut grouped = BTreeMap::<&str, Vec<&str>>::new();
        for (group, difference) in self.difference_owners.iter().zip(&self.differences) {
            grouped
                .entry(group.as_str())
                .or_default()
                .push(difference.as_str());
        }
        let mut lines = vec![format!(
            "All {} differences, by group:",
            self.differences.len()
        )];
        for (group, differences) in grouped {
            lines.push(format!("  {group}: {} differences", differences.len()));
            lines.extend(differences.iter().map(|line| format!("    {line}")));
        }
        lines
    }

    fn limitation(&mut self, reason: &str, subject: impl Into<String>) {
        self.limitations
            .entry(reason.into())
            .or_default()
            .push(subject.into());
    }

    fn scalar(&mut self, tag: &str, field: &str, captured: Option<i64>, saved: Option<&Decimal>) {
        match (captured, saved) {
            (Some(captured), Some(saved)) => {
                self.scalar_checks += 1;
                if captured != saved.raw {
                    self.difference(format!("{tag}.{field}"), captured, saved.raw);
                }
            }
            (None, _) => self.limitation(
                "captured scalar is explicitly unavailable",
                format!("{tag}.{field}"),
            ),
            (_, None) => self.difference(
                format!("{tag}.{field}"),
                captured,
                "missing serialized scalar",
            ),
        }
    }

    fn print(&self) {
        println!(
            "Compared: {} provinces, {} states, {} divisions, {} war relations, {} factions, {} faction members, {} countries, {} exact field comparisons",
            self.provinces,
            self.states,
            self.divisions,
            self.war_relations,
            self.factions,
            self.faction_members,
            self.countries,
            self.scalar_checks
        );
        println!(
            "Map tables: {} uniquely matched players, {} land combats, {} fleets, {} task forces, {} ships",
            self.players, self.land_combats, self.fleets, self.task_forces, self.ships
        );
        println!(
            "Map tables: {} naval combats, {} naval combat participants, {} air wings, {} army orders",
            self.naval_combats, self.naval_combat_participants, self.air_wings, self.army_orders
        );
        println!("Differences: {}", self.differences.len());
        for (group, count) in &self.difference_groups {
            println!("  Differences {group}: {count}");
        }
        println!(
            "Comparison status: {}; {} table availability issues; {} failed or skipped comparison groups",
            if self.unavailable_tables() == 0 && self.failed_comparisons.is_empty() {
                "complete"
            } else {
                "incomplete"
            },
            self.unavailable_tables(),
            self.failed_comparisons.len(),
        );
        for (table, rows) in &self.compared_tables {
            println!("Compared {table}: {rows}");
        }
        if !self.recorded_tables.is_empty() {
            println!(
                "Recorded table coverage (row counts alone do not establish save comparison):"
            );
            for (table, rows) in &self.recorded_tables {
                match rows {
                    Ok(rows) => println!("  {table:?}: {rows} rows"),
                    Err(reason) => println!("  {table:?}: {reason}"),
                }
            }
        }
        for (group, reason) in &self.failed_comparisons {
            println!("  Comparison {group} incomplete: {reason}");
        }
        for line in self.difference_lines() {
            println!("{line}");
        }
        println!("Boundary witness (engine allocation counters at the sampled tick):");
        match &self.boundary_verdict {
            Some(verdict) => println!("  {verdict}"),
            None => println!(
                "  not evaluated; the counter comparison itself failed and is listed above"
            ),
        }
        for line in &self.boundary_witness {
            println!("  {line}");
        }
        println!("Limitations:");
        for (reason, subjects) in &self.limitations {
            const EXAMPLES: usize = 5;
            println!("  {reason}: {} entries", subjects.len());
            for subject in subjects.iter().take(EXAMPLES) {
                println!("    {subject}");
            }
            if subjects.len() > EXAMPLES {
                println!(
                    "    {} additional entries omitted",
                    subjects.len() - EXAMPLES
                );
            }
        }
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

fn compare_core(
    frame: &DecodedFrame,
    save: &Save,
    membership: ContentRead<'_, save::MapMembership>,
    equipment_types: ContentRead<'_, save::EquipmentTypes>,
) -> io::Result<Report> {
    validate_identity(frame, save)?;
    let mut report = Report::default();
    report.record_tables(frame);
    report.attempt("content map membership", |_| {
        required_context("verified map membership", &membership).map(|_| ())
    });
    report.attempt("content equipment types", |_| {
        required_context("verified equipment types", &equipment_types).map(|_| ())
    });
    report.limitation(
        "commands after the hourly callback can change state before saving without advancing the hour; matching hours do not prove the same command boundary. The boundary witness below measures that gap from the engine's own allocation counters; differences still fail either way",
        "all captured tables",
    );
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
            compare_countries(save, &equipment_types, by_tag, report);
            Ok(())
        });
        report.attempt("provinces", |report| {
            compare_provinces(
                frame,
                save,
                required_context("verified map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("states", |report| {
            compare_states(
                frame,
                save,
                required_context("verified map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("divisions", |report| {
            compare_divisions(
                frame,
                save,
                required_context("verified map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("war relations", |report| {
            compare_war_relations(frame, save, links, report)
        });
        report.attempt("RelationOpinions", |report| {
            compare_relation_opinions(frame, save, links, report)
        });
        report.attempt("CountryIntel", |report| {
            compare_country_intel(frame, save, links, report)
        });
        report.attempt("factions", |report| {
            compare_factions(frame, save, links, report)
        });
        report.attempt("players", |report| {
            compare_players(frame, save, links, report)
        });
        report.attempt("naval units", |report| {
            compare_naval_units(
                frame,
                save,
                required_context("verified map membership", &membership)?,
                links,
                report,
            )
        });
        report.attempt("naval combats", |report| {
            compare_naval_combats(
                frame,
                save,
                required_context("verified map membership", &membership)?,
                links,
                report,
            )
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
            required_context("verified map membership", &membership)?,
            report,
        )
    });
    Ok(report)
}

fn compare_countries(
    save: &Save,
    equipment_types: &ContentRead<'_, save::EquipmentTypes>,
    by_tag: &BTreeMap<&[u8], &CountryRow>,
    report: &mut Report,
) {
    for (&tag, captured) in by_tag {
        let label = String::from_utf8_lossy(tag);
        let Some(saved) = save.countries.get(tag) else {
            report.difference(format!("country {label}"), "present", "absent");
            continue;
        };
        report.countries += 1;
        report.scalar(
            &label,
            "political_power",
            captured.political_power_raw,
            saved.political_power.as_ref(),
        );
        report.scalar(
            &label,
            "command_power",
            Some(captured.command_power_raw),
            saved.command_power.as_ref(),
        );
        report.scalar(
            &label,
            "stability",
            Some(captured.stability_raw),
            saved.stability.as_ref(),
        );
        report.scalar(
            &label,
            "war_support",
            Some(captured.war_support_raw),
            saved.war_support.as_ref(),
        );
        // CCountry's rendered name is not emitted by its save serializer.
        if saved.name.is_some() {
            report.limitation(
                "direct saved name has no proven equivalence to the rendered engine country name",
                label.to_string(),
            );
        }
        match (
            captured.convoys_raw,
            required_context("verified equipment types", equipment_types)
                .map_err(|error| error.to_string())
                .and_then(|types| convoy_total(saved, save, types)),
        ) {
            (None, _) => report.limitation(
                "convoy cache was invalid at capture; no value was invented",
                label.to_string(),
            ),
            (Some(actual), Ok(expected)) => {
                report.scalar_checks += 1;
                if actual != expected {
                    report.difference(format!("{label}.convoys"), actual, expected);
                }
            }
            (Some(_), Err(reason)) => {
                report.failed_comparisons.insert(
                    format!("country {label} convoys"),
                    format!("{label}.convoys cannot be compared: {reason}"),
                );
            }
        }
    }
    report.limitation(
        "rendered country names are not serialized by the country writer",
        format!("all {} countries", by_tag.len()),
    );
    for tag in save
        .countries
        .keys()
        .filter(|tag| !by_tag.contains_key(tag.as_slice()))
    {
        report.difference(
            format!("country {}", String::from_utf8_lossy(tag)),
            "absent",
            "present",
        );
    }
}

fn compare_provinces(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut provinces = BTreeMap::new();
    for province in frame.complete::<tables::Provinces>()? {
        if provinces.insert(province.id, province).is_some() {
            return Err(invalid("duplicate province row"));
        }
    }
    for expected in save.province_control(membership)? {
        let Some(actual) = provinces.remove(&expected.id) else {
            report.difference(format!("province {}", expected.id), "absent", "present");
            continue;
        };
        report.provinces += 1;
        // E6CA90 omits +0x38 iff zero. 1F2A20 visits every province;
        // AEBA0/AED30/AB010 can omit only an empty serialized object.
        let victory_points = save
            .provinces
            .get(&expected.id)
            .and_then(|saved| saved.victory_points)
            .unwrap_or(0);
        report.compared("Provinces.victory_points", 1);
        if actual.victory_points != victory_points {
            report.difference(
                format!("province {}.victory_points", expected.id),
                actual.victory_points,
                victory_points,
            );
        }
        if actual.state != expected.state.unwrap_or(0) {
            report.difference(
                format!("province {}.state", expected.id),
                actual.state,
                expected.state,
            );
        }
        let owner = country_tag(actual.owner, links)?;
        if owner != expected.owner.as_deref() {
            report.difference(
                format!("province {}.owner", expected.id),
                tag_text(owner),
                tag_text(expected.owner.as_deref()),
            );
        }
        let controller = if actual.controller == 0 && !expected.controller_explicit {
            owner
        } else {
            country_tag(actual.controller, links)?
        };
        if controller != expected.controller.as_deref() {
            report.difference(
                format!("province {}.controller", expected.id),
                tag_text(controller),
                tag_text(expected.controller.as_deref()),
            );
        }
    }
    for id in provinces.keys() {
        report.difference(format!("province {id}"), "present", "absent");
    }
    Ok(())
}

fn compare_states(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut states = BTreeMap::new();
    for state in frame.complete::<tables::States>()? {
        if states.insert(state.id, state).is_some() {
            return Err(invalid("duplicate state row"));
        }
    }
    for expected in save.state_control(membership)? {
        let Some(actual) = states.remove(&expected.id) else {
            report.difference(format!("state {}", expected.id), "absent", "present");
            continue;
        };
        report.states += 1;
        let owner = country_tag(actual.owner, links)?;
        if owner != Some(expected.owner.as_slice()) {
            report.difference(
                format!("state {}.owner", expected.id),
                tag_text(owner),
                tag_text(Some(&expected.owner)),
            );
        }
        // CState writer 1409d4040 emits zero controller when owner is positive;
        // only raw or canonical equality permits omission. Never apply the
        // province writer's zero-controller normalization to this table.
        let controller = country_tag(actual.controller, links)?;
        if controller != Some(expected.controller.as_slice()) {
            report.difference(
                format!("state {}.controller", expected.id),
                tag_text(controller),
                tag_text(Some(&expected.controller)),
            );
        }
    }
    for id in states.keys() {
        report.difference(format!("state {id}"), "present", "absent");
    }
    report.limitation(
        "state serializer omits controllers equal to owner by raw or canonical country-link identity; omitted fields compare to the saved owner without accepting captured zero",
        "states with no explicit saved controller",
    );
    Ok(())
}

fn required_context<'a, T, E: std::fmt::Display>(
    name: &str,
    context: &'a Result<T, E>,
) -> io::Result<&'a T> {
    context
        .as_ref()
        .map_err(|error| invalid(format!("requires {name}: {error}")))
}

fn compare_extended(
    frame: &DecodedFrame,
    save: &Save,
    membership: ContentRead<'_, save::MapMembership>,
    base_resources: ContentRead<'_, save::construction::BaseResources>,
    railway_adjacency: ContentRead<'_, save::naval_history::RailwayAdjacency>,
    report: &mut Report,
) {
    let countries = country_context(frame);
    let tokens = lexical_tokens(frame, report);
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
    report.attempt("economy", |report| {
        economy::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction StateBuildings", |report| {
        construction::buildings(
            complete::<tables::StateBuildings>(frame)?,
            &save.construction.state_buildings,
            false,
            required_context("verified map membership", &membership)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction ProvinceBuildings", |report| {
        construction::buildings(
            complete::<tables::ProvinceBuildings>(frame)?,
            &save.construction.province_buildings,
            true,
            required_context("verified map membership", &membership)?,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("construction StateBaseResources", |report| {
        construction::base_resources(
            complete::<tables::StateBaseResources>(frame)?,
            required_context("verified base resources", &base_resources)?,
            required_context("lexical dictionary", &tokens)?,
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
            report,
        )
    });
    report.attempt("RelationTuples", |report| {
        compare_relation_tuples(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            report,
        )
    });
    report.attempt("intel", |report| {
        intel::compare(
            frame,
            &save.intel,
            &required_context("country links", &countries)?.links,
            required_context("lexical dictionary", &tokens)?,
            intel::ScopeKinds {
                global: 0,
                country: 1,
                state: 2,
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
    report.attempt("weather", |report| weather::compare(frame, save, report));
    report.attempt("fog", |report| {
        fog::compare(
            frame,
            save,
            &required_context("country links", &countries)?.links,
            report,
        )
    });
}

fn compare_players(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut users = BTreeMap::<&[u8], Vec<&replay::rows::PlayerRow>>::new();
    let mut keys = BTreeSet::new();
    for player in frame.complete::<tables::Players>()? {
        if player.id == -1 || !keys.insert(player.id) {
            return Err(invalid("invalid or duplicate native player id"));
        }
        country_tag(player.country, links)?;
        users.entry(&player.user).or_default().push(player);
    }
    let mut saved_users = BTreeMap::<&[u8], Vec<&save::Player>>::new();
    for player in &save.players {
        if player.country != b"---" && !save.countries.contains_key(&player.country) {
            return Err(invalid("saved player has unknown country tag"));
        }
        saved_users.entry(&player.user).or_default().push(player);
    }
    for (&user, players) in &users {
        let label = format!("player user {:?}", String::from_utf8_lossy(user));
        let saved = saved_users.remove(user).unwrap_or_default();
        if saved.len() != 1 {
            report.difference(format!("{label}.saved refreshed entries"), 1, saved.len());
            continue;
        }
        let expected = saved[0];
        if players.len() != 1 {
            report.limitation("native save upsert collapses equal current user bytes; only one surviving entry can be checked, not every live player or live order", format!("{label}: {} live entries", players.len()));
            let matches = players.iter().any(|actual| {
                let tag = country_tag(actual.country, links)
                    .ok()
                    .flatten()
                    .unwrap_or(b"---");
                actual.id == expected.id
                    && (actual.flags & 1 != 0) == expected.country_leader
                    && tag == expected.country
            });
            if !matches {
                report.difference(&label, "no live player matches saved survivor", expected);
            }
            continue;
        }
        let actual = players[0];
        report.players += 1;
        if actual.id != expected.id {
            report.difference(format!("{label}.id"), actual.id, expected.id);
        }
        let tag = country_tag(actual.country, links)?.unwrap_or(b"---");
        if tag != expected.country {
            report.difference(
                format!("{label}.country"),
                tag_text(Some(tag)),
                tag_text(Some(&expected.country)),
            );
        }
        if (actual.flags & 1 != 0) != expected.country_leader {
            report.difference(
                format!("{label}.country_leader"),
                actual.flags & 1,
                expected.country_leader,
            );
        }
    }
    for (user, players) in saved_users {
        report.limitation("saved header retains users absent from the current live roster; saved-only entries do not prove missing live players", format!("user {:?}: {} header entries", String::from_utf8_lossy(user), players.len()));
    }
    let mut controller_countries = BTreeSet::new();
    for controller in frame.complete::<tables::HumanControllers>()? {
        let tag = required_country_tag(controller.country, links)?;
        if !controller_countries.insert(tag) {
            return Err(invalid("duplicate resolved human-controller country"));
        }
    }
    if controller_countries.len() != save.countries.len() {
        report.difference(
            "human controller canonical country coverage",
            controller_countries.len(),
            save.countries.len(),
        );
    }
    report.limitation("saved player header is refreshed by user-based upserts and is not an independent complete live-roster census", "Players");
    report.limitation("save carries CHuman user, signed id, resolved country and leader bit only; display name, upper flag bits and raw country alias bits have no independent save representation", "Players");
    report.limitation("native signed controller counts are not serialized and are not universally equal to saved player entry counts", "HumanControllers");
    Ok(())
}

fn compare_land_combats(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    report: &mut Report,
) -> io::Result<()> {
    let mut combats = BTreeMap::new();
    for combat in frame.complete::<tables::LandCombats>()? {
        if combat.id == 0 || combats.insert(combat.id, combat).is_some() {
            return Err(invalid("null or duplicate full land combat handle"));
        }
    }
    for (&id, expected) in &save.land_combats {
        let label = format!("land combat type={} id={}", id as u32, id >> 32);
        if expected.province == 0 || !membership.provinces.contains_key(&expected.province) {
            return Err(invalid(
                "saved land combat location is absent from verified map",
            ));
        }
        let Some(actual) = combats.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.land_combats += 1;
        if actual.province != expected.province {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        if actual.day != expected.day {
            report.difference(format!("{label}.day"), actual.day, expected.day);
        }
        if actual.duration != expected.duration {
            report.difference(
                format!("{label}.duration"),
                actual.duration,
                expected.duration,
            );
        }
        for (side, actual, expected) in [
            ("attackers", &actual.attackers, &expected.attackers),
            ("defenders", &actual.defenders, &expected.defenders),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{side}"), actual, expected);
            }
        }
    }
    for id in combats.keys() {
        report.difference(
            format!("land combat type={} id={}", *id as u32, id >> 32),
            "present",
            "absent",
        );
    }
    report.limitation("land combat comparison covers recorded locations, native day/duration and ordered side participants; reduced saved combat details cannot certify intervening combat events", "LandCombats");
    Ok(())
}

fn compare_naval_units(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    fn province(value: Option<u32>, membership: &save::MapMembership) -> io::Result<u32> {
        match value {
            Some(value) if value == 0 || !membership.provinces.contains_key(&value) => Err(
                invalid("saved naval unit province is absent from verified map or explicitly zero"),
            ),
            value => Ok(value.unwrap_or(0)),
        }
    }
    let mut fleets = BTreeMap::new();
    for row in frame.complete::<tables::Fleets>()? {
        if row.id == 0 || fleets.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate fleet handle"));
        }
    }
    for (&id, expected) in &save.fleets {
        let label = format!("fleet type={} id={}", id as u32, id >> 32);
        let home = province(expected.home_base, membership)?;
        let Some(actual) = fleets.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.fleets += 1;
        let owner = required_country_tag(actual.owner, links)?;
        if owner != expected.owner {
            report.difference(
                format!("{label}.owner"),
                tag_text(Some(owner)),
                tag_text(Some(&expected.owner)),
            );
        }
        if actual.home_base != home {
            report.difference(
                format!("{label}.home_base"),
                actual.home_base,
                expected.home_base,
            );
        }
    }
    for id in fleets.keys() {
        report.difference(format!("fleet {id}"), "present", "absent");
    }
    let mut forces = BTreeMap::new();
    for row in frame.complete::<tables::TaskForces>()? {
        if row.id == 0 || forces.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate task force handle"));
        }
    }
    for (&id, expected) in &save.task_forces {
        let label = format!("task force type={} id={}", id as u32, id >> 32);
        let location = province(expected.province, membership)?;
        if !save.fleets.contains_key(&expected.fleet) {
            return Err(invalid("saved task force references missing fleet"));
        }
        let Some(actual) = forces.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.task_forces += 1;
        if actual.fleet != expected.fleet {
            report.difference(format!("{label}.fleet"), actual.fleet, expected.fleet);
        }
        if actual.province != location {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        if actual.mission as i32 != expected.mission {
            report.difference(
                format!("{label}.mission"),
                actual.mission as i32,
                expected.mission,
            );
        }
        for (field, raw, expected) in [
            (
                "roster_owner",
                actual.roster_owner,
                Some(expected.roster_owner.as_slice()),
            ),
            (
                "logical_owner",
                actual.logical_owner,
                Some(expected.logical_owner.as_slice()),
            ),
            (
                "expeditionary_owner",
                actual.expeditionary_owner,
                expected.expeditionary_owner.as_deref(),
            ),
        ] {
            let actual = if field == "expeditionary_owner" && (raw as i32) <= 0 {
                None
            } else {
                country_tag(raw, links)?
            };
            if actual != expected {
                report.difference(
                    format!("{label}.{field}"),
                    tag_text(actual),
                    tag_text(expected),
                );
            }
        }
    }
    for id in forces.keys() {
        report.difference(format!("task force {id}"), "present", "absent");
    }
    let mut ships = BTreeMap::new();
    for row in frame.complete::<tables::Ships>()? {
        if row.id == 0 || ships.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate ship handle"));
        }
    }
    for (&id, expected) in &save.ships {
        let label = format!("ship type={} id={}", id as u32, id >> 32);
        if !save.task_forces.contains_key(&expected.task_force) {
            return Err(invalid("saved ship references missing task force"));
        }
        let Some(actual) = ships.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.ships += 1;
        if actual.task_force != expected.task_force {
            report.difference(
                format!("{label}.task_force"),
                actual.task_force,
                expected.task_force,
            );
        }
        if actual.sunk_convoys != expected.sunk_convoys {
            report.difference(
                format!("{label}.sunk_convoys"),
                actual.sunk_convoys,
                expected.sunk_convoys,
            );
        }
        report.scalar(
            &label,
            "strength",
            Some(actual.strength),
            Some(&expected.strength),
        );
        report.scalar(
            &label,
            "organisation",
            Some(actual.organisation),
            Some(&expected.organisation),
        );
        report.scalar(
            &label,
            "experience",
            Some(actual.experience),
            Some(&expected.experience),
        );
    }
    for id in ships.keys() {
        report.difference(format!("ship {id}"), "present", "absent");
    }
    report.limitation("ship category and numeric class id are not independently serialized; saved definition names are not substituted for native ids", "Ships");
    report.limitation("task-force serializer omits signed-nonpositive expeditionary references; absent fields verify the omission predicate, not those raw bits", "TaskForces");
    Ok(())
}

fn compare_naval_combats(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut combats = BTreeMap::new();
    for row in frame.complete::<tables::NavalCombats>()? {
        if row.id == 0 || combats.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate full naval combat handle"));
        }
    }
    for (&id, expected) in &save.naval_combats {
        let label = format!("naval combat type={} id={}", id as u32, id >> 32);
        if expected.province == 0 || !membership.provinces.contains_key(&expected.province) {
            return Err(invalid(
                "saved naval combat location is absent from verified map",
            ));
        }
        let Some(actual) = combats.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.naval_combats += 1;
        if actual.province != expected.province {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        for (field, captured, saved) in [
            (
                "attacker_positioning",
                actual.attacker_positioning,
                &expected.attacker_positioning,
            ),
            (
                "defender_positioning",
                actual.defender_positioning,
                &expected.defender_positioning,
            ),
            (
                "attacker_anti_air",
                actual.attacker_anti_air,
                &expected.attacker_anti_air,
            ),
            (
                "defender_anti_air",
                actual.defender_anti_air,
                &expected.defender_anti_air,
            ),
        ] {
            report.scalar(&label, field, Some(captured), Some(saved));
        }
    }
    for id in combats.keys() {
        report.difference(format!("naval combat {id}"), "present", "absent");
    }
    let mut participants = BTreeMap::new();
    for row in frame.complete::<tables::NavalCombatParticipants>()? {
        if row.combat == 0
            || row.side > 1
            || participants
                .insert((row.combat, row.side, row.battle_slot), row)
                .is_some()
        {
            return Err(invalid(
                "invalid or duplicate naval combat participant identity",
            ));
        }
    }
    for (&key, expected) in &save.naval_combat_participants {
        let label = format!(
            "naval participant combat={} side={} slot={}",
            key.0, key.1, key.2
        );
        if !save.naval_combats.contains_key(&expected.combat) {
            return Err(invalid("saved participant has missing combat"));
        }
        let Some(actual) = participants.remove(&key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.naval_combat_participants += 1;
        if actual.ship != expected.ship.unwrap_or(0) {
            report.difference(format!("{label}.ship"), actual.ship, expected.ship);
        }
        if actual.state != expected.state {
            report.difference(format!("{label}.state"), actual.state, expected.state);
        }
        let owner = country_tag(actual.owner, links)?.unwrap_or(b"---");
        if owner != expected.owner {
            report.difference(
                format!("{label}.owner"),
                tag_text(Some(owner)),
                tag_text(Some(&expected.owner)),
            );
        }
    }
    for key in participants.keys() {
        report.difference(
            format!(
                "naval participant combat={} side={} slot={}",
                key.0, key.1, key.2
            ),
            "present",
            "absent",
        );
    }
    report.limitation("naval combat comparison covers captured location, side scalars and participant identity/state; convoy/client handles and other reduced save details are not recorded, and hourly state cannot certify intervening events", "NavalCombats / NavalCombatParticipants");
    Ok(())
}

fn compare_air_wings(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut wings = BTreeMap::new();
    for row in frame.complete::<tables::AirWings>()? {
        if row.id == 0 || wings.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate full air wing handle"));
        }
    }
    for (&id, expected) in &save.air_wings {
        let label = format!("air wing type={} id={}", id as u32, id >> 32);
        let Some(actual) = wings.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.air_wings += 1;
        let owner = required_country_tag(actual.owner, links)?;
        if owner != expected.owner {
            report.difference(
                format!("{label}.pool_owner"),
                tag_text(Some(owner)),
                tag_text(Some(&expected.owner)),
            );
        }
        let country_override =
            country_tag(actual.country_override as u32, links)?.unwrap_or(b"---");
        if country_override != expected.country_override {
            report.difference(
                format!("{label}.country_override"),
                tag_text(Some(country_override)),
                tag_text(Some(&expected.country_override)),
            );
        }
        for (field, actual, expected) in [
            ("air_base", actual.air_base, expected.air_base),
            (
                "mission_mask",
                u64::from(actual.mission_mask),
                u64::from(expected.mission_mask),
            ),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{field}"), actual, expected);
            }
        }
        for (field, actual, expected) in [
            ("planes", actual.planes, expected.planes),
            (
                "effective_planes",
                actual.effective_planes,
                expected.effective_planes,
            ),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{field}"), actual, expected);
            }
        }
        if actual.mission_region != expected.mission_region {
            report.difference(
                format!("{label}.mission_region"),
                actual.mission_region,
                expected.mission_region,
            );
        }
    }
    for id in wings.keys() {
        report.difference(format!("air wing {id}"), "present", "absent");
    }
    report.limitation("air wing transfer override is serialized through a resolved state or carrier, not its raw airbase handle; an omitted transfer field can also be an unresolved override", "AirWings.base_override");
    Ok(())
}

fn compare_army_orders(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut orders = BTreeMap::new();
    for row in frame.complete::<tables::ArmyOrders>()? {
        if row.owner_group == 0
            || orders
                .insert((row.owner_group, row.instance), row)
                .is_some()
        {
            return Err(invalid("null or duplicate scoped army order identity"));
        }
    }
    for (&key, expected) in &save.army_orders {
        let label = format!("army order group={} instance={}", key.0, key.1);
        let Some(actual) = orders.remove(&key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.army_orders += 1;
        let country = required_country_tag(actual.country, links)?;
        if country != expected.country {
            report.difference(
                format!("{label}.country"),
                tag_text(Some(country)),
                tag_text(Some(&expected.country)),
            );
        }
        if actual.order_type != expected.order_type {
            report.difference(
                format!("{label}.type"),
                actual.order_type,
                expected.order_type,
            );
        }
        if actual.active != expected.active {
            report.difference(
                format!("{label}.can_execute"),
                actual.active,
                expected.active,
            );
        }
        if actual.paradrop_target != expected.paradrop_target {
            report.difference(
                format!("{label}.invasion_source"),
                actual.paradrop_target,
                expected.paradrop_target,
            );
        }
        for (field, actual, expected) in [
            ("path", &actual.path, &expected.path),
            ("states", &actual.target_states, &expected.target_states),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{field}"), actual, expected);
            }
        }
    }
    for key in orders.keys() {
        report.difference(
            format!("army order group={} instance={}", key.0, key.1),
            "present",
            "absent",
        );
    }
    Ok(())
}

fn required_country_tag<'a>(
    reference: u32,
    links: &BTreeMap<u32, &'a CountryRow>,
) -> io::Result<&'a [u8]> {
    country_tag(reference, links)?.ok_or_else(|| invalid("null country reference"))
}

fn saved_country_slot(
    tag: &[u8],
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<u32> {
    if !save.countries.contains_key(tag) {
        return Err(invalid(format!(
            "unknown saved country {}",
            String::from_utf8_lossy(tag)
        )));
    }
    links
        .values()
        .find(|country| country.tag == tag)
        .map(|country| country.canonical_slot)
        .ok_or_else(|| {
            invalid(format!(
                "saved country {} has no captured country link",
                String::from_utf8_lossy(tag)
            ))
        })
}

fn compare_war_relations(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut relations = BTreeMap::new();
    for relation in frame.complete::<tables::WarRelations>()? {
        let first = required_country_tag(relation.first_country, links)?;
        let second = required_country_tag(relation.second_country, links)?;
        if first == second || relations.insert((first, second), relation).is_some() {
            return Err(invalid("self or duplicate resolved ordered war relation"));
        }
        if relation.cancel > 1 || relation.first_was_instigator > 1 {
            return Err(invalid(
                "war relation has a raw flag outside 0/1; yes/no save fields cannot verify these bits",
            ));
        }
    }
    let mut saved_pairs = BTreeSet::new();
    for expected in &save.war_relations {
        let first_slot = saved_country_slot(&expected.first, save, links)?;
        let second_slot = saved_country_slot(&expected.second, save, links)?;
        if first_slot == second_slot
            || saved_country_slot(&expected.roster_owner, save, links)? != first_slot
            || saved_country_slot(&expected.target, save, links)? != second_slot
        {
            return Err(invalid(
                "saved war relation endpoints disagree with their country/target envelope or resolve to one country",
            ));
        }
        if !saved_pairs.insert((first_slot, second_slot)) {
            return Err(invalid("duplicate resolved ordered war relation in save"));
        }
        let label = format!(
            "war relation {} -> {}",
            String::from_utf8_lossy(&expected.first),
            String::from_utf8_lossy(&expected.second)
        );
        let key = (expected.first.as_slice(), expected.second.as_slice());
        let Some(actual) = relations.remove(&key) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.war_relations += 1;
        if (actual.cancel != 0) != expected.cancel {
            report.difference(format!("{label}.cancel"), actual.cancel, expected.cancel);
        }
        if let Some(start) = expected.start_date {
            let start = start.engine_hour()?;
            if actual.start_hour != start {
                report.difference(format!("{label}.start_hour"), actual.start_hour, start);
            }
        } else {
            report.limitation(
                "cancelled relation serializer omits its raw start hour",
                label.clone(),
            );
        }
        let emits_end = actual.start_hour < actual.end_hour;
        if emits_end != expected.end_date.is_some() {
            report.difference(
                format!("{label}.end_date presence (start < end)"),
                emits_end,
                expected.end_date.is_some(),
            );
        }
        if let Some(end) = expected.end_date {
            let end = end.engine_hour()?;
            if actual.end_hour != end {
                report.difference(format!("{label}.end_hour"), actual.end_hour, end);
            }
        } else {
            report.limitation("omitted relation end_date proves end_hour <= start_hour, not one exact raw end hour; the inequality is checked", label.clone());
        }
        for (field, captured, saved) in [
            (
                "first_casualties",
                actual.first_casualties,
                expected.first_casualties,
            ),
            (
                "second_casualties",
                actual.second_casualties,
                expected.second_casualties,
            ),
            (
                "first_unknown_casualties",
                actual.first_unknown_casualties,
                expected.first_unknown_casualties,
            ),
            (
                "second_unknown_casualties",
                actual.second_unknown_casualties,
                expected.second_unknown_casualties,
            ),
        ] {
            if captured != saved {
                report.difference(format!("{label}.{field}"), captured, saved);
            }
        }
        if (actual.first_was_instigator != 0) != expected.first_was_instigator {
            report.difference(
                format!("{label}.first_was_instigator"),
                actual.first_was_instigator,
                expected.first_was_instigator,
            );
        }
    }
    for (first, second) in relations.keys() {
        report.difference(
            format!(
                "war relation {} -> {}",
                String::from_utf8_lossy(first),
                String::from_utf8_lossy(second)
            ),
            "present",
            "absent",
        );
    }
    report.limitation("war rows identify ordered country relations; grouped-war identities, names and participants are not recorded", "war relations");
    report.limitation("capture includes only non-retired war relations; the save writes first-owned relations without their retirement flag, so extra saved relations still fail and are never silently filtered", "war relations");
    Ok(())
}

/// Prints the engine's own allocation counters on both sides and states whether
/// the save was written at the sampled tick. It never waives a difference: the
/// witness measures the gap, and every other comparison still fails on its own.
fn compare_boundary_counters(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let [clock] = frame.complete::<tables::Clock>()? else {
        return Err(invalid("clock table must contain one clock"));
    };
    let saved = &save.counters;
    let recorded: [(&str, Option<i64>, i64); 13] = [
        (
            "debug_current_ref_id",
            clock.debug_current_ref_id.map(i64::from),
            i64::from(saved.debug_current_ref_id),
        ),
        ("unit", clock.unit.map(i64::from), i64::from(saved.unit)),
        (
            "order_index",
            clock.order_index.map(i64::from),
            i64::from(saved.order_index),
        ),
        (
            "front_index",
            clock.front_index.map(i64::from),
            i64::from(saved.front_index),
        ),
        (
            "theatre_index",
            clock.theatre_index.map(i64::from),
            i64::from(saved.theatre_index),
        ),
        (
            "theater_group_index",
            clock.theater_group_index.map(i64::from),
            i64::from(saved.theater_group_index),
        ),
        (
            "military_deployment_line_index",
            clock.military_deployment_line_index.map(i64::from),
            i64::from(saved.military_deployment_line_index),
        ),
        (
            "military_deployment_conveyor_index",
            clock.military_deployment_conveyor_index.map(i64::from),
            i64::from(saved.military_deployment_conveyor_index),
        ),
        (
            "equipment_variant_index",
            clock.equipment_variant_index.map(i64::from),
            i64::from(saved.equipment_variant_index),
        ),
        (
            "country_leader_index",
            clock.country_leader_index.map(i64::from),
            i64::from(saved.country_leader_index),
        ),
        (
            "navy_id",
            clock.navy_id.map(i64::from),
            i64::from(saved.navy_id),
        ),
        (
            "land_combat_id",
            clock.land_combat_id.map(i64::from),
            i64::from(saved.land_combat_id),
        ),
        (
            "multiplayer_random_count",
            clock.multiplayer_random_count.map(i64::from),
            i64::from(saved.multiplayer_random_count),
        ),
    ];
    let mut unreadable = 0;
    let mut advanced = Vec::new();
    let mut regressed = Vec::new();
    for (name, captured, saved) in recorded {
        match captured {
            Some(captured) => {
                let gap = saved - captured;
                report.boundary_witness.push(format!(
                    "{name}: recorded {captured}, save {saved}, gap {gap:+}"
                ));
                match gap.cmp(&0) {
                    std::cmp::Ordering::Greater => advanced.push(format!("{name} {gap:+}")),
                    std::cmp::Ordering::Less => regressed.push(format!("{name} {gap:+}")),
                    std::cmp::Ordering::Equal => {}
                }
            }
            None => {
                unreadable += 1;
                report
                    .boundary_witness
                    .push(format!("{name}: recorded unavailable, save {saved}"));
            }
        }
    }
    // A witness count, not an exact field comparison: the gap is printed and
    // explained rather than turned into a difference.
    report.compared(
        "boundary counter witnesses (printed, never a difference)",
        recorded.len() - unreadable,
    );
    let verdict = if unreadable == recorded.len() {
        "no counter was recorded at the sampled tick, so the boundary is not measured; this comparison establishes nothing about when the save was written".to_string()
    } else if advanced.is_empty() && regressed.is_empty() {
        format!(
            "every one of the {} recorded counters equals the save header, so the save was written at the sampled tick with no intervening engine allocation",
            recorded.len() - unreadable
        )
    } else {
        let mut verdict = format!(
            "the save was written after further engine work: {} counter(s) advanced ({})",
            advanced.len(),
            advanced.join(", ")
        );
        if !regressed.is_empty() {
            verdict.push_str(&format!(
                "; {} counter(s) are lower in the save ({}), which a monotonic counter cannot do without a reload or a 32-bit wrap, so neither direction is treated as a difference",
                regressed.len(),
                regressed.join(", ")
            ));
        }
        verdict
    };
    if unreadable != 0 && unreadable != recorded.len() {
        report.limitation(
            "the collector could not read some allocation counters at the sampled tick, so the boundary witness covers only the counters it did read",
            format!("{unreadable} of {} counters", recorded.len()),
        );
    }
    report.boundary_verdict = Some(verdict);
    report.limitation(
        "the boundary witness measures engine allocation between the sampled tick and the save; it never waives a difference and an equal witness does not prove that no non-allocating command ran",
        "boundary counters",
    );
    Ok(())
}

/// One row per pair whose cached opinion dword is nonzero, which is exactly the
/// native emission rule, so an absent saved `cached_sum` is an exact zero and an
/// unexpected recorded row is a difference rather than a silent extra. A country
/// holds a relation with itself: the 2026-09-11 live save serializes a self
/// `active_relations` entry with its own `cached_sum` for 93 of its 439
/// countries, and the collector reads the matching self rows, so a self pair is
/// ordinary state here and only a repeated resolved pair is uncomparable.
fn compare_relation_opinions(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut opinions = BTreeMap::new();
    for row in frame.complete::<tables::RelationOpinions>()? {
        let owner = required_country_tag(row.owner_country, links)?;
        let target = required_country_tag(row.target_country, links)?;
        if row.opinion == 0 {
            return Err(invalid(
                "recorded relation opinion is zero; the native serializer omits a zero cached_sum",
            ));
        }
        if opinions.insert((owner, target), row.opinion).is_some() {
            return Err(invalid("duplicate resolved relation opinion pair"));
        }
    }
    let mut saved_pairs = BTreeSet::new();
    for expected in &save.relation_opinions {
        let owner_slot = saved_country_slot(&expected.owner, save, links)?;
        let target_slot = saved_country_slot(&expected.target, save, links)?;
        if !saved_pairs.insert((owner_slot, target_slot)) {
            return Err(invalid("saved relation opinion repeats a resolved pair"));
        }
        let label = format!(
            "relation opinion {} -> {}",
            String::from_utf8_lossy(&expected.owner),
            String::from_utf8_lossy(&expected.target)
        );
        let key = (expected.owner.as_slice(), expected.target.as_slice());
        match opinions.remove(&key) {
            Some(actual) => {
                report.compared("RelationOpinions", 1);
                if actual != expected.cached_sum {
                    report.difference(format!("{label}.cached_sum"), actual, expected.cached_sum);
                }
            }
            None => report.difference(&label, "absent", "present"),
        }
    }
    for (owner, target) in opinions.keys() {
        report.difference(
            format!(
                "relation opinion {} -> {}",
                String::from_utf8_lossy(owner),
                String::from_utf8_lossy(target)
            ),
            "present",
            "absent",
        );
    }
    report.limitation(
        "cached opinion is the engine's summed cache; the per-modifier opinion list is not recorded, so an equal sum does not verify its contributing modifiers",
        "RelationOpinions",
    );
    Ok(())
}

/// Typed relation tuples join the save on the native serializable-vector ordinal
/// and on the class's own token spelling from the recorded lexical dictionary.
fn compare_relation_tuples(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut tuples = BTreeMap::new();
    for row in frame.complete::<tables::RelationTuples>()? {
        let owner = required_country_tag(row.owner_country, links)?;
        let target = required_country_tag(row.target_country, links)?;
        if owner == target {
            return Err(invalid(
                "recorded relation tuple endpoints resolve to one country",
            ));
        }
        if tuples.insert((owner, target, row.ordinal), row).is_some() {
            return Err(invalid("duplicate resolved relation tuple ordinal"));
        }
        if row.cancel > 1 {
            return Err(invalid(
                "relation tuple has a raw cancel byte outside 0/1; a yes/no save field cannot verify these bits",
            ));
        }
    }
    let mut unnamed = 0;
    for expected in &save.relation_tuples {
        saved_country_slot(&expected.owner, save, links)?;
        saved_country_slot(&expected.target, save, links)?;
        let ordinal = u16::try_from(expected.ordinal).map_err(|_| {
            invalid("saved relation ordinal exceeds the recorded ordinal's wire width")
        })?;
        let label = format!(
            "relation {} {} -> {} ordinal {}",
            String::from_utf8_lossy(&expected.kind),
            String::from_utf8_lossy(&expected.owner),
            String::from_utf8_lossy(&expected.target),
            expected.ordinal
        );
        let key = (
            expected.owner.as_slice(),
            expected.target.as_slice(),
            ordinal,
        );
        let Some(actual) = tuples.remove(&key) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.compared("RelationTuples", 1);
        match tokens.get(&actual.type_id) {
            Some(&spelling) => {
                if spelling != expected.kind.as_slice() {
                    report.difference(
                        format!("{label}.kind"),
                        String::from_utf8_lossy(spelling).into_owned(),
                        String::from_utf8_lossy(&expected.kind).into_owned(),
                    );
                }
            }
            None => unnamed += 1,
        }
        for (field, captured, saved) in [
            (
                "first",
                required_country_tag(actual.first_country, links)?,
                expected.first.as_slice(),
            ),
            (
                "second",
                required_country_tag(actual.second_country, links)?,
                expected.second.as_slice(),
            ),
        ] {
            if captured != saved {
                report.difference(
                    format!("{label}.{field}"),
                    String::from_utf8_lossy(captured).into_owned(),
                    String::from_utf8_lossy(saved).into_owned(),
                );
            }
        }
        if (actual.cancel != 0) != expected.cancel {
            report.difference(format!("{label}.cancel"), actual.cancel, expected.cancel);
        }
        if let Some(start) = expected.start_date {
            let start = start.engine_hour()?;
            if actual.start_hour != start {
                report.difference(format!("{label}.start_hour"), actual.start_hour, start);
            }
        } else {
            report.limitation(
                "cancelled relation serializer omits its raw start hour",
                label.clone(),
            );
        }
        let emits_end = actual.start_hour < actual.end_hour;
        if emits_end != expected.end_date.is_some() {
            report.difference(
                format!("{label}.end_date presence (start < end)"),
                emits_end,
                expected.end_date.is_some(),
            );
        }
        if let Some(end) = expected.end_date {
            let end = end.engine_hour()?;
            if actual.end_hour != end {
                report.difference(format!("{label}.end_hour"), actual.end_hour, end);
            }
        } else {
            report.limitation(
                "omitted relation end_date proves end_hour <= start_hour, not one exact raw end hour; the inequality is checked",
                label.clone(),
            );
        }
    }
    for ((owner, target, ordinal), row) in &tuples {
        report.difference(
            format!(
                "relation type {} {} -> {} ordinal {ordinal}",
                row.type_id,
                String::from_utf8_lossy(owner),
                String::from_utf8_lossy(target)
            ),
            "present",
            "absent",
        );
    }
    if unnamed != 0 {
        report.limitation(
            "the recorded lexical dictionary has no spelling for some relation type tokens, so those rows compare their endpoints, hours and flags but not their class name",
            format!("{unnamed} relation tuples"),
        );
    }
    report.limitation(
        "the retired byte is not serialized, and CWarRelation and CSubjectRelation rows belong to the WarRelations and Subjects tables while still occupying a native ordinal",
        "RelationTuples",
    );
    Ok(())
}

/// The four raw lanes compare exactly: record writer 141085bf0 prints each lane
/// through the fixed5 path and omits it only when the raw qword is zero, so an
/// omitted lane is an exact zero rather than an unknown. The save writes the
/// list positionally and the recorded key is the same native index, so the join
/// is index to index and needs no country resolution on the target side.
fn compare_country_intel(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded: BTreeMap<&[u8], BTreeMap<u32, &replay::rows::CountryIntelRow>> =
        BTreeMap::new();
    for row in frame.complete::<tables::CountryIntel>()? {
        let owner = required_country_tag(row.owner_country, links)?;
        if recorded
            .entry(owner)
            .or_default()
            .insert(row.target_index, row)
            .is_some()
        {
            return Err(invalid("duplicate resolved country intel record index"));
        }
    }
    for (tag, records) in &save.country_intel {
        saved_country_slot(tag, save, links)?;
        let owner = String::from_utf8_lossy(tag).into_owned();
        let mut captured = recorded.remove(tag.as_slice()).unwrap_or_default();
        for (index, expected) in records.iter().enumerate() {
            let index = u32::try_from(index)
                .map_err(|_| invalid("saved country intel index exceeds its wire width"))?;
            let label = format!("{owner}.intel[{index}]");
            let Some(actual) = captured.remove(&index) else {
                report.difference(&label, "absent", "present");
                continue;
            };
            report.compared("CountryIntel", 1);
            for (field, captured, saved) in [
                ("civilian", actual.civilian, expected.civilian),
                ("army", actual.army, expected.army),
                ("navy", actual.navy, expected.navy),
                ("airforce", actual.air, expected.air),
            ] {
                if captured != saved {
                    report.difference(format!("{label}.{field}"), captured, saved);
                }
            }
        }
        for index in captured.keys() {
            report.difference(format!("{owner}.intel[{index}]"), "present", "absent");
        }
    }
    for (tag, records) in &recorded {
        report.difference(
            format!(
                "{}.intel ({} records)",
                String::from_utf8_lossy(tag),
                records.len()
            ),
            "present",
            "absent",
        );
    }
    report.limitation(
        "the native list writer emits nothing for a country whose record count is not positive, so an absent saved list means no records at all, which is a different fact from a list of all-zero records",
        "CountryIntel",
    );
    Ok(())
}

fn compare_factions(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut factions = BTreeMap::new();
    for faction in frame.complete::<tables::Factions>()? {
        if faction.id == 0 || factions.insert(faction.id, faction).is_some() {
            return Err(invalid("null or duplicate full faction handle in replay"));
        }
    }
    let mut members: BTreeMap<u64, BTreeMap<u32, &[u8]>> = BTreeMap::new();
    let mut member_countries = BTreeSet::new();
    for member in frame.complete::<tables::FactionMembers>()? {
        let tag = required_country_tag(member.country, links)?;
        if !factions.contains_key(&member.faction) {
            return Err(invalid("faction member references an absent faction"));
        }
        if !member_countries.insert(tag)
            || members
                .entry(member.faction)
                .or_default()
                .insert(member.ordinal, tag)
                .is_some()
        {
            return Err(invalid(
                "duplicate resolved country or ordinal in faction members",
            ));
        }
    }
    for ordered in members.values() {
        if ordered
            .keys()
            .enumerate()
            .any(|(index, &ordinal)| index != ordinal as usize)
        {
            return Err(invalid(
                "faction member ordinals must be contiguous from zero",
            ));
        }
    }
    let mut saved_members = BTreeSet::new();
    for (&id, expected) in &save.factions {
        if id == 0 || id != expected.id {
            return Err(invalid("invalid saved full faction identity"));
        }
        for tag in &expected.members {
            if !saved_members.insert(saved_country_slot(tag, save, links)?) {
                return Err(invalid(
                    "duplicate resolved country in saved faction membership",
                ));
            }
        }
        let label = format!("faction type={} id={}", id as u32, id >> 32);
        let Some(actual) = factions.remove(&id) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.factions += 1;
        if actual.name != expected.name {
            report.difference(
                format!("{label}.literal_name"),
                &actual.name,
                &expected.name,
            );
        }
        let actual_members = members.remove(&id).unwrap_or_default();
        for (ordinal, expected_tag) in expected.members.iter().enumerate() {
            let ordinal_key = u32::try_from(ordinal)
                .map_err(|_| invalid("saved faction membership exceeds u32 ordinals"))?;
            let actual_tag = actual_members.get(&ordinal_key).copied();
            if actual_tag.is_some() {
                report.faction_members += 1;
            }
            if actual_tag != Some(expected_tag.as_slice()) {
                report.difference(
                    format!("{label}.members[{ordinal}]"),
                    tag_text(actual_tag),
                    tag_text(Some(expected_tag)),
                );
            }
        }
        for (&ordinal, &tag) in &actual_members {
            if ordinal as usize >= expected.members.len() {
                report.difference(
                    format!("{label}.members[{ordinal}]"),
                    tag_text(Some(tag)),
                    "absent",
                );
            }
        }
    }
    for id in factions.keys() {
        report.difference(
            format!("faction type={} id={}", *id as u32, id >> 32),
            "present",
            "absent",
        );
    }
    report.limitation("faction names compare only literal bytes; templates, localization fallback and rendered friendly names are not recorded", "factions");
    Ok(())
}

fn compare_divisions(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut divisions = BTreeMap::new();
    for division in frame.complete::<tables::Divisions>()? {
        if division.id == 0 || divisions.insert(division.id, division).is_some() {
            return Err(invalid("null or duplicate full division handle in replay"));
        }
    }
    for (&id, expected) in &save.divisions {
        let label = format!("division type={} id={}", id as u32, id >> 32);
        for tag in [
            Some(&expected.roster_owner),
            Some(&expected.logical_owner),
            expected.expeditionary_owner.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !save.countries.contains_key(tag) {
                return Err(invalid(format!(
                    "{label} references unknown saved country {}",
                    String::from_utf8_lossy(tag)
                )));
            }
        }
        if expected
            .province
            .is_some_and(|province| province == 0 || !membership.provinces.contains_key(&province))
        {
            return Err(invalid(format!(
                "{label} location is absent from the verified map or is its sentinel"
            )));
        }
        let Some(actual) = divisions.remove(&id) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.divisions += 1;
        for (field, captured, saved) in [
            (
                "roster_owner",
                actual.roster_owner,
                Some(expected.roster_owner.as_slice()),
            ),
            (
                "logical_owner",
                actual.logical_owner,
                Some(expected.logical_owner.as_slice()),
            ),
            (
                "expeditionary_owner",
                actual.expeditionary_owner,
                expected.expeditionary_owner.as_deref(),
            ),
        ] {
            // BF7010 drops signed-nonpositive expeditionary tags. BF4E40
            // clears to zero, but a negative raw value cannot be verified as
            // that null. Refuse it instead of reporting an exact comparison.
            if field == "expeditionary_owner" && (captured as i32) < 0 {
                return Err(invalid(format!(
                    "{label}.expeditionary_owner has invalid signed country reference {}; the save omits nonpositive references and cannot verify these bits",
                    captured as i32
                )));
            }
            let captured = country_tag(captured, links)?;
            if captured != saved {
                report.difference(
                    format!("{label}.{field}"),
                    tag_text(captured),
                    tag_text(saved),
                );
            }
        }
        if actual.province != expected.province.unwrap_or(0) {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        if actual.template != expected.template.unwrap_or(0) {
            report.difference(
                format!("{label}.template"),
                actual.template,
                expected.template,
            );
        } else if expected.template.is_none() {
            report.limitation("division has a null standard template and the save omits its template reference; no fake-intel template was substituted", label.clone());
        }
        report.scalar(
            &label,
            "hit_points (save strength)",
            Some(actual.hit_points),
            Some(&expected.hit_points),
        );
        report.scalar(
            &label,
            "organisation",
            Some(actual.organisation),
            Some(&expected.organisation),
        );
    }
    for id in divisions.keys() {
        report.difference(
            format!("division type={} id={}", *id as u32, id >> 32),
            "present",
            "absent",
        );
    }
    Ok(())
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

fn convoy_total(
    country: &save::Country,
    save: &Save,
    types: &save::EquipmentTypes,
) -> Result<i64, String> {
    let pool = country
        .convoy_equipment
        .as_ref()
        .ok_or("save lacks the convoy equipment pool")?;
    let mut total = 0;
    for entry in pool {
        let definition = save
            .equipment_definitions
            .get(&(entry.id, entry.id_type))
            .ok_or_else(|| {
                format!(
                    "save lacks equipment definition ({}, {})",
                    entry.id, entry.id_type
                )
            })?;
        let convoy = types.is_convoy(definition).ok_or_else(|| {
            format!(
                "verified content cannot classify equipment {}: {}",
                String::from_utf8_lossy(definition),
                types
                    .limitations
                    .get(definition)
                    .map(String::as_str)
                    .unwrap_or("definition absent")
            )
        })?;
        if convoy {
            total = convoy_add(total, entry.amount.raw);
        }
    }
    Ok(total)
}

fn convoy_add(total: i64, amount: i64) -> i64 {
    (i128::from(total) + i128::from(amount))
        .clamp(-i128::from(CONVOY_LIMIT), i128::from(CONVOY_LIMIT)) as i64
}

fn default_game_directory() -> io::Result<PathBuf> {
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
mod reporting_tests {
    use super::*;

    #[test]
    fn every_difference_prints_under_its_own_comparison_group() {
        let mut report = Report::default();
        for group in ["alpha", "beta"] {
            report.attempt(group, |report| {
                for index in 0..80 {
                    report.difference(format!("{group}.field{index}"), index, index + 1);
                }
                Ok(())
            });
        }
        assert_eq!(report.differences.len(), 160);
        assert_eq!(report.difference_owners.len(), 160);
        let lines = report.difference_lines();
        // One header, one count line per group, and every single difference.
        assert_eq!(lines.len(), 1 + 2 + 160);
        assert!(lines[0].contains("All 160 differences"));
        assert!(lines.contains(&"  alpha: 80 differences".to_owned()));
        assert!(lines.contains(&"  beta: 80 differences".to_owned()));
        for group in ["alpha", "beta"] {
            for index in 0..80 {
                let expected = format!(
                    "    {group}.field{index}: replay {index}, save {}",
                    index + 1
                );
                assert!(lines.contains(&expected), "{expected} was not printed");
            }
        }
        assert_eq!(
            report.difference_groups,
            BTreeMap::from([("alpha".to_owned(), 80), ("beta".to_owned(), 80)])
        );
    }

    #[test]
    fn a_report_without_differences_prints_no_difference_section() {
        assert!(Report::default().difference_lines().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::Metrics,
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, DivisionRow, FactionMemberRow, FactionRow,
            ProvinceRow, StateRow, WarRelationRow,
        },
    };

    fn compare(
        frame: &DecodedFrame,
        save: &Save,
        membership: &save::MapMembership,
        equipment: &save::EquipmentTypes,
    ) -> io::Result<Report> {
        compare_core(frame, save, Ok(membership), Ok(equipment))
    }

    fn comparison_error(result: io::Result<Report>) -> String {
        match result {
            Err(error) => error.to_string(),
            Ok(report) => {
                assert!(report.outcome().is_err());
                report
                    .failed_comparisons
                    .values()
                    .chain(
                        report
                            .recorded_tables
                            .values()
                            .filter_map(|rows| rows.as_ref().err()),
                    )
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }

    fn fixture() -> (
        DecodedFrame,
        Save,
        save::MapMembership,
        save::EquipmentTypes,
    ) {
        let save = save::parse(br#"HOI4txt
date="1936.1.1.1" game_unique_id="campaign" game_unique_seed=7 debug_current_ref_id=65580 unit=4008 order_index=7484 front_index=8230 theatre_index=112 theater_group_index=910 military_deployment_line_index=7783 military_deployment_conveyor_index=736 equipment_variant_index=6284 country_leader_index=1 navy_id=8184 land_combat_id=15224 multiplayer_random_count=-1334833904 version="build"
states={1={owner="GER"}} provinces={2={controller="GER"}}
countries={GER={politics={political_power=10.00001} command_power=2 stability=0.7 war_support=0.3 convoys={equipment={}}}}
"#).unwrap();
        let hour = save.date.engine_hour().unwrap();
        let frame = DecodedFrame {
            branch: 2,
            hour,
            metrics: Metrics::default(),
            tables: {
                let mut tables = std::array::from_fn(|index| {
                    TableView::Complete(TableRows::empty(TableId::ALL[index]))
                });
                for (index, table) in [
                    TableView::Complete(TableRows::Clock(vec![ClockRow {
                        hour,
                        seed: 7,
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                        id: b"campaign".to_vec(),
                    }])),
                    TableView::Complete(TableRows::Countries(vec![CountryRow {
                        id: 101,
                        canonical_slot: 7,
                        tag: b"GER".to_vec(),
                        name: b"Germany".to_vec(),
                        political_power_raw: Some(1_000_001),
                        command_power_raw: 200_000,
                        stability_raw: 70_000,
                        war_support_raw: 30_000,
                        convoys_raw: Some(0),
                    }])),
                    TableView::Complete(TableRows::CountryLinks(vec![
                        CountryLinkRow {
                            id: 101,
                            canonical_slot: 7,
                        },
                        CountryLinkRow {
                            id: 102,
                            canonical_slot: 7,
                        },
                    ])),
                    TableView::Complete(TableRows::Provinces(vec![
                        ProvinceRow {
                            id: 1,
                            state: 1,
                            owner: 102,
                            controller: 0,
                            victory_points: 0,
                        },
                        ProvinceRow {
                            id: 2,
                            state: 0,
                            owner: 0,
                            controller: 101,
                            victory_points: 0,
                        },
                    ])),
                    TableView::Complete(TableRows::States(vec![StateRow {
                        id: 1,
                        owner: 102,
                        controller: 101,
                    }])),
                    TableView::Complete(TableRows::Divisions(Vec::new())),
                    TableView::Complete(TableRows::WarRelations(Vec::new())),
                    TableView::Complete(TableRows::Factions(Vec::new())),
                    TableView::Complete(TableRows::FactionMembers(Vec::new())),
                ]
                .into_iter()
                .enumerate()
                {
                    tables[index] = table;
                }
                tables[TableId::HumanControllers.index()] =
                    TableView::Complete(TableRows::HumanControllers(vec![
                        replay::rows::HumanControllerRow {
                            country: 101,
                            count: 0,
                        },
                    ]));
                tables[TableId::SunkShips.index()] =
                    TableView::Complete(TableRows::SunkShips(vec![
                        replay::rows::SunkShipsRow::default(),
                    ]));
                tables[TableId::WeatherLimits.index()] =
                    TableView::Complete(TableRows::WeatherLimits(vec![
                        replay::rows::WeatherLimitsRow::default(),
                    ]));
                tables
            },
        };
        let membership = save::MapMembership {
            provinces: BTreeMap::from([(0, None), (1, Some(1)), (2, None)]),
        };
        (frame, save, membership, save::EquipmentTypes::default())
    }

    fn add_country(frame: &mut DecodedFrame, save: &mut Save, id: u32, slot: u32, tag: &[u8]) {
        let mut saved = save.countries[b"GER".as_slice()].clone();
        saved.tag = tag.to_vec();
        save.countries.insert(tag.to_vec(), saved);
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        let mut row = rows[0].clone();
        row.id = id;
        row.canonical_slot = slot;
        row.tag = tag.to_vec();
        rows.push(row);
        let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut frame.tables[TableId::CountryLinks.index()]
        else {
            unreachable!()
        };
        rows.push(CountryLinkRow {
            id,
            canonical_slot: slot,
        });
        let TableView::Complete(TableRows::HumanControllers(rows)) =
            &mut frame.tables[TableId::HumanControllers.index()]
        else {
            unreachable!()
        };
        rows.push(replay::rows::HumanControllerRow {
            country: id,
            count: 0,
        });
    }

    fn division_fixture() -> (
        DecodedFrame,
        Save,
        save::MapMembership,
        save::EquipmentTypes,
    ) {
        let (mut frame, mut save, membership, equipment) = fixture();
        add_country(&mut frame, &mut save, 201, 8, b"D01");
        add_country(&mut frame, &mut save, 301, 9, b"SOV");
        let id = (2_147_483_649_u64 << 32) | 4919;
        let template = (u64::from(u32::MAX) << 32) | 6000;
        save.divisions.insert(
            id,
            save::Division {
                id,
                roster_owner: b"GER".to_vec(),
                logical_owner: b"D01".to_vec(),
                expeditionary_owner: Some(b"SOV".to_vec()),
                province: Some(1),
                template: Some(template),
                hit_points: Decimal::parse(b"90071992547.40993").unwrap(),
                organisation: Decimal::parse(b"53.45454").unwrap(),
                country_intel: Vec::new(),
            },
        );
        frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id,
                roster_owner: 102,
                logical_owner: 201,
                expeditionary_owner: 301,
                province: 1,
                template,
                hit_points: 9_007_199_254_740_993,
                organisation: 5_345_454,
                orders_group: 0,
            }]));
        (frame, save, membership, equipment)
    }

    fn division_row_mut(frame: &mut DecodedFrame) -> &mut DivisionRow {
        let TableView::Complete(TableRows::Divisions(rows)) =
            &mut frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
    }

    fn relations_fixture() -> (
        DecodedFrame,
        Save,
        save::MapMembership,
        save::EquipmentTypes,
    ) {
        let (mut frame, mut save, membership, equipment) = fixture();
        add_country(&mut frame, &mut save, 201, 8, b"ENG");
        let start = save::GameDate::parse(b"1935.6.1.1").unwrap();
        save.war_relations.push(save::WarRelation {
            roster_owner: b"GER".to_vec(),
            target: b"ENG".to_vec(),
            first: b"GER".to_vec(),
            second: b"ENG".to_vec(),
            start_date: Some(start),
            end_date: None,
            cancel: false,
            first_casualties: 9_007_199_254_740_993,
            second_casualties: i64::MAX,
            first_unknown_casualties: i64::MIN,
            second_unknown_casualties: -17,
            first_was_instigator: false,
        });
        frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: 102,
                second_country: 201,
                start_hour: start.engine_hour().unwrap(),
                end_hour: 43_808_760,
                cancel: 0,
                first_casualties: 9_007_199_254_740_993,
                second_casualties: i64::MAX,
                first_unknown_casualties: i64::MIN,
                second_unknown_casualties: -17,
                first_was_instigator: 0,
            }]));
        // Both partial-zero handles are valid; do not collapse them to null or a shared type.
        save.factions.insert(
            88,
            save::Faction {
                id: 88,
                name: Vec::new(),
                members: vec![b"ENG".to_vec(), b"GER".to_vec()],
            },
        );
        save.factions.insert(
            1_u64 << 63,
            save::Faction {
                id: 1_u64 << 63,
                name: b"Literal".to_vec(),
                members: Vec::new(),
            },
        );
        frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![
            FactionRow {
                id: 88,
                name: Vec::new(),
            },
            FactionRow {
                id: 1_u64 << 63,
                name: b"Literal".to_vec(),
            },
        ]));
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 102,
                    faction: 88,
                    ordinal: 1,
                },
                FactionMemberRow {
                    country: 201,
                    faction: 88,
                    ordinal: 0,
                },
            ]));
        (frame, save, membership, equipment)
    }

    fn war_row_mut(frame: &mut DecodedFrame) -> &mut WarRelationRow {
        let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut frame.tables[TableId::WarRelations.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
    }

    #[test]
    fn table_counts_collect_all_issues_and_unproved_sources_remain_incomplete() {
        let (mut frame, _, _, _) = fixture();
        frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "global lexer writer is not proved".into(),
        };
        frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
            reason: "script scope roots are not proved".into(),
        };
        let counts = recorded_table_counts(&frame);
        assert_eq!(counts.len(), TableId::ALL.len());
        assert!(counts[&TableId::LexicalTokens].is_err());
        assert!(counts[&TableId::ScriptVariables].is_err());
        assert_eq!(counts[&TableId::Countries], Ok(1));
        let mut report = Report::default();
        report.record_tables(&frame);
        assert!(lexical_tokens(&frame, &mut report).unwrap().is_empty());
        assert!(report.limitations.values().any(|subjects| {
            subjects
                .iter()
                .any(|subject| subject == "global lexer writer is not proved")
        }));

        frame.tables[TableId::Stockpiles.index()] = TableView::Unavailable {
            reason: "failed native walk".into(),
        };
        let counts = recorded_table_counts(&frame);
        let error = counts[&TableId::Stockpiles].as_ref().unwrap_err();
        assert!(error.contains("Stockpiles") && error.contains("failed native walk"));
        assert_eq!(counts.values().filter(|count| count.is_err()).count(), 3);
        assert!(report.differences.is_empty());
        assert!(
            report
                .outcome()
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
        frame.tables[TableId::Stockpiles.index()] =
            TableView::Complete(TableRows::Stockpiles(Vec::new()));
        frame.tables[TableId::LexicalTokens.index()] = TableView::Truncated {
            rows: TableRows::LexicalTokens(Vec::new()),
            missing: 7,
        };
        let counts = recorded_table_counts(&frame);
        assert!(
            counts[&TableId::LexicalTokens]
                .as_ref()
                .unwrap_err()
                .contains("7 missing")
        );
        assert!(lexical_tokens(&frame, &mut report).is_err());
    }

    #[test]
    fn lexical_dictionary_preserves_exact_bytes_and_rejects_duplicate_or_wrong_tables() {
        use replay::rows::LexicalTokenRow;

        let (mut frame, _, _, _) = fixture();
        let entry = LexicalTokenRow {
            token: u32::MAX,
            literal: b"name\0suffix".to_vec(),
        };
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 0,
                    literal: Vec::new(),
                },
                entry.clone(),
            ]));
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        assert_eq!(tokens[&0], b"");
        assert_eq!(tokens[&u32::MAX], b"name\0suffix");
        assert!(report.limitations.is_empty());
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![entry.clone(), entry]));
        assert!(lexical_tokens(&frame, &mut report).is_err());
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::Stockpiles(Vec::new()));
        assert!(lexical_tokens(&frame, &mut report).is_err());
        assert!(recorded_table_counts(&frame)[&TableId::LexicalTokens].is_err());
    }

    #[test]
    fn nested_comparison_counts_assign_each_difference_once_and_retain_failure() {
        let mut report = Report::default();
        assert!(report.attempt("outer", |report| {
            report.difference("before", 1, 2);
            assert!(!report.attempt("inner", |report| {
                report.difference("inside", 3, 4);
                Err(invalid("independent row failure"))
            }));
            report.difference("after", 5, 6);
            Ok(())
        }));
        report.difference("direct", 7, 8);
        assert_eq!(report.difference_groups["outer"], 2);
        assert_eq!(report.difference_groups["inner"], 1);
        assert_eq!(report.difference_groups["direct comparisons"], 1);
        assert_eq!(
            report.difference_groups.values().sum::<usize>(),
            report.differences.len()
        );
        assert_eq!(
            report.failed_comparisons["inner"],
            "independent row failure"
        );
        assert!(report.active_comparison.is_none());
        assert!(report.outcome().is_err());
    }

    #[test]
    fn failed_content_reads_preserve_independent_map_and_pack_diagnostics() {
        let (frame, saved, membership, equipment) = fixture();
        let mut report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.outcome().is_ok());
        compare_extended(
            &frame,
            &saved,
            Ok(&membership),
            Err("history/states/101-Greenland.txt: malformed resource value".into()),
            Err("map/provinces.bmp: malformed adjacency input".into()),
            &mut report,
        );
        assert_eq!(report.countries, 1);
        assert_eq!(report.provinces, 2);
        assert!(report.failed_comparisons["content base resources"].contains("101-Greenland.txt"));
        assert!(
            report.failed_comparisons["construction StateBaseResources"]
                .contains("101-Greenland.txt")
        );
        assert!(report.compared_tables.contains_key("WeatherLimits"));
        assert!(
            report.failed_comparisons["content railway adjacency"].contains("map/provinces.bmp")
        );
        assert!(report.failed_comparisons["RailEdges"].contains("map/provinces.bmp"));
        assert!(report.compared_tables.contains_key("SunkConvoys"));
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("weather.provinces"))
        );
        assert!(report.outcome().is_err());

        let (mut frame, mut saved, _, _) = fixture();
        add_country(&mut frame, &mut saved, 202, 9, b"ENG");
        let report = compare_core(
            &frame,
            &saved,
            Err("state membership is malformed".into()),
            Err("equipment definition is malformed".into()),
        )
        .unwrap();
        assert_eq!(report.countries, 2);
        assert_eq!(report.scalar_checks, 8);
        assert_eq!(report.provinces, 0);
        assert!(report.failed_comparisons["provinces"].contains("state membership is malformed"));
        assert!(
            report.failed_comparisons["country GER convoys"]
                .contains("equipment definition is malformed")
        );
        assert!(
            report.failed_comparisons["country ENG convoys"]
                .contains("equipment definition is malformed")
        );
        assert!(report.outcome().is_err());
    }

    #[test]
    fn unavailable_sources_and_failed_groups_do_not_hide_later_independent_mismatches() {
        let (mut frame, mut saved, membership, equipment) = fixture();
        for (table, reason) in [
            (TableId::Divisions, "division read failed"),
            (TableId::ScriptVariables, "scope roots are not proved"),
            (TableId::LexicalTokens, "global lexer writer is not proved"),
        ] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: reason.into(),
            };
        }
        frame.tables[TableId::Players.index()] =
            TableView::Complete(TableRows::Players(vec![replay::rows::PlayerRow {
                id: 1,
                country: 101,
                user: b"later-player".to_vec(),
                ..Default::default()
            }]));
        let weather = installed_content::parser::parse(
            b"weather={provinces={0={province=0 temperature=0.00001}} regions={}}",
        )
        .unwrap();
        saved.weather = save::weather::parse(&weather.items).unwrap();
        frame.tables[TableId::WeatherProvinces.index()] =
            TableView::Complete(TableRows::WeatherProvinces(vec![
                replay::rows::WeatherProvinceRow {
                    id: 0,
                    temperature: 2,
                    ..Default::default()
                },
            ]));
        let mut report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.countries, 1);
        assert_eq!(report.provinces, 2);
        assert!(report.failed_comparisons["divisions"].contains("division read failed"));
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("later-player"))
        );
        compare_extended(
            &frame,
            &saved,
            Ok(&membership),
            Ok(&save::construction::BaseResources::default()),
            Err("railway content is outside this fixture".into()),
            &mut report,
        );
        assert!(report.failed_comparisons.contains_key("intel"));
        assert!(
            report.failed_comparisons["ScriptVariables"].contains("scope roots are not proved")
        );
        assert_eq!(report.compared_tables["WeatherProvinces"], 1);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("weather province 0.temperature"))
        );
        assert_eq!(report.unavailable_tables(), 3);
        assert!(
            report
                .outcome()
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );

        frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "country links failed".into(),
        };
        let mut report = compare(&frame, &saved, &membership, &equipment).unwrap();
        compare_extended(
            &frame,
            &saved,
            Ok(&membership),
            Ok(&save::construction::BaseResources::default()),
            Err("railway content is outside this fixture".into()),
            &mut report,
        );
        assert!(report.failed_comparisons["map country context"].contains("country links failed"));
        assert!(report.failed_comparisons["air"].contains("requires country links"));
        assert_eq!(report.compared_tables["WeatherProvinces"], 1);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("weather province 0.temperature"))
        );
        assert!(report.outcome().is_err());
    }

    #[test]
    fn relations_compare_full_integer_precision_aliases_and_ordered_faction_members() {
        let (mut frame, save, membership, equipment) = relations_fixture();
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            (
                report.war_relations,
                report.factions,
                report.faction_members
            ),
            (1, 2, 2)
        );
        let row = war_row_mut(&mut frame);
        row.first_casualties -= 1;
        row.second_casualties -= 1;
        row.first_unknown_casualties += 1;
        row.second_unknown_casualties += 1;
        row.first_was_instigator = 1;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 5, "{:?}", report.differences);
    }

    #[test]
    fn omitted_relation_dates_check_the_proven_writer_conditions() {
        let (mut frame, mut save, membership, equipment) = relations_fixture();
        // Omission permits any end <= start, not only the constructor sentinel.
        let row = war_row_mut(&mut frame);
        row.end_hour = row.start_hour - 1;
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .is_empty()
        );
        let row = war_row_mut(&mut frame);
        row.end_hour = row.start_hour + 1;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("end_date presence"));
        save.war_relations[0].end_date = Some(save.date);
        war_row_mut(&mut frame).end_hour = save.date.engine_hour().unwrap();
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .is_empty()
        );
        save.war_relations[0].cancel = true;
        save.war_relations[0].start_date = None;
        war_row_mut(&mut frame).cancel = 1;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("raw start hour"))
        );
        war_row_mut(&mut frame).cancel = 0;
        assert_eq!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            1
        );
    }

    #[test]
    fn war_pair_direction_missing_rows_and_saved_envelopes_are_not_guessed() {
        let (mut frame, mut save, membership, equipment) = relations_fixture();
        let row = war_row_mut(&mut frame);
        std::mem::swap(&mut row.first_country, &mut row.second_country);
        assert_eq!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            2
        );
        save.war_relations[0].roster_owner = b"ENG".to_vec();
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment)).contains("envelope")
        );
        save.war_relations[0].roster_owner = b"UNKNOWN".to_vec();
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("unknown saved country")
        );
    }

    #[test]
    fn war_flags_and_duplicate_alias_pairs_that_saves_cannot_verify_are_refused() {
        let (frame, save, membership, equipment) = relations_fixture();
        for (cancel, instigator) in [(2, 0), (0, 255)] {
            let mut changed = frame.clone();
            let row = war_row_mut(&mut changed);
            row.cancel = cancel;
            row.first_was_instigator = instigator;
            assert!(
                comparison_error(compare(&changed, &save, &membership, &equipment))
                    .contains("cannot verify these bits")
            );
        }
        let mut changed = frame.clone();
        let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut changed.tables[TableId::WarRelations.index()]
        else {
            unreachable!()
        };
        let mut duplicate = rows[0].clone();
        duplicate.first_country = 101;
        rows.push(duplicate);
        assert!(
            comparison_error(compare(&changed, &save, &membership, &equipment))
                .contains("duplicate resolved")
        );
        let mut duplicate_save = save.clone();
        duplicate_save
            .war_relations
            .push(save.war_relations[0].clone());
        assert!(
            compare(&frame, &duplicate_save, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
    }

    #[test]
    fn faction_names_handles_and_membership_order_are_exact() {
        let (mut frame, save, membership, equipment) = relations_fixture();
        let TableView::Complete(TableRows::Factions(rows)) =
            &mut frame.tables[TableId::Factions.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.name = b"Axis".to_vec();
        let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut frame.tables[TableId::FactionMembers.index()]
        else {
            unreachable!()
        };
        for row in rows {
            row.ordinal = 1 - row.ordinal;
        }
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let (mut frame, save, membership, equipment) = relations_fixture();
        let TableView::Complete(TableRows::Factions(rows)) =
            &mut frame.tables[TableId::Factions.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[1];
        row.id ^= 1;
        assert_eq!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            2
        );
        let (mut frame, save, membership, equipment) = relations_fixture();
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(Vec::new()));
        assert_eq!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            2
        );
    }

    #[test]
    fn faction_members_cannot_duplicate_aliases_skip_ordinals_or_reference_absent_factions() {
        let (frame, save, membership, equipment) = relations_fixture();
        for member in [
            FactionMemberRow {
                country: 101,
                faction: 88,
                ordinal: 2,
            },
            FactionMemberRow {
                country: 301,
                faction: 88,
                ordinal: 2,
            },
            FactionMemberRow {
                country: 102,
                faction: 777,
                ordinal: 0,
            },
        ] {
            let mut changed = frame.clone();
            let TableView::Complete(TableRows::FactionMembers(rows)) =
                &mut changed.tables[TableId::FactionMembers.index()]
            else {
                unreachable!()
            };
            rows.push(member);
            assert!(
                compare(&changed, &save, &membership, &equipment)
                    .and_then(|report| report.outcome())
                    .is_err()
            );
        }
        let mut changed = frame.clone();
        let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut changed.tables[TableId::FactionMembers.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.ordinal = 3;
        assert!(
            comparison_error(compare(&changed, &save, &membership, &equipment))
                .contains("contiguous")
        );
        let mut duplicate_save = save.clone();
        duplicate_save
            .factions
            .get_mut(&(1_u64 << 63))
            .unwrap()
            .members
            .push(b"GER".to_vec());
        assert!(
            comparison_error(compare(&frame, &duplicate_save, &membership, &equipment))
                .contains("duplicate resolved country")
        );
    }

    #[test]
    fn incomplete_or_wrong_relation_and_faction_tables_never_pass() {
        let (frame, save, membership, equipment) = relations_fixture();
        for table in [
            TableId::WarRelations,
            TableId::Factions,
            TableId::FactionMembers,
        ] {
            for view in [
                TableView::Unavailable {
                    reason: "unreadable".into(),
                },
                TableView::Truncated {
                    rows: TableRows::empty(table),
                    missing: 1,
                },
                TableView::Complete(TableRows::Provinces(vec![ProvinceRow::default()])),
            ] {
                let mut changed = frame.clone();
                changed.tables[table.index()] = view;
                assert!(
                    compare(&changed, &save, &membership, &equipment)
                        .and_then(|report| report.outcome())
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn full_comparison_resolves_aliases_and_independent_map_membership() {
        let (frame, save, membership, equipment) = fixture();
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            (
                report.provinces,
                report.states,
                report.countries,
                report.scalar_checks
            ),
            (2, 1, 1, 5)
        );
        assert!(!report.limitations.is_empty());
    }

    #[test]
    fn province_victory_points_compare_native_integer_extremes_and_proven_field_omission() {
        let (mut frame, mut saved, membership, equipment) = fixture();
        for value in [i32::MIN, -1, 0, i32::MAX] {
            saved.provinces.get_mut(&2).unwrap().victory_points = Some(value);
            let TableView::Complete(TableRows::Provinces(rows)) =
                &mut frame.tables[TableId::Provinces.index()]
            else {
                unreachable!()
            };
            rows[1].victory_points = value;
            let report = compare(&frame, &saved, &membership, &equipment).unwrap();
            assert!(report.outcome().is_ok(), "{report:?}");
            assert_eq!(report.compared_tables["Provinces.victory_points"], 2);
        }
        saved.provinces.get_mut(&2).unwrap().victory_points = None;
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        rows[1].victory_points = 1;
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1, "{report:?}");
        assert!(report.differences[0].contains("province 2.victory_points"));
        assert!(report.outcome().is_err());
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        rows[1].victory_points = 0;
        assert!(
            compare(&frame, &saved, &membership, &equipment)
                .unwrap()
                .outcome()
                .is_ok()
        );
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        rows[0].victory_points = -1; // The whole saved province entry is absent.
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("province 1.victory_points"));
    }

    #[test]
    fn missing_extra_and_changed_province_rows_are_differences() {
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        rows.remove(0);
        let province = &mut rows[0];
        // Explicit saved control cannot be manufactured from captured raw zero.
        province.controller = 0;
        rows.push(ProvinceRow {
            id: 3,
            ..Default::default()
        });
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    }

    #[test]
    fn state_ownership_and_controller_differences_are_independent_of_provinces() {
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(TableRows::States(rows)) =
            &mut frame.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        let state = &mut rows[0];
        state.owner = 0;
        state.controller = 0;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("state 1.owner:"));
        assert!(report.differences[1].starts_with("state 1.controller:"));
        assert_eq!(report.provinces, 2);
    }

    #[test]
    fn omitted_state_controller_does_not_accept_raw_zero_or_hide_missing_rows() {
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(TableRows::States(rows)) =
            &mut frame.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        let state = &mut rows[0];
        state.controller = 0;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("state 1.controller:"));

        frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 2,
                owner: 101,
                controller: 101,
            }]));
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert_eq!(report.states, 0);
    }

    #[test]
    fn explicit_state_controller_is_compared_even_when_owner_and_provinces_match() {
        let (mut frame, mut save, membership, equipment) = fixture();
        add_country(&mut frame, &mut save, 201, 8, b"D01");
        save.states.get_mut(&1).unwrap().controller = Some(b"D01".to_vec());
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("state 1.controller:"));
        let TableView::Complete(TableRows::States(rows)) =
            &mut frame.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        let state = &mut rows[0];
        state.controller = 201;
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .is_empty()
        );
    }

    #[test]
    fn invalid_incomplete_or_unresolvable_states_are_refused() {
        let (frame, save, membership, equipment) = fixture();
        for view in [
            TableView::Unavailable {
                reason: "state pointer unreadable".into(),
            },
            TableView::Truncated {
                rows: TableRows::empty(TableId::States),
                missing: 1,
            },
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 1,
                owner: 777,
                controller: 101,
            }])),
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 1,
                owner: 101,
                controller: 777,
            }])),
            TableView::Complete(TableRows::States(vec![
                StateRow::default(),
                StateRow::default(),
            ])),
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow::default()])),
        ] {
            let mut invalid = frame.clone();
            invalid.tables[TableId::States.index()] = view;
            assert!(
                compare(&invalid, &save, &membership, &equipment)
                    .and_then(|report| report.outcome())
                    .is_err()
            );
        }
    }

    #[test]
    fn division_oracle_compares_native_precision_and_three_distinct_owners() {
        let (frame, save, membership, equipment) = division_fixture();
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.divisions, 1);
        assert_eq!(report.scalar_checks, 17);
        let mut changed = frame.clone();
        let row = division_row_mut(&mut changed);
        row.hit_points -= 1;
        row.organisation += 1;
        row.roster_owner = 201;
        row.logical_owner = 301;
        row.expeditionary_owner = 101;
        row.province = 2;
        row.template ^= 1;
        let report = compare(&changed, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 7, "{:?}", report.differences);
    }

    #[test]
    fn division_type_word_is_part_of_identity_and_duplicate_full_handles_are_refused() {
        let (mut frame, save, membership, equipment) = division_fixture();
        division_row_mut(&mut frame).id ^= 1;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert_eq!(report.divisions, 0);
        let (mut frame, save, membership, equipment) = division_fixture();
        let TableView::Complete(TableRows::Divisions(rows)) =
            &mut frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        rows.push(rows[0].clone());
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("duplicate full division handle")
        );
    }

    #[test]
    fn division_optional_omissions_require_captured_nulls() {
        let (mut frame, mut save, membership, equipment) = division_fixture();
        let expected = save.divisions.values_mut().next().unwrap();
        expected.province = None;
        expected.template = None;
        expected.expeditionary_owner = None;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let row = division_row_mut(&mut frame);
        row.province = 0;
        row.template = 0;
        row.expeditionary_owner = 0;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("null standard template"))
        );
        division_row_mut(&mut frame).expeditionary_owner = u32::MAX;
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("cannot verify these bits")
        );
    }

    #[test]
    fn division_incomplete_tables_and_unresolved_references_cannot_pass() {
        let (frame, save, membership, equipment) = division_fixture();
        for table in [
            TableView::Unavailable {
                reason: "division unreadable".into(),
            },
            TableView::Truncated {
                rows: TableRows::empty(TableId::Divisions),
                missing: 1,
            },
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow::default()])),
            TableView::Complete(TableRows::Divisions(vec![DivisionRow::default()])),
        ] {
            let mut invalid = frame.clone();
            invalid.tables[TableId::Divisions.index()] = table;
            assert!(
                compare(&invalid, &save, &membership, &equipment)
                    .and_then(|report| report.outcome())
                    .is_err()
            );
        }
        let mut invalid = frame.clone();
        division_row_mut(&mut invalid).logical_owner = 777;
        assert!(
            comparison_error(compare(&invalid, &save, &membership, &equipment))
                .contains("unresolved country reference")
        );
        let mut invalid_save = save.clone();
        invalid_save
            .divisions
            .values_mut()
            .next()
            .unwrap()
            .logical_owner = b"UNKNOWN".to_vec();
        assert!(
            comparison_error(compare(&frame, &invalid_save, &membership, &equipment))
                .contains("unknown saved country")
        );
        let mut invalid_save = save;
        invalid_save.divisions.values_mut().next().unwrap().province = Some(999_999);
        assert!(
            comparison_error(compare(&frame, &invalid_save, &membership, &equipment))
                .contains("verified map")
        );
    }

    #[test]
    fn incomplete_tables_and_unresolved_links_cannot_pass() {
        let (mut frame, save, membership, equipment) = fixture();
        frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
            reason: "bad pointer".into(),
        };
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("bad pointer")
        );
        frame.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: TableRows::empty(TableId::Provinces),
            missing: 2,
        };
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut frame.tables[TableId::CountryLinks.index()]
        else {
            unreachable!()
        };
        rows.pop();
        assert!(comparison_error(compare(&frame, &save, &membership, &equipment)).contains("102"));
    }

    #[test]
    fn ambiguous_branch_and_wrong_hour_are_refused() {
        assert_eq!(unique_branch(&[2]).unwrap(), 2);
        assert!(
            unique_branch(&[1, 2])
                .unwrap_err()
                .to_string()
                .contains("--branch")
        );
        assert!(unique_branch(&[]).is_err());
        assert_eq!(matching_hour(20, None).unwrap(), 20);
        assert!(matching_hour(20, Some(21)).is_err());
    }

    #[test]
    fn fixed_point_difference_of_one_is_not_rounded_away() {
        let mut report = Report::default();
        report.scalar(
            "GER",
            "political_power",
            Some(9_007_199_254_740_992),
            Some(&Decimal::parse(b"90071992547.40993").unwrap()),
        );
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.scalar_checks, 1);
        report.scalar("GER", "command_power", Some(0), None);
        assert_eq!(report.differences.len(), 2);
        report.scalar(
            "GER",
            "political_power",
            None,
            Some(&Decimal::parse(b"1").unwrap()),
        );
        assert_eq!(report.limitations.len(), 1);
    }

    #[test]
    fn missing_convoy_data_is_an_error_when_capture_has_a_value() {
        let (frame, mut save, membership, equipment) = fixture();
        save.countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .convoy_equipment = None;
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("convoy equipment pool")
        );
        save.countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .convoy_equipment = Some(vec![save::EquipmentAmount {
            id: 7,
            id_type: 70,
            amount: Decimal::parse(b"1").unwrap(),
        }]);
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("equipment definition")
        );
        save.equipment_definitions
            .insert((7, 70), b"unknown_variant".to_vec());
        assert!(
            comparison_error(compare(&frame, &save, &membership, &equipment))
                .contains("cannot classify")
        );
    }

    #[test]
    fn convoy_clamp_is_per_row_and_preserves_order() {
        assert_eq!(convoy_add(convoy_add(0, i64::MAX), -10), CONVOY_LIMIT - 10);
        assert_eq!(convoy_add(convoy_add(0, i64::MIN), 10), -CONVOY_LIMIT + 10);
    }

    #[test]
    fn save_checksum_suffix_does_not_hide_a_different_build() {
        assert!(same_version(b"build (d245)", "build"));
        assert!(same_version(b"build", "build"));
        assert!(!same_version(b"build2 (d245)", "build"));
        assert!(!same_version(b"build (xxxx)", "build"));
    }

    #[test]
    fn player_oracle_compares_refreshed_fields_and_reports_header_limits() {
        let (mut frame, mut saved, membership, equipment) = fixture();
        frame.tables[TableId::Players.index()] =
            TableView::Complete(TableRows::Players(vec![replay::rows::PlayerRow {
                id: i32::MIN,
                country: 102,
                flags: 0x81,
                user: b"user".to_vec(),
                name: b"different display".to_vec(),
            }]));
        saved.players = vec![
            save::Player {
                id: i32::MIN,
                country: b"GER".to_vec(),
                user: b"user".to_vec(),
                country_leader: true,
            },
            save::Player {
                id: 20,
                country: b"GER".to_vec(),
                user: b"retained".to_vec(),
                country_leader: false,
            },
        ];
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.players, 1);
        assert!(
            report
                .limitations
                .keys()
                .any(|key| key.contains("retains users"))
        );
        saved.players[0].id += 1;
        saved.players[0].country_leader = false;
        assert_eq!(
            compare(&frame, &saved, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            2
        );
        saved.players[0].user = b"renamed".to_vec();
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.players, 0);
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn player_duplicate_user_collapse_never_certifies_every_current_player() {
        let (mut frame, mut saved, membership, equipment) = fixture();
        frame.tables[TableId::Players.index()] = TableView::Complete(TableRows::Players(vec![
            replay::rows::PlayerRow {
                id: 1,
                country: 101,
                user: Vec::new(),
                ..Default::default()
            },
            replay::rows::PlayerRow {
                id: 2,
                country: 0,
                user: Vec::new(),
                ..Default::default()
            },
        ]));
        saved.players = vec![save::Player {
            id: 2,
            country: b"---".to_vec(),
            user: Vec::new(),
            country_leader: false,
        }];
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.players, 0);
        assert!(
            report
                .limitations
                .keys()
                .any(|key| key.contains("collapses equal"))
        );
        saved.players[0].id = 3;
        assert_eq!(
            compare(&frame, &saved, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            1
        );
    }

    #[test]
    fn map_combat_and_order_oracle_preserves_type_words_order_and_raw_fields() {
        let (mut frame, mut saved, membership, equipment) = fixture();
        let combat = (u64::from(u32::MAX) << 32) | 62;
        let unit = (1_u64 << 32) | 51;
        saved.land_combats.insert(
            combat,
            save::LandCombat {
                id: combat,
                province: 1,
                day: -1,
                duration: 2,
                attackers: vec![unit, unit + 1],
                defenders: vec![],
            },
        );
        frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![replay::rows::LandCombatRow {
                id: combat,
                province: 1,
                day: -1,
                duration: 2,
                attackers: vec![unit, unit + 1],
                defenders: vec![],
            }]));
        let group = (u64::from(u32::MAX) << 32) | 53;
        saved.army_orders.insert(
            (group, u32::MAX),
            save::ArmyOrder {
                owner_group: group,
                instance: u32::MAX,
                country: b"GER".to_vec(),
                order_type: 4,
                active: 128,
                paradrop_target: u32::MAX,
                path: vec![2, 1, 2],
                target_states: vec![1],
            },
        );
        frame.tables[TableId::ArmyOrders.index()] =
            TableView::Complete(TableRows::ArmyOrders(vec![replay::rows::ArmyOrderRow {
                owner_group: group,
                instance: u32::MAX,
                country: 102,
                order_type: 4,
                active: 128,
                paradrop_target: u32::MAX,
                path: vec![2, 1, 2],
                target_states: vec![1],
            }]));
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!((report.land_combats, report.army_orders), (1, 1));
        saved
            .land_combats
            .get_mut(&combat)
            .unwrap()
            .attackers
            .reverse();
        let order = saved.army_orders.get_mut(&(group, u32::MAX)).unwrap();
        order.path.swap(0, 1);
        order.active = 0;
        assert_eq!(
            compare(&frame, &saved, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            3
        );
    }

    #[test]
    fn air_and_naval_oracle_compares_native_values_and_slot_identity() {
        let (mut frame, mut saved, membership, equipment) = fixture();
        let wing = (1_u64 << 32) | 54;
        saved.air_wings.insert(
            wing,
            save::AirWing {
                id: wing,
                owner: b"GER".to_vec(),
                country_override: b"---".to_vec(),
                air_base: 63,
                planes: 100,
                effective_planes: 57,
                mission_mask: 2,
                mission_region: Some(3),
            },
        );
        frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![replay::rows::AirWingRow {
                id: wing,
                owner: 102,
                country_override: 0,
                air_base: 63,
                base_override: 99,
                planes: 100,
                effective_planes: 57,
                mission_mask: 2,
                mission_region: Some(3),
            }]));
        let combat = (1_u64 << 32) | 61;
        let zero = Decimal::parse(b"0").unwrap();
        saved.naval_combats.insert(
            combat,
            save::NavalCombat {
                id: combat,
                province: 2,
                attacker_positioning: Decimal::parse(b"0.00001").unwrap(),
                defender_positioning: zero.clone(),
                attacker_anti_air: zero.clone(),
                defender_anti_air: zero,
            },
        );
        frame.tables[TableId::NavalCombats.index()] =
            TableView::Complete(TableRows::NavalCombats(vec![
                replay::rows::NavalCombatRow {
                    id: combat,
                    province: 2,
                    attacker_positioning: 1,
                    ..Default::default()
                },
            ]));
        saved.naval_combat_participants.insert(
            (combat, 1, u32::MAX),
            save::NavalCombatParticipant {
                combat,
                side: 1,
                battle_slot: u32::MAX,
                ship: None,
                convoy: Some(69),
                owner: b"GER".to_vec(),
                state: 32,
            },
        );
        frame.tables[TableId::NavalCombatParticipants.index()] =
            TableView::Complete(TableRows::NavalCombatParticipants(vec![
                replay::rows::NavalCombatParticipantRow {
                    combat,
                    side: 1,
                    battle_slot: u32::MAX,
                    ship: 0,
                    owner: 101,
                    state: 32,
                },
            ]));
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            (
                report.air_wings,
                report.naval_combats,
                report.naval_combat_participants
            ),
            (1, 1, 1)
        );
        saved.air_wings.get_mut(&wing).unwrap().effective_planes -= 1;
        saved
            .naval_combats
            .get_mut(&combat)
            .unwrap()
            .attacker_positioning
            .raw = 0;
        saved
            .naval_combat_participants
            .get_mut(&(combat, 1, u32::MAX))
            .unwrap()
            .state = 0;
        assert_eq!(
            compare(&frame, &saved, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            3
        );
    }

    // Uses that module's existing fixture() and add_country() helpers.

    fn naval_fixture() -> (
        DecodedFrame,
        Save,
        save::MapMembership,
        save::EquipmentTypes,
    ) {
        let (mut frame, mut saved, membership, equipment) = fixture();
        add_country(&mut frame, &mut saved, 201, 8, b"SOV");
        add_country(&mut frame, &mut saved, 301, 9, b"D01");
        let fleet = (u64::from(u32::MAX) << 32) | 61;
        let force = (2_147_483_648_u64 << 32) | 4713;
        let ship = (u64::from(u32::MAX) << 32) | 51;
        saved.fleets.insert(
            fleet,
            save::Fleet {
                id: fleet,
                owner: b"GER".to_vec(),
                home_base: Some(2),
            },
        );
        saved.task_forces.insert(
            force,
            save::TaskForce {
                id: force,
                fleet,
                roster_owner: b"GER".to_vec(),
                logical_owner: b"SOV".to_vec(),
                expeditionary_owner: Some(b"D01".to_vec()),
                province: Some(1),
                mission: -1,
                country_intel: Vec::new(),
            },
        );
        saved.ships.insert(
            ship,
            save::Ship {
                id: ship,
                task_force: force,
                strength: Decimal::parse(b"90071992547.40993").unwrap(),
                organisation: Decimal::parse(b"78.72001").unwrap(),
                experience: Decimal::parse(b"34.32537").unwrap(),
                sunk_convoys: -1,
            },
        );
        frame.tables[TableId::Fleets.index()] =
            TableView::Complete(TableRows::Fleets(vec![replay::rows::FleetRow {
                id: fleet,
                owner: 102,
                home_base: 2,
            }]));
        frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![replay::rows::TaskForceRow {
                id: force,
                fleet,
                roster_owner: 102,
                logical_owner: 201,
                expeditionary_owner: 301,
                province: 1,
                mission: u32::MAX,
            }]));
        frame.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![replay::rows::ShipRow {
                id: ship,
                task_force: force,
                strength: 9_007_199_254_740_993,
                organisation: 7_872_001,
                experience: 3_432_537,
                sunk_convoys: -1,
                category: 9,
                class_id: -1,
            }]));
        (frame, saved, membership, equipment)
    }

    #[test]
    fn naval_oracle_checks_precision_full_identity_and_parent_membership() {
        let (frame, saved, membership, equipment) = naval_fixture();
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!((report.fleets, report.task_forces, report.ships), (1, 1, 1));
        let mut changed = frame.clone();
        let TableView::Complete(TableRows::Ships(rows)) =
            &mut changed.tables[TableId::Ships.index()]
        else {
            unreachable!()
        };
        let ship = &mut rows[0];
        ship.strength -= 1;
        ship.organisation += 1;
        ship.task_force ^= 1;
        let report = compare(&changed, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let mut changed = frame.clone();
        let TableView::Complete(TableRows::Ships(rows)) =
            &mut changed.tables[TableId::Ships.index()]
        else {
            unreachable!()
        };
        let ship = &mut rows[0];
        ship.id ^= 1; // Same numeric id, different type: missing and unexpected rows.
        let report = compare(&changed, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.ships, 0);
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        for table in [TableId::Fleets, TableId::TaskForces, TableId::Ships] {
            let mut incomplete = frame.clone();
            incomplete.tables[table.index()] = TableView::Unavailable {
                reason: "source unavailable".into(),
            };
            assert!(
                compare(&incomplete, &saved, &membership, &equipment)
                    .and_then(|report| report.outcome())
                    .is_err()
            );
        }
    }

    #[test]
    fn naval_oracle_distinguishes_null_locations_and_expeditionary_omission_predicate() {
        let (mut frame, mut saved, membership, equipment) = naval_fixture();
        saved.fleets.values_mut().next().unwrap().home_base = None;
        let force = saved.task_forces.values_mut().next().unwrap();
        force.province = None;
        force.expeditionary_owner = None;
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let TableView::Complete(TableRows::Fleets(rows)) =
            &mut frame.tables[TableId::Fleets.index()]
        else {
            unreachable!()
        };
        let fleet = &mut rows[0];
        fleet.home_base = 0;
        let TableView::Complete(TableRows::TaskForces(rows)) =
            &mut frame.tables[TableId::TaskForces.index()]
        else {
            unreachable!()
        };
        let force = &mut rows[0];
        force.province = 0;
        force.expeditionary_owner = u32::MAX; // Signed -1 is omitted, but not certified as raw zero.
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("signed-nonpositive expeditionary"))
        );
        saved.task_forces.values_mut().next().unwrap().province = Some(0);
        assert!(
            compare(&frame, &saved, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
    }
    /// A frame and save with two extra countries, so relation and intel rows can
    /// use raw references that are aliases of a canonical slot.
    fn relation_fixture() -> (
        DecodedFrame,
        Save,
        save::MapMembership,
        save::EquipmentTypes,
    ) {
        let (mut frame, mut save, membership, equipment) = fixture();
        add_country(&mut frame, &mut save, 201, 8, b"D01");
        add_country(&mut frame, &mut save, 301, 9, b"SOV");
        (frame, save, membership, equipment)
    }

    fn saved_relation(
        save: &mut Save,
        owner: &[u8],
        target: &[u8],
        cached_sum: Option<i32>,
        tuples: &[(&[u8], u32)],
    ) {
        if let Some(cached_sum) = cached_sum {
            save.relation_opinions.push(save::RelationOpinion {
                owner: owner.to_vec(),
                target: target.to_vec(),
                cached_sum,
            });
        }
        for (kind, ordinal) in tuples {
            save.relation_tuples.push(save::RelationTuple {
                owner: owner.to_vec(),
                target: target.to_vec(),
                kind: kind.to_vec(),
                ordinal: *ordinal,
                first: owner.to_vec(),
                second: target.to_vec(),
                start_date: Some(save::GameDate {
                    year: 1936,
                    month: 1,
                    day: 1,
                    hour: 1,
                }),
                end_date: None,
                cancel: false,
            });
        }
    }

    fn opinions(frame: &mut DecodedFrame, rows: Vec<replay::rows::RelationOpinionRow>) {
        frame.tables[TableId::RelationOpinions.index()] =
            TableView::Complete(TableRows::RelationOpinions(rows));
    }

    fn tuples(frame: &mut DecodedFrame, rows: Vec<replay::rows::RelationTupleRow>) {
        frame.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(rows));
    }

    fn tuple_row(owner: u32, target: u32, ordinal: u16) -> replay::rows::RelationTupleRow {
        replay::rows::RelationTupleRow {
            owner_country: owner,
            target_country: target,
            type_id: 0x28da,
            ordinal,
            first_country: owner,
            second_country: target,
            start_hour: save::GameDate {
                year: 1936,
                month: 1,
                day: 1,
                hour: 1,
            }
            .engine_hour()
            .unwrap(),
            end_hour: 0,
            cancel: 0,
            retired: 0,
        }
    }

    fn differences(report: &Report) -> Vec<String> {
        report.differences.clone()
    }

    /// RelationTuples is registered in the extended pass, which needs verified
    /// content; the comparator itself is exercised directly with the same
    /// country links and recorded dictionary that pass supplies.
    fn tuple_report(frame: &DecodedFrame, save: &Save, tokens: &[(u32, &'static [u8])]) -> Report {
        let mut report = Report::default();
        let context = country_context(frame).unwrap();
        let names: TokenNames<'static> = tokens.iter().copied().collect();
        report.attempt("RelationTuples", |report| {
            compare_relation_tuples(frame, save, &context.links, &names, report)
        });
        report
    }

    #[test]
    fn relation_opinions_compare_exactly_and_report_both_one_sided_pairs() {
        let (mut frame, mut save, membership, equipment) = relation_fixture();
        saved_relation(&mut save, b"GER", b"D01", Some(-100), &[]);
        opinions(
            &mut frame,
            vec![replay::rows::RelationOpinionRow {
                owner_country: 102,
                target_country: 201,
                opinion: -100,
            }],
        );
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(differences(&report).is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["RelationOpinions"], 1);

        // A changed value, a save-only pair and a replay-only pair all fail.
        let mut save = save.clone();
        save.relation_opinions[0].cached_sum = -35;
        saved_relation(&mut save, b"D01", b"GER", Some(7), &[]);
        let mut frame = frame.clone();
        opinions(
            &mut frame,
            vec![
                replay::rows::RelationOpinionRow {
                    owner_country: 102,
                    target_country: 201,
                    opinion: -100,
                },
                replay::rows::RelationOpinionRow {
                    owner_country: 201,
                    target_country: 301,
                    opinion: 5,
                },
            ],
        );
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        let differences = differences(&report);
        assert_eq!(differences.len(), 3, "{differences:?}");
        assert!(differences.iter().any(|line| line.contains("cached_sum")));
        assert!(
            differences
                .iter()
                .any(|line| line.contains("D01 -> GER") && line.contains("\"absent\""))
        );
    }

    /// A country's relation with itself is ordinary state: the live 2026-09-11
    /// save serializes a self `active_relations` entry with its own cached sum,
    /// so the comparator must compare it instead of refusing the whole group.
    #[test]
    fn a_self_relation_opinion_is_compared_like_any_other_pair() {
        let (mut frame, mut save, membership, equipment) = relation_fixture();
        saved_relation(&mut save, b"GER", b"GER", Some(-100), &[]);
        saved_relation(&mut save, b"GER", b"D01", Some(-35), &[]);
        opinions(
            &mut frame,
            vec![
                replay::rows::RelationOpinionRow {
                    owner_country: 102,
                    target_country: 102,
                    opinion: -100,
                },
                replay::rows::RelationOpinionRow {
                    owner_country: 102,
                    target_country: 201,
                    opinion: -35,
                },
            ],
        );
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(differences(&report).is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["RelationOpinions"], 2);

        // A wrong self opinion is still an ordinary difference, not a refusal.
        let mut save = save.clone();
        save.relation_opinions[0].cached_sum = -99;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        let differences = differences(&report);
        assert_eq!(differences.len(), 1, "{differences:?}");
        assert!(
            differences[0].contains("GER -> GER") && differences[0].contains("cached_sum"),
            "{differences:?}"
        );
    }

    #[test]
    fn a_recorded_zero_opinion_or_duplicate_pair_fails_the_comparison() {
        let (mut frame, save, membership, equipment) = relation_fixture();
        opinions(
            &mut frame,
            vec![replay::rows::RelationOpinionRow {
                owner_country: 102,
                target_country: 201,
                opinion: 0,
            }],
        );
        let error = comparison_error(compare(&frame, &save, &membership, &equipment));
        assert!(error.contains("omits a zero cached_sum"), "{error}");
        // Aliases 101 and 102 resolve to the same tag, so this is one pair twice.
        opinions(
            &mut frame,
            vec![
                replay::rows::RelationOpinionRow {
                    owner_country: 101,
                    target_country: 201,
                    opinion: 1,
                },
                replay::rows::RelationOpinionRow {
                    owner_country: 102,
                    target_country: 201,
                    opinion: 2,
                },
            ],
        );
        let error = comparison_error(compare(&frame, &save, &membership, &equipment));
        assert!(
            error.contains("duplicate resolved relation opinion"),
            "{error}"
        );
    }

    #[test]
    fn relation_tuples_join_on_the_native_ordinal_and_the_recorded_token_spelling() {
        let (mut frame, mut save, _, _) = relation_fixture();
        saved_relation(&mut save, b"GER", b"D01", None, &[(b"guarantee", 3)]);
        tuples(&mut frame, vec![tuple_row(102, 201, 3)]);
        let named: &[(u32, &'static [u8])] = &[(0x28da, b"guarantee")];
        let report = tuple_report(&frame, &save, named);
        assert!(differences(&report).is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["RelationTuples"], 1);

        // A different ordinal is a different row on both sides.
        let mut moved = frame.clone();
        tuples(&mut moved, vec![tuple_row(102, 201, 4)]);
        let report = tuple_report(&moved, &save, named);
        assert_eq!(differences(&report).len(), 2, "{:?}", report.differences);

        // A spelling the dictionary contradicts is a named difference.
        let renamed: &[(u32, &'static [u8])] = &[(0x28da, b"embargo")];
        let report = tuple_report(&frame, &save, renamed);
        assert!(
            differences(&report)
                .iter()
                .any(|line| line.contains(".kind")),
            "{:?}",
            report.differences
        );

        // An unknown token compares every other field and prints the gap.
        let report = tuple_report(&frame, &save, &[]);
        assert!(differences(&report).is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no spelling for some relation type tokens"))
        );

        // Endpoints, cancel and the conditional hours all compare exactly.
        let mut changed = frame.clone();
        tuples(&mut changed, {
            let mut row = tuple_row(102, 201, 3);
            row.second_country = 101;
            row.cancel = 1;
            row.end_hour = row.start_hour + 1;
            vec![row]
        });
        let report = tuple_report(&changed, &save, named);
        let lines = differences(&report);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines.iter().any(|line| line.contains(".second")));
        assert!(lines.iter().any(|line| line.contains(".cancel")));
        assert!(lines.iter().any(|line| line.contains("end_date presence")));

        // A raw cancel byte outside 0/1 cannot be checked against yes/no.
        let mut raw = frame.clone();
        tuples(&mut raw, {
            let mut row = tuple_row(102, 201, 3);
            row.cancel = 0x80;
            vec![row]
        });
        let report = tuple_report(&raw, &save, named);
        assert!(report.failed_comparisons["RelationTuples"].contains("outside 0/1"));
    }

    #[test]
    fn country_intel_compares_every_positional_record_and_both_one_sided_lists() {
        let (mut frame, mut save, membership, equipment) = relation_fixture();
        save.country_intel.insert(
            b"GER".to_vec(),
            vec![
                save::CountryIntelRecord::default(),
                save::CountryIntelRecord {
                    civilian: 3_331_020,
                    army: -1_500_000,
                    navy: i64::MAX,
                    air: i64::MIN,
                },
            ],
        );
        frame.tables[TableId::CountryIntel.index()] =
            TableView::Complete(TableRows::CountryIntel(vec![
                replay::rows::CountryIntelRow {
                    owner_country: 102,
                    target_index: 0,
                    ..Default::default()
                },
                replay::rows::CountryIntelRow {
                    owner_country: 102,
                    target_index: 1,
                    civilian: 3_331_020,
                    army: -1_500_000,
                    navy: i64::MAX,
                    air: i64::MIN,
                },
            ]));
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(differences(&report).is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryIntel"], 2);

        // One changed lane, one save-only index and one replay-only country.
        let mut save = save.clone();
        save.country_intel
            .get_mut(b"GER".as_slice())
            .unwrap()
            .push(save::CountryIntelRecord::default());
        save.country_intel.get_mut(b"GER".as_slice()).unwrap()[1].navy = 0;
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::CountryIntel(rows)) =
            &mut frame.tables[TableId::CountryIntel.index()]
        else {
            unreachable!()
        };
        rows.push(replay::rows::CountryIntelRow {
            owner_country: 201,
            target_index: 0,
            ..Default::default()
        });
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        let differences = differences(&report);
        assert_eq!(differences.len(), 3, "{differences:?}");
        assert!(
            differences
                .iter()
                .any(|line| line.contains("GER.intel[1].navy"))
        );
        assert!(differences.iter().any(|line| line.contains("GER.intel[2]")));
        assert!(differences.iter().any(|line| line.contains("D01.intel")));
    }

    #[test]
    fn the_boundary_witness_states_whether_the_save_was_written_at_the_sampled_tick() {
        let (mut frame, save, membership, equipment) = relation_fixture();
        let counters = save.counters;
        let TableView::Complete(TableRows::Clock(rows)) = &mut frame.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].debug_current_ref_id = Some(counters.debug_current_ref_id);
        rows[0].unit = Some(counters.unit);
        rows[0].order_index = Some(counters.order_index);
        rows[0].front_index = Some(counters.front_index);
        rows[0].theatre_index = Some(counters.theatre_index);
        rows[0].theater_group_index = Some(counters.theater_group_index);
        rows[0].military_deployment_line_index = Some(counters.military_deployment_line_index);
        rows[0].military_deployment_conveyor_index =
            Some(counters.military_deployment_conveyor_index);
        rows[0].equipment_variant_index = Some(counters.equipment_variant_index);
        rows[0].country_leader_index = Some(counters.country_leader_index);
        rows[0].navy_id = Some(counters.navy_id);
        rows[0].land_combat_id = Some(counters.land_combat_id);
        rows[0].multiplayer_random_count = Some(counters.multiplayer_random_count);
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        let verdict = report.boundary_verdict.clone().unwrap();
        assert!(verdict.contains("written at the sampled tick"), "{verdict}");
        assert_eq!(report.boundary_witness.len(), 13);
        assert!(differences(&report).is_empty());
        assert_eq!(
            report.compared_tables["boundary counter witnesses (printed, never a difference)"],
            13
        );

        // Three objects allocated after the sample; the witness names them and
        // the comparison still passes, because it never waives a difference.
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::Clock(rows)) = &mut frame.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].debug_current_ref_id = Some(counters.debug_current_ref_id - 3);
        rows[0].navy_id = Some(counters.navy_id - 1);
        rows[0].multiplayer_random_count = None;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        let verdict = report.boundary_verdict.clone().unwrap();
        assert!(verdict.contains("after further engine work"), "{verdict}");
        assert!(verdict.contains("debug_current_ref_id +3"), "{verdict}");
        assert!(verdict.contains("navy_id +1"), "{verdict}");
        assert_eq!(
            report.compared_tables["boundary counter witnesses (printed, never a difference)"],
            12
        );
        assert!(
            report
                .boundary_witness
                .iter()
                .any(|line| line.contains("multiplayer_random_count: recorded unavailable"))
        );
        assert!(differences(&report).is_empty());

        // Counters that are not recorded at all leave the boundary unmeasured.
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::Clock(rows)) = &mut frame.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0] = ClockRow {
            hour: rows[0].hour,
            seed: rows[0].seed,
            ..Default::default()
        };
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert!(
            report
                .boundary_verdict
                .clone()
                .unwrap()
                .contains("not measured"),
        );
        assert!(differences(&report).is_empty());
    }
}
