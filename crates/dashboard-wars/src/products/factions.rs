
#[path = "factions/tiles.rs"]
pub mod tiles;

use dashboard_base::products::{Country, LoadContext, validate_country};
use dashboard_base::{Source, products::at_hour::{At, AtHour, Clock, Owner, Read, ReadsAt}};
use dashboard_world::products::{group_members::GroupsAt, scalars::factory_channel_value, territory::{slot_tags, territory_value}};
use dashboard_declare::{Measure, Series};
use dashboard_query::{
    coverage::{Witness, witnessed},
    identity::Holder,
    series::{MissingSpan, ranked::{GroupMembership, GroupSelection}},
};
use replay_data::rows::{CountryFactoriesRow, GroupSpanRow as Span, TableId, TerritoryTotalRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const KINDS: [&str; 2] = ["territory", "country-factories"];

fn measure_of(kind: &str, measure: &str) -> Result<(&'static Series, &'static Measure), &'static str> {
    if !KINDS.contains(&kind) {
        return Err("Faction totals sum territory or factory counts");
    }
    let series = dashboard_declare::series(kind).ok_or("Undeclared faction series")?;
    Ok((series, series.measure(measure).ok_or("Unknown faction measure")?))
}

struct Rosters {
    members: BTreeMap<u64, BTreeSet<u32>>,
    incomplete: BTreeSet<u64>,
    unread: bool,
}

fn rosters(frame: &DecodedFrame) -> Result<Rosters, String> {
    let spans = frame.complete::<tables::GroupSpans>().map_err(|error| error.to_string())?;
    let groups = frame.complete::<tables::Groups>().map_err(|error| error.to_string())?;
    let numbers: HashMap<u32, u64> = groups.iter().map(|group| (group.number, group.faction)).collect();
    let mut out = Rosters { members: BTreeMap::new(), incomplete: BTreeSet::new(), unread: false };
    for span in spans {
        match span.kind {
            Span::UNOBSERVED if span.group == Span::FACTIONS => out.unread = true,
            Span::MEMBER if span.group == Span::HUMANS => {}
            Span::MEMBER | Span::PRESENT | Span::INCOMPLETE => {
                let Some(&id) = numbers.get(&span.group) else { continue };
                let members = out.members.entry(id).or_default();
                match span.kind {
                    Span::MEMBER => {
                        members.insert(span.value);
                    }
                    Span::INCOMPLETE => {
                        out.incomplete.insert(id);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

pub const GROUP_LINE: &str = "group";

fn merged(mut spans: Vec<(i32, i32)>, from: i32, to: i32) -> Vec<(i32, i32)> {
    spans.retain_mut(|span| {
        *span = (span.0.max(from), span.1.min(to));
        span.0 <= span.1
    });
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if i64::from(held.1) + 1 >= i64::from(span.0) => held.1 = held.1.max(span.1),
            _ => out.push(span),
        }
    }
    out
}

fn within(missing: &[MissingSpan], spans: &[(i32, i32)]) -> Vec<MissingSpan> {
    let mut out = Vec::new();
    for gap in missing {
        for &(first, last) in spans {
            let (a, b) = (gap.first_hour.max(first), gap.last_hour.min(last));
            if a <= b {
                out.push(MissingSpan { first_hour: a, last_hour: b, reason: gap.reason.clone() });
            }
        }
    }
    out
}

pub struct FactionTotalsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionTotalsAtParams {
    pub branch_id: String,
    pub kind: String,
    pub measure: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub members: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub membership: Option<GroupMembership>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalRow {
    pub id: String,
    pub members: Vec<String>,
    pub member_countries: Vec<Country>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub total: Option<i64>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionTotalsAtReply {
    pub hour: i32,
    pub kind: String,
    pub measure: String,
    pub rows: Vec<FactionTotalRow>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactionTotalsAtReply);

enum MeasureTable<'f> {
    Territory(HashMap<u32, &'f TerritoryTotalRow>),
    Factories(HashMap<u32, &'f CountryFactoriesRow>),
}

impl<'f> MeasureTable<'f> {
    fn read(frame: &'f DecodedFrame, kind: &str) -> Result<Self, String> {
        if kind == "territory" {
            let rows = frame.complete::<tables::TerritoryTotals>().map_err(|error| format!("Territory totals are not known at this hour: {error}"))?;
            Ok(Self::Territory(rows.iter().map(|row| (row.slot, row)).collect()))
        } else {
            let rows = frame.complete::<tables::CountryFactories>().map_err(|error| format!("Factories are not known at this hour: {error}"))?;
            Ok(Self::Factories(rows.iter().map(|row| (row.country, row)).collect()))
        }
    }

    fn value(&self, slot: u32, id: u32, measure: &str) -> Option<i64> {
        match self {
            Self::Territory(rows) => rows.get(&slot).and_then(|row| territory_value(row, measure)),
            Self::Factories(rows) => rows.get(&id).and_then(|row| factory_channel_value(row, measure)),
        }
    }
}

fn witnessed_value(frame: &DecodedFrame, kind: &str, slot: u32, id: u32, measure: &str, tag: &str) -> Result<i64, String> {
    if kind == "territory" {
        match witnessed::<tables::TerritoryTotals>(frame, u128::from(slot)) {
            Witness::Row(row) => territory_value(row, measure).ok_or_else(|| format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::Absent => Err(format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::NotRecorded(reason) => Err(reason),
        }
    } else {
        match witnessed::<tables::CountryFactories>(frame, u128::from(id)) {
            Witness::Row(row) => factory_channel_value(row, measure).ok_or_else(|| format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::Absent => Err(format!("A member's value ({tag}) is not recorded at this hour")),
            Witness::NotRecorded(reason) => Err(reason),
        }
    }
}

fn ever_factions_valid_through(context: &GroupsAt, hour: i32) -> i32 {
    let Some(reply) = context.reply() else { return hour };
    let mut until = i32::MAX;
    let mut edge = |start: i32| {
        if start > hour {
            until = until.min(start - 1);
        }
    };
    for span in &reply.unobserved.factions {
        edge(span.first_hour);
    }
    for faction in &reply.factions {
        edge(faction.first_hour);
        for run in &faction.members {
            edge(run.first_hour);
        }
        for span in &faction.incomplete {
            edge(span.first_hour);
        }
    }
    until.max(hour)
}

impl AtHour for FactionTotalsAt {
    const KEY: &'static str = "faction-totals-at";
    const TITLE: &'static str = "Faction totals";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::Boundaries;
    type Params = FactionTotalsAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, kind, measure, top, members, group, membership, "members[]"];
    type Reply = FactionTotalsAtReply;
    type Context = GroupsAt;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactionTotalsAtParams::decl(), FactionTotalRow::decl(), FactionTotalsAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")?;
        measure_of(&params.kind, &params.measure)?;
        if params.top == Some(0) {
            return Err("top must be at least 1");
        }
        if params.group.is_some() && params.members.is_some() {
            return Err("Choose a group or explicit members, not both");
        }
        if params.top.is_some() && (params.group.is_some() || params.members.is_some()) {
            return Err("top applies to factions, not an aggregated group");
        }
        if params.membership.is_some() && (params.group.is_some() || params.members.is_some()) {
            return Err("membership applies to factions; a group carries its own");
        }
        if let Some(members) = &params.members {
            for country in members {
                validate_country(country)?;
            }
        }
        Ok(())
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let target = if params.kind == "territory" { TableId::TerritoryTotals } else { TableId::CountryFactories };
        if params.members.is_some() || params.group.is_some() || params.membership == Some(GroupMembership::Ever) {
            vec![target]
        } else {
            vec![TableId::GroupSpans, TableId::Groups, target]
        }
    }

    fn retained_reads(params: &Self::Params, _owner: Option<Owner>, stage: usize, _held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        if stage > 0 {
            return (Vec::new(), i32::MAX);
        }
        let target = if params.kind == "territory" { TableId::TerritoryTotals } else { TableId::CountryFactories };
        let key_of = |slot: u32, id: u32| if params.kind == "territory" { u128::from(slot) } else { u128::from(id) };
        let valid = at.context.valid_through(params.group.as_ref(), at.hour);
        if let Some(group) = &params.group {
            let Ok(members) = at.context.members(group, at.hour) else {
                return (Vec::new(), valid);
            };
            if members.refused.is_some() || members.tags.is_empty() {
                return (Vec::new(), valid);
            }
            let keys: Vec<u128> = members
                .tags
                .iter()
                .filter_map(|tag| match at.holders.at(&Country::Tag(tag.clone()), at.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => Some(key_of(slot, id)),
                    _ => None,
                })
                .collect();
            return (vec![Read::these(target, keys)], valid);
        }
        if let Some(members) = &params.members {
            let keys: Vec<u128> = members
                .iter()
                .filter_map(|country| match at.holders.at(country, at.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => Some(key_of(slot, id)),
                    _ => None,
                })
                .collect();
            return (vec![Read::these(target, keys)], i32::MAX);
        }
        if params.membership == Some(GroupMembership::Ever) {
            let valid = ever_factions_valid_through(at.context, at.hour);
            let Some(reply) = at.context.reply() else {
                return (Vec::new(), valid);
            };
            if reply.unobserved.factions.iter().any(|span| span.first_hour <= at.hour) {
                return (Vec::new(), i32::MAX);
            }
            let mut keys: Vec<u128> = Vec::new();
            for faction in &reply.factions {
                if faction.first_hour > at.hour || faction.incomplete.iter().any(|span| span.first_hour <= at.hour) {
                    continue;
                }
                for run in &faction.members {
                    if run.first_hour > at.hour {
                        continue;
                    }
                    if let Some(Holder::Found { slot, id }) = at.holders.at(&Country::Tag(run.tag.clone()), at.hour).map(|r| r.holder) {
                        keys.push(key_of(slot, id));
                    }
                }
            }
            keys.sort_unstable();
            keys.dedup();
            return (vec![Read::these(target, keys)], valid);
        }
        (vec![Read::all(TableId::GroupSpans), Read::all(TableId::Groups), Read::all(target)], i32::MAX)
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        if params.membership == Some(GroupMembership::Ever) {
            return ever_factions_valid_through(at.context, at.hour);
        }
        at.context.valid_through(params.group.as_ref(), at.hour)
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(GroupsAt::default())
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        if params.membership == Some(GroupMembership::Ever) {
            return GroupsAt::load_all(source, load, &params.branch_id);
        }
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let empty = |missing: String| FactionTotalsAtReply {
            hour: frame.hour,
            kind: params.kind.clone(),
            measure: params.measure.clone(),
            rows: Vec::new(),
            missing: Some(missing),
            warnings: Vec::new(),
        };
        let aggregate_members: Option<Vec<Country>> = if let Some(group) = &params.group {
            let members = at.context.members(group, frame.hour)?;
            if let Some(refused) = members.refused {
                return Ok(empty(refused));
            }
            if members.tags.is_empty() {
                return Ok(FactionTotalsAtReply {
                    hour: frame.hour,
                    kind: params.kind.clone(),
                    measure: params.measure.clone(),
                    rows: Vec::new(),
                    missing: None,
                    warnings: Vec::new(),
                });
            }
            Some(members.tags.into_iter().map(Country::Tag).collect())
        } else {
            params.members.clone()
        };

        if let Some(members) = aggregate_members {
            let mut sum = 0i64;
            let mut missing: Option<String> = None;
            let mut tags = Vec::with_capacity(members.len());
            let mut member_countries = Vec::with_capacity(members.len());
            for country in &members {
                let tag = at.holders.name(country, frame.hour);
                tags.push(tag.clone());
                match at.holders.at(country, frame.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, id }) => {
                        member_countries.push(Country::Slot(slot));
                        match witnessed_value(frame, &params.kind, slot, id, &params.measure, &tag) {
                            Ok(value) => match sum.checked_add(value) {
                                Some(next) => sum = next,
                                None if missing.is_none() => missing = Some("Group total overflowed at this hour".to_owned()),
                                None => {}
                            },
                            Err(reason) if missing.is_none() => missing = Some(reason),
                            Err(_) => {}
                        }
                    }
                    _ => {
                        member_countries.push(country.clone());
                        if missing.is_none() {
                            missing = Some(format!("A member ({tag}) is not present at this hour"));
                        }
                    }
                }
            }
            tags.sort_unstable();
            tags.dedup();
            member_countries.sort_unstable();
            member_countries.dedup();
            let row = FactionTotalRow {
                id: GROUP_LINE.to_owned(),
                members: tags,
                member_countries,
                total: missing.is_none().then_some(sum),
                missing,
            };
            return Ok(FactionTotalsAtReply {
                hour: frame.hour,
                kind: params.kind.clone(),
                measure: params.measure.clone(),
                rows: vec![row],
                missing: None,
                warnings: Vec::new(),
            });
        }

        if params.membership == Some(GroupMembership::Ever) {
            let Some(reply) = at.context.reply() else {
                return Ok(empty("Faction memberships were not loaded".to_owned()));
            };
            if reply.unobserved.factions.iter().any(|span| span.first_hour <= frame.hour) {
                return Ok(empty("Faction memberships were not read whole through this hour".to_owned()));
            }
            let mut rows_out: Vec<FactionTotalRow> = Vec::new();
            for faction in &reply.factions {
                if faction.first_hour > frame.hour {
                    continue;
                }
                let mut missing = faction
                    .incomplete
                    .iter()
                    .any(|span| span.first_hour <= frame.hour)
                    .then(|| "Its roster was not read whole through this hour".to_owned());
                let mut seen: BTreeSet<&str> = BTreeSet::new();
                for run in &faction.members {
                    if run.first_hour <= frame.hour {
                        seen.insert(run.tag.as_str());
                    }
                }
                if seen.is_empty() {
                    continue;
                }
                let mut sum = 0i64;
                let mut member_tags: Vec<String> = Vec::with_capacity(seen.len());
                let mut member_countries: Vec<Country> = Vec::with_capacity(seen.len());
                for tag in seen {
                    member_tags.push(tag.to_owned());
                    let country = Country::Tag(tag.to_owned());
                    match at.holders.at(&country, frame.hour).map(|run| run.holder) {
                        Some(Holder::Found { slot, id }) => {
                            member_countries.push(Country::Slot(slot));
                            if missing.is_none() {
                                match witnessed_value(frame, &params.kind, slot, id, &params.measure, tag) {
                                    Ok(value) => match sum.checked_add(value) {
                                        Some(next) => sum = next,
                                        None => missing = Some("Faction total overflowed at this hour".to_owned()),
                                    },
                                    Err(reason) => missing = Some(reason),
                                }
                            }
                        }
                        _ => {
                            member_countries.push(country);
                            if missing.is_none() {
                                missing = Some(format!("A member ({tag}) is not present at this hour"));
                            }
                        }
                    }
                }
                member_countries.sort_unstable();
                member_countries.dedup();
                rows_out.push(FactionTotalRow {
                    id: faction.id.clone(),
                    members: member_tags,
                    member_countries,
                    total: missing.is_none().then_some(sum),
                    missing,
                });
            }
            rows_out.sort_unstable_by(|a, b| match (a.total, b.total) {
                (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.id.cmp(&b.id)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.id.cmp(&b.id),
            });
            if let Some(limit) = params.top {
                let limit = usize::try_from(limit).unwrap_or(usize::MAX);
                if rows_out.iter().any(|row| row.missing.is_some()) {
                    return Ok(empty(format!(
                        "The top {limit} factions cannot be proved: a faction's total is not known at this hour"
                    )));
                }
                rows_out.truncate(limit);
            }
            return Ok(FactionTotalsAtReply {
                hour: frame.hour,
                kind: params.kind.clone(),
                measure: params.measure.clone(),
                rows: rows_out,
                missing: None,
                warnings: Vec::new(),
            });
        }

        let table = match MeasureTable::read(frame, &params.kind) {
            Ok(table) => table,
            Err(reason) => return Ok(empty(reason)),
        };
        let rosters = match rosters(frame) {
            Ok(rosters) if rosters.unread => return Ok(empty("Faction memberships were not read at this hour".to_owned())),
            Ok(rosters) => rosters,
            Err(reason) => return Ok(empty(format!("Faction memberships are not known at this hour: {reason}"))),
        };
        let tags = slot_tags(at.holders, frame.hour);
        let mut rows_out: Vec<FactionTotalRow> = rosters
            .members
            .iter()
            .filter(|(_, slots)| !slots.is_empty())
            .map(|(faction_id, slots)| {
                let mut missing = rosters.incomplete.contains(faction_id).then(|| "Its roster was not read whole at this hour".to_owned());
                let mut sum = 0i64;
                let mut member_tags = Vec::with_capacity(slots.len());
                let mut member_countries = Vec::with_capacity(slots.len());
                for &slot in slots {
                    member_countries.push(Country::Slot(slot));
                    match tags.get(&slot) {
                        Some(&(tag, country_id)) => {
                            member_tags.push(tag.to_owned());
                            if missing.is_none() {
                                match table.value(slot, country_id, &params.measure) {
                                    Some(value) => match sum.checked_add(value) {
                                        Some(next) => sum = next,
                                        None => missing = Some("Faction total overflowed at this hour".to_owned()),
                                    },
                                    None => missing = Some(format!("A member's value ({tag}) is not recorded at this hour")),
                                }
                            }
                        }
                        None if missing.is_none() => {
                            missing = Some("A member's country is not known at this hour".to_owned());
                        }
                        None => {}
                    }
                }
                member_tags.sort_unstable();
                member_countries.sort_unstable();
                member_countries.dedup();
                FactionTotalRow {
                    id: faction_id.to_string(),
                    members: member_tags,
                    member_countries,
                    total: missing.is_none().then_some(sum),
                    missing,
                }
            })
            .collect();
        rows_out.sort_unstable_by(|a, b| match (a.total, b.total) {
            (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.id.cmp(&b.id)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });
        if let Some(limit) = params.top {
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            if rows_out.iter().any(|row| row.missing.is_some()) {
                return Ok(empty(format!(
                    "The top {limit} factions cannot be proved: a faction's total is not known at this hour"
                )));
            }
            rows_out.truncate(limit);
        }
        Ok(FactionTotalsAtReply {
            hour: frame.hour,
            kind: params.kind.clone(),
            measure: params.measure.clone(),
            rows: rows_out,
            missing: None,
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_cut_to_the_view_and_merged_where_they_meet() {
        assert_eq!(merged(vec![(5, 9), (-3, 2), (3, 4), (20, 30)], 0, 25), vec![(0, 9), (20, 25)]);
        let gaps = [MissingSpan { first_hour: 0, last_hour: 10, reason: "why".into() }];
        let cut = within(&gaps, &[(3, 4), (8, 12)]);
        assert_eq!(cut.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect::<Vec<_>>(), [(3, 4), (8, 10)]);
    }
}
