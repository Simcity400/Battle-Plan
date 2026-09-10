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
    rows::{CountryRow, Row, TableId},
};

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
    // Refuse missing tables before paying for content resolution.
    for table in TableId::ALL {
        complete(&frame, table)?;
    }
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
    let membership = save::read_map_membership(&content.layers)?;
    let equipment_types = save::read_equipment_types(&content.layers)?;
    let report = compare(&frame, &save, &membership, &equipment_types)?;
    report.print();
    println!("Oracle elapsed: {:.3} s", started.elapsed().as_secs_f64());
    if report.differences.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "oracle found {} differences",
            report.differences.len()
        )))
    }
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

fn complete(frame: &DecodedFrame, table: TableId) -> io::Result<&[Row]> {
    match &frame.tables[table.index()] {
        TableView::Complete(rows) => Ok(rows),
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

#[derive(Debug, Default)]
struct Report {
    countries: usize,
    provinces: usize,
    states: usize,
    divisions: usize,
    war_relations: usize,
    factions: usize,
    faction_members: usize,
    scalar_checks: usize,
    differences: Vec<String>,
    limitations: BTreeMap<String, Vec<String>>,
}

impl Report {
    fn difference(
        &mut self,
        field: impl AsRef<str>,
        replay: impl std::fmt::Debug,
        save: impl std::fmt::Debug,
    ) {
        self.differences.push(format!(
            "{}: replay {replay:?}, save {save:?}",
            field.as_ref()
        ));
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
            "Compared: {} provinces, {} states, {} divisions, {} war relations, {} factions, {} faction members, {} countries, {} exact scalar values (/100000)",
            self.provinces,
            self.states,
            self.divisions,
            self.war_relations,
            self.factions,
            self.faction_members,
            self.countries,
            self.scalar_checks
        );
        println!("Differences: {}", self.differences.len());
        for difference in self.differences.iter().take(100) {
            println!("  {difference}");
        }
        if self.differences.len() > 100 {
            println!(
                "  {} additional differences omitted",
                self.differences.len() - 100
            );
        }
        println!("Limitations:");
        for (reason, subjects) in &self.limitations {
            println!("  {reason}: {}", subjects.join(", "));
        }
    }
}

fn compare(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::MapMembership,
    equipment_types: &save::EquipmentTypes,
) -> io::Result<Report> {
    matching_hour(save.date.engine_hour()?, Some(frame.hour))?;
    let [Row::Clock(clock)] = complete(frame, TableId::Clock)? else {
        return Err(invalid("clock table must contain one clock"));
    };
    let [Row::Campaign(campaign)] = complete(frame, TableId::Campaign)? else {
        return Err(invalid("campaign table must contain one identity"));
    };
    if clock.hour != frame.hour || clock.seed != save.seed || campaign.id != save.campaign_id {
        return Err(invalid(
            "frame clock, seed or campaign identity differs from the save",
        ));
    }
    let mut report = Report::default();
    report.limitation(
        "commands after the hourly callback can change state before saving without advancing the hour; matching hours do not prove the same command boundary, and differences still fail",
        "all captured tables",
    );
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
    let mut by_tag = BTreeMap::new();
    let mut by_slot = BTreeMap::new();
    for row in complete(frame, TableId::Countries)? {
        let Row::Country(country) = row else {
            return Err(invalid("non-country row in country table"));
        };
        if by_tag.insert(country.tag.as_slice(), country).is_some()
            || by_slot.insert(country.canonical_slot, country).is_some()
        {
            return Err(invalid("ambiguous country tag or canonical slot"));
        }
    }
    let mut links = BTreeMap::new();
    for row in complete(frame, TableId::CountryLinks)? {
        let Row::CountryLink(link) = row else {
            return Err(invalid("non-link row in link table"));
        };
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
            convoy_total(saved, save, equipment_types),
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
                return Err(invalid(format!(
                    "{label}.convoys cannot be compared: {reason}"
                )));
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
    let mut provinces = BTreeMap::new();
    for row in complete(frame, TableId::Provinces)? {
        let Row::Province(province) = row else {
            return Err(invalid("non-province row in province table"));
        };
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
        if actual.state != expected.state.unwrap_or(0) {
            report.difference(
                format!("province {}.state", expected.id),
                actual.state,
                expected.state,
            );
        }
        let owner = country_tag(actual.owner, &links)?;
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
            country_tag(actual.controller, &links)?
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
    let mut states = BTreeMap::new();
    for row in complete(frame, TableId::States)? {
        let Row::State(state) = row else {
            return Err(invalid("non-state row in state table"));
        };
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
        let owner = country_tag(actual.owner, &links)?;
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
        let controller = country_tag(actual.controller, &links)?;
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
    compare_divisions(frame, save, membership, &links, &mut report)?;
    compare_war_relations(frame, save, &links, &mut report)?;
    compare_factions(frame, save, &links, &mut report)?;
    Ok(report)
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
    for row in complete(frame, TableId::WarRelations)? {
        let Row::WarRelation(relation) = row else {
            return Err(invalid("non-war-relation row in war relation table"));
        };
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

fn compare_factions(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut factions = BTreeMap::new();
    for row in complete(frame, TableId::Factions)? {
        let Row::Faction(faction) = row else {
            return Err(invalid("non-faction row in faction table"));
        };
        if faction.id == 0 || factions.insert(faction.id, faction).is_some() {
            return Err(invalid("null or duplicate full faction handle in replay"));
        }
    }
    let mut members: BTreeMap<u64, BTreeMap<u32, &[u8]>> = BTreeMap::new();
    let mut member_countries = BTreeSet::new();
    for row in complete(frame, TableId::FactionMembers)? {
        let Row::FactionMember(member) = row else {
            return Err(invalid("non-member row in faction member table"));
        };
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
    for row in complete(frame, TableId::Divisions)? {
        let Row::Division(division) = row else {
            return Err(invalid("non-division row in division table"));
        };
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
mod tests {
    use super::*;
    use replay::{
        frame::Metrics,
        rows::{
            CampaignRow, ClockRow, CountryLinkRow, DivisionRow, FactionMemberRow, FactionRow,
            ProvinceRow, StateRow, WarRelationRow,
        },
    };

    fn fixture() -> (
        DecodedFrame,
        Save,
        save::MapMembership,
        save::EquipmentTypes,
    ) {
        let save = save::parse(br#"HOI4txt
date="1936.1.1.1" game_unique_id="campaign" game_unique_seed=7 version="build"
states={1={owner="GER"}} provinces={2={controller="GER"}}
countries={GER={politics={political_power=10.00001} command_power=2 stability=0.7 war_support=0.3 convoys={equipment={}}}}
"#).unwrap();
        let hour = save.date.engine_hour().unwrap();
        let frame = DecodedFrame {
            branch: 2,
            hour,
            metrics: Metrics::default(),
            tables: [
                TableView::Complete(vec![Row::Clock(ClockRow {
                    hour,
                    seed: 7,
                    ..Default::default()
                })]),
                TableView::Complete(vec![Row::Campaign(CampaignRow {
                    id: b"campaign".to_vec(),
                })]),
                TableView::Complete(vec![Row::Country(CountryRow {
                    id: 101,
                    canonical_slot: 7,
                    tag: b"GER".to_vec(),
                    name: b"Germany".to_vec(),
                    political_power_raw: Some(1_000_001),
                    command_power_raw: 200_000,
                    stability_raw: 70_000,
                    war_support_raw: 30_000,
                    convoys_raw: Some(0),
                })]),
                TableView::Complete(vec![
                    Row::CountryLink(CountryLinkRow {
                        id: 101,
                        canonical_slot: 7,
                    }),
                    Row::CountryLink(CountryLinkRow {
                        id: 102,
                        canonical_slot: 7,
                    }),
                ]),
                TableView::Complete(vec![
                    Row::Province(ProvinceRow {
                        id: 1,
                        state: 1,
                        owner: 102,
                        controller: 0,
                    }),
                    Row::Province(ProvinceRow {
                        id: 2,
                        state: 0,
                        owner: 0,
                        controller: 101,
                    }),
                ]),
                TableView::Complete(vec![Row::State(StateRow {
                    id: 1,
                    owner: 102,
                    controller: 101,
                })]),
                TableView::Complete(Vec::new()),
                TableView::Complete(Vec::new()),
                TableView::Complete(Vec::new()),
                TableView::Complete(Vec::new()),
            ],
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
        let TableView::Complete(rows) = &mut frame.tables[TableId::Countries.index()] else {
            unreachable!()
        };
        let Row::Country(mut row) = rows[0].clone() else {
            unreachable!()
        };
        row.id = id;
        row.canonical_slot = slot;
        row.tag = tag.to_vec();
        rows.push(Row::Country(row));
        let TableView::Complete(rows) = &mut frame.tables[TableId::CountryLinks.index()] else {
            unreachable!()
        };
        rows.push(Row::CountryLink(CountryLinkRow {
            id,
            canonical_slot: slot,
        }));
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
            },
        );
        frame.tables[TableId::Divisions.index()] =
            TableView::Complete(vec![Row::Division(DivisionRow {
                id,
                roster_owner: 102,
                logical_owner: 201,
                expeditionary_owner: 301,
                province: 1,
                template,
                hit_points: 9_007_199_254_740_993,
                organisation: 5_345_454,
            })]);
        (frame, save, membership, equipment)
    }

    fn division_row_mut(frame: &mut DecodedFrame) -> &mut DivisionRow {
        let TableView::Complete(rows) = &mut frame.tables[TableId::Divisions.index()] else {
            unreachable!()
        };
        let Row::Division(row) = &mut rows[0] else {
            unreachable!()
        };
        row
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
            TableView::Complete(vec![Row::WarRelation(WarRelationRow {
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
            })]);
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
        frame.tables[TableId::Factions.index()] = TableView::Complete(vec![
            Row::Faction(FactionRow {
                id: 88,
                name: Vec::new(),
            }),
            Row::Faction(FactionRow {
                id: 1_u64 << 63,
                name: b"Literal".to_vec(),
            }),
        ]);
        frame.tables[TableId::FactionMembers.index()] = TableView::Complete(vec![
            Row::FactionMember(FactionMemberRow {
                country: 102,
                faction: 88,
                ordinal: 1,
            }),
            Row::FactionMember(FactionMemberRow {
                country: 201,
                faction: 88,
                ordinal: 0,
            }),
        ]);
        (frame, save, membership, equipment)
    }

    fn war_row_mut(frame: &mut DecodedFrame) -> &mut WarRelationRow {
        let TableView::Complete(rows) = &mut frame.tables[TableId::WarRelations.index()] else {
            unreachable!()
        };
        let Row::WarRelation(row) = &mut rows[0] else {
            unreachable!()
        };
        row
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
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
                .contains("envelope")
        );
        save.war_relations[0].roster_owner = b"UNKNOWN".to_vec();
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
                compare(&changed, &save, &membership, &equipment)
                    .unwrap_err()
                    .to_string()
                    .contains("cannot verify these bits")
            );
        }
        let mut changed = frame.clone();
        let TableView::Complete(rows) = &mut changed.tables[TableId::WarRelations.index()] else {
            unreachable!()
        };
        let Row::WarRelation(mut duplicate) = rows[0].clone() else {
            unreachable!()
        };
        duplicate.first_country = 101;
        rows.push(Row::WarRelation(duplicate));
        assert!(
            compare(&changed, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
                .contains("duplicate resolved")
        );
        let mut duplicate_save = save.clone();
        duplicate_save
            .war_relations
            .push(save.war_relations[0].clone());
        assert!(compare(&frame, &duplicate_save, &membership, &equipment).is_err());
    }

    #[test]
    fn faction_names_handles_and_membership_order_are_exact() {
        let (mut frame, save, membership, equipment) = relations_fixture();
        let TableView::Complete(rows) = &mut frame.tables[TableId::Factions.index()] else {
            unreachable!()
        };
        let Row::Faction(row) = &mut rows[0] else {
            unreachable!()
        };
        row.name = b"Axis".to_vec();
        let TableView::Complete(rows) = &mut frame.tables[TableId::FactionMembers.index()] else {
            unreachable!()
        };
        for row in rows {
            let Row::FactionMember(row) = row else {
                unreachable!()
            };
            row.ordinal = 1 - row.ordinal;
        }
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let (mut frame, save, membership, equipment) = relations_fixture();
        let TableView::Complete(rows) = &mut frame.tables[TableId::Factions.index()] else {
            unreachable!()
        };
        let Row::Faction(row) = &mut rows[1] else {
            unreachable!()
        };
        row.id ^= 1;
        assert_eq!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap()
                .differences
                .len(),
            2
        );
        let (mut frame, save, membership, equipment) = relations_fixture();
        frame.tables[TableId::FactionMembers.index()] = TableView::Complete(Vec::new());
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
            let TableView::Complete(rows) = &mut changed.tables[TableId::FactionMembers.index()]
            else {
                unreachable!()
            };
            rows.push(Row::FactionMember(member));
            assert!(compare(&changed, &save, &membership, &equipment).is_err());
        }
        let mut changed = frame.clone();
        let TableView::Complete(rows) = &mut changed.tables[TableId::FactionMembers.index()] else {
            unreachable!()
        };
        let Row::FactionMember(row) = &mut rows[0] else {
            unreachable!()
        };
        row.ordinal = 3;
        assert!(
            compare(&changed, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
            compare(&frame, &duplicate_save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
                    rows: Vec::new(),
                    missing: 1,
                },
                TableView::Complete(vec![Row::Province(ProvinceRow::default())]),
            ] {
                let mut changed = frame.clone();
                changed.tables[table.index()] = view;
                assert!(compare(&changed, &save, &membership, &equipment).is_err());
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
    fn missing_extra_and_changed_province_rows_are_differences() {
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(rows) = &mut frame.tables[TableId::Provinces.index()] else {
            unreachable!()
        };
        rows.remove(0);
        let Row::Province(province) = &mut rows[0] else {
            unreachable!()
        };
        // Explicit saved control cannot be manufactured from captured raw zero.
        province.controller = 0;
        rows.push(Row::Province(ProvinceRow {
            id: 3,
            ..Default::default()
        }));
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    }

    #[test]
    fn state_ownership_and_controller_differences_are_independent_of_provinces() {
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(rows) = &mut frame.tables[TableId::States.index()] else {
            unreachable!()
        };
        let Row::State(state) = &mut rows[0] else {
            unreachable!()
        };
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
        let TableView::Complete(rows) = &mut frame.tables[TableId::States.index()] else {
            unreachable!()
        };
        let Row::State(state) = &mut rows[0] else {
            unreachable!()
        };
        state.controller = 0;
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("state 1.controller:"));

        frame.tables[TableId::States.index()] = TableView::Complete(vec![Row::State(StateRow {
            id: 2,
            owner: 101,
            controller: 101,
        })]);
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert_eq!(report.states, 0);
    }

    #[test]
    fn explicit_state_controller_is_compared_even_when_owner_and_provinces_match() {
        let (mut frame, mut save, membership, equipment) = fixture();
        let mut other = save.countries[b"GER".as_slice()].clone();
        other.tag = b"D01".to_vec();
        save.countries.insert(b"D01".to_vec(), other);
        save.states.get_mut(&1).unwrap().controller = Some(b"D01".to_vec());
        let TableView::Complete(rows) = &mut frame.tables[TableId::Countries.index()] else {
            unreachable!()
        };
        let Row::Country(mut other) = rows[0].clone() else {
            unreachable!()
        };
        other.id = 201;
        other.canonical_slot = 8;
        other.tag = b"D01".to_vec();
        rows.push(Row::Country(other));
        let TableView::Complete(rows) = &mut frame.tables[TableId::CountryLinks.index()] else {
            unreachable!()
        };
        rows.push(Row::CountryLink(CountryLinkRow {
            id: 201,
            canonical_slot: 8,
        }));
        let report = compare(&frame, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("state 1.controller:"));
        let TableView::Complete(rows) = &mut frame.tables[TableId::States.index()] else {
            unreachable!()
        };
        let Row::State(state) = &mut rows[0] else {
            unreachable!()
        };
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
                rows: Vec::new(),
                missing: 1,
            },
            TableView::Complete(vec![Row::State(StateRow {
                id: 1,
                owner: 777,
                controller: 101,
            })]),
            TableView::Complete(vec![Row::State(StateRow {
                id: 1,
                owner: 101,
                controller: 777,
            })]),
            TableView::Complete(vec![
                Row::State(StateRow::default()),
                Row::State(StateRow::default()),
            ]),
            TableView::Complete(vec![Row::Province(ProvinceRow::default())]),
        ] {
            let mut invalid = frame.clone();
            invalid.tables[TableId::States.index()] = view;
            assert!(compare(&invalid, &save, &membership, &equipment).is_err());
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
        let TableView::Complete(rows) = &mut frame.tables[TableId::Divisions.index()] else {
            unreachable!()
        };
        rows.push(rows[0].clone());
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
                rows: Vec::new(),
                missing: 1,
            },
            TableView::Complete(vec![Row::Province(ProvinceRow::default())]),
            TableView::Complete(vec![Row::Division(DivisionRow::default())]),
        ] {
            let mut invalid = frame.clone();
            invalid.tables[TableId::Divisions.index()] = table;
            assert!(compare(&invalid, &save, &membership, &equipment).is_err());
        }
        let mut invalid = frame.clone();
        division_row_mut(&mut invalid).logical_owner = 777;
        assert!(
            compare(&invalid, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
            compare(&frame, &invalid_save, &membership, &equipment)
                .unwrap_err()
                .to_string()
                .contains("unknown saved country")
        );
        let mut invalid_save = save;
        invalid_save.divisions.values_mut().next().unwrap().province = Some(999_999);
        assert!(
            compare(&frame, &invalid_save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
                .contains("bad pointer")
        );
        frame.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: Vec::new(),
            missing: 2,
        };
        assert!(compare(&frame, &save, &membership, &equipment).is_err());
        let (mut frame, save, membership, equipment) = fixture();
        let TableView::Complete(rows) = &mut frame.tables[TableId::CountryLinks.index()] else {
            unreachable!()
        };
        rows.pop();
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
                .contains("102")
        );
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
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
                .contains("equipment definition")
        );
        save.equipment_definitions
            .insert((7, 70), b"unknown_variant".to_vec());
        assert!(
            compare(&frame, &save, &membership, &equipment)
                .unwrap_err()
                .to_string()
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
}
