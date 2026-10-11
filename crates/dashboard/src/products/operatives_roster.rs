//! Agency operatives, dated by each membership read. The active roster is
//! separate from the three recruitment vectors. Capture is assignment 0,
//! independent of the capturer link.
use super::country_name_cache::CountryNameCache;
use super::{
    LoadContext, Product,
    operative_rows::{self, CountryIdentity, identity, tag_at, tag_of},
    validate_countries,
};
use crate::{
    Source,
    catalogue::BoundedRefusals,
    metadata::{IdentityTrack, link_slot, tagged_countries, validate_campaign},
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{
        CharacterOperativeRow, CountryLinkRow, CountryRow, OperativeMembershipRow, TableId, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct OperativesRoster;
// Leave room for the transport envelope inside the complete-response limit.
const REPLY_BUDGET: usize = crate::transport::MAX_REPLY_BYTES - 4096;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativeRosterParams {
    pub branch_id: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativeRosterValues {
    pub name: Option<String>,
    pub unnamed: bool,
    pub name_missing: Option<u32>,
    pub portrait: Option<u32>,
    pub portrait_missing: Option<u32>,
    pub skill: Option<i32>,
    pub experience_raw: String,
    pub experience_text: Option<String>,
    pub experience_ratio: Option<f64>,
    pub experience_missing: Option<u32>,
    pub assignment_name: Option<String>,
    pub assignment_duration: Option<String>,
    pub assignment_progress: Option<f64>,
    pub assignment_missing: Option<u32>,
    pub assignment_icon: Option<u32>,
    pub assignment_icon_missing: Option<u32>,
    /// Assignment4 uses the separately dated Operations observations.
    pub assignment_operation: bool,
    pub target_tag: Option<String>,
    pub captured: Option<bool>,
    pub capturer_tag: Option<String>,
    pub captured_hour: Option<i32>,
    pub owner_tag: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativeRosterRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: OperativeRosterValues,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativeRosterEntry {
    pub key: String,
    pub id: String,
    /// First membership read showing this episode, not a recruitment date.
    pub joined_hour: i32,
    /// First complete read establishing absence or killed status; an upper bound.
    pub departed_hour: Option<i32>,
    pub departed_reason: Option<String>,
    pub runs: Vec<OperativeRosterRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativeRosterObservation {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativeRosterCountry {
    pub tag: String,
    pub identities: Vec<super::country_history::CountryIdentityRun>,
    pub observed: Vec<OperativeRosterObservation>,
    pub operatives: Vec<OperativeRosterEntry>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativeRosterReply {
    pub branch_id: String,
    pub countries: Vec<OperativeRosterCountry>,
    pub operations: Vec<OperativeRosterObservation>,
    pub agency_absent: Vec<super::country_groups::HourSpan>,
    pub artwork: Vec<String>,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

/// Artwork and missing explanations are sent once, never inside daily runs.
#[derive(Default)]
struct Strings {
    values: Vec<String>,
    indices: HashMap<String, u32>,
}
impl Strings {
    fn intern(&mut self, value: Option<String>, budget: &mut Budget) -> io::Result<Option<u32>> {
        let Some(value) = value else {
            return Ok(None);
        };
        if let Some(index) = self.indices.get(&value) {
            return Ok(Some(*index));
        }
        budget.add(&value)?;
        let index = u32::try_from(self.values.len())
            .map_err(|_| io::Error::other("Too many operative reply strings"))?;
        self.indices.insert(value.clone(), index);
        self.values.push(value);
        Ok(Some(index))
    }
}
#[derive(Default)]
struct ReplyTables {
    artwork: Strings,
    reasons: Strings,
}

struct Held {
    value: OperativeRosterValues,
    owner: Option<CountryIdentity>,
    target: Option<CountryIdentity>,
    capturer: Option<CountryIdentity>,
    operation: Option<u64>,
    captured_input: Option<(i32, Result<String, String>)>,
    cooldown: Option<(u32, i32, i32)>,
    cooldown_remaining: Option<i32>,
}

struct Projection<'a> {
    countries: &'a [CountryRow],
    links: &'a [CountryLinkRow],
    content: Option<&'a super::operative_content::OperativeContent>,
    names: &'a CountryNameCache,
    tables: &'a mut ReplyTables,
    refused: &'a mut BoundedRefusals,
    budget: &'a mut Budget,
}

impl Held {
    fn project(&mut self, hour: i32, context: &mut Projection<'_>) -> io::Result<()> {
        let Projection {
            countries,
            links,
            content,
            names,
            tables,
            budget,
            ..
        } = context;
        self.value.owner_tag = tag_at(self.owner, countries, links);
        self.value.target_tag = tag_at(self.target, countries, links);
        self.value.capturer_tag = tag_at(self.capturer, countries, links);
        if self.value.captured == Some(true)
            && let Some(content) = content
        {
            // Captured status uses the current definite capturer name and day
            // operands. The capture date stays read-dated.
            let name = self
                .capturer
                .ok_or("The captured operative's country identity is unknown at this hour")
                .and_then(|identity| names.name(identity));
            let input = (
                hour.wrapping_sub(engine::cdate::HOUR_EPOCH) / 24,
                name.map(str::to_owned).map_err(str::to_owned),
            );
            if self.captured_input.as_ref() == Some(&input) {
                return Ok(());
            }
            let status = content.captured_status(
                self.value
                    .captured_hour
                    .unwrap_or(engine::cgame_date::NULL_RAW_HOUR),
                hour,
                input.1.as_ref().ok().map(String::as_str),
            );
            self.value.assignment_name = status.name;
            self.value.assignment_duration = status.duration;
            self.value.assignment_progress = status.progress.map(f64::from);
            let missing = match (input.1.as_ref().err(), status.missing) {
                (Some(reason), Some(other)) => Some(format!("{reason} {other}")),
                (Some(reason), None) => Some(reason.clone()),
                (None, other) => other,
            };
            self.value.assignment_missing = tables.reasons.intern(missing, budget)?;
            self.captured_input = Some(input);
        }
        if let (Some(content), Some((kind, start, end))) = (content, self.cooldown) {
            let remaining = super::operative_content::cooldown_remaining(kind, end, hour);
            if self.cooldown_remaining != Some(remaining) {
                let status = content.cooldown_status(kind, start, end, hour);
                self.value.assignment_name = status.name;
                self.value.assignment_duration = status.duration;
                self.value.assignment_progress = status.progress.map(f64::from);
                self.value.assignment_missing = tables.reasons.intern(status.missing, budget)?;
                self.cooldown_remaining = Some(remaining);
            }
        }
        Ok(())
    }
}

fn operative_values(
    content: Option<&super::operative_content::OperativeContent>,
    frame: &replay::frame::DecodedFrame,
    row: &OperativeMembershipRow,
    character: Option<u32>,
    operation_token: Option<&str>,
    output: (&mut ReplyTables, &mut Budget, &mut BoundedRefusals),
) -> io::Result<OperativeRosterValues> {
    let (tables, budget, refused) = output;
    let name =
        operative_rows::recorded_name(content.map(|c| c.catalogue.localisation.as_ref()), row);
    let portrait = match (content, character) {
        (Some(content), Some(character)) => content.portrait(character, frame.complete::<tables::CharacterPortraits>().ok()),
        _ => Err("No recorded operative-to-character portrait link; the operative's own portrait path is not recorded".into()),
    };
    let experience = content.map(|content| content.experience(row.skill, row.experience_raw));
    // Captured text has only the hourly path; no second capturer-name build.
    let assignment = content
        .filter(|_| !operative_rows::captured(row.assignment) && row.assignment != 1)
        .map(|content| content.assignment(row, operation_token, None));
    let experience_missing = experience
        .as_ref()
        .and_then(|info| info.missing.clone())
        .or_else(|| {
            content.is_none().then(|| {
                "Experience display needs skill definitions from the recorded content stack".into()
            })
        });
    let mission_missing = (row.assignment == 3)
        .then(|| operative_rows::mission_variant(row.action_variant).err())
        .flatten();
    let cooldown_missing = operative_rows::cooldown(row).err();
    let assignment_missing = cooldown_missing.or(mission_missing).or_else(|| {
        assignment
            .as_ref()
            .and_then(|info| info.missing.clone())
            .or_else(|| {
                content
                    .is_none()
                    .then(|| "Assignment text needs the recorded content stack".into())
            })
    });
    let portrait_missing = portrait.as_ref().err().cloned();
    let icon_missing = assignment
        .as_ref()
        .and_then(|info| info.icon_missing.clone());
    for (reason, field) in [
        (name.missing.clone(), "name"),
        (portrait_missing.clone(), "portrait"),
        (experience_missing.clone(), "experience"),
        (assignment_missing.clone(), "assignment"),
        (icon_missing.clone(), "assignment icon"),
    ] {
        if let Some(reason) = reason {
            let identity = if field == "name" {
                format!(
                    "operative {} name {:?}",
                    row.id,
                    String::from_utf8_lossy(
                        row.name
                            .require("OperativeMemberships.name")
                            .map(Vec::as_slice)
                            .unwrap_or_default()
                    )
                )
            } else {
                format!("operative {} {field}", row.id)
            };
            refused.note(reason, identity);
        }
    }
    let captured = Some(operative_rows::captured(row.assignment));
    let capture_known =
        captured == Some(true) && row.captured_hour != engine::cgame_date::NULL_RAW_HOUR;
    Ok(OperativeRosterValues {
        name: name.name,
        unnamed: name.unnamed,
        name_missing: tables.reasons.intern(name.missing, budget)?,
        portrait: tables.artwork.intern(portrait.ok(), budget)?,
        portrait_missing: tables.reasons.intern(portrait_missing, budget)?,
        skill: row.skill,
        experience_raw: row.experience_raw.to_string(),
        experience_text: experience.as_ref().and_then(|info| info.text.clone()),
        experience_ratio: experience
            .as_ref()
            .and_then(|info| info.progress_raw)
            .map(|raw| raw as f64 / 100_000.0),
        experience_missing: tables.reasons.intern(experience_missing, budget)?,
        assignment_name: assignment.as_ref().and_then(|info| info.name.clone()),
        assignment_duration: None,
        assignment_progress: None,
        assignment_icon: tables.artwork.intern(
            assignment.as_ref().and_then(|info| info.icon.clone()),
            budget,
        )?,
        assignment_icon_missing: tables.reasons.intern(icon_missing, budget)?,
        assignment_operation: row.assignment == 4,
        assignment_missing: tables.reasons.intern(assignment_missing, budget)?,
        captured,
        captured_hour: capture_known.then_some(row.captured_hour),
        capturer_tag: None,
        owner_tag: None,
        target_tag: None,
    })
}

/// An incremental serialized-size bound stops history growth, rather than
/// truncating the roster or finding the response too large only after building it.
struct Budget(usize);
impl Budget {
    fn measure(value: &impl Serialize, available: usize) -> io::Result<usize> {
        json_len(value, available).ok_or_else(|| {
            io::Error::other("Operatives roster exceeds its complete-response budget")
        })
    }

    fn add(&mut self, value: &impl Serialize) -> io::Result<()> {
        let size = Self::measure(value, REPLY_BUDGET.saturating_sub(self.0))?;
        self.0 = self.0.saturating_add(size + 1);
        if self.0 > REPLY_BUDGET {
            return Err(io::Error::other(
                "Operatives roster exceeds its complete-response budget",
            ));
        }
        Ok(())
    }
}

struct History {
    reply: OperativeRosterCountry,
    track: IdentityTrack,
    country: Option<CountryIdentity>,
    held: BTreeMap<u64, Held>,
    active: BTreeMap<u64, usize>,
    latest: BTreeMap<u64, usize>,
    open: bool,
    read_hour: i32,
}

impl History {
    fn new(tag: &str) -> Self {
        Self {
            reply: OperativeRosterCountry {
                tag: tag.into(),
                identities: Vec::new(),
                observed: Vec::new(),
                operatives: Vec::new(),
            },
            track: IdentityTrack::default(),
            country: None,
            held: BTreeMap::new(),
            active: BTreeMap::new(),
            latest: BTreeMap::new(),
            open: false,
            read_hour: 0,
        }
    }

    fn interrupt(&mut self) {
        self.open = false;
        self.held.clear();
    }

    fn read(
        &mut self,
        hour: i32,
        values: BTreeMap<u64, Held>,
        departed: Vec<(u64, Held, &'static str)>,
        budget: &mut Budget,
    ) -> io::Result<()> {
        let present: std::collections::BTreeSet<_> = values.keys().copied().collect();
        let departed_reasons: BTreeMap<_, _> = departed
            .iter()
            .map(|(id, _, reason)| (*id, *reason))
            .collect();
        for (&id, &index) in &self.active {
            if !present.contains(&id) {
                let entry = &mut self.reply.operatives[index];
                entry.departed_hour = Some(hour);
                entry.departed_reason = Some(if let Some(reason) = departed_reasons.get(&id) {
                    (*reason).into()
                } else {
                    "Left (reason not recorded)".into()
                });
                budget.add(&(&entry.departed_hour, &entry.departed_reason))?;
            }
        }
        self.active.retain(|id, _| present.contains(id));
        // Terminal assignments can first appear at the read establishing them.
        // Repeated terminal rows never create repeated departures.
        for (id, held, reason) in departed {
            if let Some(&index) = self.latest.get(&id) {
                let entry = &mut self.reply.operatives[index];
                if entry.departed_hour == Some(hour) {
                    let run = OperativeRosterRun {
                        first_hour: hour,
                        last_hour: hour,
                        value: held.value,
                    };
                    budget.add(&run)?;
                    entry.runs.push(run);
                }
            } else {
                let mut entry = self.entry(id, hour);
                entry.departed_hour = Some(hour);
                entry.departed_reason = Some(reason.into());
                entry.runs.push(OperativeRosterRun {
                    first_hour: hour,
                    last_hour: hour,
                    value: held.value,
                });
                budget.add(&entry)?;
                self.latest.insert(id, self.reply.operatives.len());
                self.reply.operatives.push(entry);
            }
        }
        for &id in values.keys() {
            if !self.active.contains_key(&id) {
                let entry = self.entry(id, hour);
                budget.add(&entry)?;
                self.active.insert(id, self.reply.operatives.len());
                self.latest.insert(id, self.reply.operatives.len());
                self.reply.operatives.push(entry);
            }
        }
        self.held = values;
        self.read_hour = hour;
        self.open = true;
        Ok(())
    }

    fn entry(&self, id: u64, hour: i32) -> OperativeRosterEntry {
        OperativeRosterEntry {
            key: format!(
                "{}:{id}:{hour}:{}",
                self.country.map_or(0, |c| c.id),
                self.reply.operatives.len()
            ),
            id: id.to_string(),
            joined_hour: hour,
            departed_hour: None,
            departed_reason: None,
            runs: Vec::new(),
        }
    }

    fn publish(&mut self, hour: i32, context: &mut Projection<'_>) -> io::Result<()> {
        if !self.open {
            return Ok(());
        }
        match self.reply.observed.last_mut() {
            Some(observation)
                if observation.last_hour.checked_add(1) == Some(hour)
                    && observation.read_hour == self.read_hour =>
            {
                observation.last_hour = hour
            }
            _ => {
                let observation = OperativeRosterObservation {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour: self.read_hour,
                };
                context.budget.add(&observation)?;
                self.reply.observed.push(observation);
            }
        }
        for (&id, held) in &mut self.held {
            held.project(hour, context)?;
            let value = &held.value;
            let entry = &mut self.reply.operatives[self.active[&id]];
            match entry.runs.last_mut() {
                Some(run) if run.last_hour.checked_add(1) == Some(hour) && run.value == *value => {
                    run.last_hour = hour
                }
                _ => {
                    if let Some(reason) = &value.assignment_missing {
                        context.refused.note(
                            &context.tables.reasons.values[*reason as usize],
                            format!("operative {id} assignment"),
                        );
                    }
                    if let Some(reason) = &value.assignment_icon_missing {
                        context.refused.note(
                            &context.tables.reasons.values[*reason as usize],
                            format!("operative {id} assignment icon"),
                        );
                    }
                    let run = OperativeRosterRun {
                        first_hour: hour,
                        last_hour: hour,
                        value: value.clone(),
                    };
                    context.budget.add(&run)?;
                    entry.runs.push(run);
                }
            }
        }
        Ok(())
    }
}

pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    tags: &[String],
    content: Option<&super::operative_content::OperativeContent>,
    cancelled: &AtomicBool,
    content_error: Option<String>,
    selected_identities: Option<&BTreeSet<(u32, u32)>>,
) -> io::Result<OperativeRosterReply> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SimulationFeatures,
        TableId::OperativeMemberships,
        TableId::Operations,
        TableId::LexicalTokens,
        TableId::CharacterOperatives,
        TableId::CharacterPortraits,
        TableId::CountryAppearance,
        TableId::ReservedCountryNaming,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
    ];
    let mut histories: Vec<_> = tags.iter().map(|tag| History::new(tag)).collect();
    let mut refused = BoundedRefusals::new(REPLY_BUDGET);
    let mut tables = ReplyTables::default();
    let mut names = CountryNameCache::default();
    let mut budget = Budget(1024);
    if let Some(content) = content {
        budget.add(&content.warnings)?;
    }
    budget.add(&content_error)?;
    let mut operation_read_hour = None;
    let mut operations: Vec<OperativeRosterObservation> = Vec::new();
    let mut agency_absent: Vec<super::country_groups::HourSpan> = Vec::new();
    let mut last_hour: Option<i32> = None;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    for history in &mut histories {
                        history.interrupt();
                    }
                    operation_read_hour = None;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Operatives roster request cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        for history in &mut histories {
                            history.interrupt();
                        }
                        operation_read_hour = None;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    if last_hour.is_some_and(|last| last.checked_add(1) != Some(frame.hour)) {
                        operation_read_hour = None;
                    }
                    last_hour = Some(frame.hour);
                    // collector::extended::intel times the operation/assignment walk
                    // under Operations itself. A held table or slot peer is no read.
                    let operations_read = decoded.complete::<tables::Operations>().is_ok()
                        && operative_rows::operations_read(decoded);
                    if decoded.complete::<tables::Operations>().is_err() {
                        operation_read_hour = None;
                    } else if operations_read {
                        operation_read_hour = Some(frame.hour);
                    }
                    if let Some(read_hour) = operation_read_hour {
                        match operations.last_mut() {
                            Some(observation)
                                if observation.last_hour.checked_add(1) == Some(frame.hour)
                                    && observation.read_hour == read_hour =>
                            {
                                observation.last_hour = frame.hour
                            }
                            _ => {
                                let observation = OperativeRosterObservation {
                                    first_hour: frame.hour,
                                    last_hour: frame.hour,
                                    read_hour,
                                };
                                budget.add(&observation)?;
                                operations.push(observation);
                            }
                        }
                    }
                    // Feature availability is independent of whether the roster table
                    // exists: an absent agency must be reported even in old recordings.
                    let feature = operative_rows::agency_feature(decoded);
                    if feature != Some(true) {
                        for history in &mut histories {
                            history.interrupt();
                            history.track.step(None, frame.hour);
                        }
                        if feature == Some(false) {
                            match agency_absent.last_mut() {
                                Some(span) if span.last_hour.checked_add(1) == Some(frame.hour) => {
                                    span.last_hour = frame.hour
                                }
                                _ => {
                                    let span = super::country_groups::HourSpan {
                                        first_hour: frame.hour,
                                        last_hour: frame.hour,
                                    };
                                    budget.add(&span)?;
                                    agency_absent.push(span);
                                }
                            }
                            refused.note(
                                "The intelligence agency feature is absent",
                                "SimulationFeatures DLC mask bit 5",
                            );
                        } else {
                            refused.note(
                                "The intelligence agency feature was not recorded",
                                "SimulationFeatures",
                            );
                        }
                        continue;
                    }
                    let (Ok(countries), Ok(links), Ok(rows)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::OperativeMemberships>(),
                    ) else {
                        for history in &mut histories {
                            history.interrupt();
                            history.track.step(None, frame.hour);
                        }
                        continue;
                    };
                    let read = operative_rows::membership_read(decoded);
                    // Build the full identity catalogue only for actual roster reads.
                    let slots = if read {
                        let (slots, unresolved) =
                            operative_rows::country_rows_partial(countries, links);
                        for id in unresolved {
                            refused.note("country identity is unresolved", format!("country {id}"));
                        }
                        slots
                    } else {
                        HashMap::new()
                    };
                    // Country tags are native identities. Reuse a validated holder by
                    // binary lookup; rebuild the catalogue at reads or identity changes.
                    let identity_changed = histories.iter().any(|history| {
                        history.country.is_some_and(|id| {
                            operative_rows::country_at(id, countries, links)
                                .is_none_or(|row| row.tag != history.reply.tag.as_bytes())
                        })
                    });
                    let tagged =
                        (read || identity_changed).then(|| tagged_countries(countries, links));
                    let mut renamed = BTreeMap::new();
                    if !read && identity_changed {
                        let (current_slots, _) =
                            operative_rows::country_rows_partial(countries, links);
                        let mut aliases = BTreeMap::new();
                        for row in countries {
                            let current = current_slots
                                .get(&row.canonical_slot)
                                .filter(|found| found.id == row.id)
                                .map(|_| CountryIdentity {
                                    slot: row.canonical_slot,
                                    id: row.id,
                                });
                            aliases
                                .entry(row.tag.as_slice())
                                .and_modify(|found| *found = None)
                                .or_insert(current);
                        }
                        for history in &mut histories {
                            let Some(country) = history.country else {
                                continue;
                            };
                            let Some(row) = operative_rows::country_at(country, countries, links)
                            else {
                                continue;
                            };
                            if !history.open
                                || history.reply.observed.last().is_none_or(|observation| {
                                    observation.last_hour.checked_add(1) != Some(frame.hour)
                                })
                                || row.tag == history.reply.tag.as_bytes()
                                || aliases.get(row.tag.as_slice()).copied().flatten()
                                    != Some(country)
                            {
                                continue;
                            }
                            let joined: Vec<_> = history
                                .active
                                .iter()
                                .map(|(&id, &index)| {
                                    (id, history.reply.operatives[index].joined_hour)
                                })
                                .collect();
                            // Move the actual held census, not the carried table's rows.
                            // Duplicate sources cannot choose a roster for this holder.
                            renamed
                                .entry(country)
                                .and_modify(|carry| *carry = None)
                                .or_insert(Some((
                                    std::mem::take(&mut history.held),
                                    joined,
                                    history.read_hour,
                                )));
                        }
                    }
                    let characters = if read {
                        character_links(
                            decoded
                                .complete::<tables::CharacterOperatives>()
                                .unwrap_or(&[]),
                            &mut refused,
                        )
                    } else {
                        HashMap::new()
                    };
                    // Group once; never country x operative scanning, nor a player/AI test.
                    let mut by_slot: HashMap<u32, Vec<&OperativeMembershipRow>> = HashMap::new();
                    let mut roster_links_known = true;
                    if read {
                        for row in rows {
                            if let Some(slot) = link_slot(links, row.roster_country) {
                                by_slot.entry(slot).or_default().push(row);
                            } else {
                                roster_links_known = false;
                                refused.note(
                                    "unresolved roster country",
                                    format!("operative {} / roster {}", row.id, row.roster_country),
                                );
                            }
                        }
                    }
                    for history in &mut histories {
                        let current = (if let Some(tagged) = &tagged {
                            tagged
                                .get(history.reply.tag.as_bytes())
                                .and_then(|country| country.found())
                                .map(|(country, slot)| CountryIdentity {
                                    slot,
                                    id: country.id,
                                })
                        } else {
                            history.country
                        })
                        .filter(|country| {
                            selected_identities.is_none_or(|selected| {
                                selected.contains(&(country.slot, country.id))
                            })
                        });
                        let continues = history
                            .track
                            .step(current.map(|c| (c.slot, c.id)), frame.hour);
                        if !continues {
                            history.interrupt();
                        }
                        if current != history.country {
                            // A new holder of the tag is not evidence that the old holder's
                            // operatives died or retired. End identity, without inventing exits.
                            history.active.clear();
                            history.latest.clear();
                            history.country = current;
                        }
                        let Some(current) = current else {
                            continue;
                        };
                        if let Some(Some((held, joined, read_hour))) = renamed.remove(&current) {
                            history.active.clear();
                            history.latest.clear();
                            for (id, joined_hour) in joined {
                                let entry = history.entry(id, joined_hour);
                                budget.add(&entry)?;
                                history.active.insert(id, history.reply.operatives.len());
                                history.latest.insert(id, history.reply.operatives.len());
                                history.reply.operatives.push(entry);
                            }
                            history.held = held;
                            history.read_hour = read_hour;
                            history.open = true;
                        }
                        if read {
                            let mut values = BTreeMap::new();
                            let mut departed = Vec::new();
                            let mut unique = BTreeMap::new();
                            // An unassigned row could belong to any requested country;
                            // it cannot turn that country's unknown roster into empty.
                            let mut complete = roster_links_known;
                            for row in by_slot.get(&current.slot).into_iter().flatten() {
                                if let Err(reason) = operative_rows::supported_kind_assignment(
                                    row.roster_kind,
                                    row.assignment,
                                ) {
                                    refused.note(reason, format!("operative {}", row.id));
                                    complete = false;
                                    continue;
                                }
                                // Only the active agency roster establishes membership.
                                if row.roster_kind != 0 {
                                    continue;
                                }
                                match operative_rows::admit(&mut unique, row, links, &slots) {
                                    Ok(true) => {}
                                    Ok(false) => continue,
                                    Err(reason) => {
                                        refused.note(reason, format!("operative {}", row.id));
                                        complete = false;
                                        continue;
                                    }
                                }
                                let operation_token = row
                                    .operation
                                    .require("OperativeMemberships.operation")
                                    .ok()
                                    .copied()
                                    .flatten()
                                    .and_then(|id| operation_name(decoded, id))
                                    .filter(|_| operation_read_hour.is_some());
                                let mut value = operative_values(
                                    content,
                                    decoded,
                                    row,
                                    characters.get(&row.id).copied().flatten(),
                                    operation_token,
                                    (&mut tables, &mut budget, &mut refused),
                                )?;
                                let owner = identity(row.owner_country, links, &slots);
                                let target = row
                                    .target_country
                                    .require("OperativeMemberships.target_country")
                                    .ok()
                                    .copied()
                                    .flatten()
                                    .and_then(|raw| identity(raw, links, &slots));
                                // Captured status resolves raw zero to the reserved slot;
                                // it needs no Countries row.
                                let capturer = (value.captured == Some(true))
                                    .then(|| {
                                        if row.capturer_country == 0
                                            || link_slot(links, row.capturer_country) == Some(0)
                                        {
                                            Some(CountryIdentity { slot: 0, id: 0 })
                                        } else {
                                            identity(row.capturer_country, links, &slots)
                                        }
                                    })
                                    .flatten();
                                for (needed, resolved, role) in [
                                    (true, owner, "owner"),
                                    (
                                        row.target_country
                                            .require("OperativeMemberships.target_country")
                                            .map_or(true, Option::is_some),
                                        target,
                                        "target",
                                    ),
                                    (value.captured == Some(true), capturer, "capturer"),
                                ] {
                                    if needed && resolved.is_none() {
                                        refused.note(
                                            format!("operative {role} not recorded or unresolved"),
                                            format!("operative {} {role}", row.id),
                                        );
                                    }
                                }
                                value.owner_tag = tag_of(owner, &slots);
                                value.target_tag = tag_of(target, &slots);
                                value.capturer_tag = tag_of(capturer, &slots);
                                let held = Held {
                                    value,
                                    owner,
                                    target,
                                    capturer,
                                    operation: (row.assignment == 4)
                                        .then(|| {
                                            row.operation
                                                .require("OperativeMemberships.operation")
                                                .ok()
                                                .copied()
                                                .flatten()
                                        })
                                        .flatten(),
                                    captured_input: None,
                                    cooldown: operative_rows::cooldown(row).ok().flatten(),
                                    cooldown_remaining: None,
                                };
                                // Disband writes assignment 2 and death writes 5;
                                // the agency status dispatcher has neither arm.
                                if matches!(row.assignment, 2 | 5) {
                                    departed.push((
                                        row.id,
                                        held,
                                        if row.assignment == 2 {
                                            "Disbanded"
                                        } else {
                                            "Killed"
                                        },
                                    ));
                                } else if values.insert(row.id, held).is_some() {
                                    refused.note(
                                        "conflicting active memberships",
                                        format!("operative {}", row.id),
                                    );
                                    complete = false;
                                }
                            }
                            if complete {
                                history.read(frame.hour, values, departed, &mut budget)?;
                            } else {
                                history.interrupt();
                            }
                        }
                        if history.open
                            && operations_read
                            && let Some(content) = content
                        {
                            // Operations has its own daily slot. A newly recorded definition
                            // may name a held membership before the next membership read;
                            // date that identity separately, without changing assignment or target.
                            for (&id, held) in &mut history.held {
                                if let Some(operation) = held.operation {
                                    let token = operation_name(decoded, operation);
                                    let assignment = content.assignment(
                                        &OperativeMembershipRow {
                                            id,
                                            assignment: 4,
                                            operation: Some(operation).into(),
                                            ..Default::default()
                                        },
                                        token,
                                        None,
                                    );
                                    held.value.assignment_name = assignment.name;
                                    held.value.assignment_icon =
                                        tables.artwork.intern(assignment.icon, &mut budget)?;
                                    held.value.assignment_missing = tables
                                        .reasons
                                        .intern(assignment.missing.clone(), &mut budget)?;
                                    held.value.assignment_icon_missing = tables
                                        .reasons
                                        .intern(assignment.icon_missing.clone(), &mut budget)?;
                                    if let Some(reason) = assignment.missing {
                                        refused.note(reason, format!("operative {id} assignment"));
                                    }
                                    if let Some(reason) = assignment.icon_missing {
                                        refused.note(
                                            reason,
                                            format!("operative {id} assignment icon"),
                                        );
                                    }
                                }
                            }
                        }
                        if operation_read_hour.is_none() {
                            for held in history
                                .held
                                .values_mut()
                                .filter(|held| held.operation.is_some())
                            {
                                held.value.assignment_name = None;
                                held.value.assignment_icon = None;
                                held.value.assignment_missing = tables.reasons.intern(
                                Some(
                                    "Operation identity is unavailable until a fresh Operations read"
                                        .into(),
                                ),
                                &mut budget,
                            )?;
                                held.value.assignment_icon_missing = held.value.assignment_missing;
                            }
                        }
                    }
                    if let Some(content) = content {
                        let capturers = histories
                            .iter()
                            .flat_map(|history| history.held.values())
                            .filter(|held| held.value.captured == Some(true))
                            .filter_map(|held| held.capturer)
                            .collect();
                        names.update(decoded, capturers, &content.catalogue.localisation);
                    }
                    for history in &mut histories {
                        history.publish(
                            frame.hour,
                            &mut Projection {
                                countries,
                                links,
                                content,
                                names: &names,
                                tables: &mut tables,
                                refused: &mut refused,
                                budget: &mut budget,
                            },
                        )?;
                    }
                    if budget.0.saturating_add(refused.check()?) > REPLY_BUDGET {
                        return Err(io::Error::other(
                            "Operatives roster exceeds its complete-response budget",
                        ));
                    }
                }
            }
            Ok(())
        },
    )?;
    refused.check()?;
    let mut warnings = content.map_or_else(Vec::new, |content| content.warnings.clone());
    warnings.extend(refused.lines("operative fields"));
    if content.is_none() {
        warnings.push("Operative assignments and experience display need the recorded content stack. Names show their saved literal/key until that stack is available.".into());
    }
    warnings.extend(content_error.map(|error| format!("Recorded content unavailable: {error}")));
    let reply = OperativeRosterReply {
        branch_id: branch.to_string(),
        countries: histories.into_iter().map(|h| h.reply).collect(),
        operations,
        agency_absent,
        artwork: tables.artwork.values,
        reasons: tables.reasons.values,
        warnings,
    };
    Budget::measure(&reply, REPLY_BUDGET)?;
    Ok(reply)
}

// Character lookup matches the full leader reference; only operative
// category 3 reaches role 4 portrait selection.
fn character_links(
    rows: &[CharacterOperativeRow],
    refused: &mut BoundedRefusals,
) -> HashMap<u64, Option<u32>> {
    let mut result = HashMap::new();
    for row in rows.iter().filter(|row| row.resolved && row.category == 3) {
        let linked = result.entry(row.reference).or_insert(Some(row.character));
        if *linked != Some(row.character) {
            *linked = None;
            refused.note(
                "conflicting character links for one operative reference; its portrait is unknown",
                format!("operative {} portrait", row.reference),
            );
        }
    }
    result
}

fn operation_name(frame: &replay::frame::DecodedFrame, id: u64) -> Option<&str> {
    let operations = frame.complete::<tables::Operations>().ok()?;
    let operation = operations
        .binary_search_by_key(&id, |row| row.id)
        .ok()
        .map(|index| &operations[index])?;
    let lexical = frame.complete::<tables::LexicalTokens>().ok()?;
    let key = lexical
        .binary_search_by_key(&operation.definition_token, |row| row.token)
        .ok()
        .map(|index| &lexical[index])?;
    std::str::from_utf8(&key.literal).ok()
}

impl Product for OperativesRoster {
    const KEY: &'static str = "operatives-roster";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OperativeRosterParams;
    type Reply = OperativeRosterReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_countries(&params.branch_id, &params.tags)
    }
    fn types() -> Vec<String> {
        vec![
            OperativeRosterParams::decl(),
            OperativeRosterValues::decl(),
            OperativeRosterRun::decl(),
            OperativeRosterEntry::decl(),
            OperativeRosterObservation::decl(),
            OperativeRosterCountry::decl(),
            OperativeRosterReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let content = super::operative_content::content(source, context);
        let branch = params.branch_id.parse().expect("validated branch");
        let history =
            super::country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let selected = super::country_history::selected_identities(&history, &params.tags);
        let mut tags = super::country_history::selected_aliases(&history, &params.tags);
        let mut seen: BTreeSet<_> = tags.iter().cloned().collect();
        for tag in &params.tags {
            if seen.insert(tag.clone()) {
                tags.push(tag.clone());
            }
        }
        let mut reply = scan(
            &source.replay(),
            branch,
            &tags,
            content.as_ref().ok().and_then(|value| value.as_deref()),
            cancelled,
            content.as_ref().err().map(ToString::to_string),
            Some(&selected),
        )?;
        let mut budget = Budget(Budget::measure(&reply, REPLY_BUDGET)?);
        for country in &reply.countries {
            if let Some(recorded) = history.countries.get(&country.tag) {
                budget.add(recorded.identities.as_ref())?;
            }
        }
        for country in &mut reply.countries {
            if let Some(recorded) = history.countries.get(&country.tag) {
                country.identities = recorded.identities.as_ref().clone();
            }
        }
        Budget::measure(&reply, REPLY_BUDGET)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
