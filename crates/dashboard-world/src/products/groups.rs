use content_reader_politics::faction_templates::FactionTemplateProducts as _;

use dashboard_base::products::{BranchParams, LoadContext, Product};
use super::identities::CountryIdentityRun;
use dashboard_base::{Source, catalogue, content, span::HourSpan};
use dashboard_query::{Query, identity::Holder, read};
use content_reader_politics::faction_templates::Catalog;
use content_reader_localisation::localisation::Localisation;
use replay_recording::columnar_frames::Recording;
use replay_columnar::store::{TableView, Value};
use replay_data::rows::{GroupSpanRow as Span, TableId, TagHolderRow, WarEventRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MembershipRun {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionNameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionGroup {
    #[doc = " The recorded faction id, as a decimal string."]
    pub id: String,
    #[doc = " The faction's latest name as the game showed it; null when it cannot be resolved."]
    pub name: Option<String>,
    #[doc = " Why the latest name cannot be resolved."]
    pub name_reason: Option<String>,
    #[doc = " Display names dated by the Factions row that supplied their operands."]
    pub names: Vec<FactionNameRun>,
    #[doc = " The latest engine literal, copied unchanged; empty when there is none. Stable across"]
    #[doc = " languages and content, so settings can key a faction by it."]
    pub recorded_name: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub members: Vec<MembershipRun>,
    #[doc = " Who led the faction (its member at order zero), as runs of consecutive recorded hours."]
    pub leaders: Vec<MembershipRun>,
    #[doc = " Hours this faction's roster named a country the links cannot resolve: its roster is not"]
    #[doc = " known then (other factions' rosters are)."]
    pub incomplete: Vec<HourSpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarPairRun {
    pub first: String,
    pub second: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UnobservedHours {
    pub humans: Vec<HourSpan>,
    pub factions: Vec<HourSpan>,
    pub wars: Vec<HourSpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarStart {
    pub hour: i32,
    pub attacking: String,
    pub defending: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Capitulation {
    pub hour: i32,
    pub tag: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdentityChange {
    pub hour: i32,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentityHistory {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryGroupReply {
    pub branch_id: String,
    pub humans: Vec<MembershipRun>,
    pub factions: Vec<FactionGroup>,
    pub wars: Vec<WarPairRun>,
    #[doc = " Every war start the recorded war relations name, in hour order."]
    pub war_starts: Vec<WarStart>,
    #[doc = " Every capitulation the recording saw, in hour order."]
    pub capitulations: Vec<Capitulation>,
    #[doc = " Every change of an id's tag between two consecutive recorded hours, in hour order."]
    pub identity_changes: Vec<IdentityChange>,
    #[doc = " Every tag's holders."]
    pub country_identities: Vec<CountryIdentityHistory>,
    #[doc = " Hours inside the timeline where each history was not observed: a run that ends or starts"]
    #[doc = " next to one of these is a gap, not a join or a leave."]
    pub unobserved: UnobservedHours,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryGroupReply);

fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    spans.push(HourSpan { first_hour, last_hour });
}

fn settle(spans: &mut Vec<HourSpan>) {
    spans.sort_unstable_by_key(|span| span.first_hour);
    let mut out: Vec<HourSpan> = Vec::with_capacity(spans.len());
    for span in spans.drain(..) {
        match out.last_mut() {
            Some(held) if i64::from(held.last_hour) + 1 >= i64::from(span.first_hour) => held.last_hour = held.last_hour.max(span.last_hour),
            _ => out.push(span),
        }
    }
    *spans = out;
}

fn joined<R: Clone>(mut runs: Vec<R>, hours: impl Fn(&R) -> (i32, i32), extend: impl Fn(&mut R, i32), same: impl Fn(&R, &R) -> bool) -> Vec<R> {
    runs.sort_by_key(|run| hours(run));
    let mut out: Vec<R> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some(held) if same(held, &run) && i64::from(hours(held).1) + 1 >= i64::from(hours(&run).0) => {
                let last = hours(held).1.max(hours(&run).1);
                extend(held, last);
            }
            _ => out.push(run),
        }
    }
    out
}

fn cell_u32(table: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    match table.value(column, slot) {
        Value::Number(bits) => Some(bits as u32),
        Value::Text(id) => Some(id),
        _ => None,
    }
}

fn cell_u64(table: &TableView<'_>, column: usize, slot: u32) -> Option<u64> {
    match table.value(column, slot) {
        Value::Number(bits) => Some(bits),
        _ => None,
    }
}

#[derive(Clone)]
struct NameOperands {
    literal: Vec<u8>,
    token: Option<u32>,
    spelling: Vec<u8>,
}

pub struct RunGroups {
    spans: Vec<(u8, u32, u32, i32, i32)>,
    events: Vec<(u8, i32, u32, u32, i32)>,
    factions: HashMap<u32, u64>,
    names: HashMap<u32, NameOperands>,
}

pub fn run_groups(recording: &Recording, run: u32) -> io::Result<RunGroups> {
    let held = recording.run(run)?;
    let end = held.first_hour + held.ordinals as i32 - 1;
    let (mut factions, mut names) = (HashMap::new(), HashMap::new());
    let held = read::at(recording, run, &[read::Read::all(TableId::Groups), read::Read::all(TableId::GroupNames)], end)?;
    let rows = held[0].view();
    for slot in rows.rows() {
        if let (Some(key), Some(id)) = (rows.key(slot), cell_u64(&rows, 0, slot)) {
            factions.insert(key as u32, id);
        }
    }
    let rows = held[1].view();
    let text = |column: usize, slot: u32| read::text(&rows, column, slot).map(<[u8]>::to_vec).unwrap_or_default();
    for slot in rows.rows() {
        let Some(key) = rows.key(slot) else { continue };
        names.insert(key as u32, NameOperands { literal: text(0, slot), token: cell_u32(&rows, 1, slot), spelling: text(2, slot) });
    }
    let spans = dashboard_query::cost::lifetimes(recording, run, TableId::GroupSpans)?
        .into_iter()
        .map(|(key, first, last)| ((key >> 96) as u8, (key >> 64) as u32, (key >> 32) as u32, first as i32, last as i32))
        .collect();
    let events = dashboard_query::cost::lifetimes(recording, run, TableId::WarEvents)?
        .into_iter()
        .map(|(key, first, _)| ((key >> 96) as u8, (key >> 64) as u32 as i32, (key >> 32) as u32, key as u32, first as i32))
        .collect();
    Ok(RunGroups { spans, events, factions, names })
}

pub fn faction_members(source: &Source, query: &Query<'_>) -> io::Result<(BTreeMap<u64, Vec<(String, i32, i32)>>, Vec<HourSpan>)> {
    let holders = slot_holders(query);
    let (mut members, mut unknown): (BTreeMap<u64, Vec<(String, i32, i32)>>, Vec<HourSpan>) = (BTreeMap::new(), Vec::new());
    for gap in &query.timeline.unheld {
        mark(&mut unknown, gap.first, gap.last);
    }
    for stretch in &query.timeline.stretches {
        let run = crate::products::groups::cached_groups(source, stretch.run)?;
        for &(kind, group, value, first, last) in &run.spans {
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if first > last {
                continue;
            }
            match kind {
                Span::MEMBER if group != Span::HUMANS => {
                    let Some(&id) = run.factions.get(&group) else { continue };
                    let held = members.entry(id).or_default();
                    held.extend(named(&holders, value, first, last).map(|(tag, a, b)| (tag.to_owned(), a, b)));
                }
                Span::INCOMPLETE => mark(&mut unknown, first, last),
                Span::UNOBSERVED if group == Span::FACTIONS => mark(&mut unknown, first, last),
                _ => {}
            }
        }
    }
    settle(&mut unknown);
    Ok((members, unknown))
}

fn slot_holders(query: &Query<'_>) -> HashMap<u32, Vec<(String, i32, i32)>> {
    let mut out: HashMap<u32, Vec<(String, i32, i32)>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, .. } = run.holder {
                out.entry(slot).or_default().push((tag.clone(), run.first, run.last));
            }
        }
    }
    for runs in out.values_mut() {
        runs.sort_unstable_by_key(|run| run.1);
    }
    out
}

fn named<'h>(holders: &'h HashMap<u32, Vec<(String, i32, i32)>>, slot: u32, first: i32, last: i32) -> impl Iterator<Item = (&'h str, i32, i32)> + 'h {
    holders.get(&slot).into_iter().flatten().filter_map(move |(tag, held_first, held_last)| {
        let (a, b) = (first.max(*held_first), last.min(*held_last));
        (a <= b).then_some((tag.as_str(), a, b))
    })
}

fn tag_at<'h>(holders: &'h HashMap<u32, Vec<(String, i32, i32)>>, slot: u32, hour: i32) -> Option<&'h str> {
    named(holders, slot, hour, hour).next().map(|(tag, ..)| tag)
}

struct Naming {
    templates: Result<Arc<Catalog>, String>,
    localisation: Arc<Localisation>,
    recorded_language: String,
}

fn naming(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> Naming {
    let recorded_language = content::recorded_language(source.replay_ref());
    let loaded = context.content.as_ref().map(|options| {
        content::check_replay(source.replay_ref(), options.allow_unverified, warnings)?;
        let layers = source.content(&options.options)?;
        let localisation = content::localisation(source, options)?;
        Ok::<_, io::Error>((layers.layers.faction_templates(), localisation))
    });
    let stored = || {
        let mut stored = Localisation::default();
        stored.language = recorded_language.clone();
        Arc::new(stored)
    };
    let (templates, localisation) = match loaded {
        Some(Ok((templates, localisation))) => {
            if let Ok(templates) = &templates {
                warnings.extend(templates.warnings.iter().cloned());
            }
            (templates, localisation)
        }
        Some(Err(error)) => (Err(format!("Faction naming content is unavailable: {error}")), stored()),
        None => (Err("Faction naming content is unavailable until the game folders are chosen".into()), stored()),
    };
    Naming { templates, localisation, recorded_language }
}

fn faction_name(naming: &Naming, name: &NameOperands, lexical: &mut dyn FnMut(&[u8]) -> Result<Option<u32>, String>) -> Result<String, String> {
    let template = match name.token {
        None => Ok(None),
        Some(token) if name.spelling.is_empty() => Err(format!("Faction template token {token} has no lexical tokens spelling")),
        Some(token) => String::from_utf8(name.spelling.clone()).map(Some).map_err(|_| format!("Faction template token {token} is not UTF-8")),
    };
    let text = dashboard_text::faction_names::name(
        &name.literal,
        template.as_ref().map(|value| value.as_deref()).map_err(Clone::clone),
        naming.templates.as_deref().map_err(Clone::clone),
        &naming.localisation,
        &naming.recorded_language,
        lexical,
    )?;
    let plain = catalogue::plain_text(&text)?;
    if !text.is_empty() && plain.is_empty() {
        return Err("Faction name has no plain display text".into());
    }
    Ok(plain)
}

fn lexicon(recording: &Recording, run: u32) -> io::Result<HashMap<Vec<u8>, u32>> {
    let held = recording.run(run)?;
    let end = held.first_hour + held.ordinals as i32 - 1;
    let mut out = HashMap::new();
    let held = read::at(recording, run, &[read::Read::all(TableId::LexicalTokens)], end)?;
    let rows = held[0].view();
    for slot in rows.rows() {
        if let (Some(key), Some(text)) = (rows.key(slot), read::text(&rows, 0, slot)) {
            out.insert(text.to_vec(), key as u32);
        }
    }
    Ok(out)
}

pub fn groups(source: &Source, query: &Query<'_>, branch_id: &str, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<CountryGroupReply> {
    let mut warnings = Vec::new();
    let naming = naming(source, context, &mut warnings);
    let recording = query.replay.recording();
    let timeline = &query.timeline;
    let holders = slot_holders(query);
    let mut read: HashMap<u32, Arc<RunGroups>> = HashMap::new();
    let mut lexicons: HashMap<u32, HashMap<Vec<u8>, u32>> = HashMap::new();
    let mut refused = catalogue::Refusals::default();
    let mut humans = Vec::new();
    let mut wars = Vec::new();
    let mut unobserved = UnobservedHours::default();
    struct Faction {
        first: i32,
        last: i32,
        names: Vec<(FactionNameRun, Vec<u8>)>,
        members: Vec<MembershipRun>,
        leaders: Vec<MembershipRun>,
        incomplete: Vec<HourSpan>,
    }
    let mut factions: BTreeMap<u64, Faction> = BTreeMap::new();
    let mut war_starts = BTreeSet::new();
    let mut capitulations = BTreeSet::new();
    let mut unattributed = 0usize;
    let mut resolved: HashMap<(u32, u32), Result<String, String>> = HashMap::new();
    for stretch in &timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !read.contains_key(&stretch.run) {
            read.insert(stretch.run, crate::products::groups::cached_groups(source, stretch.run)?);
        }
        let run = &read[&stretch.run];
        for &(kind, group, value, first, last) in &run.spans {
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if first > last {
                continue;
            }
            let faction = || run.factions.get(&group).copied();
            match kind {
                Span::MEMBER | Span::LEADER if group == Span::HUMANS => {
                    for (tag, a, b) in named(&holders, value, first, last) {
                        humans.push(MembershipRun { tag: tag.to_owned(), first_hour: a, last_hour: b });
                    }
                }
                Span::MEMBER | Span::LEADER | Span::PRESENT | Span::NAME | Span::INCOMPLETE => {
                    let Some(id) = faction() else { continue };
                    let held = factions.entry(id).or_insert_with(|| Faction { first, last, names: Vec::new(), members: Vec::new(), leaders: Vec::new(), incomplete: Vec::new() });
                    match kind {
                        Span::PRESENT => {
                            held.first = held.first.min(first);
                            held.last = held.last.max(last);
                        }
                        Span::INCOMPLETE => mark(&mut held.incomplete, first, last),
                        Span::NAME => {
                            let Some(operands) = run.names.get(&value) else { continue };
                            let text = resolved
                                .entry((stretch.run, value))
                                .or_insert_with(|| {
                                    let mut lexical = |literal: &[u8]| -> Result<Option<u32>, String> {
                                        if !lexicons.contains_key(&stretch.run) {
                                            let read = lexicon(recording, stretch.run).map_err(|error| error.to_string())?;
                                            lexicons.insert(stretch.run, read);
                                        }
                                        Ok(lexicons[&stretch.run].get(literal).copied())
                                    };
                                    faction_name(&naming, operands, &mut lexical)
                                })
                                .clone();
                            let (name, reason) = match text {
                                Ok(name) => (Some(name), None),
                                Err(reason) => {
                                    refused.note(&reason, format!("faction {id}"));
                                    (None, Some(reason))
                                }
                            };
                            held.names.push((FactionNameRun { first_hour: first, last_hour: last, name, reason }, operands.literal.clone()));
                        }
                        _ => {
                            let list = if kind == Span::MEMBER { &mut held.members } else { &mut held.leaders };
                            for (tag, a, b) in named(&holders, value, first, last) {
                                list.push(MembershipRun { tag: tag.to_owned(), first_hour: a, last_hour: b });
                            }
                        }
                    }
                }
                Span::WAR => {
                    for (one, a1, b1) in named(&holders, group, first, last) {
                        for (other, a2, b2) in named(&holders, value, a1, b1) {
                            let (first_tag, second_tag) = if one <= other { (one, other) } else { (other, one) };
                            wars.push(WarPairRun { first: first_tag.to_owned(), second: second_tag.to_owned(), first_hour: a2.max(a1), last_hour: b2.min(b1) });
                        }
                    }
                }
                Span::UNOBSERVED => {
                    let spans = match group {
                        Span::HUMANS => &mut unobserved.humans,
                        Span::FACTIONS => &mut unobserved.factions,
                        _ => &mut unobserved.wars,
                    };
                    mark(spans, first, last);
                }
                _ => {}
            }
        }
        for &(kind, hour, first, second, read_at) in &run.events {
            if read_at < stretch.first || read_at > stretch.last || hour < timeline.first {
                continue;
            }
            let tag = |slot: u32| (slot != TagHolderRow::NONE).then(|| tag_at(&holders, slot, hour).or_else(|| tag_at(&holders, slot, read_at))).flatten();
            match kind {
                WarEventRow::START => match (tag(first), tag(second)) {
                    (Some(attacking), Some(defending)) => {
                        war_starts.insert(WarStart { hour, attacking: attacking.to_owned(), defending: defending.to_owned() });
                    }
                    _ => unattributed += usize::from(hour >= timeline.first),
                },
                WarEventRow::CAPITULATION => match tag(first) {
                    Some(tag) => {
                        capitulations.insert(Capitulation { hour, tag: tag.to_owned() });
                    }
                    None => unattributed += usize::from(hour >= timeline.first),
                },
                _ => {}
            }
        }
    }
    for (first, last, _) in &query.holders.unknown {
        for spans in [&mut unobserved.humans, &mut unobserved.factions, &mut unobserved.wars] {
            mark(spans, *first, *last);
        }
    }
    for gap in &timeline.unheld {
        for spans in [&mut unobserved.humans, &mut unobserved.factions, &mut unobserved.wars] {
            mark(spans, gap.first, gap.last);
        }
    }
    for spans in [&mut unobserved.humans, &mut unobserved.factions, &mut unobserved.wars] {
        settle(spans);
    }
    let membership = |runs: Vec<MembershipRun>| {
        joined(runs, |run| (run.first_hour, run.last_hour), |run, last| run.last_hour = last, |a, b| a.tag == b.tag)
    };
    let mut humans = membership(humans);
    humans.sort_by(|a, b| (a.first_hour, &a.tag).cmp(&(b.first_hour, &b.tag)));
    let mut wars = joined(wars, |run| (run.first_hour, run.last_hour), |run, last| run.last_hour = last, |a, b| (&a.first, &a.second) == (&b.first, &b.second));
    wars.sort_by(|a, b| (a.first_hour, &a.first, &a.second).cmp(&(b.first_hour, &b.first, &b.second)));
    let mut invalid_names = 0;
    let factions: Vec<FactionGroup> = factions
        .into_iter()
        .map(|(id, mut faction)| {
            faction.names.sort_by_key(|(run, _)| run.first_hour);
            let recorded = faction.names.last().map(|(_, literal)| literal.clone()).unwrap_or_default();
            if std::str::from_utf8(&recorded).is_err() {
                invalid_names += 1;
            }
            let names = joined(
                faction.names.into_iter().map(|(run, _)| run).collect(),
                |run| (run.first_hour, run.last_hour),
                |run, last| run.last_hour = last,
                |a, b| (&a.name, &a.reason) == (&b.name, &b.reason),
            );
            let (name, name_reason) = match names.last() {
                Some(latest) => (latest.name.clone(), latest.reason.clone()),
                None => (None, Some("Faction has no recorded name read".to_owned())),
            };
            let mut members = membership(faction.members);
            members.sort_by(|a, b| (a.first_hour, &a.tag).cmp(&(b.first_hour, &b.tag)));
            let mut leaders = membership(faction.leaders);
            leaders.sort_by_key(|run| run.first_hour);
            settle(&mut faction.incomplete);
            FactionGroup {
                id: id.to_string(),
                name,
                name_reason,
                names,
                recorded_name: String::from_utf8(recorded).unwrap_or_default(),
                first_hour: faction.first,
                last_hour: faction.last,
                members,
                leaders,
                incomplete: faction.incomplete,
            }
        })
        .collect();
    let mut by_id: HashMap<u32, Vec<(i32, i32, &str)>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { id, .. } = run.holder {
                by_id.entry(id).or_default().push((run.first, run.last, tag.as_str()));
            }
        }
    }
    let mut identity_changes = BTreeSet::new();
    for runs in by_id.values_mut() {
        runs.sort_unstable();
        for pair in runs.windows(2) {
            let ((_, last, from), (first, _, to)) = (pair[0], pair[1]);
            if i64::from(last) + 1 == i64::from(first) && from != to {
                identity_changes.insert(IdentityChange { hour: first, from: from.to_owned(), to: to.to_owned() });
            }
        }
    }
    if invalid_names > 0 {
        warnings.push(format!("{invalid_names} faction names were not valid text and are not recorded."));
    }
    if unattributed > 0 {
        warnings.push(format!(
            "{unattributed} war starts or capitulations name a country no link resolved; which countries they were is not known, so they are left out."
        ));
    }
    warnings.extend(refused.lines("faction names are"));
    let country_identities = query
        .holders
        .tags
        .iter()
        .map(|(tag, runs)| CountryIdentityHistory {
            tag: tag.clone(),
            identities: runs
                .iter()
                .filter_map(|run| match run.holder {
                    Holder::Found { slot, id } => Some(CountryIdentityRun { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
                    Holder::Unresolved => None,
                })
                .collect(),
        })
        .filter(|history| !history.identities.is_empty())
        .collect();
    let mut identity_changes: Vec<IdentityChange> = identity_changes.into_iter().collect();
    identity_changes.sort_by_key(|change| change.hour);
    Ok(CountryGroupReply {
        branch_id: branch_id.to_owned(),
        humans,
        factions,
        wars,
        war_starts: war_starts.into_iter().collect(),
        capitulations: capitulations.into_iter().collect(),
        identity_changes,
        country_identities,
        unobserved,
        warnings,
    })
}

pub struct CountryGroups;

impl Product for CountryGroups {
    const KEY: &'static str = "country-groups";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = CountryGroupReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            MembershipRun::decl(),
            FactionNameRun::decl(),
            FactionGroup::decl(),
            WarPairRun::decl(),
            WarStart::decl(),
            Capitulation::decl(),
            IdentityChange::decl(),
            CountryIdentityHistory::decl(),
            CountryIdentityRun::decl(),
            HourSpan::decl(),
            UnobservedHours::decl(),
            CountryGroupReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid country group branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid country group branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        groups(source, &query, &params.branch_id, context, cancelled)
    }
}

enum RunKey {}

type Runs = dashboard_base::queries::RunCache<RunKey, RunGroups>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    Runs::forget(source);
}

pub fn cached_groups(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunGroups>> {
    Runs::held(&source.state::<Runs>(), run, || run_groups(source.replay_ref().recording(), run))
}
