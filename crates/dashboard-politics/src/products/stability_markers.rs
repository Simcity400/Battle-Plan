use super::ideas::{IdeaHistory, IdeaHistoryParams, IdeaHistoryReply, HoldingStart};
use super::national_focus::{FocusReply, NationalFocus};
use super::party_history::{self, RulingParties};
use dashboard_base::Source;
use dashboard_base::products::{Country, CountryParams, LoadContext, Product, check_reply_budget};
use dashboard_base::span::HourSpan;
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

fn war_markers(out: &mut Markers, holds: &dyn Fn(&str, i32) -> bool, groups: &CountryGroupReply) {
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
        out.push(made);
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
            out.push(made);
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
            out.push(made);
        }
    }
}

fn faction_markers(out: &mut Markers, holds: &dyn Fn(&str, i32) -> bool, groups: &CountryGroupReply, branch: (i32, i32)) {
    for faction in &groups.factions {
        let unknown: Vec<HourSpan> = groups.unobserved.factions.iter().chain(&faction.incomplete).cloned().collect();
        let founders = faction.members.iter().filter(|member| member.first_hour == faction.first_hour).count();
        let sole = founders == 1 && !within(&unknown, faction.first_hour);
        let name = faction.name.as_deref().unwrap_or(FACTION_UNNAMED);
        let member_at = |hour: i32| faction.members.iter().any(|other| other.first_hour <= hour && hour <= other.last_hour && holds(&other.tag, hour));
        for run in &faction.members {
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
            if params.wars {
                war_markers(&mut out, &holds, &groups);
            }
            if params.factions {
                faction_markers(&mut out, &holds, &groups, (query.timeline.first, query.timeline.last));
            }
        }
        out.shown.sort_by(|a, b| a.hour.cmp(&b.hour).then_with(|| a.id.cmp(&b.id)));
        warnings.sort();
        warnings.dedup();
        let reply = StabilityMarkersReply { hour, next_change: out.next, markers: out.shown, warnings };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
