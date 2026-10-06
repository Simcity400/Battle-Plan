//! Every country's recorded agency, without a player/AI admission gate.
//!
//! The agency screen `140F172A0` shows +0xE4 / +0xF0, not the slowly growing
//! recruitment admission ceiling +0xF4. `140FC9F60` stores the computed maximum
//! (`140FC2B80`) in +0xF0; the capture's `unknown_f0` preserves those bits.
//! Defence comes verbatim from +0x128 (`140FC7CB0`), then the screen localises
//! AGENCY_DEFENSE_LEVEL with its fixed5 LEVEL argument (`1402E2DA0`).
//!
//! Agencies are hourly. Equal consecutive reads compress, but missing rows,
//! failed tables, gaps and identity changes end a run. Membership counts have
//! their own daily read date and never replace the screen's recorded +0xE4.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::HourSpan,
    country_history::{Identity, observe_identity, unique_by_slot},
    validate_tag,
};
use crate::{
    Source,
    catalogue::plain_text,
    metadata::{
        IdentityTrack, TaggedCountry, link_slot, slot_read, tagged_countries, validate_campaign,
    },
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{IntelligenceAgencyRow, OperativeMembershipRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct AgencyComparison;

const CACHED_BRANCHES: usize = 2;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum AgencyRankField {
    Defense,
    Active,
    Maximum,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyTop {
    pub count: u32,
    pub field: AgencyRankField,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyComparisonParams {
    pub branch_id: String,
    /// Empty means all countries; tags are never restricted by human status.
    pub tags: Vec<String>,
    #[serde(default)]
    pub include_uncreated: bool,
    pub top: Option<AgencyTop>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyUpgradeItem {
    /// This read's array position, preserving repeated definitions and all levels
    /// as Save140FCA0F0 writes the native +0x60/+0x6C entries.
    pub ordinal: u32,
    pub definition_pointer: String,
    pub branch_pointer: String,
    /// CAgencyUpgrade+0x260 owns the branch; branch+8 and definition+8 are
    /// separate lexical tokens (140615D50, 140618C40, 140FCA0F0). A token alone is ambiguous.
    pub branch_token: Option<String>,
    pub token: Option<String>,
    /// Raw lexical IDs survive a missing recorded token-name dictionary.
    pub branch_token_id: Option<u32>,
    pub token_id: Option<u32>,
    pub level: i32,
    /// Capture authentication status; refused entries retain their raw values.
    pub identity_status: u8,
    pub identity_refusal: Option<String>,
    pub name: Option<String>,
    pub content_reason: Option<String>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyUpgrades {
    pub read_hour: Option<i32>,
    /// A failed or incomplete acquired-list read is distinct from absent capture.
    pub unavailable: Option<String>,
    /// Mathematical sum of max(0, signed level), unknown for an incomplete or
    /// unauthenticated list. The nonnegative term comes from 140FC2B80; this
    /// inventory sum omits its created gate, i32 wrapping, base and slot formula.
    pub nonnegative_level_sum: Option<String>,
    pub items: Vec<AgencyUpgradeItem>,
}

/// The one upgrade adapter. The captured scalar +0xC8 is elapsed creation or
/// upgrade work, not acquired upgrades (`140FC6AB0`, `140FC9710`). Acquired
/// levels live in +0x60/+0x6C entries, saved by `140FCA0F0`, not captured on main.
/// A follow-up capture wires that table here, with its content names and icons.
fn upgrades_at_read(_row: &AgencyProjection) -> AgencyUpgrades {
    AgencyUpgrades {
        read_hour: None,
        unavailable: None,
        nonnegative_level_sum: None,
        items: Vec::new(),
    }
}

fn upgrades_unrecorded() -> Option<String> {
    Some("Not recorded: this recording has no acquired agency upgrade levels; upgrade progress measures work on the current creation or upgrade.".into())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyValue {
    pub created: bool,
    /// Created names are literal recorded bytes, including an empty name.
    /// Uncreated names use GetAgency's content-localised default (141A7B350).
    pub name: Option<String>,
    pub defense_raw: String,
    /// The full native content-localised text, retained for the tooltip.
    pub defense_text: Option<String>,
    /// Only LEVEL's native formatted output (1402E2DA0 -> 142232AB0).
    pub defense_value: Option<String>,
    pub active_count: i32,
    pub maximum: i32,
    pub upgrades: AgencyUpgrades,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyEntry {
    pub first_hour: i32,
    pub last_hour: i32,
    /// First supplying read; every hour in this run independently reconfirms
    /// the agency scalars. The operative list retains its separate daily date.
    pub read_hour: i32,
    pub value: AgencyValue,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencySeries {
    pub tag: String,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    pub entries: Vec<AgencyEntry>,
    pub rosters: Vec<AgencyRosterRun>,
}

/// Active-list counts carry independently from agency-screen scalars. Their
/// latest supplying read is in the reply's operative_read_hours, not repeated
/// for each country or incorporated into the agency-screen run identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyRosterRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyComparisonReply {
    pub branch_id: String,
    /// False only when the branch contains no readable agency table at all.
    pub available: bool,
    /// Complete country and agency tables at these hours. A missing country
    /// row inside them is still missing, never a zero-valued agency.
    pub observed: Vec<HourSpan>,
    pub operative_read_hours: Vec<i32>,
    pub series: Vec<AgencySeries>,
    /// Label from the screen's AGENCY_DEFENSE_LEVEL template (140F172A0).
    pub defense_label: Option<String>,
    /// One recording-level reason, rather than repeating it at every agency read.
    pub upgrades_unrecorded: Option<String>,
    pub warnings: Vec<String>,
}

/// Only agency-screen values participate in run identity. DailyUpdate140FC6AB0
/// changes +0xC8 work progress and +0xF4 admission independently; neither is
/// displayed by the active/maximum screen140F172A0 and neither is stored here.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AgencyProjection {
    created: bool,
    name: Vec<u8>,
    defense_raw: i64,
    active_count: i32,
    unknown_f0: u32,
}

impl From<&IntelligenceAgencyRow> for AgencyProjection {
    fn from(row: &IntelligenceAgencyRow) -> Self {
        let created = row.created != 0;
        Self {
            created,
            // GetAgency141A7B350 uses the content default until created.
            name: if created {
                row.name.clone()
            } else {
                Vec::new()
            },
            defense_raw: row.defense_raw,
            active_count: row.active_count,
            unknown_f0: row.unknown_f0,
        }
    }
}

impl AgencyProjection {
    fn matches(&self, row: &IntelligenceAgencyRow) -> bool {
        let created = row.created != 0;
        self.created == created
            && (!created || self.name == row.name)
            && self.defense_raw == row.defense_raw
            && self.active_count == row.active_count
            && self.unknown_f0 == row.unknown_f0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawEntry {
    first_hour: i32,
    last_hour: i32,
    read_hour: i32,
    row: AgencyProjection,
}

#[derive(Clone, Default)]
struct Track {
    entries: Vec<RawEntry>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    last_value: Option<(Identity, i32)>,
    roster: Option<(Identity, i32, u32)>,
    rosters: Vec<AgencyRosterRun>,
}

#[derive(Default)]
struct ScannedAgencies {
    tracks: BTreeMap<String, Track>,
    available: bool,
    observed: Vec<HourSpan>,
    operative_read_hours: Vec<i32>,
    unresolved_rows: usize,
    untext_tags: usize,
    unsupported_rosters: BTreeMap<u8, usize>,
    warnings: BTreeMap<&'static str, usize>,
}

impl ScannedAgencies {
    fn note_roster_read(&mut self, hour: i32) {
        if self.operative_read_hours.last() != Some(&hour) {
            self.operative_read_hours.push(hour);
        }
    }

    fn observe(
        &mut self,
        hour: i32,
        tag: &str,
        identity: Identity,
        row: Option<&IntelligenceAgencyRow>,
        roster: Option<Option<u32>>,
    ) {
        // Avoid allocating the owned tag on every country-hour.
        let track = match self.tracks.get_mut(tag) {
            Some(track) => track,
            None => self.tracks.entry(tag.into()).or_default(),
        };
        if !observe_identity(
            &mut track.segments,
            &mut track.last_identity,
            identity,
            hour,
        ) {
            track.roster = None;
        }
        let previous_roster = track.roster;
        match roster {
            Some(value) => track.roster = value.map(|count| (identity, hour, count)),
            None => {
                if let Some((seen, at, count)) = track.roster {
                    track.roster = (seen == identity && at.checked_add(1) == Some(hour))
                        .then_some((seen, hour, count));
                }
            }
        }
        if let Some((_, _, count)) = track.roster {
            let follows = previous_roster.is_some_and(|(seen, at, previous)| {
                seen == identity && at.checked_add(1) == Some(hour) && previous == count
            });
            if follows {
                track.rosters.last_mut().expect("previous roster").last_hour = hour;
            } else {
                track.rosters.push(AgencyRosterRun {
                    first_hour: hour,
                    last_hour: hour,
                    count,
                });
            }
        }
        let Some(row) = row else {
            track.last_value = None;
            return;
        };
        let follows = track
            .last_value
            .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
        let equal = follows
            && track
                .entries
                .last()
                .is_some_and(|entry| entry.row.matches(row));
        if equal {
            track.entries.last_mut().expect("previous value").last_hour = hour;
        } else {
            let row = AgencyProjection::from(row);
            track.entries.push(RawEntry {
                first_hour: hour,
                last_hour: hour,
                read_hour: hour,
                row,
            });
        }
        track.last_value = Some((identity, hour));
    }
}

/// Scalar active_count is the +0xE4 active-list count (140FC80B0). Native recruitment
/// pools are separate memberships: roster_kind 0 is that active list, not all
/// entries an operative can have. Constructor140FC1D70 embeds recruitment at
/// +8; recruitment constructor141566E10 and remove141569770 establish its
/// three vectors, projected as kinds1..3 by engine::coperative_leader::walk.
/// CaptureOperative140FC5830 retains active membership, so a captured operative
/// still counts in the native active/maximum display; free-operatives is different.
fn roster_counts(
    rows: &[OperativeMembershipRow],
    links: &[replay::rows::CountryLinkRow],
) -> (HashMap<u32, u32>, usize, BTreeMap<u8, usize>) {
    let mut counts = HashMap::new();
    let mut unresolved = 0;
    let mut unsupported = BTreeMap::new();
    for row in rows {
        match row.roster_kind {
            0 => {}
            1..=3 => continue,
            other => {
                *unsupported.entry(other).or_default() += 1;
                unresolved += 1;
                continue;
            }
        }
        match link_slot(links, row.roster_country) {
            Some(slot) => {
                *counts.entry(slot).or_insert(0) += 1;
            }
            None => unresolved += 1,
        }
    }
    (counts, unresolved, unsupported)
}

fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedAgencies> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::IntelligenceAgencies,
        TableId::OperativeMemberships,
    ];
    let mut scanned = ScannedAgencies::default();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        check_cancelled(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            *scanned
                .warnings
                .entry("Unread replay block; agency history has a gap")
                .or_default() += 1;
            continue;
        };
        for frame in frames {
            check_cancelled(cancelled)?;
            let Ok(decoded) = decoder.advance(&frame) else {
                *scanned
                    .warnings
                    .entry("Unread replay frame; agency history has a gap")
                    .or_default() += 1;
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                *scanned
                    .warnings
                    .entry("Replay campaign identity mismatch; agency history has a gap")
                    .or_default() += 1;
                break;
            }
            let roster_read =
                slot_read(&decoded.metrics, TableId::OperativeMemberships, frame.hour);
            let memberships = decoded.complete::<tables::OperativeMemberships>().ok();
            if roster_read && memberships.is_some() {
                scanned.note_roster_read(frame.hour);
            }
            let (Ok(countries), Ok(links)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            ) else {
                continue;
            };
            let rows = decoded.complete::<tables::IntelligenceAgencies>().ok();
            scanned.available |= rows.is_some();
            if rows.is_some() {
                match scanned.observed.last_mut() {
                    Some(span) if span.last_hour.checked_add(1) == Some(frame.hour) => {
                        span.last_hour = frame.hour
                    }
                    _ => scanned.observed.push(HourSpan {
                        first_hour: frame.hour,
                        last_hour: frame.hour,
                    }),
                }
            }
            let by_slot = unique_by_slot(rows.into_iter().flatten(), links, |row| row.country);
            scanned.unresolved_rows +=
                by_slot.unresolved_rows + by_slot.duplicate_rows + by_slot.ambiguous_slots;
            let roster = if roster_read {
                memberships.map(|rows| roster_counts(rows, links))
            } else {
                None
            };
            if let Some((_, unresolved, unsupported)) = &roster {
                scanned.unresolved_rows += unresolved;
                for (kind, count) in unsupported {
                    *scanned.unsupported_rosters.entry(*kind).or_default() += count;
                }
            }
            for (tag, lookup) in tagged_countries(countries, links) {
                let TaggedCountry::Found(country, slot) = lookup else {
                    continue;
                };
                let Ok(tag) = std::str::from_utf8(tag) else {
                    scanned.untext_tags += 1;
                    continue;
                };
                let count = if memberships.is_none() {
                    Some(None)
                } else if let Some((counts, unresolved, _)) = &roster {
                    // A complete world roster proves an empty active list, except
                    // when a nonempty raw country reference could name this slot.
                    Some((*unresolved == 0).then(|| counts.get(&slot).copied().unwrap_or(0)))
                } else {
                    None
                };
                scanned.observe(
                    frame.hour,
                    tag,
                    (slot, country.id),
                    by_slot.rows.get(&slot).copied().flatten(),
                    count,
                );
            }
        }
    }
    Ok(scanned)
}

fn check_cancelled(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Agency comparison request cancelled",
        ))
    } else {
        Ok(())
    }
}

/// GetAgency copies even an empty stored name when created; otherwise it
/// localises the executable's AGENCY_NAME key (141A7B350 case 0x19).
fn agency_name(row: &AgencyProjection, default: impl FnOnce() -> Option<String>) -> Option<String> {
    if row.created {
        std::str::from_utf8(&row.name)
            .ok()
            .and_then(|name| plain_text(name).ok())
    } else {
        default()
    }
}

fn value(
    entry: &RawEntry,
    default: impl FnOnce() -> Option<String>,
    defense: impl FnOnce(i64) -> (Option<String>, Option<String>),
) -> AgencyValue {
    let row = &entry.row;
    let (defense_text, defense_value) = defense(row.defense_raw);
    AgencyValue {
        created: row.created,
        name: agency_name(row, default),
        defense_raw: row.defense_raw.to_string(),
        defense_text,
        defense_value,
        active_count: row.active_count,
        // 140FC7850 returns these bits; Int32ToString1424B56A0 displays signed.
        maximum: row.unknown_f0 as i32,
        upgrades: upgrades_at_read(row),
    }
}

fn rank(entry: &RawEntry, field: AgencyRankField) -> i64 {
    match field {
        AgencyRankField::Defense => entry.row.defense_raw,
        AgencyRankField::Active => i64::from(entry.row.active_count),
        AgencyRankField::Maximum => i64::from(entry.row.unknown_f0 as i32),
    }
}

#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, Arc<ScannedAgencies>)>,
}

fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedAgencies>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, result)) = scans.entries.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(result));
    }
    let result = Arc::new(scan(&source.replay(), branch, cancelled)?);
    scans.entries.push_back((branch, Arc::clone(&result)));
    while scans.entries.len() > CACHED_BRANCHES {
        scans.entries.pop_front();
    }
    Ok(result)
}

fn requested_series(
    scanned: &ScannedAgencies,
    params: &AgencyComparisonParams,
    default: impl Fn() -> Option<String>,
    defense: impl Fn(i64) -> (Option<String>, Option<String>),
    cancelled: &AtomicBool,
) -> io::Result<Vec<AgencySeries>> {
    let tags: BTreeSet<_> = params.tags.iter().collect();
    let candidates: BTreeMap<String, Vec<&RawEntry>> = scanned
        .tracks
        .iter()
        .filter(|(tag, _)| tags.is_empty() || tags.contains(tag))
        .map(|(tag, track)| {
            (
                tag.clone(),
                track
                    .entries
                    .iter()
                    .filter(|entry| params.include_uncreated || entry.row.created)
                    .collect(),
            )
        })
        .filter(|(_, entries): &(_, Vec<&RawEntry>)| !entries.is_empty())
        .collect();
    let memberships = params
        .top
        .as_ref()
        .map(|top| {
            crate::ranking::top_membership(
                &candidates,
                top.count as usize,
                |entry| (entry.first_hour, entry.last_hour),
                |entry| Some(rank(entry, top.field)),
                cancelled,
            )
        })
        .transpose()?;
    let mut result = Vec::new();
    // GetAgency141A7B350 resolves the content default only for uncreated agencies.
    let default_name = std::cell::LazyCell::new(default);
    let mut defense_texts = HashMap::new();
    for (tag, entries) in candidates {
        check_cancelled(cancelled)?;
        let membership = memberships.as_ref().and_then(|all| all.get(&tag)).cloned();
        if memberships.is_some() && membership.is_none() {
            continue;
        }
        let mut series = AgencySeries {
            segments: scanned.tracks[&tag].segments.clone(),
            rosters: scanned.tracks[&tag].rosters.clone(),
            tag,
            membership,
            entries: Vec::new(),
        };
        let mut append = |entry: &&RawEntry, first_hour, last_hour| -> io::Result<()> {
            let item = AgencyEntry {
                first_hour,
                last_hour,
                read_hour: entry.read_hour,
                value: value(
                    entry,
                    || (*default_name).clone(),
                    |_| {
                        defense_texts
                            .entry(entry.row.defense_raw)
                            .or_insert_with(|| defense(entry.row.defense_raw))
                            .clone()
                    },
                ),
            };
            series.entries.push(item);
            Ok(())
        };
        if let Some(spans) = &series.membership {
            crate::ranking::for_each_piece(
                &entries,
                spans,
                |entry| (entry.first_hour, entry.last_hour),
                cancelled,
                &mut append,
            )?;
        } else {
            for entry in &entries {
                append(entry, entry.first_hour, entry.last_hour)?;
            }
        }
        result.push(series);
    }
    Ok(result)
}

fn measured(value: &impl Serialize) -> io::Result<usize> {
    crate::transport::json_len(value, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        io::Error::other(
            "Agency comparison exceeds one complete reply's budget; no countries were omitted.",
        )
    })
}

impl Product for AgencyComparison {
    const KEY: &'static str = "agency-comparison";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AgencyComparisonParams;
    type Reply = AgencyComparisonReply;

    fn types() -> Vec<String> {
        vec![
            AgencyComparisonParams::decl(),
            AgencyRankField::decl(),
            AgencyTop::decl(),
            AgencyUpgradeItem::decl(),
            AgencyUpgrades::decl(),
            AgencyValue::decl(),
            AgencyEntry::decl(),
            AgencySeries::decl(),
            AgencyRosterRun::decl(),
            AgencyComparisonReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params
            .top
            .as_ref()
            .is_some_and(|top| !params.tags.is_empty() || top.count == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scanned = scanned_branch(
            source,
            params.branch_id.parse().expect("validated branch"),
            cancelled,
        )?;
        let mut warnings: Vec<_> = scanned
            .warnings
            .iter()
            .map(|(warning, count)| format!("{warning} ({count})"))
            .collect();
        let content = match context
            .content()
            .and_then(|options| super::intelligence_content::intelligence_content(source, options))
        {
            Ok(content) => Some(content),
            Err(error) => {
                warnings.push(format!("Agency localisation is unavailable: {error}"));
                None
            }
        };
        if scanned.unresolved_rows > 0 {
            warnings.push(format!("{} agency or operative membership rows do not name one resolved country; their values are unknown.", scanned.unresolved_rows));
        }
        if scanned.untext_tags > 0 {
            warnings.push(format!(
                "{} country-hours have non-text tags; they cannot be named.",
                scanned.untext_tags
            ));
        }
        for (kind, count) in &scanned.unsupported_rosters {
            warnings.push(format!("{count} operative memberships use unsupported roster_kind {kind}; their roster counts are unknown."));
        }
        let series = requested_series(
            &scanned,
            params,
            || {
                content
                    .as_ref()
                    .and_then(|content| content.default_agency_name())
            },
            |raw| {
                let text = content
                    .as_ref()
                    .and_then(|content| content.defense_text(raw));
                let value = content
                    .as_ref()
                    .zip(text.as_deref())
                    .and_then(|(content, text)| content.defense_value(text));
                (text, value)
            },
            cancelled,
        )?;
        let unnamed = series
            .iter()
            .flat_map(|series| &series.entries)
            .filter(|entry| entry.value.created && entry.value.name.is_none())
            .count();
        if unnamed > 0 {
            warnings.push(format!("{unnamed} agency name runs have unsupported non-text bytes or controls; their names are unknown."));
        }
        let defense_label = content.as_ref().and_then(|content| content.defense_label());
        if let Some(content) = content {
            warnings.extend(content.warnings.iter().cloned());
            warnings.extend(content.catalogue.refusal_lines());
        }
        let reply = AgencyComparisonReply {
            branch_id: params.branch_id.clone(),
            available: scanned.available,
            observed: scanned.observed.clone(),
            operative_read_hours: scanned.operative_read_hours.clone(),
            series,
            defense_label,
            upgrades_unrecorded: upgrades_unrecorded(),
            warnings,
        };
        measured(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(country: u32, defense_raw: i64) -> IntelligenceAgencyRow {
        IntelligenceAgencyRow {
            country,
            defense_raw,
            created: 1,
            name: b"Recorded agency".to_vec(),
            active_count: 2,
            unknown_f0: 7,
            admission_ceiling: 3,
            ..Default::default()
        }
    }

    fn params() -> AgencyComparisonParams {
        AgencyComparisonParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            include_uncreated: false,
            top: None,
        }
    }

    #[test]
    fn agency_comparison_gaps_missing_rows_and_identity_changes_stay_missing() {
        let mut scanned = ScannedAgencies::default();
        let agency = row(1, 25_000);
        scanned.note_roster_read(10);
        scanned.observe(10, "SYN", (1, 1), Some(&agency), Some(Some(2)));
        scanned.observe(11, "SYN", (1, 1), None, None);
        scanned.observe(12, "SYN", (1, 1), Some(&agency), None);
        // Hour 13 is a gap. A later equal read cannot bridge it.
        scanned.observe(14, "SYN", (1, 1), Some(&agency), None);
        scanned.observe(15, "SYN", (1, 2), Some(&agency), None);
        let series = requested_series(
            &scanned,
            &params(),
            || None,
            |_| (None, None),
            &AtomicBool::new(false),
        )
        .unwrap();
        let entries = &series[0].entries;
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.first_hour, entry.last_hour))
                .collect::<Vec<_>>(),
            [(10, 10), (12, 12), (14, 14), (15, 15)]
        );
        assert_eq!(
            series[0].rosters,
            [AgencyRosterRun {
                first_hour: 10,
                last_hour: 12,
                count: 2
            }]
        );
        assert_eq!(scanned.operative_read_hours, [10]);
        assert_eq!(entries[3].value.defense_raw, "25000");
        assert_eq!(entries[3].value.defense_text, None);
        assert_eq!(entries[3].value.defense_value, None);
    }

    #[test]
    fn agency_comparison_never_created_uses_localised_default_not_stored_name() {
        let mut agency = row(1, 0);
        agency.created = 0;
        agency.name = b"Stale stored bytes".to_vec();
        assert_eq!(
            agency_name(&AgencyProjection::from(&agency), || Some(
                "Agence du mod".into()
            )),
            Some("Agence du mod".into())
        );
        agency.created = 1;
        agency.name.clear();
        assert_eq!(
            agency_name(&AgencyProjection::from(&agency), || Some("Wrong".into())),
            Some(String::new())
        );
        let value = value(
            &RawEntry {
                first_hour: 10,
                last_hour: 10,
                read_hour: 10,
                row: AgencyProjection::from(&agency),
            },
            || None,
            |_| (Some("Localised zero: 0.00".into()), Some("0.00".into())),
        );
        assert_eq!(value.maximum, 7);
        assert_eq!(value.defense_text.as_deref(), Some("Localised zero: 0.00"));
        assert_eq!(value.defense_value.as_deref(), Some("0.00"));
        assert_eq!(value.upgrades.nonnegative_level_sum, None);
        assert!(upgrades_unrecorded().unwrap().starts_with("Not recorded:"));
    }

    #[test]
    fn agency_comparison_daily_work_and_admission_changes_keep_one_displayed_run() {
        let mut scanned = ScannedAgencies::default();
        let mut agency = row(1, 25_000);
        for hour in 0..(12 * 365 * 24) {
            agency.upgrade_progress_raw = i64::from(hour / 24) * 100_000;
            agency.admission_ceiling = hour / 24;
            let roster = (hour % 24 == 0).then_some(Some(2));
            if roster.is_some() {
                scanned.note_roster_read(hour);
            }
            scanned.observe(hour, "SYN", (1, 1), Some(&agency), roster);
        }
        let entries = &scanned.tracks["SYN"].entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].first_hour, entries[0].last_hour),
            (0, 12 * 365 * 24 - 1)
        );
        assert_eq!(
            scanned.tracks["SYN"].rosters,
            [AgencyRosterRun {
                first_hour: 0,
                last_hour: 12 * 365 * 24 - 1,
                count: 2
            }]
        );
        assert_eq!(
            scanned.operative_read_hours,
            (0..12 * 365).map(|day| day * 24).collect::<Vec<_>>()
        );
        // The same independently dated history for 5,000 synthetic countries
        // stays bounded; dates are shared, not repeated in each country's runs.
        let track = scanned.tracks.remove("SYN").unwrap();
        for country in 0..5000 {
            scanned
                .tracks
                .insert(format!("SYN_{country}"), track.clone());
        }
        let reply = AgencyComparisonReply {
            branch_id: "1".into(),
            available: true,
            observed: vec![HourSpan {
                first_hour: 0,
                last_hour: 12 * 365 * 24 - 1,
            }],
            operative_read_hours: scanned.operative_read_hours.clone(),
            series: requested_series(
                &scanned,
                &params(),
                || None,
                |_| {
                    (
                        Some("Synthetic protection: 0.25".into()),
                        Some("0.25".into()),
                    )
                },
                &AtomicBool::new(false),
            )
            .unwrap(),
            defense_label: Some("Synthetic protection".into()),
            upgrades_unrecorded: upgrades_unrecorded(),
            warnings: Vec::new(),
        };
        assert_eq!(reply.series.len(), 5000);
        assert_eq!(reply.defense_label.as_deref(), Some("Synthetic protection"));
        assert!(measured(&reply).unwrap() < MAX_PRODUCT_REPLY_BYTES);
    }

    #[test]
    fn agency_comparison_request_accepts_two_tags_and_u32_max_top_and_deduplicates_selection() {
        let mut request = params();
        request.tags = vec!["SYN".into(), "SYN".into()];
        assert!(AgencyComparison::validate(&request).is_ok());
        let mut scanned = ScannedAgencies::default();
        scanned.observe(1, "SYN", (1, 1), Some(&row(1, 0)), None);
        assert_eq!(
            requested_series(
                &scanned,
                &request,
                || None,
                |_| (None, None),
                &AtomicBool::new(false)
            )
            .unwrap()
            .len(),
            1
        );
        request.top = Some(AgencyTop {
            count: 1,
            field: AgencyRankField::Defense,
        });
        assert_eq!(
            AgencyComparison::validate(&request),
            Err("Invalid top countries")
        );
        request.tags.clear();
        request.top.as_mut().unwrap().count = u32::MAX;
        assert!(AgencyComparison::validate(&request).is_ok());
        request.top.as_mut().unwrap().count = 0;
        assert_eq!(
            AgencyComparison::validate(&request),
            Err("Invalid top countries")
        );
        request.top.as_mut().unwrap().count = 5_000;
        assert!(AgencyComparison::validate(&request).is_ok());
    }

    #[test]
    fn agency_comparison_refuses_the_shared_complete_reply_budget() {
        let mut scanned = ScannedAgencies::default();
        scanned.observe(1, "SYN", (1, 1), Some(&row(1, 0)), None);
        let mut reply = AgencyComparisonReply {
            branch_id: "1".into(),
            available: true,
            observed: vec![HourSpan {
                first_hour: 1,
                last_hour: 1,
            }],
            operative_read_hours: Vec::new(),
            series: requested_series(
                &scanned,
                &params(),
                || None,
                |_| (Some("x".repeat(MAX_PRODUCT_REPLY_BYTES)), None),
                &AtomicBool::new(false),
            )
            .unwrap(),
            defense_label: None,
            upgrades_unrecorded: upgrades_unrecorded(),
            warnings: Vec::new(),
        };
        let length = crate::transport::json_len(&reply, usize::MAX).unwrap();
        let text = reply.series[0].entries[0]
            .value
            .defense_text
            .as_mut()
            .unwrap();
        text.truncate(text.len() - (length - MAX_PRODUCT_REPLY_BYTES));
        assert_eq!(measured(&reply).unwrap(), MAX_PRODUCT_REPLY_BYTES);
        reply.series[0].entries[0]
            .value
            .defense_text
            .as_mut()
            .unwrap()
            .push('x');
        let error = measured(&reply).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Agency comparison exceeds one complete reply's budget")
        );
    }

    #[test]
    fn agency_comparison_unused_default_name_is_not_resolved() {
        let mut scanned = ScannedAgencies::default();
        scanned.observe(1, "SYN", (1, 1), Some(&row(1, 0)), None);
        let series = requested_series(
            &scanned,
            &params(),
            || panic!("created agencies must not resolve an unused default name"),
            |_| (None, None),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            series[0].entries[0].value.name.as_deref(),
            Some("Recorded agency")
        );
    }

    #[test]
    fn agency_comparison_localises_each_defense_value_and_default_name_once() {
        use std::cell::Cell;
        let mut scanned = ScannedAgencies::default();
        for country in 1..=5000 {
            let mut agency = row(country, i64::from(country % 2) * 100_000);
            agency.created = 0;
            scanned.observe(
                1,
                &format!("SYN_{country}"),
                (country, country),
                Some(&agency),
                None,
            );
        }
        let default_calls = Cell::new(0);
        let defense_calls = Cell::new(0);
        let mut request = params();
        request.include_uncreated = true;
        let series = requested_series(
            &scanned,
            &request,
            || {
                default_calls.set(default_calls.get() + 1);
                Some("Localised default".into())
            },
            |raw| {
                defense_calls.set(defense_calls.get() + 1);
                (
                    Some(format!("Localised defense: {raw}")),
                    Some(raw.to_string()),
                )
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(series.len(), 5000);
        assert_eq!(default_calls.get(), 1);
        assert_eq!(defense_calls.get(), 2);
        assert!(series.iter().all(|series| {
            let value = &series.entries[0].value;
            value.defense_value.as_deref() == Some(value.defense_raw.as_str())
        }));
        assert!(
            series
                .iter()
                .all(|series| series.entries[0].value.name.as_deref() == Some("Localised default"))
        );
    }

    #[test]
    fn agency_comparison_5000_countries_top_n_and_complete_response_budget() {
        let mut scanned = ScannedAgencies::default();
        for country in 1..=5000 {
            let agency = row(country, i64::from(country) * 100_000);
            scanned.observe(
                10,
                &format!("SYN_{country:04}"),
                (country, country),
                Some(&agency),
                None,
            );
            scanned.observe(
                11,
                &format!("SYN_{country:04}"),
                (country, country),
                Some(&agency),
                None,
            );
        }
        let cancelled = AtomicBool::new(false);
        let all =
            requested_series(&scanned, &params(), || None, |_| (None, None), &cancelled).unwrap();
        assert_eq!(all.len(), 5000);
        assert!(measured(&all).unwrap() < MAX_PRODUCT_REPLY_BYTES);
        assert!(
            all.iter()
                .all(|series| series.entries.len() == 1 && series.entries[0].last_hour == 11)
        );
        let mut top_params = params();
        top_params.top = Some(AgencyTop {
            count: 3,
            field: AgencyRankField::Defense,
        });
        let top =
            requested_series(&scanned, &top_params, || None, |_| (None, None), &cancelled).unwrap();
        assert_eq!(
            top.iter()
                .map(|series| series.tag.as_str())
                .collect::<Vec<_>>(),
            ["SYN_4998", "SYN_4999", "SYN_5000"]
        );
        assert_eq!(
            top[0].membership.as_ref().unwrap()[0],
            HourSpan {
                first_hour: 10,
                last_hour: 11
            }
        );
    }

    #[test]
    fn agency_comparison_replay_reads_5000_countries_and_refuses_gaps_without_zeroes() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        let mut first = source(1, 16, Some(1));
        *first.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5000)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("SYN_{id:04}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *first.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=5000)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *first.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(
                (1..=5000)
                    .map(|id| row(id, i64::from(id) * 100_000))
                    .collect(),
            ));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(Vec::new()));
        let mut missing = first.clone();
        missing.hour = 17;
        *missing.tables[TableId::IntelligenceAgencies.index()] = TableView::Unavailable {
            reason: "synthetic missing read".into(),
        };
        let mut later = first.clone();
        later.hour = 19;
        *later.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![row(1, 0)]));
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, missing, later])])
                .unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "agency-synthetic".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        let reply = AgencyComparison::load(
            &source,
            &params(),
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(reply.series.len(), 5000);
        assert_eq!(reply.defense_label, None);
        assert_eq!(
            reply.observed,
            [
                HourSpan {
                    first_hour: 16,
                    last_hour: 16
                },
                HourSpan {
                    first_hour: 19,
                    last_hour: 19
                }
            ]
        );
        let first = &reply.series[0];
        assert_eq!(first.entries.len(), 2);
        assert_eq!(
            first.rosters,
            [AgencyRosterRun {
                first_hour: 16,
                last_hour: 17,
                count: 0
            }]
        );
        assert_eq!(reply.operative_read_hours, [16]);
        assert_eq!(first.entries[1].value.defense_raw, "0");
        assert_eq!(first.entries[1].value.defense_value, None);
        assert_eq!(reply.series[1].entries.len(), 1);
        assert!(measured(&reply).unwrap() < MAX_PRODUCT_REPLY_BYTES);
    }

    #[test]
    fn agency_comparison_partial_rosters_stay_unknown_and_refuse_by_kind() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows},
        };
        let mut first = source(1, 16, Some(1));
        *first.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]));
        *first.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]));
        *first.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![row(1, 0)]));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![
                OperativeMembershipRow {
                    id: 1,
                    roster_country: 1,
                    roster_kind: 0,
                    ..Default::default()
                },
                OperativeMembershipRow {
                    id: 2,
                    roster_country: 99,
                    roster_kind: 0,
                    ..Default::default()
                },
                OperativeMembershipRow {
                    id: 3,
                    roster_country: 1,
                    roster_kind: 4,
                    ..Default::default()
                },
            ]));
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first])]).unwrap());
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "agency-partial".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        let reply = AgencyComparison::load(
            &source,
            &params(),
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        let value = &reply.series[0].entries[0].value;
        assert_eq!((value.active_count, value.maximum), (2, 7));
        assert!(reply.series[0].rosters.is_empty());
        assert_eq!(reply.operative_read_hours, [16]);
        assert!(reply.warnings.iter().any(|warning| {
            warning.starts_with("1 operative memberships use unsupported roster_kind 4")
        }));
        assert!(
            reply
                .warnings
                .iter()
                .any(|warning| warning.starts_with("2 agency or operative membership rows"))
        );
    }

    #[test]
    fn agency_comparison_corrupt_blocks_report_the_missing_history() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![source(1, 16, Some(1))])])
                .unwrap();
        let mut bytes = replay.bytes().to_vec();
        let block = &replay.index[0];
        bytes[(block.offset + block.length - 1) as usize] ^= 0x40;
        let damaged = Replay::open(bytes).unwrap();
        let scanned = scan(&damaged, 1, &AtomicBool::new(false)).unwrap();
        assert!(!scanned.available);
        assert!(scanned.observed.is_empty());
        assert_eq!(
            scanned
                .warnings
                .get("Unread replay block; agency history has a gap"),
            Some(&1)
        );
    }
}
