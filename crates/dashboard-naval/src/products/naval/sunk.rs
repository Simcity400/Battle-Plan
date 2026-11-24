
use super::sinking_details::{BattleIndex, resolve_attribution};
pub use super::sinking_details::{SINKING_BATTLE_NOT_RECORDED, SINKING_CAUSE_NOT_RECORDED, ShipSinkingAttribution, ShipSinkingBattle};
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids};
use dashboard_world::products::army_windows::ArmySpan;
use dashboard_base::Source;
use dashboard_base::catalogue::{self, Catalogue, CachedResult, Refusals, View};
use dashboard_query::{Query, read};
use content_reader_key_names::unit_leader::{SubUnits, sub_units};
use content_reader_localisation::localisation::display_text;
use replay_lanes::at::Seeker;
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{SunkShipEntry, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const ICON_EDGE: u32 = 48;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipClassInfo {
    pub token: u32,
    #[doc = " The recorded lexical token, resolved to the content's sub-unit spelling."]
    pub key: Option<String>,
    pub name: Option<String>,
    #[doc = " PNG sent once per class, never once per sinking."]
    pub icon: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ShipSinkingCause {
    Hostile,
    Assisted,
    Cause,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkRow {
    pub sink_hour: i32,
    #[doc = " The first read listing it."]
    pub known_at: i32,
    pub name: Option<String>,
    pub class_index: u32,
    pub recorded_name_index: Option<u32>,
    pub victim: Option<String>,
    pub attacker: Option<String>,
    #[doc = " Always null: both countries are named at the sinking hour."]
    pub victim_as_of: Option<i32>,
    pub attacker_as_of: Option<i32>,
    pub place_index: Option<u32>,
    pub cause: ShipSinkingCause,
    pub attribution_name: Option<String>,
    pub assisted_kill: bool,
    pub sinking_cause: ShipSinkingAttribution,
    pub battle: ShipSinkingBattle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkReply {
    pub branch_id: String,
    pub window: usize,
    #[doc = " The game's list was read at or before the window's end; otherwise nothing is known."]
    pub available: bool,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The window's first read of the list."]
    pub first_read: Option<i32>,
    #[doc = " The window's hours the timeline does not hold."]
    pub unread: Vec<ArmySpan>,
    pub classes: Vec<ShipClassInfo>,
    pub places: Vec<String>,
    pub recorded_names: Vec<String>,
    #[doc = " Those pruned within the window first, then the game's list order at its end; a row's index"]
    #[doc = " in the window is its place here."]
    pub rows: Vec<ShipsSunkRow>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkManifestReply {
    pub branch_id: String,
    pub windows: Vec<ArmySpan>,
    #[doc = " The list was read at some hour of the branch."]
    pub available: bool,
    pub first_read: Option<i32>,
    #[doc = " The classes of every sinking the game listed at each run's end, for the class choice."]
    pub classes: Vec<ShipClassInfo>,
    pub recorded_names: Vec<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ShipsSunkReply, ShipsSunkManifestReply);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShipsSunkParams {
    pub branch_id: String,
    pub window: usize,
}

fn branch(branch_id: &str) -> io::Result<u64> {
    branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

pub fn cause(entry: &SunkShipEntry) -> ShipSinkingCause {
    if entry.cause_record != 0 {
        ShipSinkingCause::Cause
    } else if entry.assisted_kill != 0 {
        ShipSinkingCause::Assisted
    } else {
        ShipSinkingCause::Hostile
    }
}

pub fn known_at(reads: &[(i32, i32)], hour: i32) -> Option<i32> {
    let at = reads.partition_point(|run| run.1 < hour);
    reads.get(at).map(|run| run.0.max(hour))
}

pub fn pruned<'a>(earlier: &'a [SunkShipEntry], later: &[SunkShipEntry], first_read: i32, reads: &[(i32, i32)]) -> Vec<&'a SunkShipEntry> {
    let mut left: BTreeMap<i32, Vec<&SunkShipEntry>> = BTreeMap::new();
    for entry in later {
        left.entry(entry.sink_hour).or_default().push(entry);
    }
    earlier
        .iter()
        .filter(|entry| known_at(reads, entry.sink_hour) == Some(first_read))
        .filter(|entry| {
            let Some(held) = left.get_mut(&entry.sink_hour) else { return true };
            match held.iter().position(|other| **other == **entry) {
                Some(at) => {
                    held.swap_remove(at);
                    false
                }
                None => true,
            }
        })
        .collect()
}

pub fn sinkings_at(recording: &Recording, run: u32, hour: i32) -> io::Result<Option<(i32, Vec<SunkShipEntry>)>> {
    sinkings_in(&replay_lanes::at::frame(recording, run, hour, &[read::Read::all(TableId::SunkShips)])?)
}

pub fn sinkings_seen(seeker: &mut Seeker<'_>, run: u32, hour: i32) -> io::Result<Option<(i32, Vec<SunkShipEntry>)>> {
    sinkings_in(&dashboard_query::cost::storage(|| seeker.frame(run, hour, &[read::Read::all(TableId::SunkShips)]))?)
}

fn sinkings_in(frame: &replay_frame::frame::DecodedFrame) -> io::Result<Option<(i32, Vec<SunkShipEntry>)>> {
    let sampled = frame.tables[TableId::SunkShips.index()].sampled_at();
    let Ok(rows) = frame.complete::<tables::SunkShips>() else { return Ok(None) };
    Ok(sampled.map(|sampled| (sampled, rows.first().map(|row| row.entries.clone()).unwrap_or_default())))
}

pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    units: Result<Arc<SubUnits>, String>,
    regions: Result<(Arc<content_reader_regions::regions::StrategicRegions>, BTreeMap<u32, Vec<u32>>), String>,
}

impl Content {
    pub fn load(source: &Source, context: &LoadContext) -> Self {
        let catalogue = context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| error.to_string());
        let units = catalogue
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|cat| cat.content.layers.derived::<CachedResult<SubUnits>>().get_or_init(|| sub_units(&cat.content.layers)).map_err(|error| error.to_string()));
        let regions = catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
            let regions = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| error.to_string())?;
            let mut memberships = BTreeMap::<u32, Vec<u32>>::new();
            for (&id, region) in &regions.regions {
                let mut seen = BTreeSet::new();
                for &province in &region.provinces {
                    if seen.insert(province) {
                        memberships.entry(province).or_default().push(id);
                    }
                }
            }
            Ok((regions, memberships))
        });
        Self { catalogue, units, regions }
    }

    pub fn catalogue(&self) -> Option<&Catalogue> {
        self.catalogue.as_deref().ok()
    }

    pub fn warnings(&self) -> Vec<String> {
        match &self.catalogue {
            Ok(cat) => cat.verification.iter().chain(&cat.warnings).cloned().collect(),
            Err(reason) => vec![format!("Ship and fleet names are unavailable: {reason}")],
        }
    }
}

pub fn ship_names(seeker: &mut Seeker<'_>, run: u32, hour: i32, entries: &[SunkShipEntry], content: &Content, refused: &mut Refusals) -> io::Result<Vec<(Option<String>, Option<String>)>> {
    Ok(ship_details(seeker, run, hour, entries, content, refused)?
        .into_iter()
        .map(|(name, class, _)| (name, class))
        .collect())
}

pub fn ship_details(
    seeker: &mut Seeker<'_>,
    run: u32,
    hour: i32,
    entries: &[SunkShipEntry],
    content: &Content,
    refused: &mut Refusals,
) -> io::Result<Vec<(Option<String>, Option<String>, ShipSinkingAttribution)>> {
    let held: Vec<&SunkShipEntry> = entries.iter().collect();
    let named = Named::new(seeker, run, hour, &held, content)?;
    refused.merge(&named.refused);
    Ok(entries
        .iter()
        .map(|entry| {
            let class = named.classes.get(named.class_at[&entry.class_token] as usize).and_then(|class| class.name.clone()).or_else(|| {
                named.recorded_at.get(&entry.class_name).copied().flatten().map(|index| format!("{} (recorded class)", named.recorded_names[index as usize]))
            });
            let sinking_cause = named.attribution_for(entry, content, refused);
            (stored(&entry.ship_name, "ship name", content, refused), class, sinking_cause)
        })
        .collect())
}

#[derive(Default)]
struct Named {
    classes: Vec<ShipClassInfo>,
    class_at: HashMap<u32, u32>,
    attr_classes: HashMap<u32, (Option<String>, Option<String>)>,
    recorded_names: Vec<String>,
    recorded_at: HashMap<Vec<u8>, Option<u32>>,
    places: Vec<String>,
    place_at: HashMap<u32, Option<u32>>,
    refused: Refusals,
    warnings: Vec<String>,
}

fn literals(seeker: &mut Seeker<'_>, run: u32, hour: i32, tokens: &BTreeSet<u32>) -> io::Result<HashMap<u32, String>> {
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let keys: Vec<u128> = tokens.iter().map(|&token| u128::from(token)).collect();
    let frame = dashboard_query::cost::storage(|| seeker.frame(run, hour, &[read::Read { table: TableId::LexicalTokens, columns: None, keys: read::Keys::These(&keys) }]))?;
    let Ok(rows) = frame.complete::<tables::LexicalTokens>() else { return Ok(HashMap::new()) };
    Ok(rows.iter().map(|row| (row.token, String::from_utf8_lossy(&row.literal).into_owned())).collect())
}

fn class_info(token: u32, literal: Option<String>, content: &Content, refused: &mut Refusals) -> ShipClassInfo {
    let mut info = ShipClassInfo { token, key: literal, name: None, icon: None };
    let (Ok(cat), Ok(units), Some(literal)) = (&content.catalogue, &content.units, info.key.clone()) else {
        if info.key.is_none() {
            refused.note("class is absent from lexical tokens", format!("token {token}"));
        }
        return info;
    };
    match units.token_name(&content_reader_parser::tokens::Token::c_str(literal.as_bytes())) {
        Ok(spelling) if !spelling.is_empty() => {
            let spelling = spelling.to_owned();
            if cat.localisation.get(&spelling).is_some() {
                info.name = cat.localised(&spelling, View::Plain);
            }
            if info.name.is_none() {
                refused.note("class localisation is unavailable", &spelling);
            }
            let sprite = format!("GFX_unit_{spelling}_icon_medium");
            match cat.sprites.find(&sprite) {
                Ok(Some(found)) => match dashboard_base::products::artwork::resolve_sprite_type(cat, found, 0, ICON_EDGE).into_result() {
                    Ok(Some(image)) => info.icon = Some(image),
                    Ok(None) => {}
                    Err(reason) => refused.note(reason, &sprite),
                },
                Ok(None) => {}
                Err(reason) => refused.note(reason, &sprite),
            }
            info.key = Some(spelling);
        }
        Ok(_) => refused.note("recorded class is absent from common/units", &literal),
        Err(reason) => refused.note(reason, &literal),
    }
    info
}

fn text(bytes: &[u8], what: &str, refused: &mut Refusals) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    match display_text(bytes) {
        Ok(text) if !text.is_empty() => Some(text),
        Ok(_) => None,
        Err(reason) => {
            refused.note(reason, what);
            None
        }
    }
}

fn stored(bytes: &[u8], what: &str, content: &Content, refused: &mut Refusals) -> Option<String> {
    let Ok(cat) = &content.catalogue else { return text(bytes, what, refused) };
    match dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, what) {
        Ok(value) => text(value.as_bytes(), what, refused),
        Err(reason) => {
            refused.note(reason, what);
            None
        }
    }
}

fn class_key_and_name(literal: Option<String>, content: &Content) -> (Option<String>, Option<String>) {
    let Some(literal) = literal.filter(|text| !text.is_empty()) else {
        return (None, None);
    };
    let (Ok(cat), Ok(units)) = (&content.catalogue, &content.units) else {
        return (Some(literal), None);
    };
    match units.token_name(&content_reader_parser::tokens::Token::c_str(literal.as_bytes())) {
        Ok(spelling) if !spelling.is_empty() => {
            let spelling = spelling.to_owned();
            let name = cat.localisation.get(&spelling).and_then(|_| cat.localised(&spelling, View::Plain));
            (Some(spelling), name)
        }
        _ => (Some(literal), None),
    }
}

impl Named {
    fn attribution_for(&self, entry: &SunkShipEntry, content: &Content, refused: &mut Refusals) -> ShipSinkingAttribution {
        let attr_text = stored(
            &entry.attribution_name,
            if entry.cause_record != 0 { "sinking cause" } else { "attribution ship name" },
            content,
            refused,
        );
        let (key, mut name) = if entry.attribution_class_present != 0 && entry.attribution_class_token != 0 {
            self.attr_classes.get(&entry.attribution_class_token).cloned().unwrap_or((None, None))
        } else {
            (None, None)
        };
        if entry.attribution_class_present != 0 && name.is_none() {
            name = text(&entry.attribution_class_name, "recorded attribution class name", refused);
        }
        resolve_attribution(entry, attr_text, key, name)
    }

    fn new(seeker: &mut Seeker<'_>, run: u32, hour: i32, entries: &[&SunkShipEntry], content: &Content) -> io::Result<Self> {
        let mut out = Self::default();
        let tokens: BTreeSet<u32> = entries.iter().map(|entry| entry.class_token).collect();
        let attr_tokens: BTreeSet<u32> = entries
            .iter()
            .filter(|entry| entry.attribution_class_present != 0 && entry.attribution_class_token != 0)
            .map(|entry| entry.attribution_class_token)
            .collect();
        let all_tokens: BTreeSet<u32> = tokens.union(&attr_tokens).copied().collect();
        let literals = literals(seeker, run, hour, &all_tokens)?;
        if let Err(reason) = &content.catalogue {
            out.warnings.push(format!("Ship class names, artwork and sea zones are unavailable: {reason}"));
        } else if let Err(reason) = &content.units {
            out.warnings.push(format!("Ship class content is unavailable: {reason}; classes show their recorded name or token without artwork."));
        }
        if let Ok(units) = &content.units {
            out.warnings.extend(units.warnings().iter().cloned());
        }
        for token in tokens {
            let info = class_info(token, literals.get(&token).cloned(), content, &mut out.refused);
            out.class_at.insert(token, out.classes.len() as u32);
            out.classes.push(info);
        }
        for token in attr_tokens {
            let resolved = class_key_and_name(literals.get(&token).cloned(), content);
            out.attr_classes.insert(token, resolved);
        }
        let mut names = HashMap::<String, u32>::new();
        for entry in entries {
            if out.recorded_at.contains_key(&entry.class_name) {
                continue;
            }
            let index = text(&entry.class_name, "recorded class name", &mut out.refused).map(|name| {
                *names.entry(name.clone()).or_insert_with(|| {
                    out.recorded_names.push(name);
                    out.recorded_names.len() as u32 - 1
                })
            });
            out.recorded_at.insert(entry.class_name.clone(), index);
        }
        match (&content.catalogue, &content.regions) {
            (Ok(cat), Ok((regions, memberships))) => {
                out.warnings.extend(regions.notes.iter().cloned());
                let mut indices = HashMap::<String, u32>::new();
                for province in entries.iter().filter(|entry| entry.province_present != 0).map(|entry| entry.province).collect::<BTreeSet<_>>() {
                    let name = match memberships.get(&province).map(Vec::as_slice) {
                        Some([region]) => {
                            let key = &regions.regions[region].name_key;
                            let name = cat.localised(key, View::Plain);
                            if name.is_none() {
                                out.refused.note("place localisation is unavailable", key);
                            }
                            name
                        }
                        Some(_) => {
                            out.refused.note("province belongs to multiple content strategic regions", format!("province {province}"));
                            None
                        }
                        None => {
                            out.refused.note("province is absent from content strategic regions", format!("province {province}"));
                            None
                        }
                    };
                    let index = name.map(|name| {
                        *indices.entry(name.clone()).or_insert_with(|| {
                            out.places.push(name);
                            out.places.len() as u32 - 1
                        })
                    });
                    out.place_at.insert(province, index);
                }
            }
            (Ok(_), Err(reason)) => out.warnings.push(format!("Ship sinking places are unavailable: {reason}; no place name is invented.")),
            _ => {}
        }
        if let Ok(cat) = &content.catalogue {
            out.warnings.extend(cat.verification.iter().chain(&cat.warnings).cloned());
            out.warnings.extend(cat.refusal_lines());
        }
        Ok(out)
    }
}

pub struct ShipsSunk;
pub struct ShipsSunkManifest;

impl Product for ShipsSunk {
    const KEY: &'static str = "ships-sunk";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ShipsSunkParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, window];
    type Reply = ShipsSunkReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ShipsSunkParams::decl(),
            ShipsSunkReply::decl(),
            ShipClassInfo::decl(),
            ShipSinkingCause::decl(),
            ShipSinkingAttribution::decl(),
            ShipSinkingBattle::decl(),
            ShipsSunkRow::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let content = Content::load(source, context);
        let reply = window(source, &query, &params.branch_id, params.window, &content, cancelled)?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub fn window(source: &Source, query: &Query<'_>, branch_id: &str, index: usize, content: &Content, cancelled: &AtomicBool) -> io::Result<ShipsSunkReply> {
    let windows = dashboard_world::products::army_windows::windows(query)?;
    let span = windows.get(index).cloned().ok_or_else(|| invalid("Unknown history window"))?;
    let (first, last) = (span.first_hour, span.last_hour);
    let stretch = query.timeline.at(first).ok_or_else(|| invalid("The window's hours are not recorded"))?;
    let run = stretch.run;
    let recording = query.replay.recording();
    let mut reply = ShipsSunkReply {
        branch_id: branch_id.to_owned(),
        window: index,
        available: false,
        first_hour: first,
        last_hour: last,
        first_read: None,
        unread: query.timeline.unheld_within(first, last).into_iter().map(|gap| ArmySpan { first_hour: gap.first, last_hour: gap.last }).collect(),
        classes: Vec::new(),
        places: Vec::new(),
        recorded_names: Vec::new(),
        rows: Vec::new(),
        warnings: Vec::new(),
    };
    let Some((sampled, entries)) = sinkings_at(recording, run, last)? else { return Ok(reply) };
    reply.available = true;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let mut reads = replay_lanes::lanes::reads(recording, run, TableId::SunkShips, (first - 720).max(stretch.first), last)?;
    if reads.first().is_none_or(|run| run.0 >= first) && stretch.first < first - 720 {
        reads = replay_lanes::lanes::reads(recording, run, TableId::SunkShips, stretch.first, last)?;
    }
    reply.first_read = known_at(&reads, first).filter(|&hour| hour <= last);
    let earlier = match reply.first_read {
        Some(read) if read < sampled => sinkings_at(recording, run, read)?.map(|(_, entries)| entries).unwrap_or_default(),
        _ => Vec::new(),
    };
    let gone = reply.first_read.map(|read| pruned(&earlier, &entries, read, &reads)).unwrap_or_default();
    let shown: Vec<(&SunkShipEntry, i32)> = gone
        .into_iter()
        .chain(&entries)
        .filter_map(|entry| known_at(&reads, entry.sink_hour).filter(|known| (first..=last).contains(known)).map(|known| (entry, known)))
        .collect();
    let named = Named::new(&mut Seeker::new(recording)?, run, last, &shown.iter().map(|(entry, _)| *entry).collect::<Vec<_>>(), content)?;
    let needed_combats: BTreeSet<u64> = shown.iter().map(|(entry, _)| entry.battle).filter(|&battle| battle != 0).collect();
    let battles = BattleIndex::build(source, query, last, &needed_combats, &BTreeSet::new())?;
    let links = country_ids::run_links(recording, run)?;
    let names = country_ids::Names::new(&query.holders);
    let mut refused = named.refused.clone();
    for (entry, known) in shown {
        let at = entry.sink_hour.clamp(stretch.first, last);
        let tag = |raw: u32| (raw != 0).then(|| names.tag_at(&links, raw, at).map(str::to_owned)).flatten();
        let sinking_cause = named.attribution_for(entry, content, &mut refused);
        let battle = battles.resolve_sinking(entry.battle, entry.sink_hour);
        reply.rows.push(ShipsSunkRow {
            sink_hour: entry.sink_hour,
            known_at: known,
            name: stored(&entry.ship_name, "ship name", content, &mut refused),
            class_index: named.class_at[&entry.class_token],
            recorded_name_index: named.recorded_at.get(&entry.class_name).copied().flatten(),
            victim: tag(entry.victim_country),
            attacker: tag(entry.attacker_country),
            victim_as_of: None,
            attacker_as_of: None,
            place_index: (entry.province_present != 0).then(|| named.place_at.get(&entry.province).copied().flatten()).flatten(),
            cause: cause(entry),
            attribution_name: stored(&entry.attribution_name, if entry.cause_record != 0 { "sinking cause" } else { "attribution ship name" }, content, &mut refused),
            assisted_kill: entry.assisted_kill != 0,
            sinking_cause,
            battle,
        });
    }
    reply.classes = named.classes;
    reply.places = named.places;
    reply.recorded_names = named.recorded_names;
    reply.warnings = named.warnings;
    reply.warnings.extend(refused.lines("Ships sunk fields"));
    Ok(reply)
}

impl Product for ShipsSunkManifest {
    const KEY: &'static str = "ships-sunk-manifest";
    const CONTENT_DEPENDENT: bool = true;
    type Params = dashboard_base::products::BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = ShipsSunkManifestReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ShipsSunkManifestReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let content = Content::load(source, context);
        let mut reply = ShipsSunkManifestReply {
            branch_id: params.branch_id.clone(),
            windows: dashboard_world::products::army_windows::windows(&query)?,
            available: false,
            first_read: None,
            classes: Vec::new(),
            recorded_names: Vec::new(),
            warnings: Vec::new(),
        };
        let mut ends: BTreeMap<u32, i32> = BTreeMap::new();
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if reply.first_read.is_none() {
                reply.first_read = replay_lanes::lanes::reads(recording, stretch.run, TableId::SunkShips, stretch.first, stretch.last)?.first().map(|run| run.0);
            }
            let end = ends.entry(stretch.run).or_insert(stretch.last);
            *end = (*end).max(stretch.last);
        }
        reply.available = reply.first_read.is_some();
        let mut tokens = BTreeMap::<u32, ShipClassInfo>::new();
        let mut recorded = BTreeSet::<String>::new();
        let mut seeker = Seeker::new(recording)?;
        for (&run, &end) in &ends {
            let Some((_, entries)) = sinkings_at(recording, run, end)? else { continue };
            let entries: Vec<&SunkShipEntry> = entries.iter().collect();
            let named = Named::new(&mut seeker, run, end, &entries, &content)?;
            for class in named.classes {
                tokens.entry(class.token).or_insert(class);
            }
            recorded.extend(named.recorded_names);
            reply.warnings.extend(named.warnings);
        }
        reply.classes = tokens.into_values().collect();
        reply.recorded_names = recorded.into_iter().collect();
        reply.warnings.sort();
        reply.warnings.dedup();
        Ok(reply)
    }
}
