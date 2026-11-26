use super::ideas::{IdeaHistory, IdeaHistoryParams, IdeaHistoryReply, HoldingStart};
use super::national_focus::{FocusReply, NationalFocus};
use super::party_history::{self, RulingParties};
use dashboard_base::Source;
use dashboard_base::products::{Country, CountryParams, LoadContext, Product, check_reply_budget};
use dashboard_base::span::HourSpan;
use dashboard_query::identity::{Holder, Holders, Key, TAG_UNLINKED};
use dashboard_text::division_history::date;
use replay_data::temperature::EPOCH;
use dashboard_world::products::groups::{self, CountryGroupReply};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::atomic::AtomicBool;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StabilityMarkersParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    pub focus: bool,
    pub government: bool,
    pub wars: bool,
    pub factions: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StabilityMarkerKind {
    Focus,
    Idea,
    Party,
    War,
    Faction,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StabilityMarkerVerb {
    Began,
    Capitulated,
    Founded,
    Joined,
    Left,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityMarker {
    pub id: String,
    pub hour: i32,
    pub kind: StabilityMarkerKind,
    pub title: String,
    pub verb: Option<StabilityMarkerVerb>,
    pub other: Option<String>,
    pub faction: Option<String>,
    pub sprite: Option<String>,
    pub alert: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityMarkersReply {
    pub hour: i32,
    pub next_change: Option<i32>,
    pub markers: Vec<StabilityMarker>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StabilityMarkersReply);

const FOCUS_UNNAMED: &str = "Focus name unavailable";
const IDEA_UNNAMED: &str = "Idea name unavailable";
const NAME_NOT_RECORDED: &str = "Name not recorded";
const HOLDER_UNNAMED: &str = "holder not recorded";
const IDEOLOGY_UNNAMED: &str = "Ideology name unavailable";
const FACTION_UNNAMED: &str = "Unnamed faction";

fn within(spans: &[HourSpan], hour: i32) -> bool {
    spans.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour)
}

struct Markers {
    hour: i32,
    shown: Vec<StabilityMarker>,
    next: Option<i32>,
}

impl Markers {
    fn push(&mut self, marker: StabilityMarker) {
        if marker.hour <= self.hour {
            self.shown.push(marker);
        } else {
            self.next = Some(self.next.map_or(marker.hour, |next| next.min(marker.hour)));
        }
    }

    fn later(&mut self, hour: Option<i32>) {
        if let Some(hour) = hour.filter(|hour| *hour > self.hour) {
            self.next = Some(self.next.map_or(hour, |next| next.min(hour)));
        }
    }
}

fn marker(id: String, hour: i32, kind: StabilityMarkerKind, title: String) -> StabilityMarker {
    StabilityMarker { id, hour, kind, title, verb: None, other: None, faction: None, sprite: None, alert: false }
}

fn focus_markers(out: &mut Markers, reply: &FocusReply) {
    let names: HashMap<&str, &str> = reply.static_text.iter().flatten().filter_map(|art| Some((art.token.as_str(), art.name.as_deref()?))).collect();
    let art: HashMap<&str, &dashboard_core::dto::Artwork> = reply.artwork.iter().map(|art| (art.token.as_str(), art)).collect();
    let mut seen = BTreeSet::new();
    for segment in &reply.history.segments {
        for entry in &segment.order.completed {
            let Some(hour) = entry.completed_by_hour else { continue };
            if !seen.insert((entry.token.as_str(), entry.ordinal, hour)) {
                continue;
            }
            let look = art.get(entry.token.as_str());
            let name = names.get(entry.token.as_str()).copied().or_else(|| look.and_then(|art| art.name.as_deref())).unwrap_or(FOCUS_UNNAMED);
            let mut made = marker(format!("focus:{}:{hour}", entry.token), hour, StabilityMarkerKind::Focus, format!("Focus: {name}"));
            made.sprite = look.and_then(|art| art.sprite.clone());
            out.push(made);
        }
    }
}

fn idea_markers(out: &mut Markers, reply: &IdeaHistoryReply) {
    let laws: BTreeSet<&str> = reply.categories.iter().flat_map(|category| category.slots.iter().flat_map(|slot| slot.laws.iter().map(String::as_str))).collect();
    let seats: BTreeSet<&str> = reply.categories.iter().flat_map(|category| category.slots.iter().filter(|slot| slot.character).map(|slot| slot.token.as_str())).collect();
    for holding in &reply.holdings {
        if holding.start != HoldingStart::Seen {
            continue;
        }
        let Some(look) = reply.looks.get(holding.look as usize) else { continue };
        let (info, art) = (&look.info, &look.artwork);
        let slot = holding.slot.as_deref().or(info.slot.as_deref());
        let idea_name = || art.name.clone().unwrap_or_else(|| if info.name_missing.is_some() { NAME_NOT_RECORDED } else { IDEA_UNNAMED }.to_owned());
        let hour = holding.first_hour;
        let made = if info.law || laws.contains(holding.token.as_str()) {
            marker(format!("law:{}:{hour}", holding.token), hour, StabilityMarkerKind::Idea, format!("Law: {}", idea_name()))
        } else if slot.is_some_and(|slot| seats.contains(slot)) {
            let label = if holding.character.is_some() { art.name.clone().unwrap_or_else(|| HOLDER_UNNAMED.to_owned()) } else { idea_name() };
            marker(format!("adv:{}:{hour}", holding.token), hour, StabilityMarkerKind::Idea, format!("Advisor: {label}"))
        } else {
            continue;
        };
        out.push(StabilityMarker { sprite: art.sprite.clone(), ..made });
    }
    out.later(reply.next_change);
}

fn party_markers(out: &mut Markers, reply: &RulingParties) {
    let names: HashMap<&str, &str> = reply.ideologies.iter().filter_map(|ideology| Some((ideology.token.as_str(), ideology.name.as_deref()?))).collect();
    let name = |token: &str| names.get(token).copied().unwrap_or(IDEOLOGY_UNNAMED).to_owned();
    for pair in reply.runs.windows(2) {
        let (previous, run) = (&pair[0], &pair[1]);
        if run.starts_segment || previous.last_hour + 1 != run.first_hour || previous.token == run.token || within(&reply.unobserved, run.first_hour - 1) {
            continue;
        }
        let title = format!("{} took power from {}", name(&run.token), name(&previous.token));
        out.push(marker(format!("party:{}:{title}", run.first_hour), run.first_hour, StabilityMarkerKind::Party, title));
    }
}

fn war_markers(out: &mut Markers, holds: &dyn Fn(&str, i32) -> bool, groups: &CountryGroupReply, hidden: &[HourSpan]) {
    for war in &groups.war_starts {
        let other = if holds(&war.attacking, war.hour) {
            &war.defending
        } else if holds(&war.defending, war.hour) {
            &war.attacking
        } else {
            continue;
        };
        let mut made = marker(format!("war:{other}:{}", war.hour), war.hour, StabilityMarkerKind::War, format!("War with {other} began"));
        made.verb = Some(StabilityMarkerVerb::Began);
        made.other = Some(other.clone());
        made.alert = true;
        if !within(hidden, made.hour) {
            out.push(made);
        }
    }
    let mut fought: BTreeMap<&str, Vec<(&str, &groups::WarPairRun)>> = BTreeMap::new();
    for war in &groups.wars {
        fought.entry(war.first.as_str()).or_default().push((war.second.as_str(), war));
        fought.entry(war.second.as_str()).or_default().push((war.first.as_str(), war));
    }
    for event in &groups.capitulations {
        if holds(&event.tag, event.hour) {
            let mut made = marker(format!("cap:{}:{}", event.tag, event.hour), event.hour, StabilityMarkerKind::War, "Capitulated".to_owned());
            made.alert = true;
            if !within(hidden, made.hour) {
                out.push(made);
            }
            continue;
        }
        let ends_now = |war: &groups::WarPairRun| event.hour == war.last_hour + 1 && !within(&groups.unobserved.wars, event.hour);
        let enemy = fought.get(event.tag.as_str()).is_some_and(|wars| {
            wars.iter().any(|&(side, war)| war.first_hour <= event.hour && (event.hour <= war.last_hour || ends_now(war)) && holds(side, event.hour.min(war.last_hour)))
        });
        if enemy {
            let mut made = marker(format!("cap:{}:{}", event.tag, event.hour), event.hour, StabilityMarkerKind::War, format!("{} capitulated", event.tag));
            made.verb = Some(StabilityMarkerVerb::Capitulated);
            made.other = Some(event.tag.clone());
            made.alert = true;
            if !within(hidden, made.hour) {
                out.push(made);
            }
        }
    }
}

fn faction_markers(out: &mut Markers, holds: &dyn Fn(&str, i32) -> bool, groups: &CountryGroupReply, branch: (i32, i32), hidden: &[HourSpan]) -> Vec<String> {
    let mut notes = Vec::new();
    for faction in &groups.factions {
        let roster: Vec<HourSpan> = groups.unobserved.factions.iter().chain(&faction.incomplete).cloned().collect();
        let unknown: Vec<HourSpan> = roster.iter().chain(hidden).cloned().collect();
        let founders = faction.members.iter().filter(|member| member.first_hour == faction.first_hour).count();
        let sole = founders == 1 && !within(&unknown, faction.first_hour);
        let name = faction.name.as_deref().unwrap_or(FACTION_UNNAMED);
        let member_at = |hour: i32| faction.members.iter().any(|other| other.first_hour <= hour && hour <= other.last_hour && holds(&other.tag, hour));
        for run in &faction.members {
            if run.first_hour > branch.0
                && within(&roster, run.first_hour - 1)
                && !within(hidden, run.first_hour - 1)
                && holds(&run.tag, run.first_hour)
                && (within(&groups.unobserved.factions, run.first_hour - 1) || !member_at(run.first_hour - 1))
            {
                if run.first_hour <= out.hour {
                    let dated = groups::faction_name_at(faction, run.first_hour).0.unwrap_or_else(|| FACTION_UNNAMED.to_owned());
                    notes.push(format!("In {dated} by {}: when it joined is not known, as membership was not observed the hour before", when(run.first_hour)));
                } else {
                    out.later(Some(run.first_hour));
                }
            }
            if run.first_hour > branch.0 && !within(&unknown, run.first_hour - 1) && holds(&run.tag, run.first_hour) && !member_at(run.first_hour - 1) {
                let verb = if run.first_hour == faction.first_hour && sole { StabilityMarkerVerb::Founded } else { StabilityMarkerVerb::Joined };
                let word = if verb == StabilityMarkerVerb::Founded { "Founded" } else { "Joined" };
                let mut made = marker(format!("fac-j:{}:{}", faction.id, run.first_hour), run.first_hour, StabilityMarkerKind::Faction, format!("{word} {name}"));
                made.verb = Some(verb);
                made.faction = Some(faction.id.clone());
                out.push(made);
            }
            if run.last_hour < branch.1 && !within(&unknown, run.last_hour + 1) && holds(&run.tag, run.last_hour) && !member_at(run.last_hour + 1) {
                let hour = run.last_hour + 1;
                let mut made = marker(format!("fac-l:{}:{hour}", faction.id), hour, StabilityMarkerKind::Faction, format!("Left {name}"));
                made.verb = Some(StabilityMarkerVerb::Left);
                made.faction = Some(faction.id.clone());
                out.push(made);
            }
        }
    }
    notes
}

fn when(hour: i32) -> String {
    format!("{} {:02}:00", date(hour), (i64::from(hour) - i64::from(EPOCH)).rem_euclid(24))
}

fn hidden(wars: bool, factions: bool) -> &'static str {
    match (wars, factions) {
        (true, true) => "War and faction markers",
        (true, false) => "War markers",
        _ => "Faction markers",
    }
}

fn holder_gaps<'h>(holders: &'h Holders, country: &Country, from: i32) -> Vec<(i32, i32, Option<&'h str>)> {
    let runs = country.runs(holders);
    let mut spans: Vec<(i32, i32, Option<&str>)> = holders.unknown.iter().map(|(first, last, reason)| (*first, *last, Some(reason.as_str()))).collect();
    match country {
        Country::Tag(_) => spans.extend(holders.within(country, from, i32::MAX).into_iter().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last, None))),
        Country::Slot(_) => {
            for &(first, last) in holders.unlinked() {
                let mut at = first;
                for run in runs[runs.partition_point(|run| run.last < first)..].iter().take_while(|run| run.first <= last) {
                    if run.first > at {
                        spans.push((at, run.first - 1, None));
                    }
                    at = at.max(run.last.saturating_add(1));
                }
                if at <= last {
                    spans.push((at, last, None));
                }
            }
        }
    }
    spans.sort_unstable_by_key(|&(first, last, _)| (first, last));
    spans
}

fn holder_notes(out: &mut Markers, holders: &Holders, country: &Country, (from, end): (i32, i32), kinds: &str) -> Vec<String> {
    let runs = country.runs(holders);
    let said = |hour: i32, reason: &str| match country {
        Country::Tag(tag) => format!("Who holds {tag} is not known: {reason}"),
        Country::Slot(slot) => {
            let before = runs[..runs.partition_point(|run| run.first < hour)].last().and_then(|run| holders.slot_tag(*slot, run.last));
            match before {
                Some(tag) => format!("The chosen country's tag is not known: {reason}; it was last recorded as {tag}"),
                None => format!("The chosen country's tag is not known: {reason}"),
            }
        }
    };
    let spans = holder_gaps(holders, country, from);
    let mut notes = Vec::new();
    for (first, last, reason) in spans {
        if last < from {
            continue;
        }
        let first = first.max(from);
        if first > out.hour {
            out.later(Some(first));
            break;
        }
        let reason = said(first, reason.unwrap_or(TAG_UNLINKED));
        if last >= out.hour {
            if last < end {
                out.later(Some(last + 1));
            }
            notes.push(format!("{kinds} are not shown from {}: {reason}", when(first)));
        } else {
            notes.push(format!("{kinds} are not shown from {} to {}: {reason}", when(first), when(last)));
        }
    }
    notes
}

fn unattributed_note(out: &mut Markers, hours: &[i32]) -> Option<String> {
    let seen = hours.partition_point(|&at| at <= out.hour);
    out.later(hours.get(seen).copied());
    let shown = &hours[..seen];
    let (first, last) = (*shown.first()?, *shown.last()?);
    let span = if first == last { format!("at {}", when(first)) } else { format!("from {} to {}", when(first), when(last)) };
    Some(format!("{} war starts or capitulations {span} name a country no link resolved: any may be this country's, and none is marked", shown.len()))
}

pub struct StabilityMarkers;

impl Product for StabilityMarkers {
    const KEY: &'static str = "stability-markers";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StabilityMarkersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, focus, government, wars, factions];
    type Reply = StabilityMarkersReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StabilityMarkersParams::decl(), StabilityMarkersReply::decl(), StabilityMarker::decl(), StabilityMarkerKind::decl(), StabilityMarkerVerb::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryParams { branch_id: params.branch_id.clone(), country: params.country.clone() }.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let country = CountryParams { branch_id: params.branch_id.clone(), country: params.country.clone() };
        let query = dashboard_base::queries::query(source, country.branch())?;
        let hour = params.hour.min(query.timeline.last);
        let mut out = Markers { hour, shown: Vec::new(), next: None };
        let mut warnings = Vec::new();
        if params.focus {
            let reply = NationalFocus::load(source, &country, context, cancelled)?;
            focus_markers(&mut out, &reply);
            warnings.extend(reply.warnings);
        }
        if params.government {
            let ideas = IdeaHistory::load(source, &IdeaHistoryParams { branch_id: params.branch_id.clone(), country: params.country.clone(), hour }, context, cancelled)?;
            idea_markers(&mut out, &ideas);
            warnings.extend(ideas.warnings);
            let parties = party_history::ruling(source, &query, &country, context, cancelled)?;
            party_markers(&mut out, &parties);
            warnings.extend(parties.warnings);
        }
        if params.wars || params.factions {
            let groups = groups::shared(source, &query, context, cancelled)?;
            let holds = |tag: &str, at: i32| query.holders.tag_at(&params.country, at) == Some(tag);
            let unheld: Vec<HourSpan> = holder_gaps(&query.holders, &params.country, query.timeline.first).into_iter().map(|(first_hour, last_hour, _)| HourSpan { first_hour, last_hour }).collect();
            if params.wars {
                war_markers(&mut out, &holds, &groups, &unheld);
            }
            let joins = if params.factions { faction_markers(&mut out, &holds, &groups, (query.timeline.first, query.timeline.last), &unheld) } else { Vec::new() };
            if params.wars {
                warnings.extend(unattributed_note(&mut out, &groups.unobserved.unattributed));
            }
            warnings.extend(holder_notes(&mut out, &query.holders, &params.country, (query.timeline.first, query.timeline.last), hidden(params.wars, params.factions)));
            warnings.extend(joins);
        }
        out.shown.sort_by(|a, b| a.hour.cmp(&b.hour).then_with(|| a.id.cmp(&b.id)));
        warnings.sort();
        warnings.dedup();
        let reply = StabilityMarkersReply { hour, next_change: out.next.filter(|&next| next <= query.timeline.last), markers: out.shown, warnings };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
