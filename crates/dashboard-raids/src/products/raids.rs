//! One prepared observer shared by the Raid log and Raid tally. All countries
//! enter the observer. Hourly reads supply every value; gaps end runs, never imply zero.
//! Counts are observed launches/results, not an invented pre-recording census.

use dashboard_base::products::{
    MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun, Identity, RecordedHistories},
    finite_fixed64,
};
use dashboard_base::{catalogue, source::Source};
use dashboard_core::transport::json_len;
use dashboard_core::{config::LoadContext, protocol::validate_tag};
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
use dashboard_text::{
    catalogue::{Catalogue, Refusals, View},
    metadata::{IdentityTrack, LinkTags, mapped_slot, slot_read},
    scripted_text::{self, StateNames},
};
use engine_native::craid_instance as native;
use installed_content_raids::{
    raids::Catalog, raids::RaidType, raids::target::country as target_country,
    raids::target::state as target_state,
};
#[cfg(any(test, feature = "test-support"))]
use replay::container::Replay;
use replay_data::{rows::RaidRow, rows::RaidScope, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::{collections::VecDeque, sync::atomic::Ordering};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct Raids;

pub mod kept;
pub mod prepared;

#[cfg(any(test, feature = "test-support"))]
const CACHED_BRANCHES: usize = 4;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum RaidsView {
    Catalogue,
    Log,
    Tally,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum RaidMetric {
    Launched,
    Succeeded,
    Both,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RaidTop {
    pub count: u32,
    pub metric: RaidMetric,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RaidsParams {
    pub branch_id: String,
    pub view: RaidsView,
    pub tags: Vec<String>,
    pub top: Option<RaidTop>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidTypeInfo {
    pub token: String,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidArtwork {
    pub key: String,
    pub image: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidDate {
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub raider: Option<String>,
    pub victim: Option<String>,
    #[serde(rename = "type")]
    pub raid_type: Option<String>,
    pub province: Option<u32>,
    pub state: Option<u32>,
    pub target_name: Option<String>,
    /// Sprite key into RaidsReply.artwork, so each PNG is returned only once.
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub phase: i32,
    pub phase_name: Option<String>,
    pub outcome: i32,
    pub outcome_name: Option<String>,
    /// Null for an ended raid whose launch was never observed.
    pub launched: Option<bool>,
    pub succeeded: Option<bool>,
}

/// (First read, last read, preparation at first read, change per read hour).
/// Stored preparation hours supply elapsed phase time in phase 2 only.
/// Every hour in a run was read and matched its exact slope; a missing read
/// ends it, so this encoding never interpolates across a gap.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct RaidPreparationRun(pub i32, pub i32, pub i32, #[ts(type = "number")] pub i64);

impl RaidPreparationRun {
    fn extend(&mut self, hour: i32, hours: i32) -> bool {
        if self.1.checked_add(1) != Some(hour) {
            return false;
        }
        if self.0 == self.1 {
            self.3 = i64::from(hours) - i64::from(self.2);
        } else if i128::from(self.2) + (i128::from(hour) - i128::from(self.0)) * i128::from(self.3)
            != i128::from(hours)
        {
            return false;
        }
        self.1 = hour;
        true
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidEpisode {
    pub id: String,
    /// Creation/start dates are not in Raids. This is a bound, never an exact start.
    pub start: RaidDate,
    pub runs: Vec<RaidRun>,
    /// Independent exact runs: preparation changes must not split fact runs.
    pub preparation: Vec<RaidPreparationRun>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidTallyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    pub launched: u32,
    pub succeeded: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidTallySeries {
    pub tag: String,
    pub runs: Vec<RaidTallyRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidsReply {
    pub branch_id: String,
    /// Native phase IDs, shared with consumers instead of duplicated UI numbers.
    pub phase_preparing: i32,
    pub phase_ended: i32,
    pub outcome_none: i32,
    /// False only when content or the recording proves raids unavailable.
    pub available: Option<bool>,
    pub unavailable: Option<String>,
    pub types: Vec<RaidTypeInfo>,
    pub artwork: Vec<RaidArtwork>,
    pub episodes: Vec<RaidEpisode>,
    pub series: Vec<RaidTallySeries>,
    /// Successful complete census reads with every current country's parent row
    /// resolved. Partial per-country values remain in series, never a world zero.
    pub read_hours: Vec<i32>,
    pub warnings: Vec<String>,
}

/// Execution and auto-launch enter phase 4. Ending can also run directly
/// from the console for any non-ended phase, so a terminal result cannot
/// prove an unseen launch. Finite nonzero travelled proves flight: it starts
/// at zero, entering phase 4 clears it, and hourly updates add speed only
/// in phase 4. Ending preserves it; loading restores the trusted saved distance.
fn launched(row: &RaidRow) -> Option<bool> {
    if row
        .travelled
        .is_some_and(|distance| finite_fixed64(distance) && distance != 0)
    {
        return Some(true);
    }
    match row.phase {
        native::PHASE_IN_PROGRESS => Some(true),
        native::PHASE_ENDED => None,
        0..=native::PHASE_PREPARED => Some(false),
        _ => None,
    }
}

/// The success branch tests `(outcome - 3U) < 2`: limited success (2)
/// is excluded, exactly as the success-only effects are.
fn succeeded(row: &RaidRow) -> Option<bool> {
    match row.outcome {
        native::OUTCOME_SUCCESS | native::OUTCOME_CRITICAL_SUCCESS
            if row.phase == native::PHASE_ENDED =>
        {
            Some(true)
        }
        native::OUTCOME_NONE..=native::OUTCOME_CANCELED
            if (0..=native::PHASE_ENDED).contains(&row.phase) =>
        {
            Some(false)
        }
        _ => None,
    }
}

pub struct Content {
    catalogue: Arc<Catalogue>,
    raids: Catalog,
    names: StateNames,
    types: Vec<RaidTypeInfo>,
    warnings: Vec<String>,
    icons: Mutex<BTreeMap<String, Result<String, String>>>,
}

fn icon_key(cat: &Catalogue, kind: &RaidType, template: Option<&str>) -> Result<String, String> {
    // The fallback sprite key is an interface literal; whether its sprite
    // exists and all pixels are content facts.
    let refused = std::cell::RefCell::new(None);
    let key = kind
        .icon(template, |key| match cat.sprites.find(key) {
            Ok(sprite) => sprite.is_some(),
            Err(reason) => {
                *refused.borrow_mut() = Some(format!("raid sprite {key}: {reason}"));
                false
            }
        })
        .unwrap_or_else(|| "GFX_raid_type_icon_unknown".into());
    match refused.into_inner() {
        Some(reason) => Err(reason),
        None => Ok(key),
    }
}

fn decode_icon(cat: &Catalogue, key: &str) -> Result<String, String> {
    let sprite = cat
        .sprites
        .find(key)?
        .ok_or_else(|| format!("raid sprite {key} is unavailable"))?;
    catalogue::sprite_frame_png(&cat.content.layers, sprite, cat.gui_decode.clone()?, 0, 48)
        .map_err(|error| format!("raid sprite {key}: {error}"))
}

fn localised_label(cat: &Catalogue, key: &str) -> Option<String> {
    cat.localisation.get(key)?;
    cat.localised(key, View::Plain)
}

// Raid names use raid_type_<database name>.
fn type_name_key(kind: &RaidType) -> String {
    format!("raid_type_{}", kind.name)
}

fn type_info(cat: &Catalogue, kind: &RaidType) -> RaidTypeInfo {
    let name_key = type_name_key(kind);
    let name = localised_label(cat, &name_key);
    RaidTypeInfo {
        token: kind.name.clone(),
        name,
        missing: None,
    }
}

fn prepare_types(cat: &Catalogue, raids: &Catalog) -> Vec<RaidTypeInfo> {
    raids
        .types
        .iter()
        .map(|kind| type_info(cat, kind))
        .chain(raids.uncertain.iter().map(|kind| RaidTypeInfo {
            token: kind.name.clone(),
            name: None,
            missing: Some(kind.reason.clone()),
        }))
        .collect()
}

pub fn check_content(cat: &Catalogue) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    use dashboard_core::findings::{Finding, Kind};
    let raids = installed_content_raids::raids::read(&cat.content.layers)?;
    // The corpus raid reader owns native notes, including uncertain types.
    let mut findings = Vec::new();
    let mut icons = BTreeMap::new();
    for kind in &raids.types {
        let info = type_info(cat, kind);
        let subject = format!("raid type {}", kind.name);
        let name_key = type_name_key(kind);
        if info.name.is_none() && cat.localisation.get(&name_key).is_none() {
            findings.push(Finding::new(
                Kind::Fallback,
                &subject,
                format!("{name_key}: name localisation is missing; shown as unknown"),
            ));
        }
        match icon_key(cat, kind, None) {
            Ok(key) => {
                icons
                    .entry(key.clone())
                    .or_insert_with(|| decode_icon(cat, &key));
            }
            Err(reason) => findings.push(Finding::new(Kind::Refused, &subject, reason)),
        }
        findings.extend(cat.take_findings());
        findings.push(Finding::grouped(
            Kind::Offline,
            subject,
            "raid target names and building-specific icon candidates need recorded target scope; unavailable offline",
            "raid target names and building-specific icon candidates need recorded target scope; unavailable offline",
        ));
    }
    findings.extend(icons.into_iter().filter_map(|(key, decoded)| {
        decoded
            .err()
            .map(|reason| Finding::new(Kind::Fallback, key, reason))
    }));
    for key in native::PHASE_KEYS.into_iter().chain(native::OUTCOME_KEYS) {
        // Localising records a text the localiser refuses as its own finding.
        localised_label(cat, key);
        if cat.localisation.get(key).is_none() {
            findings.push(Finding::new(
                Kind::Fallback,
                key,
                "raid phase/outcome name is missing; shown as unknown",
            ));
        }
        findings.extend(cat.take_findings());
    }
    Ok(findings)
}

fn read_content(cat: Arc<Catalogue>) -> io::Result<Content> {
    let raids = installed_content_raids::raids::read(&cat.content.layers)?;
    let mut warnings = cat.verification.clone();
    warnings.extend(cat.warnings.iter().cloned());
    warnings.extend(raids.misreads.iter().map(|(_, text)| text.clone()));
    let mut refused = Refusals::default();
    for kind in &raids.uncertain {
        refused.note(&kind.reason, &kind.name);
    }
    warnings.extend(refused.lines("raid types"));
    let names = match cat.cached_states() {
        Ok(states) => states.names.clone(),
        Err(error) => {
            warnings.push(format!("Raid target state names are unavailable: {error}"));
            StateNames::new()
        }
    };
    let types = prepare_types(&cat, &raids);
    let mut unnamed = Refusals::default();
    for kind in types.iter().filter(|kind| kind.name.is_none()) {
        unnamed.note("name localisation is missing or refused", &kind.token);
    }
    warnings.extend(unnamed.lines("raid type names"));
    warnings.extend(cat.refusal_lines());
    Ok(Content {
        catalogue: cat,
        raids,
        names,
        types,
        warnings,
        icons: Mutex::new(BTreeMap::new()),
    })
}

/// Only the Log's referenced instance sprites need pixels. Catalogue and Tally
/// neither decode nor return artwork; a sprite shared by runs is returned once.
fn log_artwork(content: &Content, episodes: &[RaidEpisode]) -> Vec<RaidArtwork> {
    let keys: BTreeSet<_> = episodes
        .iter()
        .flat_map(|episode| &episode.runs)
        .filter_map(|run| run.icon.as_deref())
        .collect();
    let mut icons = content.icons.lock().unwrap_or_else(|e| e.into_inner());
    keys.into_iter()
        .map(|key| {
            let result = icons
                .entry(key.to_owned())
                .or_insert_with(|| decode_icon(&content.catalogue, key));
            if let Err(reason) = result {
                content.catalogue.refuse(reason, key);
            }
            RaidArtwork {
                key: key.to_owned(),
                image: result.as_ref().ok().cloned(),
                missing: result.as_ref().err().cloned(),
            }
        })
        .collect()
}

fn target_name(
    frame: &DecodedFrame,
    scope: &RaidScope,
    content: &Content,
) -> Result<String, String> {
    let cat = &content.catalogue;
    installed_content_raids::raids::target::location(
        frame,
        scope,
        |key| {
            if cat.localisation.get(key).is_none() {
                return Ok(None);
            }
            cat.localisation
                .localise_with(key.as_bytes(), &[])
                .map(Some)
                .map_err(|error| format!("raid target name {key}: {error}"))
        },
        |state| {
            let rows = frame
                .complete::<tables::States>()
                .map_err(|error| error.to_string())?;
            let row = rows
                .binary_search_by_key(&state, |row| row.id)
                .ok()
                .map(|at| &rows[at])
                .ok_or_else(|| format!("raid target state {state} is absent"))?;
            scripted_text::state_display_name(
                row,
                cat,
                Ok(&content.names),
                scripted_text::StateNameUsage::Raid,
            )
        },
    )
    .and_then(|text| catalogue::plain_text(&text))
}

#[derive(Default, Serialize, Deserialize)]
pub struct ScannedRaids {
    pub episodes: Vec<RaidEpisode>,
    pub series: BTreeMap<String, Vec<RaidTallyRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub read_hours: Vec<i32>,
    pub types: BTreeSet<String>,
    pub recording_present: bool,
    pub warnings: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Track {
    identity: IdentityTrack,
    segments: Vec<HourSpan>,
    runs: Vec<RaidTallyRun>,
    holder: Option<(u32, u32)>,
}

#[derive(Serialize, Deserialize)]
struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]); every observation supplies it.
    #[serde(skip)]
    format: replay_data::format::FormatVersion,
    result: ScannedRaids,
    tracks: BTreeMap<String, Track>,
    #[serde(with = "dashboard_core::map_pairs")]
    tallies: BTreeMap<Identity, (u32, u32)>,
    tags: LinkTags,
    #[serde(with = "dashboard_core::map_pairs")]
    open: BTreeMap<(u32, u32), usize>,
    counted_launch: BTreeSet<(u32, u32)>,
    counted_success: BTreeSet<(u32, u32)>,
    #[serde(with = "dashboard_core::map_pairs")]
    ended_victims: BTreeMap<(u32, u32), Option<String>>,
    refusals: Refusals,
    last_frame: Option<(u64, i32)>,
    presence_through: Option<i32>,
    raid_through: Option<i32>,
    present_tags: BTreeSet<String>,
    raid_tags: BTreeSet<String>,
    world_read: bool,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    by_slot: BTreeMap<u32, String>,
    #[serde(skip)]
    identity_ready: bool,
    #[serde(skip)]
    raid_ready: bool,
    #[serde(skip)]
    parents_ready: bool,
    #[serde(skip)]
    parent_tags: BTreeMap<u32, String>,
    #[serde(skip)]
    parents_changed: bool,
    #[serde(skip)]
    changed_tallies: BTreeSet<String>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

impl Default for Scan {
    fn default() -> Self {
        Self::new()
    }
}

impl Scan {
    fn new() -> Self {
        Self {
            format: replay_data::format::FormatVersion::default(),
            result: ScannedRaids::default(),
            tracks: BTreeMap::new(),
            tallies: BTreeMap::new(),
            tags: LinkTags::default(),
            open: BTreeMap::new(),
            counted_launch: BTreeSet::new(),
            counted_success: BTreeSet::new(),
            ended_victims: BTreeMap::new(),
            refusals: Refusals::default(),
            last_frame: None,
            presence_through: None,
            raid_through: None,
            present_tags: BTreeSet::new(),
            raid_tags: BTreeSet::new(),
            world_read: false,
            country_keys: None,
            by_slot: BTreeMap::new(),
            identity_ready: false,
            raid_ready: false,
            parents_ready: false,
            parent_tags: BTreeMap::new(),
            parents_changed: false,
            changed_tallies: BTreeSet::new(),
            #[cfg(test)]
            work: Default::default(),
        }
    }

    fn seal_presence(&mut self) {
        if let Some(hour) = self.presence_through {
            for tag in &self.present_tags {
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                }
                let track = self.tracks.get_mut(tag).expect("present country track");
                track.identity.advance(hour);
                track
                    .segments
                    .last_mut()
                    .expect("present segment")
                    .last_hour = hour;
            }
        }
    }
    fn seal_raids(&mut self) {
        if let Some(hour) = self.raid_through {
            for &index in self.open.values() {
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                }
                let episode = &mut self.result.episodes[index];
                if let Some(run) = episode.runs.last_mut() {
                    run.last_hour = hour;
                }
                if let Some(last) = episode.preparation.last_mut() {
                    if last.1 < hour && last.3 != 0 {
                        let held = i128::from(last.2)
                            + (i128::from(last.1) - i128::from(last.0)) * i128::from(last.3);
                        let first = last.1 + 1;
                        episode.preparation.push(RaidPreparationRun(
                            first,
                            hour,
                            i32::try_from(held).expect("recorded preparation value"),
                            0,
                        ));
                    } else {
                        last.1 = hour;
                    }
                }
            }
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    fn observe(&mut self, frame: &DecodedFrame, content: Option<&Content>) -> io::Result<()> {
        self.observe_changed(frame, content, None).map(|_| ())
    }
    fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&Content>,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<(bool, bool)> {
        let hour = frame.hour;
        self.parents_changed = false;
        self.changed_tallies.clear();
        let continuous = self.last_frame.is_some_and(|(branch, last)| {
            branch == frame.branch && last.checked_add(1) == Some(hour)
        });
        self.last_frame = Some((frame.branch, hour));
        if !continuous {
            self.seal_presence();
            self.seal_raids();
            self.presence_through = None;
            self.raid_through = None;
            self.identity_ready = false;
            self.raid_ready = false;
        }
        let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            self.seal_presence();
            self.seal_raids();
            self.present_tags.clear();
            self.presence_through = None;
            self.raid_through = None;
            self.identity_ready = false;
            self.raid_ready = false;
            return Ok((true, false));
        };
        let country_changed = changes.is_none_or(|changes| {
            changes.rows_changed[TableId::Countries.index()]
                && self.country_keys.as_ref().is_none_or(|keys| {
                    #[cfg(test)]
                    let before = keys.row_visits();
                    let changed = !keys.same_changes(countries, changes.rows(TableId::Countries));
                    #[cfg(test)]
                    {
                        self.work.row_visits += keys.row_visits() - before;
                    }
                    changed
                })
        });
        let identity_changed = !self.identity_ready
            || country_changed
            || changes.is_none_or(|changes| changes.rows_changed[TableId::CountryLinks.index()]);
        if identity_changed {
            #[cfg(test)]
            {
                self.work.row_visits += countries.len() + links.len();
            }
            self.tags.update(frame)?;
            self.seal_presence();
            self.present_tags.clear();
            self.by_slot.clear();
            self.country_keys = Some(dashboard_text::metadata::CountryKeys::read(countries));
            for country in countries {
                #[cfg(test)]
                {
                    self.work.row_visits += 1;
                }
                let Ok(slot) = mapped_slot(country, links) else {
                    self.refusals
                        .note("unresolved country identity", country.id.to_string());
                    continue;
                };
                let Some(tag) = self.tags.get(country.id) else {
                    self.refusals
                        .note("unresolved country tag", country.id.to_string());
                    continue;
                };
                if !self.tracks.contains_key(tag.as_str()) {
                    self.tracks.insert(tag.clone(), Track::default());
                }
                let track = self.tracks.get_mut(tag.as_str()).expect("country track");
                let holder = (slot, country.id);
                let continues = track.identity.step(Some(holder), hour);
                track.holder = Some(holder);
                if continues {
                    track.segments.last_mut().unwrap().last_hour = hour;
                } else {
                    track.segments.push(HourSpan {
                        first_hour: hour,
                        last_hour: hour,
                    });
                }
                self.present_tags.insert(tag.clone());
                self.by_slot.insert(slot, tag.clone());
            }
            self.identity_ready = true;
        }
        self.presence_through = Some(hour);
        if !changes.map_or_else(
            || slot_read(self.format, &frame.metrics, TableId::Raids, hour),
            |changes| changes.slot_read[TableId::Raids.index()],
        ) {
            self.seal_raids();
            self.raid_through = None;
            self.raid_ready = false;
            return Ok((identity_changed, false));
        }
        let (Ok(parents), Ok(raids)) = (
            frame.complete::<tables::RaidCountries>(),
            frame.complete::<tables::Raids>(),
        ) else {
            self.seal_raids();
            self.raid_through = None;
            self.raid_ready = false;
            return Ok((identity_changed, false));
        };
        let raid_changed = identity_changed
            || !self.raid_ready
            || changes.is_none_or(|changes| {
                [
                    TableId::RaidCountries,
                    TableId::Raids,
                    TableId::RaidTargets,
                    TableId::LexicalTokens,
                    TableId::Provinces,
                    TableId::States,
                    TableId::CountryAppearance,
                ]
                .iter()
                .any(|table| changes.rows_changed[table.index()])
            });
        if !raid_changed {
            if self.world_read {
                self.result.read_hours.push(hour);
            }
            self.raid_through = Some(hour);
            return Ok((identity_changed, false));
        }
        self.seal_raids();
        self.result.recording_present |= (changes
            .is_none_or(|changes| changes.rows_changed[TableId::RaidCountries.index()])
            && parents
                .iter()
                .any(|parent| !parent.eligible_types.is_empty()))
            || !raids.is_empty();
        let targets = frame.complete::<tables::RaidTargets>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        self.parents_changed = identity_changed
            || !self.parents_ready
            || changes.is_none_or(|changes| changes.rows_changed[TableId::RaidCountries.index()]);
        if self.parents_changed {
            let by_slot = &self.by_slot;
            let mut parent_tags = BTreeMap::new();
            for parent in parents {
                #[cfg(test)]
                {
                    self.work.row_visits += 1;
                }
                let valid = dashboard_text::metadata::link_slot(links, parent.country as u32)
                    == Some(parent.canonical_slot);
                if let Some(tag) = by_slot.get(&parent.canonical_slot).filter(|_| valid) {
                    parent_tags.insert(parent.canonical_slot, tag.clone());
                } else {
                    self.refusals.note(
                        "RaidCountries country link does not establish its canonical slot",
                        parent.canonical_slot.to_string(),
                    );
                }
            }
            self.world_read = by_slot.len() == countries.len()
                && parent_tags.len() == by_slot.len()
                && parent_tags.len() == parents.len();
            if !self.world_read {
                // Valid country rows keep their values, but incomplete identity or
                // parent coverage cannot prove that an empty race is a world zero.
                for (slot, tag) in by_slot {
                    if !parent_tags.contains_key(slot) {
                        self.refusals
                            .note("RaidCountries row was not recorded", tag.as_str());
                    }
                }
            }
            self.raid_tags = parent_tags.values().cloned().collect();
            self.parent_tags = parent_tags;
            self.parents_ready = true;
        }
        if self.world_read {
            self.result.read_hours.push(hour);
        }
        let parent_tags = &self.parent_tags;
        let mut next = BTreeMap::new();
        for row in raids {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let key = (row.id, row.reference_type);
            let index = match self.open.get(&key) {
                Some(index) => *index,
                None => {
                    let index = self.result.episodes.len();
                    self.result.episodes.push(RaidEpisode {
                        id: format!("{}:{}:{hour}", row.id, row.reference_type),
                        start: RaidDate { hour },
                        runs: Vec::new(),
                        preparation: Vec::new(),
                    });
                    index
                }
            };
            next.insert(key, index);
            let id = self.result.episodes[index].id.clone();
            let raider = parent_tags
                .get(&row.parent_slot)
                .map(|tag| (*tag).to_owned());
            if raider.is_none() {
                self.refusals
                    .note("raid parent country was not recorded", &id);
            }
            let target = targets.and_then(|rows| {
                rows.binary_search_by_key(&key, |r| (r.id, r.reference_type))
                    .ok()
                    .map(|at| &rows[at])
            });
            let token = target
                .and_then(|target| {
                    lexical.and_then(|rows| {
                        rows.binary_search_by_key(&target.raid_type, |r| r.token)
                            .ok()
                            .map(|at| &rows[at])
                    })
                })
                .and_then(|row| std::str::from_utf8(&row.literal).ok())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if let Some(token) = &token {
                self.result.types.insert(token.clone());
            } else {
                self.refusals
                    .note("raid type token or RaidTargets row was not recorded", &id);
            }
            let mut victim = None;
            let mut province = None;
            let mut state = None;
            let mut name = None;
            let mut icon = None;
            let mut icon_missing = None;
            if let Some(target) = target {
                province = if target.scope.province != 0 {
                    Some(target.scope.province)
                } else if target.scope.leader_province != 0 {
                    Some(target.scope.leader_province)
                } else if target.scope.building_type != 0 && target.scope.building_flags & 1 != 0 {
                    Some(target.scope.building_location)
                } else {
                    None
                };
                match target_state(frame, &target.scope) {
                    Ok(value) => state = value,
                    Err(reason) => self.refusals.note(reason, &id),
                }
                if row.phase != native::PHASE_ENDED || !self.ended_victims.contains_key(&key) {
                    match target_country(frame, &target.scope) {
                        Ok(Some(raw)) => match self.tags.get(raw) {
                            Some(tag) => victim = Some(tag.clone()),
                            None => self
                                .refusals
                                .note(format!("raid target country {raw} is unresolved"), &id),
                        },
                        Ok(None) => {}
                        Err(reason) => self.refusals.note(reason, &id),
                    }
                }
                if let Some(content) = content {
                    #[cfg(test)]
                    {
                        self.work.interpreter_calls += 1;
                    }
                    match target_name(frame, &target.scope, content) {
                        Ok(text) => name = Some(text),
                        Err(reason) => self.refusals.note(reason, &id),
                    }
                    if let Some(kind) = token.as_ref().and_then(|token| content.raids.get(token)) {
                        let template = lexical
                            .and_then(|rows| {
                                rows.binary_search_by_key(&target.scope.building_type, |r| r.token)
                                    .ok()
                                    .map(|at| &rows[at])
                            })
                            .and_then(|row| std::str::from_utf8(&row.literal).ok());
                        if target.scope.building_type != 0 && template.is_none() {
                            icon_missing = Some(format!(
                                "raid target building token {} is not recorded",
                                target.scope.building_type
                            ));
                        } else {
                            match icon_key(&content.catalogue, kind, template) {
                                Ok(key) => {
                                    icon = Some(key);
                                }
                                Err(reason) => icon_missing = Some(reason),
                            }
                        }
                    } else {
                        icon_missing = Some(format!(
                            "raid type {} is unavailable or refused",
                            token.as_deref().unwrap_or("unknown")
                        ));
                    }
                }
            }
            // Ending saves the victim. Feedback renders its flag from the saved
            // positive country first, else the live scope resolver. This format lacks
            // the saved victim: retain the first phase-5 read's victim, including an
            // unresolved one, rather than follow later conquests. Control changes
            // before that first read cannot be recovered from this recording.
            if row.phase == native::PHASE_ENDED {
                victim = self.ended_victims.entry(key).or_insert(victim).clone();
            }
            let prior_launch = self.counted_launch.contains(&key);
            if row
                .travelled
                .is_some_and(|distance| !finite_fixed64(distance))
            {
                self.refusals.note(
                    "non-finite recorded flight distance is not a launch witness",
                    format!("{id} ({})", token.as_deref().unwrap_or("unknown raid type")),
                );
            }
            if let Some(reason) = &icon_missing {
                self.refusals.note(reason, &id);
            }
            let was_launched = if prior_launch {
                Some(true)
            } else {
                launched(row)
            };
            let was_success = succeeded(row);
            if row.phase == native::PHASE_ENDED && was_launched.is_none() {
                self.refusals.note("terminal result without recorded phase 4 or finite nonzero flight distance; ending from the console can bypass launch", &id);
            }
            if let Some(tag) = &raider {
                let holder = self.tracks[tag].holder.expect("current country holder");
                let tally = self.tallies.entry(holder).or_default();
                if was_launched == Some(true) && self.counted_launch.insert(key) {
                    self.changed_tallies.insert(tag.clone());
                    tally.0 = tally
                        .0
                        .checked_add(1)
                        .ok_or_else(|| io::Error::other("Raid launch count overflow"))?;
                }
                if was_success == Some(true) && self.counted_success.insert(key) {
                    self.changed_tallies.insert(tag.clone());
                    tally.1 = tally
                        .1
                        .checked_add(1)
                        .ok_or_else(|| io::Error::other("Raid success count overflow"))?;
                }
            }
            let localise = |key| content.and_then(|c| localised_label(&c.catalogue, key));
            if !(0..=native::PHASE_ENDED).contains(&row.phase) {
                self.refusals
                    .note(format!("unsupported raid phase {}", row.phase), &id);
            }
            if !(native::OUTCOME_NONE..=native::OUTCOME_CANCELED).contains(&row.outcome) {
                self.refusals
                    .note(format!("unsupported raid outcome {}", row.outcome), &id);
            }
            let phase_name = (0..=native::PHASE_ENDED)
                .contains(&row.phase)
                .then(|| localise(native::phase_key(row.phase)))
                .flatten();
            let outcome_name = (native::OUTCOME_NONE..=native::OUTCOME_CANCELED)
                .contains(&row.outcome)
                .then(|| localise(native::outcome_key(row.outcome)))
                .flatten();
            if content.is_some()
                && phase_name.is_none()
                && (0..=native::PHASE_ENDED).contains(&row.phase)
            {
                self.refusals.note(
                    format!(
                        "raid phase localisation {} is missing or refused",
                        native::phase_key(row.phase)
                    ),
                    &id,
                );
            }
            if content.is_some()
                && outcome_name.is_none()
                && (native::OUTCOME_NONE..=native::OUTCOME_CANCELED).contains(&row.outcome)
            {
                self.refusals.note(
                    format!(
                        "raid outcome localisation {} is missing or refused",
                        native::outcome_key(row.outcome)
                    ),
                    &id,
                );
            }
            let run = RaidRun {
                first_hour: hour,
                last_hour: hour,
                raider,
                victim,
                raid_type: token,
                province,
                state,
                target_name: name,
                icon,
                icon_missing,
                phase: row.phase,
                phase_name,
                outcome: row.outcome,
                outcome_name,
                launched: was_launched,
                succeeded: was_success,
            };
            let episode = &mut self.result.episodes[index];
            if !episode
                .preparation
                .last_mut()
                .is_some_and(|last| last.extend(hour, row.preparation_hours))
            {
                episode
                    .preparation
                    .push(RaidPreparationRun(hour, hour, row.preparation_hours, 0));
            }
            let runs = &mut episode.runs;
            // Values retain the read which first supplied them; read_hours also
            // dates unchanged successful reads. Country looks use hovered hour.
            if let Some(last) = runs.last_mut()
                && last.last_hour.checked_add(1) == Some(hour)
                && (RaidRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    ..last.clone()
                }) == run
            {
                last.last_hour = hour;
            } else {
                runs.push(run);
            }
        }
        self.open = next;
        if changes.is_none() {
            for tag in parent_tags.values() {
                #[cfg(test)]
                {
                    self.work.row_visits += 1;
                }
                let track = self.tracks.get_mut(tag).expect("country track");
                let (launched, succeeded) = self
                    .tallies
                    .get(&track.holder.expect("current country holder"))
                    .copied()
                    .unwrap_or_default();
                let run = RaidTallyRun {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour: hour,
                    launched,
                    succeeded,
                };
                if let Some(last) = track.runs.last_mut()
                    && last.last_hour.checked_add(1) == Some(hour)
                    && last.launched == run.launched
                    && last.succeeded == run.succeeded
                {
                    last.last_hour = hour;
                } else {
                    track.runs.push(run);
                }
            }
        }
        self.raid_through = Some(hour);
        self.raid_ready = true;
        Ok((identity_changed, true))
    }

    fn finish(mut self) -> ScannedRaids {
        self.seal_presence();
        self.seal_raids();
        for (tag, track) in self.tracks {
            self.result.segments.insert(tag.clone(), track.segments);
            if !track.runs.is_empty() {
                self.result.series.insert(tag, track.runs);
            }
        }
        self.result
            .warnings
            .extend(self.refusals.lines("raid facts"));
        if !self.ended_victims.is_empty() {
            self.result.warnings.push("Ended victims use the target controller at each raid's first ended read. The saved completion-time country is not recorded, so control changes before that read are unknown.".into());
        }
        self.result
    }
}

#[cfg(any(test, feature = "test-support"))]
fn scan(
    replay: &Replay,
    branch: u64,
    content: Option<&Content>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedRaids> {
    let mut selected = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::RaidCountries,
        TableId::Raids,
        TableId::RaidTargets,
        TableId::LexicalTokens,
        TableId::Provinces,
        TableId::States,
    ];
    if content.is_some() {
        selected.push(TableId::CountryAppearance);
    }
    let mut scan = Scan::new();
    scan.format = replay.format;
    if !replay.branches.iter().any(|b| b.id == branch) {
        return Err(dashboard_core::invalid("Branch is not in this recording"));
    }
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|b| b.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Raids request cancelled",
                    ));
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Raids request cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    scan.observe(decoded, content)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.finish())
}

#[derive(Default)]
struct Cache {
    content: catalogue::CachedResult<Content>,
    content_catalogue: Option<Arc<Catalogue>>,
    #[cfg(any(test, feature = "test-support"))]
    scans: VecDeque<(u64, Option<Arc<Catalogue>>, Arc<ScannedRaids>)>,
}

/// Recorded feature evidence for the picker, stopping at the first positive
/// read. The allowed-type roster is populated at load/new game/database reload,
/// independently of player status. Neither a missing row nor an empty roster
/// invents a disabled feature.
#[cfg(any(test, feature = "test-support"))]
fn recording_available(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<bool>> {
    let selected = [TableId::Campaign, TableId::RaidCountries, TableId::Raids];
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|b| b.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Raids availability cancelled",
                    ));
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Raids availability cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    if decoded
                        .complete::<tables::RaidCountries>()
                        .is_ok_and(|parents| parents.iter().any(|r| !r.eligible_types.is_empty()))
                        || decoded
                            .complete::<tables::Raids>()
                            .is_ok_and(|raids| !raids.is_empty())
                    {
                        return Ok(Some(true));
                    }
                }
            }
            Ok(None)
        },
    )
}

/// Borrow the history for complete-reply accounting before copying it. The
/// reserve covers the service lease envelope as other bounded products do.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyMeasure<'a> {
    branch_id: &'a str,
    phase_preparing: i32,
    phase_ended: i32,
    outcome_none: i32,
    available: Option<bool>,
    unavailable: &'a Option<String>,
    types: &'a [RaidTypeInfo],
    artwork: &'a [RaidArtwork],
    episodes: &'a [RaidEpisode],
    series: &'a [RaidTallySeries],
    read_hours: &'a [i32],
    warnings: &'a [String],
}

fn measure_reply(reply: &RaidsReply, episodes: &[RaidEpisode]) -> io::Result<()> {
    let measure = ReplyMeasure {
        branch_id: &reply.branch_id,
        phase_preparing: reply.phase_preparing,
        phase_ended: reply.phase_ended,
        outcome_none: reply.outcome_none,
        available: reply.available,
        unavailable: &reply.unavailable,
        types: &reply.types,
        artwork: &reply.artwork,
        episodes,
        series: &reply.series,
        read_hours: &reply.read_hours,
        warnings: &reply.warnings,
    };
    if json_len(&measure, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(format!(
            "RaidReplyTooLarge: the complete raid reply exceeds {} MiB; no countries or raids were omitted",
            MAX_PRODUCT_REPLY_BYTES / (1024 * 1024),
        )));
    }
    Ok(())
}

#[cfg(test)]
fn selected_series(
    scan: &ScannedRaids,
    params: &RaidsParams,
    cancelled: &AtomicBool,
) -> io::Result<Vec<RaidTallySeries>> {
    selected_series_with_histories(scan, params, cancelled, None, 2)
}

fn selected_series_with_histories(
    scan: &ScannedRaids,
    params: &RaidsParams,
    cancelled: &AtomicBool,
    histories: Option<&RecordedHistories>,
    mut bytes: usize,
) -> io::Result<Vec<RaidTallySeries>> {
    let membership = params
        .top
        .as_ref()
        .map(|top| {
            dashboard_base::ranking::top_membership(
                &scan.series,
                top.count as usize,
                |r| (r.first_hour, r.last_hour),
                |r| {
                    let value = match top.metric {
                        RaidMetric::Launched => r.launched,
                        RaidMetric::Succeeded => r.succeeded,
                        RaidMetric::Both => r.launched,
                    };
                    (value != 0 || (top.metric == RaidMetric::Both && r.succeeded != 0))
                        .then_some(i64::from(value))
                },
                cancelled,
            )
        })
        .transpose()?;
    let selected_ids = histories
        .filter(|_| params.top.is_none())
        .map(|histories| country_history::selected_identities(histories, &params.tags));
    let requested: Vec<String> = membership
        .as_ref()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_else(|| {
            if let Some(histories) = histories {
                return country_history::selected_aliases(histories, &params.tags);
            }
            params
                .tags
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        });
    // Count borrowed clipped pieces before copying a single series. Final
    // complete-reply accounting also includes catalogue, warnings and artwork.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SeriesMeasure<'a> {
        tag: &'a str,
        runs: &'a [RaidTallyRun],
        segments: &'a [HourSpan],
        membership: Option<&'a [HourSpan]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identities: Option<&'a [&'a CountryIdentityRun]>,
    }
    let mut pieces = Vec::new();
    for tag in requested {
        let runs = scan.series.get(&tag).map_or(&[][..], Vec::as_slice);
        if histories.is_none() && runs.is_empty() {
            continue;
        }
        let ranked_spans = membership.as_ref().and_then(|m| m.get(&tag));
        let identities = histories.map(|histories| {
            histories
                .countries
                .get(&tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| {
                    selected_ids.as_ref().is_none_or(|selected| {
                        selected.contains(&(identity.slot, identity.country_id))
                    })
                })
                .collect::<Vec<_>>()
        });
        let holder_spans = selected_ids.as_ref().map(|_| {
            identities
                .as_ref()
                .into_iter()
                .flatten()
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect::<Vec<_>>()
        });
        let spans = ranked_spans.map(Vec::as_slice).or(holder_spans.as_deref());
        let segments = holder_spans
            .as_deref()
            .unwrap_or_else(|| scan.segments.get(&tag).map_or(&[][..], Vec::as_slice));
        bytes = bytes
            .saturating_add(
                json_len(
                    &SeriesMeasure {
                        tag: &tag,
                        runs: &[],
                        segments,
                        membership: ranked_spans.map(Vec::as_slice),
                        identities: identities.as_deref(),
                    },
                    MAX_PRODUCT_REPLY_BYTES,
                )
                .unwrap_or(MAX_PRODUCT_REPLY_BYTES),
            )
            .saturating_add(usize::from(!pieces.is_empty()));
        let mut has_run = false;
        let mut count = |run: &RaidTallyRun, first, last| {
            let piece = RaidTallyRun {
                first_hour: first,
                last_hour: last,
                ..run.clone()
            };
            bytes = bytes
                .saturating_add(
                    json_len(&piece, MAX_PRODUCT_REPLY_BYTES).unwrap_or(MAX_PRODUCT_REPLY_BYTES),
                )
                .saturating_add(usize::from(has_run));
            has_run = true;
            Ok(())
        };
        if let Some(spans) = spans {
            dashboard_base::ranking::for_each_piece(
                runs,
                spans,
                |r| (r.first_hour, r.last_hour),
                cancelled,
                &mut count,
            )?;
        } else {
            for run in runs {
                count(run, run.first_hour, run.last_hour)?;
            }
        }
        if bytes > MAX_PRODUCT_REPLY_BYTES {
            return Err(io::Error::other(
                "RaidReplyTooLarge: these countries' raid tally exceeds one complete response; no countries or raids were omitted",
            ));
        }
        pieces.push((tag, ranked_spans, holder_spans, identities));
    }
    let mut series = Vec::new();
    for (tag, ranked_spans, holder_spans, identities) in pieces {
        let runs = scan.series.get(&tag).map_or(&[][..], Vec::as_slice);
        let spans = ranked_spans.map(Vec::as_slice).or(holder_spans.as_deref());
        let runs = if let Some(spans) = spans {
            let mut clipped = Vec::new();
            dashboard_base::ranking::for_each_piece(
                runs,
                spans,
                |r| (r.first_hour, r.last_hour),
                cancelled,
                |run, first, last| {
                    clipped.push(RaidTallyRun {
                        first_hour: first,
                        last_hour: last,
                        ..run.clone()
                    });
                    Ok(())
                },
            )?;
            clipped
        } else {
            runs.to_vec()
        };
        series.push(RaidTallySeries {
            segments: holder_spans
                .unwrap_or_else(|| scan.segments.get(&tag).cloned().unwrap_or_default()),
            tag,
            runs,
            membership: ranked_spans.cloned(),
            identities: identities.map(|identities| identities.into_iter().cloned().collect()),
        });
    }
    Ok(series)
}

pub fn prepared_content(
    source: &Source,
    context: &LoadContext,
) -> io::Result<(Option<Arc<Content>>, Vec<String>)> {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
    let cat = context
        .content()
        .and_then(|options| catalogue::catalogue(source, options));
    let mut warnings = Vec::new();
    let content = match cat {
        Ok(cat) => {
            if !cache
                .content_catalogue
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &cat))
            {
                cache.content = catalogue::CachedResult::default();
                cache.content_catalogue = Some(Arc::clone(&cat));
            }
            Some(cache.content.get_or_init(|| read_content(cat))?)
        }
        Err(error) => {
            warnings.push(format!(
                "Raid content names and artwork are unavailable: {error}"
            ));
            None
        }
    };
    if let Some(content) = &content {
        warnings.extend(content.warnings.iter().cloned());
    }
    Ok((content, warnings))
}

fn reply(
    params: &RaidsParams,
    content: Option<&Content>,
    available: Option<bool>,
    warnings: Vec<String>,
    scanned: Option<&ScannedRaids>,
    histories: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<RaidsReply> {
    let mut reply = RaidsReply {
        branch_id: params.branch_id.clone(),
        phase_preparing: native::PHASE_PREPARING,
        phase_ended: native::PHASE_ENDED,
        outcome_none: native::OUTCOME_NONE,
        available,
        unavailable: (available == Some(false))
            .then(|| "Not in this recording: the recorded content defines no raid types".into()),
        types: content.map_or_else(Vec::new, |c| c.types.clone()),
        artwork: Vec::new(),
        episodes: Vec::new(),
        series: Vec::new(),
        read_hours: Vec::new(),
        warnings,
    };
    if let Some(scanned) = scanned {
        if scanned.recording_present {
            reply.available = Some(true);
            reply.unavailable = None;
        }
        for token in &scanned.types {
            if !reply.types.iter().any(|kind| &kind.token == token) {
                reply.types.push(RaidTypeInfo {
                    token: token.clone(),
                    name: None,
                    missing: Some(format!(
                        "raid type {token} is absent from the recorded content catalogue"
                    )),
                });
            }
        }
        reply.read_hours.clone_from(&scanned.read_hours);
        reply.warnings.extend(scanned.warnings.iter().cloned());
        reply.warnings.push("Counts are lower bounds from observed raids. A recorded in-progress phase or finite nonzero travelled distance proves launch, counted by its first proving read. A terminal result alone has unknown launch because native console completion can bypass it. Success is counted by its first successful result read. Pre-recording and entirely between-read raids are unknown. Limited success is not a native success.".into());
        if params.view == RaidsView::Tally {
            if let Some(content) = content {
                for warning in content.catalogue.refusal_lines() {
                    if !reply.warnings.contains(&warning) {
                        reply.warnings.push(warning);
                    }
                }
            }
            let bytes = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| io::Error::other("RaidReplyTooLarge: the complete raid reply exceeds one complete response; no countries or raids were omitted"))?;
            reply.series =
                selected_series_with_histories(scanned, params, cancelled, histories, bytes)?;
        }
    }
    let episodes = scanned
        .filter(|_| params.view == RaidsView::Log)
        .map_or(&[][..], |s| s.episodes.as_slice());
    if let Some(content) = content {
        if params.view == RaidsView::Log {
            reply.artwork = log_artwork(content, episodes);
        }
        for warning in content.catalogue.refusal_lines() {
            if !reply.warnings.contains(&warning) {
                reply.warnings.push(warning);
            }
        }
    }
    measure_reply(&reply, episodes)?;
    reply.episodes.extend_from_slice(episodes);
    Ok(reply)
}

impl Product for Raids {
    const KEY: &'static str = "raids";
    const CONTENT_DEPENDENT: bool = true;
    type Params = RaidsParams;
    type Reply = RaidsReply;
    fn types() -> Vec<String> {
        vec![
            RaidsView::decl(),
            RaidMetric::decl(),
            RaidTop::decl(),
            RaidsParams::decl(),
            RaidTypeInfo::decl(),
            RaidArtwork::decl(),
            RaidDate::decl(),
            RaidRun::decl(),
            RaidPreparationRun::decl(),
            RaidEpisode::decl(),
            RaidTallyRun::decl(),
            RaidTallySeries::decl(),
            RaidsReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(top) = &params.top
            && (top.count == 0 || !params.tags.is_empty() || params.view != RaidsView::Tally)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let (content, warnings) = prepared_content(source, context)?;
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let available = if content.as_ref().is_some_and(|c| !c.raids.types.is_empty()) {
            Some(true)
        } else {
            recording_available(&source.replay(), branch, cancelled)?.or_else(|| {
                content
                    .as_ref()
                    .filter(|c| c.raids.uncertain.is_empty())
                    .map(|_| false)
            })
        };
        let scanned = if params.view != RaidsView::Catalogue {
            let catalogue = content.as_ref().map(|c| Arc::clone(&c.catalogue));
            let same = |a: &Option<Arc<Catalogue>>, b: &Option<Arc<Catalogue>>| match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            let scanned = match cache
                .scans
                .iter()
                .find(|(id, cat, _)| *id == branch && same(cat, &catalogue))
            {
                Some((_, _, scan)) => Arc::clone(scan),
                None => {
                    let scanned = Arc::new(scan(
                        &source.replay(),
                        branch,
                        content.as_deref(),
                        cancelled,
                    )?);
                    cache
                        .scans
                        .push_back((branch, catalogue, Arc::clone(&scanned)));
                    while cache.scans.len() > CACHED_BRANCHES {
                        cache.scans.pop_front();
                    }
                    scanned
                }
            };
            Some(scanned)
        } else {
            None
        };
        let histories = if params.view == RaidsView::Tally {
            Some(country_history::branch_histories(
                source,
                branch,
                false,
                &[],
                cancelled,
            )?)
        } else {
            None
        };
        reply(
            params,
            content.as_deref(),
            available,
            warnings,
            scanned.as_deref(),
            histories.as_deref(),
            cancelled,
        )
    }
}

#[cfg(test)]
mod tests;
