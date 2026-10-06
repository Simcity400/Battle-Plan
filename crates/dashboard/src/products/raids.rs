//! One branch scan shared by the Raid log and Raid tally. All countries enter
//! the scan. Hourly reads supply every value; gaps end runs, never imply zero.
//! Counts are observed launches/results, not an invented pre-recording census.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_groups::HourSpan, finite_fixed64,
    validate_tag,
};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals, View},
    metadata::{IdentityTrack, LinkTags, mapped_slot, slot_read, validate_campaign},
    scripted_text::{self, StateNames},
    transport::json_len,
};
use engine::craid_instance as native;
use installed_content::raids::{Catalog, RaidType};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{RaidRow, RaidScope, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct Raids;

const CACHED_BRANCHES: usize = 4;
/// A memory budget, refused rather than truncating any country's history.
const MAX_ENTRIES: usize = 4_000_000;
/// The same display-selection bound as the other country race products.
pub const MAX_COUNTRIES: usize = 30;

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

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidDate {
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
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

/// (Read hour, stored preparation hours at CRaidInstance+0x44). The active
/// tooltip@141612820 -> phase text@140FD5730 reads it through
/// GetPhaseElapsedHours@140FD53C0 in phase 2. Other phases do not use it as
/// elapsed phase time. Every successful read is retained, without interpolation.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
pub struct RaidPreparationRead(pub i32, pub i32);

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidEpisode {
    pub id: String,
    /// Creation/start dates are not in Raids. This is a bound, never an exact start.
    pub start: RaidDate,
    pub runs: Vec<RaidRun>,
    /// Independent compact reads: preparation changes must not split fact runs.
    pub preparation: Vec<RaidPreparationRead>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
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
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaidsReply {
    pub branch_id: String,
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
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

/// Execute@140FD9070 and TryAutoLaunch@140FD7E50 call SetPhase@140FD4EF0(4).
/// End@140FD7600 also runs directly from console callback@1402574C0 for any
/// non-ended phase. A terminal result therefore cannot prove an unseen launch.
/// Finite nonzero travelled proves flight: constructor@140FD46D0 starts it at zero,
/// SetPhase(4) clears it, and HourlyUpdate@140FDA5A0 adds speed only in phase 4.
/// End preserves it; ReadToken@140FD9560 loads the trusted saved distance.
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

/// End@140FD7600's success branch tests `(outcome - 3U) < 2`: limited
/// success (2) is excluded, exactly as the native success-only effects are.
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

struct Content {
    catalogue: Arc<Catalogue>,
    raids: Catalog,
    names: StateNames,
    types: Vec<RaidTypeInfo>,
    warnings: Vec<String>,
    icons: Mutex<BTreeMap<String, Result<String, String>>>,
}

fn icon_key(cat: &Catalogue, kind: &RaidType, template: Option<&str>) -> Result<String, String> {
    // BuildSpriteName@141EDC220's fallback bytes at 142A79358. The key is a
    // native literal; whether its sprite exists and all pixels are content facts.
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

// GetLocalizedName@140A8E3B0 constructs raid_type_<database name>.
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

pub(crate) fn check_content(cat: &Catalogue) -> io::Result<Vec<crate::content_check::Finding>> {
    use crate::content_check::{Finding, Kind};
    let raids = installed_content::raids::read(&cat.content.layers)?;
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
    let raids = installed_content::raids::read(&cat.content.layers)?;
    let mut warnings = cat.verification.clone();
    warnings.extend(cat.warnings.iter().cloned());
    warnings.extend(raids.misreads.iter().map(|(_, text)| text.clone()));
    let mut refused = Refusals::default();
    for kind in &raids.uncertain {
        refused.note(&kind.reason, &kind.name);
    }
    warnings.extend(refused.lines("raid types"));
    let names = match scripted_text::read_state_content(&cat.content.layers) {
        Ok(states) => {
            warnings
                .extend(scripted_text::live_state_warnings(&states.warnings).map(str::to_owned));
            states.names
        }
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

/// country_resolver@140FD1AD0: building positive override or its location's
/// controller; province; state; resolved captive leader's captor; leader province.
fn target_country(frame: &DecodedFrame, scope: &RaidScope) -> Result<Option<u32>, String> {
    let province = |id| {
        frame
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?
            .binary_search_by_key(&id, |r| r.id)
            .map(|at| frame.complete::<tables::Provinces>().unwrap()[at].controller)
            .map_err(|_| format!("raid target province {id} is absent"))
    };
    let state = |id| {
        frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .binary_search_by_key(&id, |r| r.id)
            .map(|at| frame.complete::<tables::States>().unwrap()[at].controller)
            .map_err(|_| format!("raid target state {id} is absent"))
    };
    let raw = if scope.building_type != 0 {
        if scope.building_country > 0 {
            scope.building_country as u32
        } else if scope.building_flags & 1 != 0 {
            province(scope.building_location)?
        } else {
            state(scope.building_location)?
        }
    } else if scope.province != 0 {
        province(scope.province)?
    } else if scope.state != 0 {
        state(scope.state)?
    } else if let Some(captor) = scope.leader_country {
        captor.max(0) as u32
    } else if scope.leader_province != 0 {
        province(scope.leader_province)?
    } else {
        0
    };
    Ok((raw != 0).then_some(raw))
}

/// Location scope's state getter (plain location caller@140FD1BB0); state,
/// province, leader province,
/// then building location. Keep a missing province separate from an empty scope.
fn target_state(frame: &DecodedFrame, scope: &RaidScope) -> Result<Option<u32>, String> {
    if scope.state != 0 {
        return Ok(Some(scope.state));
    }
    let province = if scope.province != 0 {
        scope.province
    } else if scope.leader_province != 0 {
        scope.leader_province
    } else if scope.building_type != 0 && scope.building_flags & 1 == 0 {
        return Ok(Some(scope.building_location));
    } else {
        scope.building_location
    };
    if province == 0 {
        return Ok(None);
    }
    let rows = frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&province, |r| r.id)
        .map(|at| Some(rows[at].state))
        .map_err(|_| format!("raid target province {province} is absent"))
}

fn target_name(
    frame: &DecodedFrame,
    scope: &RaidScope,
    content: &Content,
    tags: &LinkTags,
) -> Result<String, String> {
    let cat = &content.catalogue;
    // Plain location@140FD1BB0 -> ResolveVictoryPointLocalization@140E68E30;
    // state-owner tag key first, then unqualified VP, then display_name@1409CC380.
    let province = if scope.province != 0 {
        scope.province
    } else {
        scope.leader_province
    };
    if province != 0 {
        let rows = frame
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?;
        let row = rows
            .binary_search_by_key(&province, |r| r.id)
            .ok()
            .map(|at| &rows[at])
            .ok_or_else(|| format!("raid target province {province} is absent"))?;
        let states = frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?;
        let state = states
            .binary_search_by_key(&row.state, |r| r.id)
            .ok()
            .map(|at| &states[at])
            .ok_or_else(|| format!("raid target state {} is absent", row.state))?;
        let tag = if state.owner == 0 {
            None
        } else {
            Some(tags.get(state.owner).ok_or_else(|| {
                format!("raid target state's owner {} is unresolved", state.owner)
            })?)
        };
        let mut keys = Vec::new();
        if let Some(tag) = tag {
            keys.push(format!("{tag}_VICTORY_POINTS_{province}"));
        }
        keys.push(format!("VICTORY_POINTS_{province}"));
        for key in keys {
            if cat.localisation.get(&key).is_some() {
                let name = cat
                    .localised(&key, View::Plain)
                    .ok_or_else(|| format!("raid target name {key} is refused"))?;
                if !name.is_empty() {
                    return Ok(name);
                }
            }
        }
    }
    let Some(state) = target_state(frame, scope)? else {
        return Ok("no_target_name".into());
    };
    let rows = frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?;
    let row = rows
        .binary_search_by_key(&state, |r| r.id)
        .ok()
        .map(|at| &rows[at])
        .ok_or_else(|| format!("raid target state {state} is absent"))?;
    scripted_text::state_name_from(
        row,
        |bytes, what| {
            scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, what)
        },
        |id| {
            let key = content
                .names
                .get(&id)
                .ok_or_else(|| format!("state {id} has no content name"))?
                .as_ref()
                .map_err(Clone::clone)?;
            cat.localised(key, View::Plain)
                .ok_or_else(|| format!("state {id} name {key} is refused"))
        },
    )
    .and_then(|text| catalogue::plain_text(&text))
}

#[derive(Default)]
pub struct ScannedRaids {
    pub episodes: Vec<RaidEpisode>,
    pub series: BTreeMap<String, Vec<RaidTallyRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub read_hours: Vec<i32>,
    pub unobserved: Vec<HourSpan>,
    pub types: BTreeSet<String>,
    pub recording_present: bool,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Track {
    identity: IdentityTrack,
    segments: Vec<HourSpan>,
    runs: Vec<RaidTallyRun>,
    launched: u32,
    succeeded: u32,
    holder: Option<(u32, u32)>,
}

fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(last) = spans.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
    {
        last.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

struct Scan {
    result: ScannedRaids,
    tracks: BTreeMap<String, Track>,
    tags: LinkTags,
    open: BTreeMap<(u32, u32), usize>,
    counted_launch: BTreeSet<(u32, u32)>,
    counted_success: BTreeSet<(u32, u32)>,
    ended_victims: BTreeMap<(u32, u32), Option<String>>,
    refusals: Refusals,
    last: Option<i32>,
    entries: usize,
}

impl Scan {
    fn new() -> Self {
        Self {
            result: ScannedRaids::default(),
            tracks: BTreeMap::new(),
            tags: LinkTags::default(),
            open: BTreeMap::new(),
            counted_launch: BTreeSet::new(),
            counted_success: BTreeSet::new(),
            ended_victims: BTreeMap::new(),
            refusals: Refusals::default(),
            last: None,
            entries: 0,
        }
    }

    fn observe(&mut self, frame: &DecodedFrame, content: Option<&Content>) -> io::Result<()> {
        let hour = frame.hour;
        if let Some(last) = self.last
            && last.checked_add(1) != Some(hour)
            && last < hour - 1
        {
            if let Some(span) = self.result.unobserved.last_mut()
                && span.last_hour.checked_add(1) == Some(last + 1)
            {
                span.last_hour = hour - 1;
            } else {
                self.result.unobserved.push(HourSpan {
                    first_hour: last + 1,
                    last_hour: hour - 1,
                });
            }
        }
        self.last = Some(hour);
        if self.tags.update(frame).is_err() {
            mark(&mut self.result.unobserved, hour);
            return Ok(());
        }
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        let mut by_slot = BTreeMap::new();
        for country in countries {
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
            if track.holder.is_some_and(|old| old != holder) {
                track.launched = 0;
                track.succeeded = 0;
            }
            track.holder = Some(holder);
            if continues {
                track.segments.last_mut().unwrap().last_hour = hour;
            } else {
                track.segments.push(HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                });
                self.entries += 1;
            }
            by_slot.insert(slot, tag.as_str());
        }
        if !slot_read(&frame.metrics, TableId::Raids, hour) {
            return Ok(());
        }
        let (Ok(parents), Ok(raids)) = (
            frame.complete::<tables::RaidCountries>(),
            frame.complete::<tables::Raids>(),
        ) else {
            mark(&mut self.result.unobserved, hour);
            return Ok(());
        };
        self.result.recording_present |= parents
            .iter()
            .any(|parent| !parent.eligible_types.is_empty())
            || !raids.is_empty();
        let targets = frame.complete::<tables::RaidTargets>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let mut parent_tags = BTreeMap::new();
        for parent in parents {
            let valid = crate::metadata::link_slot(links, parent.country as u32)
                == Some(parent.canonical_slot);
            if let Some(tag) = by_slot.get(&parent.canonical_slot).filter(|_| valid) {
                parent_tags.insert(parent.canonical_slot, *tag);
            } else {
                self.refusals.note(
                    "RaidCountries country link does not establish its canonical slot",
                    parent.canonical_slot.to_string(),
                );
            }
        }
        if by_slot.len() == countries.len()
            && parent_tags.len() == by_slot.len()
            && parent_tags.len() == parents.len()
        {
            self.result.read_hours.push(hour);
        } else {
            // Valid country rows keep their values, but incomplete identity or
            // parent coverage cannot prove that an empty race is a world zero.
            mark(&mut self.result.unobserved, hour);
            for (slot, tag) in &by_slot {
                if !parent_tags.contains_key(slot) {
                    self.refusals
                        .note("RaidCountries row was not recorded", *tag);
                }
            }
        }
        let mut next = BTreeMap::new();
        for row in raids {
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
                    self.entries += 1;
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
                    match target_name(frame, &target.scope, content, &self.tags) {
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
            // End@140FD7600 saves the victim at +0x1D0. Feedback view@141B20DE0
            // renders its flag through 140FD7E10 (saved positive country first,
            // else the live scope resolver). This format lacks +0x1D0: retain
            // the first phase-5 read's victim, including an unresolved one,
            // rather than follow later conquests. Control changes before that
            // first read cannot be recovered from this recording.
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
                self.refusals.note("terminal result without recorded phase 4 or finite nonzero flight distance; console End@1402574C0 can bypass launch", &id);
            }
            if let Some(tag) = &raider {
                let track = self.tracks.get_mut(tag).expect("country track");
                if was_launched == Some(true) && self.counted_launch.insert(key) {
                    track.launched = track
                        .launched
                        .checked_add(1)
                        .ok_or_else(|| io::Error::other("Raid launch count overflow"))?;
                }
                if was_success == Some(true) && self.counted_success.insert(key) {
                    track.succeeded = track
                        .succeeded
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
                read_hour: hour,
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
            episode
                .preparation
                .push(RaidPreparationRead(hour, row.preparation_hours));
            self.entries += 1;
            let runs = &mut episode.runs;
            // Values retain the read which first supplied them; read_hours also
            // dates unchanged successful reads. Country looks use hovered hour.
            if let Some(last) = runs.last_mut()
                && last.last_hour.checked_add(1) == Some(hour)
                && (RaidRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    read_hour: run.read_hour,
                    ..last.clone()
                }) == run
            {
                last.last_hour = hour;
            } else {
                runs.push(run);
                self.entries += 1;
            }
        }
        self.open = next;
        for tag in parent_tags.values() {
            let track = self.tracks.get_mut(*tag).expect("country track");
            let run = RaidTallyRun {
                first_hour: hour,
                last_hour: hour,
                read_hour: hour,
                launched: track.launched,
                succeeded: track.succeeded,
            };
            if let Some(last) = track.runs.last_mut()
                && last.last_hour.checked_add(1) == Some(hour)
                && last.launched == run.launched
                && last.succeeded == run.succeeded
            {
                last.last_hour = hour;
            } else {
                track.runs.push(run);
                self.entries += 1;
            }
        }
        if self.entries > MAX_ENTRIES {
            return Err(io::Error::other(
                "The complete raid history exceeds its memory budget; no countries or raids were omitted",
            ));
        }
        Ok(())
    }

    fn finish(mut self) -> ScannedRaids {
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

fn scan(
    replay: &Replay,
    branch: u64,
    content: Option<&Content>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedRaids> {
    let selected = [
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
    let mut scan = Scan::new();
    let start = replay
        .branches
        .iter()
        .find(|b| b.id == branch)
        .ok_or_else(|| crate::invalid("Branch is not in this recording"))?;
    let (first, last) = crate::metadata::branch_bounds(replay, start);
    scan.last = first.checked_sub(1);
    for block in replay.index.iter().filter(|b| b.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Raids request cancelled",
            ));
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
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
    let next = scan
        .last
        .and_then(|hour| hour.checked_add(1))
        .unwrap_or(first);
    if next <= last {
        if let Some(span) = scan.result.unobserved.last_mut()
            && span.last_hour.checked_add(1) == Some(next)
        {
            span.last_hour = last;
        } else {
            scan.result.unobserved.push(HourSpan {
                first_hour: next,
                last_hour: last,
            });
        }
    }
    Ok(scan.finish())
}

#[derive(Default)]
struct Cache {
    content: Option<Arc<Content>>,
    scans: VecDeque<(u64, Option<Arc<Catalogue>>, Arc<ScannedRaids>)>,
}

/// Recorded feature evidence for the picker, stopping at the first positive
/// read. Native allowed-type roster is
/// populated at load/new game/database reload@1414D82D0, independently of player
/// status. Neither a missing row nor an empty roster invents a disabled feature.
fn recording_available(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<bool>> {
    let selected = [TableId::Campaign, TableId::RaidCountries, TableId::Raids];
    for block in replay.index.iter().filter(|b| b.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Raids availability cancelled",
            ));
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
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
}

/// Borrow the history for complete-reply accounting before copying it. The
/// reserve covers the service lease envelope as other bounded products do.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyMeasure<'a> {
    branch_id: &'a str,
    available: Option<bool>,
    unavailable: &'a Option<String>,
    types: &'a [RaidTypeInfo],
    artwork: &'a [RaidArtwork],
    episodes: &'a [RaidEpisode],
    series: &'a [RaidTallySeries],
    read_hours: &'a [i32],
    unobserved: &'a [HourSpan],
    warnings: &'a [String],
}

fn measure_reply(reply: &RaidsReply, episodes: &[RaidEpisode]) -> io::Result<()> {
    let measure = ReplyMeasure {
        branch_id: &reply.branch_id,
        available: reply.available,
        unavailable: &reply.unavailable,
        types: &reply.types,
        artwork: &reply.artwork,
        episodes,
        series: &reply.series,
        read_hours: &reply.read_hours,
        unobserved: &reply.unobserved,
        warnings: &reply.warnings,
    };
    if json_len(&measure, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(format!(
            "The complete raid reply exceeds {} MiB; no countries or raids were omitted",
            MAX_PRODUCT_REPLY_BYTES / (1024 * 1024),
        )));
    }
    Ok(())
}

fn selected_series(
    scan: &ScannedRaids,
    params: &RaidsParams,
    cancelled: &AtomicBool,
) -> io::Result<Vec<RaidTallySeries>> {
    let membership = params
        .top
        .as_ref()
        .map(|top| {
            crate::ranking::top_membership(
                &scan.series,
                top.count as usize,
                |r| (r.first_hour, r.last_hour),
                |r| {
                    let value = match top.metric {
                        RaidMetric::Launched => r.launched,
                        RaidMetric::Succeeded => r.succeeded,
                    };
                    (value != 0).then_some(i64::from(value))
                },
                cancelled,
            )
        })
        .transpose()?;
    let requested: Vec<String> = membership
        .as_ref()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_else(|| {
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
    }
    let mut bytes = 2usize;
    let mut pieces = Vec::new();
    for tag in requested {
        let Some(runs) = scan.series.get(&tag) else {
            continue;
        };
        let spans = membership.as_ref().and_then(|m| m.get(&tag));
        let segments = scan.segments.get(&tag).map_or(&[][..], Vec::as_slice);
        bytes = bytes
            .saturating_add(
                json_len(
                    &SeriesMeasure {
                        tag: &tag,
                        runs: &[],
                        segments,
                        membership: spans.map(Vec::as_slice),
                    },
                    MAX_PRODUCT_REPLY_BYTES,
                )
                .unwrap_or(MAX_PRODUCT_REPLY_BYTES),
            )
            .saturating_add(1);
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
                .saturating_add(1);
            Ok(())
        };
        if let Some(spans) = spans {
            crate::ranking::for_each_piece(
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
                "These countries' raid tally exceeds one complete response; choose fewer countries",
            ));
        }
        pieces.push((tag, spans));
    }
    let mut series = Vec::new();
    for (tag, spans) in pieces {
        let runs = &scan.series[&tag];
        let runs = if let Some(spans) = spans {
            let mut clipped = Vec::new();
            crate::ranking::for_each_piece(
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
            runs.clone()
        };
        series.push(RaidTallySeries {
            segments: scan.segments.get(&tag).cloned().unwrap_or_default(),
            tag,
            runs,
            membership: spans.cloned(),
        });
    }
    Ok(series)
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
            RaidPreparationRead::decl(),
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
        if params.tags.len() > MAX_COUNTRIES {
            return Err("Too many countries");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(top) = &params.top
            && (top.count == 0
                || top.count as usize > MAX_COUNTRIES
                || !params.tags.is_empty()
                || params.view != RaidsView::Tally)
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
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
        let mut warnings = Vec::new();
        let content = match cat {
            Ok(cat) => {
                if !cache
                    .content
                    .as_ref()
                    .is_some_and(|c| Arc::ptr_eq(&c.catalogue, &cat))
                {
                    cache.content = Some(Arc::new(read_content(cat)?));
                }
                cache.content.clone()
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
        let mut reply = RaidsReply {
            branch_id: params.branch_id.clone(),
            available,
            unavailable: (available == Some(false)).then(|| {
                "Not in this recording: the recorded content defines no raid types".into()
            }),
            types: content.as_ref().map_or_else(Vec::new, |c| c.types.clone()),
            artwork: Vec::new(),
            episodes: Vec::new(),
            series: Vec::new(),
            read_hours: Vec::new(),
            unobserved: Vec::new(),
            warnings,
        };
        let mut log_scan = None;
        if params.view != RaidsView::Catalogue {
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
            if scanned.recording_present {
                reply.available = Some(true);
                reply.unavailable = None;
            }
            for token in scanned
                .types
                .iter()
                .filter(|token| !reply.types.iter().any(|t| &t.token == *token))
                .cloned()
                .collect::<Vec<_>>()
            {
                reply.types.push(RaidTypeInfo {
                    token: token.clone(),
                    name: None,
                    missing: Some(format!(
                        "raid type {token} is absent from the recorded content catalogue"
                    )),
                });
            }
            reply.read_hours.clone_from(&scanned.read_hours);
            reply.unobserved.clone_from(&scanned.unobserved);
            reply.warnings.extend(scanned.warnings.iter().cloned());
            reply.warnings.push("Counts are lower bounds from observed raids. A recorded in-progress phase or finite nonzero travelled distance proves launch, counted by its first proving read. A terminal result alone has unknown launch because native console completion can bypass it. Success is counted by its first successful result read. Pre-recording and entirely between-read raids are unknown. Limited success is not a native success.".into());
            if params.view == RaidsView::Log {
                log_scan = Some(scanned);
            } else {
                reply.series = selected_series(&scanned, params, cancelled)?;
            }
        }
        if let Some(content) = &content {
            if let Some(scan) = &log_scan {
                reply.artwork = log_artwork(content, &scan.episodes);
            }
            reply.warnings.extend(content.catalogue.refusal_lines());
        }
        measure_reply(
            &reply,
            log_scan.as_ref().map_or(&[][..], |s| s.episodes.as_slice()),
        )?;
        if let Some(scan) = log_scan {
            reply.episodes.clone_from(&scan.episodes);
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
