use super::groups::{Capitulation, CountryGroupReply, CountryIdentityHistory, FactionGroup, FactionNameRun, IdentityChange, MembershipRun, UnobservedHours, WarPairRun, WarStart};
use super::identities::CountryIdentityRun;
use super::subjects::{LevelStep, SubjectReply, SubjectRun};
use dashboard_base::products::{BranchParams, LoadContext, Product};
use dashboard_base::{Source, span::HourSpan};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

trait Run: Clone {
    fn span(&self) -> (i32, i32);
    fn end_at(&mut self, last: i32);
}

macro_rules! run {
    ($($ty:ty),*) => {$(
        impl Run for $ty {
            fn span(&self) -> (i32, i32) {
                (self.first_hour, self.last_hour)
            }
            fn end_at(&mut self, last: i32) {
                self.last_hour = last;
            }
        }
    )*};
}

run!(MembershipRun, FactionNameRun, WarPairRun, CountryIdentityRun, HourSpan, LevelStep);

fn last_seen(last: i32, hour: i32, gaps: &[HourSpan]) -> i32 {
    let at = gaps.partition_point(|gap| gap.last_hour < hour);
    match gaps.get(at) {
        Some(gap) if gap.first_hour <= hour && last >= gap.first_hour => last.min(gap.first_hour - 1),
        _ => last.min(hour),
    }
}

fn cut<T: Run>(runs: &[T], hour: i32, gaps: &[HourSpan]) -> Vec<T> {
    runs.iter()
        .filter(|run| run.span().0 <= hour)
        .filter_map(|run| {
            let (first, last) = run.span();
            let last = last_seen(last, hour, gaps);
            (last >= first).then(|| {
                let mut kept = run.clone();
                kept.end_at(last);
                kept
            })
        })
        .collect()
}

#[derive(Default)]
struct Next(Option<i32>);

impl Next {
    fn after(&mut self, hour: i32, at: i32) {
        if at > hour && self.0.is_none_or(|next| at < next) {
            self.0 = Some(at);
        }
    }

    fn runs<T: Run>(&mut self, hour: i32, runs: &[T]) {
        for run in runs {
            let (first, last) = run.span();
            self.after(hour, first);
            self.after(hour, last.saturating_add(1));
        }
    }
}

fn faction_at(faction: &FactionGroup, hour: i32, gaps: &[HourSpan]) -> FactionGroup {
    let last = last_seen(faction.last_hour, hour, gaps).max(faction.first_hour);
    let names = cut(&faction.names, hour, &[]);
    let named = names.iter().rev().find(|run| run.first_hour <= last);
    FactionGroup {
        id: faction.id.clone(),
        name: named.and_then(|run| run.name.clone()),
        name_reason: named.map_or_else(|| Some("Faction name was not recorded by this hour".to_owned()), |run| run.reason.clone()),
        names,
        recorded_name: faction.recorded_name.clone(),
        first_hour: faction.first_hour,
        last_hour: last,
        members: cut(&faction.members, hour, gaps),
        leaders: cut(&faction.leaders, hour, gaps),
        incomplete: cut(&faction.incomplete, hour, &[]),
    }
}

pub(crate) fn groups_at(reply: &CountryGroupReply, hour: i32) -> (CountryGroupReply, Option<i32>) {
    let mut next = Next::default();
    let gaps = &reply.unobserved;
    next.runs(hour, &reply.humans);
    next.runs(hour, &reply.wars);
    for faction in &reply.factions {
        next.after(hour, faction.first_hour);
        next.after(hour, faction.last_hour.saturating_add(1));
        next.runs(hour, &faction.names);
        next.runs(hour, &faction.members);
        next.runs(hour, &faction.leaders);
        next.runs(hour, &faction.incomplete);
    }
    for history in &reply.country_identities {
        next.runs(hour, &history.identities);
    }
    for at in reply.war_starts.iter().map(|start| start.hour).chain(reply.capitulations.iter().map(|item| item.hour)).chain(reply.identity_changes.iter().map(|item| item.hour)) {
        next.after(hour, at);
    }
    for spans in [&gaps.humans, &gaps.factions, &gaps.wars] {
        next.runs(hour, spans);
    }
    let cut_reply = CountryGroupReply {
        branch_id: reply.branch_id.clone(),
        humans: cut(&reply.humans, hour, &gaps.humans),
        factions: reply.factions.iter().filter(|faction| faction.first_hour <= hour).map(|faction| faction_at(faction, hour, &gaps.factions)).collect(),
        wars: cut(&reply.wars, hour, &gaps.wars),
        war_starts: reply.war_starts.iter().filter(|start| start.hour <= hour).cloned().collect::<Vec<WarStart>>(),
        capitulations: reply.capitulations.iter().filter(|item| item.hour <= hour).cloned().collect::<Vec<Capitulation>>(),
        identity_changes: reply.identity_changes.iter().filter(|item| item.hour <= hour).cloned().collect::<Vec<IdentityChange>>(),
        country_identities: reply
            .country_identities
            .iter()
            .map(|history| CountryIdentityHistory { tag: history.tag.clone(), identities: cut(&history.identities, hour, &[]) })
            .filter(|history| !history.identities.is_empty())
            .collect(),
        unobserved: UnobservedHours { humans: cut(&gaps.humans, hour, &[]), factions: cut(&gaps.factions, hour, &[]), wars: cut(&gaps.wars, hour, &[]) },
        warnings: reply.warnings.clone(),
    };
    (cut_reply, next.0)
}

pub(crate) fn subjects_at(reply: &SubjectReply, hour: i32) -> (SubjectReply, Option<i32>) {
    let mut next = Next::default();
    next.runs(hour, &reply.unobserved);
    let mut runs = Vec::new();
    for run in &reply.runs {
        next.after(hour, run.first_hour);
        next.after(hour, run.last_hour.saturating_add(1));
        next.runs(hour, &run.steps);
        if run.first_hour > hour {
            continue;
        }
        let last = last_seen(run.last_hour, hour, &reply.unobserved);
        if last < run.first_hour {
            continue;
        }
        runs.push(SubjectRun { last_hour: last, steps: cut(&run.steps, last, &[]), ..run.clone() });
    }
    let named: BTreeSet<&str> = runs.iter().flat_map(|run| run.steps.iter().filter_map(|step| step.level.as_deref())).collect();
    let cut_reply = SubjectReply {
        branch_id: reply.branch_id.clone(),
        levels: reply.levels.iter().filter(|level| named.contains(level.token.as_str())).cloned().collect(),
        runs,
        unobserved: cut(&reply.unobserved, hour, &[]),
        warnings: reply.warnings.clone(),
    };
    (cut_reply, next.0)
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryAtParams {
    pub branch_id: String,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryGroupsAtReply {
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which a run, an event or an unread stretch starts or ends; null when none does."]
    pub next_change: Option<i32>,
    #[doc = " The country groups recorded by `hour`: every run cut at it, or at the last hour read before an unread stretch it falls in."]
    pub groups: CountryGroupReply,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SubjectsAtReply {
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which a relation, a level or an unread stretch starts or ends; null when none does."]
    pub next_change: Option<i32>,
    #[doc = " The subject relations recorded by `hour`, cut as `country-groups-at` cuts its runs."]
    pub subjects: SubjectReply,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryGroupsAtReply, SubjectsAtReply);

fn branch(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

pub struct CountryGroupsAt;

impl Product for CountryGroupsAt {
    const KEY: &'static str = "country-groups-at";
    const CONTENT_DEPENDENT: bool = true;
    type Params = HistoryAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read];
    type Reply = CountryGroupsAtReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![HistoryAtParams::decl(), CountryGroupsAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let whole = super::groups::shared(source, &query, context, cancelled)?;
        let (groups, next_change) = groups_at(&whole, params.hour.min(query.timeline.last));
        Ok(CountryGroupsAtReply { hour: params.hour.min(query.timeline.last), next_change, groups, warnings: Vec::new() })
    }
}

pub struct SubjectsAt;

impl Product for SubjectsAt {
    const KEY: &'static str = "subjects-at";
    const CONTENT_DEPENDENT: bool = true;
    type Params = HistoryAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read];
    type Reply = SubjectsAtReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![SubjectsAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let whole = super::subjects::Subjects::load(source, &BranchParams { branch_id: params.branch_id.clone() }, context, cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let hour = params.hour.min(query.timeline.last);
        let (subjects, next_change) = subjects_at(&whole, hour);
        Ok(SubjectsAtReply { hour, next_change, subjects, warnings: Vec::new() })
    }
}

#[cfg(test)]
mod tests;
