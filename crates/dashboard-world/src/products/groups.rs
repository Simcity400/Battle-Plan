use content_reader_politics::faction_templates::FactionTemplateProducts as _;

use dashboard_base::products::{BranchParams, LoadContext, Product};
use super::identities::CountryIdentityRun;
use dashboard_base::{Source, catalogue, content, span::HourSpan};
use dashboard_query::{Query, identity::Holder, read};
use content_reader_politics::faction_templates::Catalog;
use content_reader_localisation::localisation::Localisation;
use content_reader_layers::ResolvedContent;
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
    #[doc = " This run's recorded engine literal, unchanged; null when empty or not valid text."]
    pub token: Option<String>,
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
    #[doc = " The hour of each war start or capitulation read whose country no link resolved, in hour order:"]
    #[doc = " it is left out, and who it was is not known."]
    pub unattributed: Vec<i32>,
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

fn gap_covered(gaps: &[HourSpan], first: i32, last: i32) -> bool {
    if first > last {
        return true;
    }
    let at = gaps.partition_point(|span| span.last_hour < first);
    gaps.get(at).is_some_and(|span| span.first_hour <= first && last <= span.last_hour)
}

fn joined_across<R, K: Ord>(
    mut runs: Vec<R>,
    gaps: &[HourSpan],
    key: impl Fn(&R) -> K,
    hours: impl Fn(&R) -> (i32, i32),
    extend: impl Fn(&mut R, i32),
    same: impl Fn(&R, &R) -> bool,
) -> Vec<R> {
    runs.sort_by(|a, b| key(a).cmp(&key(b)).then_with(|| hours(a).cmp(&hours(b))));
    let mut out: Vec<R> = Vec::with_capacity(runs.len());
    for run in runs {
        let (first, last) = hours(&run);
        match out.last_mut() {
            Some(held) if same(held, &run) && gap_covered(gaps, hours(held).1.saturating_add(1), first.saturating_sub(1)) => {
                extend(held, hours(held).1.max(last));
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
    pub(crate) spans: Vec<(u8, u32, u32, i32, i32)>,
    events: Vec<(u8, i32, u32, u32, i32)>,
    pub(crate) factions: HashMap<u32, u64>,
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

fn event_tag<'h>(holders: &'h HashMap<u32, Vec<(String, i32, i32)>>, slot: u32, hour: i32) -> Option<&'h str> {
    (slot != TagHolderRow::NONE).then(|| tag_at(holders, slot, hour)).flatten()
}

struct Naming {
    templates: Result<Arc<Catalog>, String>,
    localisation: Arc<Localisation>,
    recorded_language: String,
    generation: Option<Generation>,
}

#[derive(Clone)]
enum Generation {
    Uncontent,
    Of(std::sync::Weak<ResolvedContent>),
}

impl Generation {
    fn is(&self, current: Option<&Arc<ResolvedContent>>) -> bool {
        match (self, current) {
            (Self::Uncontent, None) => true,
            (Self::Of(built), Some(current)) => built.upgrade().is_some_and(|built| Arc::ptr_eq(&built, current)),
            _ => false,
        }
    }
}

fn naming(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> Naming {
    let recorded_language = content::recorded_language(source.replay_ref());
    let loaded = context.content.as_ref().map(|options| {
        content::check_replay(source.replay_ref(), options.allow_unverified, warnings)?;
        let layers = source.content(&options.options)?;
        let localisation = content::localisation(source, options)?;
        Ok::<_, io::Error>((layers.layers.faction_templates(), localisation, Arc::downgrade(&layers)))
    });
    let stored = || {
        let mut stored = Localisation::default();
        stored.language = recorded_language.clone();
        Arc::new(stored)
    };
    let (templates, localisation, generation) = match loaded {
        Some(Ok((templates, localisation, layers))) => {
            if let Ok(templates) = &templates {
                warnings.extend(templates.warnings.iter().cloned());
            }
            (templates, localisation, Some(Generation::Of(layers)))
        }
        Some(Err(error)) => (Err(format!("Faction naming content is unavailable: {error}")), stored(), None),
        None => (Err("Faction naming content is unavailable until the game folders are chosen".into()), stored(), Some(Generation::Uncontent)),
    };
    Naming { templates, localisation, recorded_language, generation }
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
    built(source, query, branch_id, context, cancelled).map(|(reply, _)| reply)
}

fn built(source: &Source, query: &Query<'_>, branch_id: &str, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<(CountryGroupReply, Option<Generation>)> {
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
    let mut resolved: HashMap<(u32, u32), Result<String, String>> = HashMap::new();
    for stretch in &timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if let std::collections::hash_map::Entry::Vacant(slot) = read.entry(stretch.run) {
            slot.insert(crate::products::groups::cached_groups(source, stretch.run)?);
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
                                        if let std::collections::hash_map::Entry::Vacant(vacant) = lexicons.entry(stretch.run) {
                                            let read = lexicon(recording, stretch.run).map_err(|error| error.to_string())?;
                                            vacant.insert(read);
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
                            let token = std::str::from_utf8(&operands.literal).ok().filter(|literal| !literal.is_empty()).map(str::to_owned);
                            held.names.push((FactionNameRun { first_hour: first, last_hour: last, name, reason, token }, operands.literal.clone()));
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
            let tag = |slot: u32| event_tag(&holders, slot, hour);
            match kind {
                WarEventRow::START => match (tag(first), tag(second)) {
                    (Some(attacking), Some(defending)) => {
                        war_starts.insert(WarStart { hour, attacking: attacking.to_owned(), defending: defending.to_owned() });
                    }
                    _ => unobserved.unattributed.push(hour),
                },
                WarEventRow::CAPITULATION => match tag(first) {
                    Some(tag) => {
                        capitulations.insert(Capitulation { hour, tag: tag.to_owned() });
                    }
                    None => unobserved.unattributed.push(hour),
                },
                _ => {}
            }
        }
    }
    let mut identity_gaps = Vec::new();
    for (first, last, _) in &query.holders.unknown {
        mark(&mut identity_gaps, *first, *last);
        for spans in [&mut unobserved.humans, &mut unobserved.factions, &mut unobserved.wars] {
            mark(spans, *first, *last);
        }
    }
    for gap in &timeline.unheld {
        mark(&mut identity_gaps, gap.first, gap.last);
        for spans in [&mut unobserved.humans, &mut unobserved.factions, &mut unobserved.wars] {
            mark(spans, gap.first, gap.last);
        }
    }
    settle(&mut identity_gaps);
    for spans in [&mut unobserved.humans, &mut unobserved.factions, &mut unobserved.wars] {
        settle(spans);
    }
    let membership = |runs: Vec<MembershipRun>, gaps: &[HourSpan]| {
        joined_across(
            runs,
            gaps,
            |run| run.tag.clone(),
            |run| (run.first_hour, run.last_hour),
            |run, last| run.last_hour = last,
            |a, b| a.tag == b.tag,
        )
    };
    let mut humans = membership(humans, &unobserved.humans);
    humans.sort_by(|a, b| (a.first_hour, &a.tag).cmp(&(b.first_hour, &b.tag)));
    let mut wars = joined_across(
        wars,
        &unobserved.wars,
        |run| (run.first.clone(), run.second.clone()),
        |run| (run.first_hour, run.last_hour),
        |run, last| run.last_hour = last,
        |a, b| (&a.first, &a.second) == (&b.first, &b.second),
    );
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
            let names = joined_across(
                faction.names.into_iter().map(|(run, _)| run).collect(),
                &unobserved.factions,
                |_| 0u8,
                |run| (run.first_hour, run.last_hour),
                |run, last| run.last_hour = last,
                |a, b| (&a.name, &a.reason, &a.token) == (&b.name, &b.reason, &b.token),
            );
            let (name, name_reason) = match names.last() {
                Some(latest) => (latest.name.clone(), latest.reason.clone()),
                None => (None, Some("Faction has no recorded name read".to_owned())),
            };
            let mut members = membership(faction.members, &unobserved.factions);
            members.sort_by(|a, b| (a.first_hour, &a.tag).cmp(&(b.first_hour, &b.tag)));
            let mut leaders = membership(faction.leaders, &unobserved.factions);
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
    unobserved.unattributed.sort_unstable();
    if !unobserved.unattributed.is_empty() {
        warnings.push(format!(
            "{} war starts or capitulations name a country no link resolved; which countries they were is not known, so they are left out.",
            unobserved.unattributed.len()
        ));
    }
    warnings.extend(refused.lines("faction names are"));
    let country_identities = query
        .holders
        .tags
        .iter()
        .map(|(tag, runs)| {
            let mut identities: Vec<CountryIdentityRun> = Vec::new();
            for run in runs {
                let Holder::Found { slot, id } = run.holder else { continue };
                match identities.last_mut() {
                    Some(held) if held.slot == slot && held.country_id == id && gap_covered(&identity_gaps, held.last_hour.saturating_add(1), run.first.saturating_sub(1)) => {
                        held.last_hour = held.last_hour.max(run.last);
                    }
                    _ => identities.push(CountryIdentityRun { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
                }
            }
            CountryIdentityHistory { tag: tag.clone(), identities }
        })
        .filter(|history| !history.identities.is_empty())
        .collect();
    let mut identity_changes: Vec<IdentityChange> = identity_changes.into_iter().collect();
    identity_changes.sort_by_key(|change| change.hour);
    let reply = CountryGroupReply {
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
    };
    Ok((reply, naming.generation))
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

pub fn faction_name_at(faction: &FactionGroup, hour: i32) -> (Option<String>, Option<String>) {
    if let Some(run) = faction.names.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour) {
        match &run.name {
            Some(name) => (Some(name.clone()), None),
            None => (None, Some(run.reason.clone().unwrap_or_else(|| "Faction name was not recorded at this hour".to_owned()))),
        }
    } else {
        (None, Some("Faction name was not recorded at this hour".to_owned()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactionAt {
    pub id: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
}

pub fn factions_at(factions: &[FactionGroup], hour: i32) -> HashMap<String, FactionAt> {
    let mut out = HashMap::new();
    for faction in factions {
        let (name, name_missing) = faction_name_at(faction, hour);
        for run in &faction.members {
            if run.first_hour <= hour && hour <= run.last_hour {
                out.entry(run.tag.clone()).or_insert_with(|| FactionAt { id: faction.id.clone(), name: name.clone(), name_missing: name_missing.clone() });
            }
        }
    }
    out
}

pub fn factions_valid_through(factions: &[FactionGroup], hour: i32) -> i32 {
    let mut through = i32::MAX;
    let mut bound = |first: i32, last: i32| {
        if first > hour {
            through = through.min(first - 1);
        } else if last >= hour {
            through = through.min(last);
        }
    };
    for faction in factions {
        for run in &faction.names {
            bound(run.first_hour, run.last_hour);
        }
        for run in &faction.members {
            bound(run.first_hour, run.last_hour);
        }
    }
    through
}

#[derive(Default)]
struct Replies(HashMap<(u64, Option<dashboard_base::content::LocalisationKey>), (Generation, Arc<CountryGroupReply>)>);

pub fn shared(source: &Source, query: &Query<'_>, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<CountryGroupReply>> {
    let key = (query.timeline.id, context.content.as_ref().map(dashboard_base::content::localisation_key));
    let current = match &context.content {
        None => Some(None),
        Some(options) => source.content(&options.options).ok().map(Some),
    };
    let held = source.state::<Replies>();
    if let Some(current) = &current
        && let Some((generation, reply)) = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.get(&key)
        && generation.is(current.as_ref())
    {
        return Ok(Arc::clone(reply));
    }
    let (reply, generation) = built(source, query, &query.timeline.id.to_string(), context, cancelled)?;
    let reply = Arc::new(reply);
    if let Some(generation) = generation {
        held.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.insert(key, (generation, Arc::clone(&reply)));
    }
    Ok(reply)
}

pub fn cached_groups(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunGroups>> {
    Runs::held(&source.state::<Runs>(), run, || run_groups(source.replay_ref().recording(), run))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branchs_group_reply_is_built_once_and_every_request_shares_it() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
        let frames = (100..=110).map(|hour| source(1, hour, Some(1))).collect();
        let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let held = Source::from_replay(dashboard_base::SourceKey { source_id: "shared-groups".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let query = dashboard_base::queries::query(&held, 1).unwrap();
        let context = LoadContext { content: None };
        let first = shared(&held, &query, &context, &AtomicBool::new(false)).unwrap();
        let cancelled = AtomicBool::new(true);
        let again = shared(&held, &query, &context, &cancelled).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "a later request reads the branch's reply already built and visits no stretch again");
    }

    #[test]
    fn a_reply_made_on_a_failed_naming_lookup_is_made_again_and_one_made_on_content_is_kept() {
        use dashboard_base::test_fixtures::{source, source_with_content};
        let root = std::env::temp_dir().join(format!("shared-groups-{}", uuid::Uuid::new_v4()));
        let (held, context) = source_with_content((100..=110).map(|hour| source(1, hour, Some(1))).collect(), &root);
        use dashboard_query::{identity::Holders, timeline::{Join, Stretch, Timeline}};
        let replay = held.replay();
        let timeline = Timeline { id: 1, first: 100, last: 110, stretches: vec![Stretch { run: 0, first: 100, last: 110, join: Join::Start }], unheld: Vec::new() };
        let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(Holders::new(Default::default(), Vec::new())) };
        let options = context.content.clone().expect("the fixture names its content");
        let refused = LoadContext { content: Some(dashboard_base::products::ContentOptions { allow_unverified: false, ..options }) };
        let failed = shared(&held, &query, &refused, &AtomicBool::new(false)).unwrap();
        let retried = shared(&held, &query, &refused, &AtomicBool::new(false)).unwrap();
        assert!(!Arc::ptr_eq(&failed, &retried), "a reply made on a failed naming lookup is not kept: the next request looks again");
        let named = shared(&held, &query, &context, &AtomicBool::new(false)).unwrap();
        assert!(Arc::ptr_eq(&named, &shared(&held, &query, &context, &AtomicBool::new(true)).unwrap()), "a reply made on the content is kept for that content");
        drop(query);
        drop(replay);
        drop(held);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_kept_reply_lasts_only_while_the_content_it_was_named_with_does() {
        use dashboard_base::test_fixtures::{source, source_with_content};
        use dashboard_query::{identity::Holders, timeline::{Join, Stretch, Timeline}};
        let root = std::env::temp_dir().join(format!("shared-groups-generation-{}", uuid::Uuid::new_v4()));
        let (held, context) = source_with_content((100..=110).map(|hour| source(1, hour, Some(1))).collect(), &root);
        let replay = held.replay();
        let timeline = Timeline { id: 1, first: 100, last: 110, stretches: vec![Stretch { run: 0, first: 100, last: 110, join: Join::Start }], unheld: Vec::new() };
        let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(Holders::new(Default::default(), Vec::new())) };
        let options = context.content.clone().expect("the fixture names its content");
        let named = shared(&held, &query, &context, &AtomicBool::new(false)).unwrap();
        let file = root.join("user/settings.txt");
        let before = held.content(&options.options).unwrap();
        before.layers.with_file_witness(&file, || std::fs::read(&file)).unwrap();
        std::fs::write(&file, "language=l_english force_pow2_textures=yes").unwrap();
        assert!(before.layers.validate_generation().is_err(), "the edit is seen as the folder watcher would report it");
        let after = held.content(&options.options).unwrap();
        assert!(!Arc::ptr_eq(&before, &after), "a file the content read was edited: the content is resolved again");
        let renamed = shared(&held, &query, &context, &AtomicBool::new(false)).unwrap();
        assert!(!Arc::ptr_eq(&named, &renamed), "a kept reply is not used once its content generation is replaced");
        assert!(Arc::ptr_eq(&renamed, &shared(&held, &query, &context, &AtomicBool::new(true)).unwrap()), "the reply made on the new content is kept for it");
        drop((before, after));
        drop(query);
        drop(replay);
        drop(held);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sparse_stretches_coalesce_group_runs_independent_of_stretch_count() {
        let mut gaps = Vec::new();
        let mut members = Vec::new();
        let mut wars = Vec::new();
        for i in 0..200i32 {
            let hour = i * 10;
            if i > 0 {
                gaps.push(HourSpan { first_hour: hour - 9, last_hour: hour - 1 });
            }
            for tag in ["GER", "ITA", "JAP"] {
                members.push(MembershipRun { tag: tag.to_owned(), first_hour: hour, last_hour: hour });
            }
            wars.push(WarPairRun { first: "ENG".to_owned(), second: "GER".to_owned(), first_hour: hour, last_hour: hour });
        }
        settle(&mut gaps);
        let coalesced_members = joined_across(
            members,
            &gaps,
            |run| run.tag.clone(),
            |run| (run.first_hour, run.last_hour),
            |run, last| run.last_hour = last,
            |a, b| a.tag == b.tag,
        );
        assert_eq!(coalesced_members.len(), 3);
        for run in &coalesced_members {
            assert_eq!((run.first_hour, run.last_hour), (0, 1990));
        }
        let coalesced_wars = joined_across(
            wars,
            &gaps,
            |run| (run.first.clone(), run.second.clone()),
            |run| (run.first_hour, run.last_hour),
            |run, last| run.last_hour = last,
            |a, b| (&a.first, &a.second) == (&b.first, &b.second),
        );
        assert_eq!(coalesced_wars.len(), 1);
        assert_eq!((coalesced_wars[0].first_hour, coalesced_wars[0].last_hour), (0, 1990));
    }

    #[test]
    fn a_backdated_war_event_is_attributed_to_the_slot_holder_at_its_own_hour_or_to_no_one() {
        let holders = HashMap::from([(7, vec![("AAA".to_owned(), 0, 99), ("BBB".to_owned(), 100, 200)]), (9, vec![("CCC".to_owned(), 120, 200)])]);
        assert_eq!(event_tag(&holders, 7, 50), Some("AAA"));
        assert_eq!(event_tag(&holders, 7, 150), Some("BBB"));
        assert_eq!(event_tag(&holders, 9, 60), None, "the later holder of the slot is never named for an earlier event");
        assert_eq!(event_tag(&holders, TagHolderRow::NONE, 150), None);
    }

    #[test]
    fn group_runs_do_not_coalesce_across_observed_hour_of_absence() {
        let gaps = vec![HourSpan { first_hour: 1, last_hour: 9 }, HourSpan { first_hour: 11, last_hour: 19 }];
        let members = vec![
            MembershipRun { tag: "GER".to_owned(), first_hour: 0, last_hour: 0 },
            MembershipRun { tag: "GER".to_owned(), first_hour: 20, last_hour: 20 },
        ];
        let coalesced = joined_across(
            members,
            &gaps,
            |run| run.tag.clone(),
            |run| (run.first_hour, run.last_hour),
            |run, last| run.last_hour = last,
            |a, b| a.tag == b.tag,
        );
        assert_eq!(coalesced.len(), 2);
    }
}

#[cfg(test)]
mod faction_name_tests {
    use super::*;

    #[test]
    fn unnamed_faction_at_hour_does_not_fall_back_to_later_name() {
        let faction = FactionGroup {
            id: "10".into(),
            name: Some("New League".into()),
            name_reason: None,
            names: vec![
                FactionNameRun { first_hour: 0, last_hour: 71, name: None, reason: Some("Faction name was not recorded".into()), token: None },
                FactionNameRun { first_hour: 72, last_hour: 200, name: Some("New League".into()), reason: None, token: None },
            ],
            recorded_name: "New League".into(),
            first_hour: 0,
            last_hour: 200,
            members: Vec::new(),
            leaders: Vec::new(),
            incomplete: Vec::new(),
        };
        assert_eq!(faction_name_at(&faction, 47), (None, Some("Faction name was not recorded".into())));
        assert_eq!(faction_name_at(&faction, 72), (Some("New League".into()), None));
    }

    #[test]
    fn faction_names_never_coalesce_across_observed_absence_or_unnamed_run() {
        let gaps = vec![HourSpan { first_hour: 1, last_hour: 9 }, HourSpan { first_hour: 11, last_hour: 19 }];
        let runs = vec![
            FactionNameRun { first_hour: 0, last_hour: 0, name: Some("Axis".into()), reason: None, token: None },
            FactionNameRun { first_hour: 20, last_hour: 20, name: Some("Axis".into()), reason: None, token: None },
            FactionNameRun { first_hour: 21, last_hour: 25, name: None, reason: Some("Faction name was not recorded".into()), token: None },
            FactionNameRun { first_hour: 26, last_hour: 30, name: Some("Axis".into()), reason: None, token: None },
        ];
        let coalesced = joined_across(
            runs,
            &gaps,
            |_| 0u8,
            |run| (run.first_hour, run.last_hour),
            |run, last| run.last_hour = last,
            |a, b| (&a.name, &a.reason) == (&b.name, &b.reason),
        );
        assert_eq!(coalesced.len(), 4);
        let faction = FactionGroup {
            id: "1".into(),
            name: Some("Axis".into()),
            name_reason: None,
            names: coalesced,
            recorded_name: "Axis".into(),
            first_hour: 0,
            last_hour: 30,
            members: Vec::new(),
            leaders: Vec::new(),
            incomplete: Vec::new(),
        };
        assert_eq!(faction_name_at(&faction, 10), (None, Some("Faction name was not recorded at this hour".into())));
        assert_eq!(faction_name_at(&faction, 22), (None, Some("Faction name was not recorded".into())));
        assert_eq!(faction_name_at(&faction, 26), (Some("Axis".into()), None));
    }
}
